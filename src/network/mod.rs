mod onion;
mod discovery;
mod protocol;

use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    core::transport::Transport,
    noise,
    yamux,
    PeerId,
    Swarm,
    TransportBuilder,
    SwarmEvent,
    ConnectionId,
    ConnectionEvent,
    ConnectionHandlerEvent,
    ConnectionHandlerUpgrErr,
    StreamProtocol,
};
use tokio::sync::mpsc;
use std::time::Duration;
use crate::crypto::CryptoService;
use std::collections::HashMap;
use protocol::{ProtocolHandler, ProtocolMessage};

pub use onion::{OnionRouter, OnionNode, OnionCircuit};
pub use discovery::NodeDiscovery;
pub use protocol::ProtocolMessage;

const PROTOCOL_NAME: &str = "/hermes/1.0.0";
const MAX_CONNECTIONS: usize = 50;
const CIRCUIT_EXTENSION_TIMEOUT: Duration = Duration::from_secs(30);
const MESSAGE_CLEANUP_INTERVAL: Duration = Duration::from_secs(10);
const MIN_HOPS: usize = 3;
const MAX_HOPS: usize = 5;

#[derive(Debug)]
pub struct NetworkService {
    peer_id: PeerId,
    swarm: Swarm<()>,
    message_sender: mpsc::Sender<NetworkMessage>,
    message_receiver: mpsc::Receiver<NetworkMessage>,
    onion_router: OnionRouter,
    node_discovery: NodeDiscovery,
    protocol_handler: ProtocolHandler,
    active_connections: HashMap<PeerId, ConnectionId>,
    pending_circuit_extensions: HashMap<String, CircuitExtensionState>,
    message_cleanup_task: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Debug)]
struct CircuitExtensionState {
    circuit_id: String,
    peer_id: PeerId,
    timestamp: std::time::Instant,
    nodes: Vec<OnionNode>,
}

#[derive(Debug)]
pub enum NetworkMessage {
    Connect(PeerId),
    SendMessage(PeerId, Vec<u8>),
    Disconnect(PeerId),
    OnionMessage(onion::OnionMessage),
    NodeDiscovery(discovery::NodeInfo),
    ConnectionEstablished(PeerId),
    ConnectionClosed(PeerId),
    MessageReceived(PeerId, Vec<u8>),
    CircuitExtended(String),
    CircuitExtensionFailed(String, String),
    Error(String),
    MessageExpired(String),
}

impl NetworkService {
    pub async fn new(crypto_service: CryptoService) -> Result<Self> {
        // Generate key pair for noise protocol
        let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
            .into_authentic(&noise::Keypair::<noise::X25519Spec>::new())
            .expect("Signing libp2p-noise static DH keypair failed.");

        // Create transport with noise encryption and yamux multiplexing
        let transport = TransportBuilder::new()
            .with_tcp()
            .with_tls(noise_keys)
            .with_yamux()
            .build()?;

        // Create swarm for managing peer connections
        let peer_id = PeerId::random();
        let swarm = Swarm::new(transport, peer_id);

        // Create message channels
        let (message_sender, message_receiver) = mpsc::channel(100);

        // Initialize onion router and node discovery
        let onion_router = OnionRouter::new(crypto_service.clone());
        let node_discovery = NodeDiscovery::new(crypto_service.clone());
        let protocol_handler = ProtocolHandler::new(crypto_service);
        
        Ok(Self {
            peer_id,
            swarm,
            message_sender,
            message_receiver,
            onion_router,
            node_discovery,
            protocol_handler,
            active_connections: HashMap::new(),
            pending_circuit_extensions: HashMap::new(),
            message_cleanup_task: None,
        })
    }

    pub async fn start(&mut self) -> Result<()> {
        // Start message cleanup task
        let mut interval = tokio::time::interval(MESSAGE_CLEANUP_INTERVAL);
        let protocol_handler = &mut self.protocol_handler;
        
        self.message_cleanup_task = Some(tokio::spawn(async move {
            loop {
                interval.tick().await;
                protocol_handler.cleanup_expired_messages().await;
            }
        }));

        // Start processing network events
        while let Some(event) = self.swarm.next().await {
            // Handle network events
            self.handle_network_event(event).await?;
        }

        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        // Cancel message cleanup task
        if let Some(task) = self.message_cleanup_task.take() {
            task.abort();
        }

        // Clean up all connections
        for (peer_id, _) in self.active_connections.drain() {
            self.disconnect_from_peer(peer_id).await?;
        }

        Ok(())
    }

    async fn handle_network_event(&mut self, event: SwarmEvent<()>) -> Result<()> {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                log::info!("Listening on {}", address);
            }
            SwarmEvent::ConnectionEstablished { peer_id, connection_id, .. } => {
                log::info!("Connection established with peer {}", peer_id);
                self.active_connections.insert(peer_id, connection_id);
                self.message_sender.send(NetworkMessage::ConnectionEstablished(peer_id)).await?;
            }
            SwarmEvent::ConnectionClosed { peer_id, connection_id, .. } => {
                log::info!("Connection closed with peer {}", peer_id);
                self.active_connections.remove(&peer_id);
                self.message_sender.send(NetworkMessage::ConnectionClosed(peer_id)).await?;
            }
            SwarmEvent::Behaviour(event) => {
                self.handle_connection_event(event).await?;
            }
            SwarmEvent::IncomingConnection { connection_id, .. } => {
                if self.active_connections.len() >= MAX_CONNECTIONS {
                    self.swarm.reject_connection(connection_id);
                } else {
                    self.swarm.accept_connection(connection_id);
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn handle_connection_event(&mut self, event: ConnectionEvent) -> Result<()> {
        match event {
            ConnectionEvent::Upgraded { peer_id, .. } => {
                // Handle successful connection upgrade
                log::info!("Connection upgraded with peer {}", peer_id);
            }
            ConnectionEvent::Failed { peer_id, error, .. } => {
                // Handle connection failure
                log::error!("Connection failed with peer {}: {:?}", peer_id, error);
                self.message_sender.send(NetworkMessage::Error(format!("Connection failed: {:?}", error))).await?;
            }
            ConnectionEvent::Handler(event) => {
                self.handle_handler_event(event).await?;
            }
        }
        Ok(())
    }

    async fn handle_handler_event(&mut self, event: ConnectionHandlerEvent<()>) -> Result<()> {
        match event {
            ConnectionHandlerEvent::Custom(_) => {
                // Handle custom protocol events
            }
            ConnectionHandlerEvent::Close(err) => {
                // Handle connection close
                if let ConnectionHandlerUpgrErr::Timeout = err {
                    log::warn!("Connection handler timeout");
                }
            }
        }
        Ok(())
    }

    pub async fn extend_circuit(&mut self, circuit_id: String, peer_id: PeerId) -> Result<()> {
        // Get available nodes for circuit extension
        let nodes = self.node_discovery.get_available_nodes(2).await?;
        
        // Create circuit extension state
        let state = CircuitExtensionState {
            circuit_id: circuit_id.clone(),
            peer_id,
            timestamp: std::time::Instant::now(),
            nodes: nodes.clone(),
        };
        
        // Add to pending extensions
        self.pending_circuit_extensions.insert(circuit_id.clone(), state);
        
        // Send circuit extension request
        let extension_request = ProtocolMessage::Control {
            command: protocol::ControlCommand::CircuitExtend,
            data: serde_json::to_vec(&nodes)?,
        };
        
        self.protocol_handler.send_message(peer_id.to_base58(), extension_request).await?;
        
        Ok(())
    }

    async fn handle_circuit_extension(&mut self, peer_id: PeerId, data: Vec<u8>) -> Result<()> {
        // Parse extension data
        let nodes: Vec<OnionNode> = serde_json::from_slice(&data)?;
        
        // Create new circuit with extended nodes
        let circuit_id = self.onion_router.create_circuit(nodes).await?;
        
        // Send extension confirmation
        let confirmation = ProtocolMessage::Control {
            command: protocol::ControlCommand::CircuitExtended,
            data: circuit_id.as_bytes().to_vec(),
        };
        
        self.protocol_handler.send_message(peer_id.to_base58(), confirmation).await?;
        
        Ok(())
    }

    async fn handle_circuit_extension_confirmation(&mut self, peer_id: PeerId, data: Vec<u8>) -> Result<()> {
        let circuit_id = String::from_utf8_lossy(&data).to_string();
        
        if let Some(state) = self.pending_circuit_extensions.remove(&circuit_id) {
            if state.timestamp.elapsed() > CIRCUIT_EXTENSION_TIMEOUT {
                self.message_sender.send(NetworkMessage::CircuitExtensionFailed(
                    circuit_id,
                    "Extension timeout".to_string(),
                )).await?;
                return Ok(());
            }
            
            self.message_sender.send(NetworkMessage::CircuitExtended(circuit_id)).await?;
        }
        
        Ok(())
    }

    pub fn get_peer_id(&self) -> PeerId {
        self.peer_id
    }

    pub async fn connect_to_peer(&mut self, peer_id: PeerId) -> Result<()> {
        if self.active_connections.contains_key(&peer_id) {
            return Err(anyhow::anyhow!("Already connected to peer"));
        }

        if self.active_connections.len() >= MAX_CONNECTIONS {
            return Err(anyhow::anyhow!("Maximum connections reached"));
        }

        self.swarm.dial(peer_id)?;
        Ok(())
    }

    pub async fn send_message(&mut self, peer_id: PeerId, message: Vec<u8>) -> Result<()> {
        // Get random number of hops between MIN_HOPS and MAX_HOPS
        let num_hops = rand::random::<usize>() % (MAX_HOPS - MIN_HOPS + 1) + MIN_HOPS;
        
        // Get available nodes for onion routing
        let nodes = self.node_discovery.get_available_nodes(num_hops).await?;
        
        // Create onion circuit
        let circuit_id = self.onion_router.create_circuit(nodes).await?;

        // Send message through onion circuit
        self.onion_router.send_message(&circuit_id, &message).await?;

        // Clean up circuit after sending
        self.onion_router.destroy_circuit(&circuit_id).await?;

        Ok(())
    }

    pub async fn disconnect_from_peer(&mut self, peer_id: PeerId) -> Result<()> {
        if let Some(connection_id) = self.active_connections.get(&peer_id) {
            self.swarm.disconnect_peer_id(peer_id, *connection_id);
        }
        Ok(())
    }

    pub async fn register_as_relay(&mut self) -> Result<()> {
        // Create node info for this peer
        let node = OnionNode {
            id: self.peer_id.to_base58(),
            public_key: self.onion_router.get_public_key().to_vec(),
            address: "TODO: Get actual address".to_string(),
        };

        // Add self to node discovery
        self.node_discovery.add_node(node).await?;

        Ok(())
    }

    pub async fn cleanup_stale_nodes(&mut self) -> Result<()> {
        self.node_discovery.cleanup_stale_nodes().await
    }

    pub async fn cleanup_pending_extensions(&mut self) {
        let now = std::time::Instant::now();
        let mut failed_extensions = Vec::new();
        
        for (circuit_id, state) in &self.pending_circuit_extensions {
            if state.timestamp.elapsed() > CIRCUIT_EXTENSION_TIMEOUT {
                failed_extensions.push(circuit_id.clone());
            }
        }
        
        for circuit_id in failed_extensions {
            if let Some(state) = self.pending_circuit_extensions.remove(&circuit_id) {
                self.message_sender.send(NetworkMessage::CircuitExtensionFailed(
                    circuit_id,
                    "Extension timeout".to_string(),
                )).await.ok();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoService;

    #[tokio::test]
    async fn test_network_service_creation() {
        let crypto_service = CryptoService::new().unwrap();
        let service = NetworkService::new(crypto_service).await.unwrap();
        assert!(service.peer_id.to_base58().len() > 0);
    }

    #[tokio::test]
    async fn test_connection_management() {
        let crypto_service = CryptoService::new().unwrap();
        let mut service = NetworkService::new(crypto_service).await.unwrap();
        
        // Test connection limit
        for _ in 0..MAX_CONNECTIONS + 1 {
            let peer_id = PeerId::random();
            if service.connect_to_peer(peer_id).await.is_ok() {
                assert!(service.active_connections.len() <= MAX_CONNECTIONS);
            }
        }
    }

    #[tokio::test]
    async fn test_circuit_extension() {
        let crypto_service = CryptoService::new().unwrap();
        let mut service = NetworkService::new(crypto_service).await.unwrap();
        
        let peer_id = PeerId::random();
        let circuit_id = "test_circuit".to_string();
        
        // Test circuit extension
        assert!(service.extend_circuit(circuit_id.clone(), peer_id).await.is_ok());
        assert!(service.pending_circuit_extensions.contains_key(&circuit_id));
        
        // Test extension timeout
        tokio::time::sleep(CIRCUIT_EXTENSION_TIMEOUT).await;
        service.cleanup_pending_extensions().await;
        assert!(!service.pending_circuit_extensions.contains_key(&circuit_id));
    }

    #[tokio::test]
    async fn test_message_cleanup() {
        let crypto_service = CryptoService::new().unwrap();
        let mut service = NetworkService::new(crypto_service).await.unwrap();
        
        // Start service
        let mut service_handle = tokio::spawn(async move {
            service.start().await.unwrap();
        });
        
        // Wait for cleanup task to run
        tokio::time::sleep(MESSAGE_CLEANUP_INTERVAL).await;
        
        // Stop service
        service_handle.abort();
    }
} 
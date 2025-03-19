mod onion;
mod discovery;
mod protocol;
pub mod crypto;

use anyhow::Result;
use futures::StreamExt;
use libp2p::{
    core::transport::Transport,
    noise,
    yamux,
    PeerId,
    swarm::{
        Swarm,
        ConnectionId,
        dummy::Behaviour,
    },
    identity,
};
#[cfg(feature = "logging")]
use log::{info, warn, error};
use tokio::sync::mpsc;
use std::time::Duration;
use crate::crypto::CryptoService;
use std::collections::HashMap;
use protocol::{ProtocolHandler, Message, MessageType, ControlMessage};

pub use onion::{OnionRouter, OnionNode, OnionCircuit};
pub use discovery::NodeDiscovery;
pub use protocol::{Message as ProtocolMessage, MessageType as ProtocolMessageType, ControlMessage as ProtocolControlMessage};

const MAX_CONNECTIONS: usize = 50;
const CIRCUIT_EXTENSION_TIMEOUT: Duration = Duration::from_secs(30);
const MESSAGE_CLEANUP_INTERVAL: Duration = Duration::from_secs(10);
const MIN_HOPS: usize = 1;
const MAX_HOPS: usize = 5;

// Create a non-derived debug implementation
pub struct NetworkService {
    peer_id: PeerId,
    swarm: Swarm<Behaviour>,
    message_sender: mpsc::Sender<NetworkMessage>,
    message_receiver: mpsc::Receiver<NetworkMessage>,
    onion_router: OnionRouter,
    node_discovery: NodeDiscovery,
    protocol_handler: ProtocolHandler,
    active_connections: HashMap<PeerId, ConnectionId>,
    pending_circuit_extensions: HashMap<String, CircuitExtensionState>,
    message_cleanup_task: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for NetworkService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkService")
            .field("peer_id", &self.peer_id)
            .field("active_connections", &self.active_connections.len())
            .field("pending_extensions", &self.pending_circuit_extensions.len())
            .finish()
    }
}

#[derive(Debug)]
struct CircuitExtensionState {
    _circuit_id: String,
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
        let id_keys = identity::Keypair::generate_ed25519();
        let peer_id = PeerId::from(id_keys.public());
        
        // Create a basic Noise config with the generated keys
        let transport = libp2p::tcp::tokio::Transport::default()
            .upgrade(libp2p::core::upgrade::Version::V1)
            .authenticate(noise::Config::new(&id_keys).unwrap())
            .multiplex(yamux::Config::default())
            .boxed();
        
        // Create message channels
        let (message_sender, message_receiver) = mpsc::channel(100);
        
        // Create network components
        let onion_router = OnionRouter::new(crypto_service.clone());
        let node_discovery = NodeDiscovery::new(crypto_service.clone());
        let protocol_handler = ProtocolHandler::new();
        
        // Create swarm with dummy behavior
        let behaviour = Behaviour{};
        let config = libp2p::swarm::Config::with_tokio_executor();
        let swarm = Swarm::new(transport, behaviour, peer_id, config);
                
        Ok(NetworkService {
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
        // Spawn a separate task for the message cleanup
        let mut interval = tokio::time::interval(MESSAGE_CLEANUP_INTERVAL);
        let handler_clone = self.protocol_handler.clone();
        
        self.message_cleanup_task = Some(tokio::spawn(async move {
            loop {
                interval.tick().await;
                if let Err(e) = handler_clone.cleanup_expired_messages().await {
                    #[cfg(feature = "logging")]
                    error!("Failed to clean up expired messages: {:?}", e);
                }
            }
        }));

        // Process events from swarm and timer
        let mut timer = tokio::time::interval(Duration::from_secs(1));
        let mut onion_timer = tokio::time::interval(Duration::from_millis(100));
        
        loop {
            tokio::select! {
                event = self.swarm.select_next_some() => {
                    if let Err(e) = self.handle_swarm_event(event).await {
                        #[cfg(feature = "logging")]
                        error!("Error handling network event: {:?}", e);
                    }
                },
                _ = timer.tick() => {
                    // Run periodic cleanup
                    self.cleanup_pending_extensions().await;
                    if let Err(e) = self.cleanup_stale_nodes().await {
                        #[cfg(feature = "logging")]
                        error!("Error cleaning up stale nodes: {:?}", e);
                    }
                },
                _ = onion_timer.tick() => {
                    // Process onion router messages
                    if let Err(e) = self.onion_router.process_incoming_messages().await {
                        #[cfg(feature = "logging")]
                        error!("Error processing onion router messages: {:?}", e);
                    }
                },
                msg = self.message_receiver.recv() => {
                    match msg {
                        Some(network_msg) => {
                            if let Err(e) = self.handle_network_message(network_msg).await {
                                #[cfg(feature = "logging")]
                                error!("Error handling network message: {:?}", e);
                            }
                        },
                        None => break, // Channel closed, exit
                    }
                },
            }
        }

        Ok(())
    }

    async fn handle_network_message(&mut self, msg: NetworkMessage) -> Result<()> {
        match msg {
            NetworkMessage::Connect(peer_id) => {
                self.connect_to_peer(peer_id).await?;
            },
            NetworkMessage::Disconnect(peer_id) => {
                self.disconnect_from_peer(peer_id).await?;
            },
            NetworkMessage::SendMessage(peer_id, data) => {
                self.send_message(peer_id, data).await?;
            },
            NetworkMessage::OnionMessage(onion_msg) => {
                self.onion_router.handle_message(onion_msg).await?;
            },
            NetworkMessage::MessageReceived(peer_id, data) => {
                // Check if this is a circuit extension or confirmation message
                if let Ok(protocol_msg) = serde_json::from_slice::<protocol::Message>(&data) {
                    match protocol_msg.message_type {
                        protocol::MessageType::Control(protocol::ControlMessage::CircuitExtend) => {
                            self.handle_circuit_extension(peer_id, protocol_msg.content).await?;
                        },
                        protocol::MessageType::Control(protocol::ControlMessage::CircuitExtended) => {
                            self.handle_circuit_extension_confirmation(peer_id, protocol_msg.content).await?;
                        },
                        _ => {
                            #[cfg(feature = "logging")]
                            info!("Received message from peer {}: {:?}", peer_id, protocol_msg.message_type);
                        }
                    }
                }
            },
            _ => {}, // Handle other message types as needed
        }
        
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        // Cancel message cleanup task
        if let Some(task) = self.message_cleanup_task.take() {
            task.abort();
        }

        // Create a copy of the peer IDs to avoid borrowing issues
        let peer_ids: Vec<PeerId> = self.active_connections.keys().cloned().collect();
        
        // Clean up all connections
        for peer_id in peer_ids {
            if let Err(e) = self.disconnect_from_peer(peer_id).await {
                #[cfg(feature = "logging")]
                warn!("Error disconnecting from peer {}: {:?}", peer_id, e);
            }
        }

        Ok(())
    }

    async fn handle_swarm_event(&mut self, event: libp2p::swarm::SwarmEvent<void::Void, void::Void>) -> Result<()> {
        match event {
            libp2p::swarm::SwarmEvent::NewListenAddr { address, .. } => {
                #[cfg(feature = "logging")]
                info!("Listening on {}", address);
                
                // Re-register as relay with our updated listener info
                if let Err(e) = self.register_as_relay().await {
                    #[cfg(feature = "logging")]
                    error!("Failed to update relay registration: {:?}", e);
                }
            }
            libp2p::swarm::SwarmEvent::ConnectionEstablished { peer_id, connection_id, .. } => {
                #[cfg(feature = "logging")]
                info!("Connection established with peer {}", peer_id);
                self.active_connections.insert(peer_id, connection_id);
                self.message_sender.send(NetworkMessage::ConnectionEstablished(peer_id)).await?;
                
                // Try to add this peer to discovery if we have connection to it
                let node = OnionNode {
                    id: peer_id.to_base58(),
                    public_key: vec![0; 32], // We don't know the public key yet, dummy value
                    address: format!("peer:{}", peer_id),
                };
                if let Err(e) = self.node_discovery.add_node(node).await {
                    #[cfg(feature = "logging")]
                    error!("Failed to add peer to discovery: {:?}", e);
                }
            }
            libp2p::swarm::SwarmEvent::ConnectionClosed { peer_id, connection_id, .. } => {
                #[cfg(feature = "logging")]
                info!("Connection closed with peer {}", peer_id);
                if let Some(conn_id) = self.active_connections.get(&peer_id) {
                    if *conn_id == connection_id {
                        self.active_connections.remove(&peer_id);
                    }
                }
                self.message_sender.send(NetworkMessage::ConnectionClosed(peer_id)).await?;
            }
            libp2p::swarm::SwarmEvent::IncomingConnection { connection_id, .. } => {
                // Handle connection limit - simplified for now
                if self.active_connections.len() >= MAX_CONNECTIONS {
                    #[cfg(feature = "logging")]
                    warn!("Connection limit reached, rejecting connection");
                    let _ = self.swarm.close_connection(connection_id);
                }
            },
            _ => {}
        }
        Ok(())
    }

    pub async fn extend_circuit(&mut self, circuit_id: String, peer_id: PeerId) -> Result<()> {
        // Get available nodes for circuit extension
        let nodes = self.node_discovery.get_available_nodes(2).await?;
        
        // Create circuit extension state
        let state = CircuitExtensionState {
            _circuit_id: circuit_id.clone(),
            peer_id,
            timestamp: std::time::Instant::now(),
            nodes: nodes.clone(),
        };
        
        // Add to pending extensions
        self.pending_circuit_extensions.insert(circuit_id.clone(), state);
        
        // Create extension message
        let extension_message = Message {
            id: uuid::Uuid::new_v4().to_string(),
            sender: self.peer_id.to_base58(),
            recipient: peer_id.to_base58(),
            content: serde_json::to_vec(&nodes)?,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            message_type: MessageType::Control(ControlMessage::CircuitExtend),
        };
        
        // Encode and send the extension message
        let _encoded_message = self.protocol_handler.encode_message(&extension_message)?;
        #[cfg(feature = "logging")]
        info!("Sending circuit extension request to peer {}", peer_id);
        
        // Send circuit extension request
        self.protocol_handler.send_message(peer_id.to_base58(), extension_message).await?;
        
        Ok(())
    }

    async fn handle_circuit_extension(&mut self, peer_id: PeerId, data: Vec<u8>) -> Result<()> {
        // Parse extension data
        let nodes: Vec<OnionNode> = serde_json::from_slice(&data)?;
        
        // Create new circuit with extended nodes
        let circuit_id = self.onion_router.create_circuit(nodes).await?;
        
        // Create confirmation message
        let confirmation = Message {
            id: uuid::Uuid::new_v4().to_string(),
            sender: self.peer_id.to_base58(),
            recipient: peer_id.to_base58(),
            content: circuit_id.as_bytes().to_vec(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            message_type: MessageType::Control(ControlMessage::CircuitExtended),
        };
        
        // Send extension confirmation
        self.protocol_handler.send_message(peer_id.to_base58(), confirmation).await?;
        
        Ok(())
    }

    async fn handle_circuit_extension_confirmation(&mut self, _peer_id: PeerId, data: Vec<u8>) -> Result<()> {
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

    pub fn swarm_mut(&mut self) -> &mut Swarm<Behaviour> {
        &mut self.swarm
    }

    pub async fn connect_to_peer(&mut self, peer_id: PeerId) -> Result<()> {
        if self.active_connections.contains_key(&peer_id) {
            return Err(anyhow::anyhow!("Already connected to peer"));
        }

        if self.active_connections.len() >= MAX_CONNECTIONS {
            return Err(anyhow::anyhow!("Maximum connections reached"));
        }

        // In a real implementation, we would use multiaddrs instead of just PeerIDs
        self.swarm.dial(peer_id)?;
        Ok(())
    }

    pub async fn send_message(&mut self, _peer_id: PeerId, message: Vec<u8>) -> Result<()> {
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
        if self.active_connections.contains_key(&peer_id) {
            // The latest libp2p API only needs the PeerId, not the ConnectionId
            let _ = self.swarm.disconnect_peer_id(peer_id);
            self.active_connections.remove(&peer_id);
        }
        Ok(())
    }

    pub async fn register_as_relay(&mut self) -> Result<()> {
        // Get the public IP address from the swarm
        let addresses: Vec<String> = self.swarm.listeners()
            .map(|addr| addr.to_string())
            .collect();
            
        let address = if !addresses.is_empty() {
            addresses.join(",")
        } else {
            "0.0.0.0:0".to_string()
        };
        
        // Create node info for this peer
        let node = OnionNode {
            id: self.peer_id.to_base58(),
            public_key: self.onion_router.get_public_key(),
            address,
        };

        // Add self to node discovery
        self.node_discovery.add_node(node).await?;

        Ok(())
    }

    pub async fn cleanup_stale_nodes(&mut self) -> Result<()> {
        self.node_discovery.cleanup_stale_nodes().await
    }

    pub async fn cleanup_pending_extensions(&mut self) {
        let _now = std::time::Instant::now();
        let mut failed_extensions = Vec::new();
        
        for (circuit_id, state) in &self.pending_circuit_extensions {
            if state.timestamp.elapsed() > CIRCUIT_EXTENSION_TIMEOUT {
                failed_extensions.push((circuit_id.clone(), state.peer_id, state.nodes.len()));
            }
        }
        
        for (circuit_id, peer_id, node_count) in failed_extensions {
            if let Some(_state) = self.pending_circuit_extensions.remove(&circuit_id) {
                #[cfg(feature = "logging")]
                error!("Circuit extension for {} with peer {} and {} nodes failed due to timeout", 
                    circuit_id, peer_id, node_count);
                    
                if let Err(e) = self.message_sender.send(NetworkMessage::CircuitExtensionFailed(
                    circuit_id,
                    "Extension timeout".to_string(),
                )).await {
                    #[cfg(feature = "logging")]
                    error!("Failed to send circuit extension failure message: {:?}", e);
                }
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
    async fn test_connection_management() -> Result<()> {
        let crypto_service = CryptoService::new().unwrap();
        let mut service = NetworkService::new(crypto_service).await?;
        
        // Test connection limit
        let random_peer = PeerId::random();
        
        // This should fail since we can't actually connect to a random peer
        let result = service.connect_to_peer(random_peer).await;
        assert!(result.is_err(), "Expected connection to random peer to fail");
        
        // But the functionality itself should execute without panicking
        assert!(service.active_connections.len() <= MAX_CONNECTIONS);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_circuit_extension() -> Result<()> {
        let crypto_service = CryptoService::new().unwrap();
        let mut service = NetworkService::new(crypto_service).await?;
        
        let peer_id = PeerId::random();
        let circuit_id = "test_circuit".to_string();
        
        // Test circuit extension
        let result = service.extend_circuit(circuit_id.clone(), peer_id).await;
        assert!(result.is_err(), "Circuit extension should fail without enough nodes");
        
        Ok(())
    }
} 
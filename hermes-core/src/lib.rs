mod onion;
mod discovery;
mod protocol;
pub mod crypto;

use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use crate::crypto::CryptoService;
use crate::protocol::{ProtocolHandler, Message, MessageType};

pub use discovery::NodeDiscovery;
pub use protocol::{Message as ProtocolMessage, MessageType as ProtocolMessageType};

#[cfg(feature = "logging")]
use log::{info, error, debug};

// Simple network service using sockets
pub struct NetworkService {
    crypto_service: CryptoService,
    protocol_handler: ProtocolHandler,
    connections: HashMap<String, mpsc::Sender<Vec<u8>>>,
    message_sender: mpsc::Sender<NetworkMessage>,
    message_receiver: mpsc::Receiver<NetworkMessage>,
    node_id: String,
}

#[derive(Debug)]
pub enum NetworkMessage {
    Connect(String, SocketAddr),
    SendMessage(String, Vec<u8>),
    Disconnect(String),
    ConnectionEstablished(String, mpsc::Sender<Vec<u8>>),
    ConnectionClosed(String),
    MessageReceived(String, Vec<u8>),
    Error(String),
}

impl NetworkService {
    pub async fn new(crypto_service: CryptoService) -> Result<Self> {
        let (message_sender, message_receiver) = mpsc::channel(100);
        let protocol_handler = ProtocolHandler::new();
        let connections = HashMap::new();
        
        // Generate a unique node ID
        let node_id = uuid::Uuid::new_v4().to_string();
        
        Ok(NetworkService {
            crypto_service,
            protocol_handler,
            connections,
            message_sender,
            message_receiver,
            node_id,
        })
    }
    
    pub async fn start(&mut self, listen_addr: SocketAddr) -> Result<()> {
        // Create TCP listener to accept incoming connections
        let listener = TcpListener::bind(listen_addr).await?;
        
        #[cfg(feature = "logging")]
        info!("Listening on {}", listen_addr);
        
        let msg_sender = self.message_sender.clone();
        
        // Spawn the listener task
        tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((socket, addr)) => {
                        #[cfg(feature = "logging")]
                        info!("New connection from {}", addr);
                        
                        // Generate connection ID
                        let conn_id = uuid::Uuid::new_v4().to_string();
                        
                        // Create a channel for sending data to this connection
                        let (tx, rx) = mpsc::channel::<Vec<u8>>(32);
                        
                        // Notify about new connection
                        if let Err(e) = msg_sender.send(NetworkMessage::ConnectionEstablished(conn_id.clone(), tx.clone())).await {
                            #[cfg(feature = "logging")]
                            error!("Failed to send connection notification: {}", e);
                        }
                        
                        // Spawn a task to handle this connection
                        let conn_id_clone = conn_id.clone();
                        let sender = msg_sender.clone();
                        
                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(socket, conn_id_clone, rx, sender).await {
                                #[cfg(feature = "logging")]
                                error!("Connection handling error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        #[cfg(feature = "logging")]
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }
        });
        
        // Process network messages
        while let Some(message) = self.message_receiver.recv().await {
            if let Err(e) = self.handle_network_message(message).await {
                #[cfg(feature = "logging")]
                error!("Failed to handle network message: {}", e);
            }
        }
        
        Ok(())
    }
    
    async fn handle_network_message(&mut self, message: NetworkMessage) -> Result<()> {
        match message {
            NetworkMessage::Connect(peer_id, addr) => {
                self.connect_to_peer(peer_id, addr).await?;
            }
            NetworkMessage::SendMessage(peer_id, data) => {
                self.send_message(peer_id, data).await?;
            }
            NetworkMessage::Disconnect(peer_id) => {
                self.disconnect_from_peer(peer_id).await?;
            }
            NetworkMessage::ConnectionEstablished(peer_id, sender) => {
                // Store the connection sender
                self.connections.insert(peer_id, sender);
            }
            NetworkMessage::ConnectionClosed(peer_id) => {
                // Remove the connection
                self.connections.remove(&peer_id);
            }
            NetworkMessage::MessageReceived(peer_id, data) => {
                #[cfg(feature = "logging")]
                debug!("Received message from {}", peer_id);
                
                // Try to decode as protocol message
                if let Ok(msg) = serde_json::from_slice::<Message>(&data) {
                    #[cfg(feature = "logging")]
                    info!("Decoded protocol message: {:?}", msg.message_type);
                }
            }
            _ => {} // Handle other cases
        }
        
        Ok(())
    }
    
    pub async fn connect_to_peer(&mut self, peer_id: String, addr: SocketAddr) -> Result<()> {
        // Check if already connected
        if self.connections.contains_key(&peer_id) {
            return Ok(()); // Already connected
        }
        
        // Attempt to connect
        match TcpStream::connect(addr).await {
            Ok(socket) => {
                // Create a channel for sending data to this connection
                let (tx, rx) = mpsc::channel::<Vec<u8>>(32);
                
                // Store the connection sender
                self.connections.insert(peer_id.clone(), tx.clone());
                
                #[cfg(feature = "logging")]
                info!("Connected to peer {} at {}", peer_id, addr);
                
                // Spawn a task to handle this connection
                let sender = self.message_sender.clone();
                let peer_id_clone = peer_id.clone();
                
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(socket, peer_id_clone, rx, sender).await {
                        #[cfg(feature = "logging")]
                        error!("Connection handling error: {}", e);
                    }
                });
                
                Ok(())
            }
            Err(e) => {
                #[cfg(feature = "logging")]
                error!("Failed to connect to {}: {}", addr, e);
                
                Err(anyhow::anyhow!("Failed to connect to peer"))
            }
        }
    }
    
    pub async fn send_message(&mut self, peer_id: String, data: Vec<u8>) -> Result<()> {
        // Check if we already have a connection to this peer
        if !self.connections.contains_key(&peer_id) {
            // No connection yet, try to find the peer's address in our discovery cache
            let addr_str = format!("{}:9000", peer_id.split('-').next().unwrap_or("0"));
            
            // Try to parse as a socket address (this is just a fallback)
            if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                // Try to connect
                if let Err(e) = self.connect_to_peer(peer_id.clone(), addr).await {
                    return Err(anyhow::anyhow!("Failed to auto-connect to peer: {}", e));
                }
                
                // Wait for connection to establish
                tokio::time::sleep(Duration::from_millis(500)).await;
            } else {
                return Err(anyhow::anyhow!("No connection to peer and couldn't auto-connect"));
            }
        }
        
        // Create protocol message
        let protocol_message = Message {
            id: uuid::Uuid::new_v4().to_string(),
            sender: self.node_id.clone(),
            recipient: peer_id.clone(),
            content: data,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            message_type: MessageType::Text,
        };
        
        // Encode the message
        let encoded = serde_json::to_vec(&protocol_message)?;
        
        // Get the connection sender
        if let Some(sender) = self.connections.get(&peer_id) {
            // Add message length as prefix
            let len = encoded.len() as u32;
            let len_bytes = len.to_be_bytes();
            
            // Create complete message
            let mut complete_msg = Vec::with_capacity(len_bytes.len() + encoded.len());
            complete_msg.extend_from_slice(&len_bytes);
            complete_msg.extend_from_slice(&encoded);
            
            // Send through the channel
            sender.send(complete_msg).await
                .map_err(|_| anyhow::anyhow!("Failed to send message to connection handler"))?;
            
            #[cfg(feature = "logging")]
            info!("Sent message to peer {}", peer_id);
            
            Ok(())
        } else {
            Err(anyhow::anyhow!("No connection to peer"))
        }
    }
    
    pub async fn disconnect_from_peer(&mut self, peer_id: String) -> Result<()> {
        // Just remove the connection sender, the task will eventually complete
        if self.connections.remove(&peer_id).is_some() {
            #[cfg(feature = "logging")]
            info!("Disconnected from peer {}", peer_id);
        }
        
        Ok(())
    }
    
    pub fn get_node_id(&self) -> String {
        self.node_id.clone()
    }
    
    // Simplified implementation that doesn't use onion routing
    pub async fn add_peer_to_discovery(&self, _peer_id: String, _address: String) -> Result<()> {
        Ok(())
    }
    
    pub async fn setup_tor_transport(&self) -> Result<()> {
        #[cfg(feature = "logging")]
        info!("Tor functionality not implemented yet");
        Ok(())
    }
    
    pub async fn add_relay(&self, _addr: SocketAddr) -> Result<()> {
        #[cfg(feature = "logging")]
        info!("Relay functionality not implemented yet");
        Ok(())
    }
    
    pub async fn register_as_relay(&self) -> Result<()> {
        #[cfg(feature = "logging")]
        info!("Relay functionality not implemented yet");
        Ok(())
    }
    
    pub async fn setup_as_relay(&self) -> Result<()> {
        #[cfg(feature = "logging")]
        info!("Relay functionality not implemented yet");
        Ok(())
    }
}

// Helper function to handle a single connection
async fn handle_connection(
    mut socket: TcpStream, 
    conn_id: String,
    mut command_rx: mpsc::Receiver<Vec<u8>>,
    sender: mpsc::Sender<NetworkMessage>,
) -> Result<()> {
    let mut buf = vec![0; 4]; // Buffer for message length
    
    loop {
        tokio::select! {
            // Handle incoming data from the socket
            read_result = socket.read_exact(&mut buf) => {
                match read_result {
                    Ok(_) => {
                        let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                        
                        // Read the message
                        let mut data = vec![0; len];
                        match socket.read_exact(&mut data).await {
                            Ok(_) => {
                                // Forward the message
                                if let Err(e) = sender.send(NetworkMessage::MessageReceived(conn_id.clone(), data)).await {
                                    #[cfg(feature = "logging")]
                                    error!("Failed to forward message: {}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                #[cfg(feature = "logging")]
                                error!("Failed to read message data: {}", e);
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        if e.kind() == std::io::ErrorKind::UnexpectedEof {
                            #[cfg(feature = "logging")]
                            info!("Connection closed by peer");
                        } else {
                            #[cfg(feature = "logging")]
                            error!("Failed to read message length: {}", e);
                        }
                        break;
                    }
                }
            }
            
            // Handle outgoing data
            Some(data) = command_rx.recv() => {
                if let Err(e) = socket.write_all(&data).await {
                    #[cfg(feature = "logging")]
                    error!("Failed to write to socket: {}", e);
                    break;
                }
            }
            
            // Exit if the sender channel closes
            else => {
                break;
            }
        }
    }
    
    // Connection closed
    sender.send(NetworkMessage::ConnectionClosed(conn_id)).await?;
    
    Ok(())
}

// Simple stub implementations for compatibility
pub struct OnionRouter;
pub struct OnionNode;
pub struct OnionCircuit; 
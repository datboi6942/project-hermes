use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::mpsc;
use crate::crypto::CryptoService;

const MAX_HOPS: usize = 3;
const MIN_HOPS: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnionNode {
    pub id: String,
    pub public_key: Vec<u8>,
    pub address: String,
}

#[derive(Debug, Clone)]
pub struct OnionCircuit {
    pub id: String,
    pub nodes: Vec<OnionNode>,
    pub session_key: Vec<u8>,
}

#[derive(Debug)]
pub struct OnionRouter {
    crypto_service: CryptoService,
    circuits: HashMap<String, OnionCircuit>,
    node_sender: mpsc::Sender<OnionMessage>,
    node_receiver: mpsc::Receiver<OnionMessage>,
}

#[derive(Debug)]
pub enum OnionMessage {
    CreateCircuit(Vec<OnionNode>),
    RelayMessage(String, Vec<u8>), // circuit_id, encrypted_data
    DestroyCircuit(String),
}

impl OnionRouter {
    pub fn new(crypto_service: CryptoService) -> Self {
        let (node_sender, node_receiver) = mpsc::channel(100);
        
        Self {
            crypto_service,
            circuits: HashMap::new(),
            node_sender,
            node_receiver,
        }
    }

    pub async fn create_circuit(&mut self, nodes: Vec<OnionNode>) -> Result<String> {
        if nodes.len() < MIN_HOPS || nodes.len() > MAX_HOPS {
            return Err(anyhow::anyhow!("Invalid number of hops"));
        }

        // Generate session key for this circuit
        let session_key = self.crypto_service.generate_session_key()?;
        
        let circuit_id = uuid::Uuid::new_v4().to_string();
        let circuit = OnionCircuit {
            id: circuit_id.clone(),
            nodes: nodes.clone(),
            session_key: session_key.clone(),
        };

        // Store circuit
        self.circuits.insert(circuit_id.clone(), circuit);

        // Send circuit creation message to first hop
        let _create_msg = self.prepare_circuit_creation_message(&self.circuits.get(&circuit_id).unwrap())?;
        self.node_sender.send(OnionMessage::CreateCircuit(nodes)).await?;

        Ok(circuit_id)
    }

    pub async fn send_message(&mut self, circuit_id: &str, message: &[u8]) -> Result<()> {
        let circuit = self.circuits.get(circuit_id)
            .ok_or_else(|| anyhow::anyhow!("Circuit not found"))?;

        // Encrypt message with session key
        let encrypted_data = self.crypto_service.encrypt_message(message, &circuit.session_key)?;

        // Send encrypted message through circuit
        self.node_sender.send(OnionMessage::RelayMessage(
            circuit_id.to_string(),
            encrypted_data,
        )).await?;

        Ok(())
    }

    pub async fn destroy_circuit(&mut self, circuit_id: &str) -> Result<()> {
        if let Some(_circuit) = self.circuits.remove(circuit_id) {
            // Send circuit destruction message
            self.node_sender.send(OnionMessage::DestroyCircuit(circuit_id.to_string())).await?;
        }
        Ok(())
    }

    fn prepare_circuit_creation_message(&self, _circuit: &OnionCircuit) -> Result<Vec<u8>> {
        // Placeholder implementation that returns empty data for now
        Ok(Vec::new())
    }

    pub async fn handle_message(&mut self, message: OnionMessage) -> Result<()> {
        match message {
            OnionMessage::CreateCircuit(nodes) => {
                // Handle circuit creation
                self.handle_circuit_creation(nodes).await?;
            }
            OnionMessage::RelayMessage(circuit_id, data) => {
                // Handle message relay
                self.handle_message_relay(&circuit_id, &data).await?;
            }
            OnionMessage::DestroyCircuit(circuit_id) => {
                // Handle circuit destruction
                self.handle_circuit_destruction(&circuit_id).await?;
            }
        }
        Ok(())
    }

    async fn handle_circuit_creation(&mut self, _nodes: Vec<OnionNode>) -> Result<()> {
        // Placeholder implementation
        Ok(())
    }

    async fn handle_message_relay(&mut self, _circuit_id: &str, _data: &[u8]) -> Result<()> {
        // Placeholder implementation
        Ok(())
    }

    async fn handle_circuit_destruction(&mut self, _circuit_id: &str) -> Result<()> {
        // Placeholder implementation
        Ok(())
    }
    
    pub fn get_public_key(&self) -> Vec<u8> {
        self.crypto_service.get_public_key()
    }

    // Process any incoming messages from other nodes
    pub async fn process_incoming_messages(&mut self) -> Result<()> {
        if let Ok(Some(message)) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            self.node_receiver.recv()
        ).await {
            self.handle_message(message).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoService;

    #[tokio::test]
    async fn test_circuit_creation() {
        let crypto_service = CryptoService::new().unwrap();
        let mut router = OnionRouter::new(crypto_service);
        
        let nodes = vec![
            OnionNode {
                id: "node1".to_string(),
                public_key: vec![0u8; 32],
                address: "127.0.0.1:8080".to_string(),
            },
            OnionNode {
                id: "node2".to_string(),
                public_key: vec![0u8; 32],
                address: "127.0.0.1:8081".to_string(),
            },
        ];

        let circuit_id = router.create_circuit(nodes).await.unwrap();
        assert!(!circuit_id.is_empty());
    }
} 
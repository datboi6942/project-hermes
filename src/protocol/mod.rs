use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub sender: String,
    pub recipient: String,
    pub content: Vec<u8>,
    pub timestamp: u64,
    pub message_type: MessageType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageType {
    Text,
    File,
    Control(ControlMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMessage {
    Handshake,
    KeyExchange,
    Disconnect,
}

impl Message {
    pub fn new(sender: String, recipient: String, content: Vec<u8>, message_type: MessageType) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            sender,
            recipient,
            content,
            timestamp,
            message_type,
        }
    }
}

pub struct ProtocolHandler {
    // TODO: Add protocol state management
}

impl ProtocolHandler {
    pub fn new() -> Self {
        Self {}
    }

    pub fn encode_message(&self, message: &Message) -> Result<Vec<u8>> {
        // TODO: Implement message encoding with proper framing
        unimplemented!()
    }

    pub fn decode_message(&self, data: &[u8]) -> Result<Message> {
        // TODO: Implement message decoding
        unimplemented!()
    }

    pub fn create_handshake_message(&self) -> Message {
        Message::new(
            "system".to_string(),
            "peer".to_string(),
            Vec::new(),
            MessageType::Control(ControlMessage::Handshake),
        )
    }

    pub fn create_key_exchange_message(&self, public_key: &[u8]) -> Message {
        Message::new(
            "system".to_string(),
            "peer".to_string(),
            public_key.to_vec(),
            MessageType::Control(ControlMessage::KeyExchange),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_creation() {
        let message = Message::new(
            "sender".to_string(),
            "recipient".to_string(),
            b"Hello, World!".to_vec(),
            MessageType::Text,
        );

        assert_eq!(message.sender, "sender");
        assert_eq!(message.recipient, "recipient");
        assert_eq!(message.content, b"Hello, World!");
        assert!(message.timestamp > 0);
    }
} 
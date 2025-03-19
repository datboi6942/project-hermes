use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use crate::crypto::CryptoService;
use super::onion::OnionNode;

const NODE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes
const MAX_NODES: usize = 100;
const MIN_NODES: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node: OnionNode,
    pub last_seen: u64,
    pub bandwidth: u64,
    pub uptime: u64,
}

#[derive(Debug)]
pub struct NodeDiscovery {
    nodes: RwLock<HashMap<String, NodeInfo>>,
    crypto_service: CryptoService,
}

impl NodeDiscovery {
    pub fn new(crypto_service: CryptoService) -> Self {
        Self {
            nodes: RwLock::new(HashMap::new()),
            crypto_service,
        }
    }

    pub async fn add_node(&self, node: OnionNode) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        
        if nodes.len() >= MAX_NODES {
            // Remove oldest node if we're at capacity
            let oldest_key = nodes.iter()
                .min_by_key(|(_, info)| info.last_seen)
                .map(|(key, _)| key.clone());
            
            if let Some(key) = oldest_key {
                nodes.remove(&key);
            }
        }

        let now = Instant::now().elapsed().as_secs();
        let info = NodeInfo {
            node,
            last_seen: now,
            bandwidth: 0, // Will be updated with actual measurements
            uptime: 0,    // Will be updated with actual uptime
        };

        nodes.insert(info.node.id.clone(), info);
        Ok(())
    }

    pub async fn remove_node(&self, node_id: &str) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        nodes.remove(node_id);
        Ok(())
    }

    pub async fn update_node_info(&self, node_id: &str, bandwidth: u64, uptime: u64) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        
        if let Some(info) = nodes.get_mut(node_id) {
            info.last_seen = Instant::now().elapsed().as_secs();
            info.bandwidth = bandwidth;
            info.uptime = uptime;
        }
        
        Ok(())
    }

    pub async fn get_available_nodes(&self, count: usize) -> Result<Vec<OnionNode>> {
        let nodes = self.nodes.read().await;
        
        // Filter out stale nodes and sort by bandwidth
        let mut available_nodes: Vec<_> = nodes.values()
            .filter(|info| {
                let age = Instant::now().elapsed().as_secs() - info.last_seen;
                age < NODE_TIMEOUT.as_secs()
            })
            .collect();

        // Sort by bandwidth (descending) and uptime (descending)
        available_nodes.sort_by(|a, b| {
            b.bandwidth.cmp(&a.bandwidth)
                .then(b.uptime.cmp(&a.uptime))
        });

        // Take requested number of nodes
        let selected_nodes: Vec<_> = available_nodes
            .iter()
            .take(count)
            .map(|info| info.node.clone())
            .collect();

        if selected_nodes.len() < MIN_NODES {
            return Err(anyhow::anyhow!("Not enough available nodes"));
        }

        Ok(selected_nodes)
    }

    pub async fn get_node_count(&self) -> usize {
        self.nodes.read().await.len()
    }

    pub async fn cleanup_stale_nodes(&self) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        let now = Instant::now().elapsed().as_secs();
        
        nodes.retain(|_, info| {
            now - info.last_seen < NODE_TIMEOUT.as_secs()
        });
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoService;

    #[tokio::test]
    async fn test_node_discovery() {
        let crypto_service = CryptoService::new().unwrap();
        let discovery = NodeDiscovery::new(crypto_service);
        
        // Add test nodes
        let node1 = OnionNode {
            id: "node1".to_string(),
            public_key: vec![0u8; 32],
            address: "127.0.0.1:8080".to_string(),
        };
        
        let node2 = OnionNode {
            id: "node2".to_string(),
            public_key: vec![0u8; 32],
            address: "127.0.0.1:8081".to_string(),
        };
        
        discovery.add_node(node1.clone()).await.unwrap();
        discovery.add_node(node2.clone()).await.unwrap();
        
        // Test getting available nodes
        let nodes = discovery.get_available_nodes(2).await.unwrap();
        assert_eq!(nodes.len(), 2);
        
        // Test node removal
        discovery.remove_node(&node1.id).await.unwrap();
        assert_eq!(discovery.get_node_count().await, 1);
    }
} 
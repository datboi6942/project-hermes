use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use crate::crypto::CryptoService;
use super::onion::OnionNode;

const NODE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes
const MAX_NODES: usize = 100;
const MIN_NODES: usize = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeInfo {
    pub node: OnionNode,
    pub last_seen: u64,
    pub bandwidth: u64,
    pub uptime: u64,
}

pub struct NodeDiscovery {
    nodes: RwLock<HashMap<String, NodeInfo>>,
    crypto_service: CryptoService,
}

impl std::fmt::Debug for NodeDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeDiscovery")
         .field("nodes_count", &"<RwLock>")
         .finish()
    }
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
        // Create a node verification token using the crypto service
        let _node_id_bytes = node.id.as_bytes();
        let _verification_key = self.crypto_service.generate_session_key()?;
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

    pub async fn get_node(&self, node_id: &str) -> Result<Option<NodeInfo>> {
        let nodes = self.nodes.read().await;
        Ok(nodes.get(node_id).cloned())
    }

    pub async fn get_all_nodes(&self) -> Result<Vec<NodeInfo>> {
        let nodes = self.nodes.read().await;
        Ok(nodes.values().cloned().collect())
    }

    pub async fn get_available_nodes(&self, count: usize) -> Result<Vec<OnionNode>> {
        let nodes = self.nodes.read().await;
        
        if nodes.len() < MIN_NODES {
            return Err(anyhow::anyhow!("Not enough nodes available"));
        }
        
        // Get the most recently seen nodes
        let mut available_nodes: Vec<NodeInfo> = nodes.values().cloned().collect();
        available_nodes.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
        
        let actual_count = std::cmp::min(count, available_nodes.len());
        let selected_nodes = available_nodes
            .into_iter()
            .take(actual_count)
            .map(|info| info.node)
            .collect();
        
        Ok(selected_nodes)
    }

    pub async fn update_node_status(&self, node_id: &str, bandwidth: u64, uptime: u64) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        
        if let Some(info) = nodes.get_mut(node_id) {
            info.last_seen = Instant::now().elapsed().as_secs();
            info.bandwidth = bandwidth;
            info.uptime = uptime;
        }
        
        Ok(())
    }

    pub async fn cleanup_stale_nodes(&self) -> Result<()> {
        let mut nodes = self.nodes.write().await;
        let now = Instant::now().elapsed().as_secs();
        
        let stale_nodes: Vec<String> = nodes
            .iter()
            .filter(|(_, info)| now.saturating_sub(info.last_seen) > NODE_TIMEOUT.as_secs())
            .map(|(id, _)| id.clone())
            .collect();
        
        for node_id in stale_nodes {
            nodes.remove(&node_id);
        }
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::CryptoService;

    #[tokio::test]
    async fn test_node_discovery() -> Result<()> {
        let crypto_service = CryptoService::new()?;
        let discovery = NodeDiscovery::new(crypto_service);
        
        // Add test nodes
        for i in 0..5 {
            let node = OnionNode {
                id: format!("node{}", i),
                public_key: vec![0u8; 32],
                address: format!("127.0.0.1:{}", 8000 + i),
            };
            discovery.add_node(node).await?;
        }
        
        // Test get_available_nodes
        let nodes = discovery.get_available_nodes(3).await?;
        assert_eq!(nodes.len(), 3);
        
        // Test cleanup_stale_nodes
        discovery.cleanup_stale_nodes().await?;
        
        // All nodes should still be there as they're not stale yet
        let all_nodes = discovery.get_all_nodes().await?;
        assert_eq!(all_nodes.len(), 5);
        
        Ok(())
    }
} 
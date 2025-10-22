use super::inventory::{InventoryManager, InvId};
use super::peer::{Peer, PeerState};
use anyhow::Result;
use bitcoin::{Block, BlockHash, Network, Transaction, Txid};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use parking_lot::RwLock;

/// P2P manager coordinating all peer connections
pub struct P2PManager {
    /// Network
    network: Network,

    /// Connected peers
    peers: Arc<RwLock<HashMap<SocketAddr, Peer>>>,

    /// Inventory manager
    inventory: Arc<RwLock<InventoryManager>>,

    /// User agent string
    user_agent: String,

    /// Current block height
    block_height: Arc<RwLock<i32>>,

    /// Maximum peers
    max_peers: usize,
}

impl P2PManager {
    pub fn new(network: Network, user_agent: String) -> Self {
        Self {
            network,
            peers: Arc::new(RwLock::new(HashMap::new())),
            inventory: Arc::new(RwLock::new(InventoryManager::new())),
            user_agent,
            block_height: Arc::new(RwLock::new(0)),
            max_peers: 125,
        }
    }

    /// Add a peer connection
    pub async fn add_peer(&self, addr: SocketAddr) -> Result<()> {
        let peers = self.peers.read();
        if peers.len() >= self.max_peers {
            return Ok(());
        }
        drop(peers);

        let peer = Peer::connect(addr, self.network).await?;
        self.peers.write().insert(addr, peer);

        Ok(())
    }

    /// Remove a peer
    pub fn remove_peer(&self, addr: &SocketAddr) {
        self.peers.write().remove(addr);
    }

    /// Get peer count
    pub fn peer_count(&self) -> usize {
        self.peers.read().len()
    }

    /// Announce transaction to all peers
    pub fn announce_tx(&self, txid: Txid) {
        let inv_id = InvId::Tx(txid);
        // Broadcast to all connected peers would go here
        // This is a simplified version
    }

    /// Announce block to all peers
    pub fn announce_block(&self, block_hash: BlockHash) {
        let inv_id = InvId::Block(block_hash);
        // Broadcast to all connected peers would go here
    }

    /// Request a transaction
    pub fn request_tx(&self, txid: Txid) {
        self.inventory.write().want(InvId::Tx(txid));
    }

    /// Request a block
    pub fn request_block(&self, block_hash: BlockHash) {
        self.inventory.write().want(InvId::Block(block_hash));
    }

    /// Mark transaction as received
    pub fn mark_tx_received(&self, txid: Txid) {
        self.inventory.write().mark_received(&InvId::Tx(txid));
    }

    /// Mark block as received
    pub fn mark_block_received(&self, block_hash: BlockHash) {
        self.inventory
            .write()
            .mark_received(&InvId::Block(block_hash));
    }

    /// Update block height
    pub fn update_block_height(&self, height: i32) {
        *self.block_height.write() = height;
    }

    /// Get current block height
    pub fn get_block_height(&self) -> i32 {
        *self.block_height.read()
    }

    /// Get inventory stats
    pub fn get_inventory_stats(&self) -> (usize, usize) {
        let inv = self.inventory.read();
        (inv.wanted_count(), inv.in_flight_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_p2p_manager_creation() {
        let mgr = P2PManager::new(Network::Bitcoin, "/test:0.1.0/".to_string());
        assert_eq!(mgr.peer_count(), 0);
        assert_eq!(mgr.get_block_height(), 0);
    }

    #[test]
    fn test_block_height_update() {
        let mgr = P2PManager::new(Network::Bitcoin, "/test:0.1.0/".to_string());
        mgr.update_block_height(100);
        assert_eq!(mgr.get_block_height(), 100);
    }
}

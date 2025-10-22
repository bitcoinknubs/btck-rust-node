use bitcoin::{BlockHash, Txid};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

/// Inventory item type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvType {
    /// Transaction
    Tx,
    /// Block
    Block,
    /// Filtered block (BIP 37)
    FilteredBlock,
    /// Compact block (BIP 152)
    CompactBlock,
    /// Witness transaction
    WitnessTx,
    /// Witness block
    WitnessBlock,
}

impl InvType {
    pub fn to_u32(&self) -> u32 {
        match self {
            InvType::Tx => 1,
            InvType::Block => 2,
            InvType::FilteredBlock => 3,
            InvType::CompactBlock => 4,
            InvType::WitnessTx => 0x40000001,
            InvType::WitnessBlock => 0x40000002,
        }
    }

    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            1 => Some(InvType::Tx),
            2 => Some(InvType::Block),
            3 => Some(InvType::FilteredBlock),
            4 => Some(InvType::CompactBlock),
            0x40000001 => Some(InvType::WitnessTx),
            0x40000002 => Some(InvType::WitnessBlock),
            _ => None,
        }
    }
}

/// Inventory item identifier
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InvId {
    Tx(Txid),
    Block(BlockHash),
}

impl InvId {
    pub fn as_tx(&self) -> Option<Txid> {
        match self {
            InvId::Tx(txid) => Some(*txid),
            _ => None,
        }
    }

    pub fn as_block(&self) -> Option<BlockHash> {
        match self {
            InvId::Block(hash) => Some(*hash),
            _ => None,
        }
    }
}

/// Request state for an inventory item
#[derive(Debug, Clone)]
struct RequestState {
    /// Who we requested from
    peer: SocketAddr,
    /// When we requested
    requested_at: SystemTime,
    /// Number of times requested
    attempts: u32,
}

/// Inventory manager
pub struct InventoryManager {
    /// Items we want to download
    wanted: HashSet<InvId>,

    /// Items currently being downloaded
    in_flight: HashMap<InvId, RequestState>,

    /// Items we have downloaded
    have: HashSet<InvId>,

    /// Items announced by peers
    announced: HashMap<InvId, HashSet<SocketAddr>>,

    /// Request timeout
    request_timeout: Duration,

    /// Maximum requests per peer
    max_per_peer: usize,

    /// Maximum total in-flight
    max_in_flight: usize,
}

impl InventoryManager {
    pub fn new() -> Self {
        Self {
            wanted: HashSet::new(),
            in_flight: HashMap::new(),
            have: HashSet::new(),
            announced: HashMap::new(),
            request_timeout: Duration::from_secs(120),
            max_per_peer: 16,
            max_in_flight: 256,
        }
    }

    /// Mark item as wanted
    pub fn want(&mut self, id: InvId) {
        if !self.have.contains(&id) && !self.in_flight.contains_key(&id) {
            self.wanted.insert(id);
        }
    }

    /// Mark item as received
    pub fn mark_received(&mut self, id: &InvId) {
        self.wanted.remove(id);
        self.in_flight.remove(id);
        self.have.insert(id.clone());
    }

    /// Mark item announced by peer
    pub fn announce(&mut self, id: InvId, peer: SocketAddr) {
        self.announced
            .entry(id.clone())
            .or_insert_with(HashSet::new)
            .insert(peer);

        // If we don't have it, mark as wanted
        if !self.have.contains(&id) {
            self.wanted.insert(id);
        }
    }

    /// Get items to request from a peer
    pub fn get_requests(&mut self, peer: SocketAddr) -> Vec<InvId> {
        let mut requests = Vec::new();

        // Check in-flight capacity
        if self.in_flight.len() >= self.max_in_flight {
            return requests;
        }

        // Count current requests to this peer
        let current_peer_requests = self
            .in_flight
            .values()
            .filter(|state| state.peer == peer)
            .count();

        if current_peer_requests >= self.max_per_peer {
            return requests;
        }

        // Select items to request
        for id in &self.wanted {
            if requests.len() >= self.max_per_peer - current_peer_requests {
                break;
            }

            if self.in_flight.len() + requests.len() >= self.max_in_flight {
                break;
            }

            // Check if peer has announced this item
            if let Some(peers) = self.announced.get(id) {
                if peers.contains(&peer) {
                    requests.push(id.clone());
                }
            }
        }

        // Mark as in-flight
        for id in &requests {
            self.wanted.remove(id);
            self.in_flight.insert(
                id.clone(),
                RequestState {
                    peer,
                    requested_at: SystemTime::now(),
                    attempts: 1,
                },
            );
        }

        requests
    }

    /// Check for timed-out requests and retry
    pub fn check_timeouts(&mut self) -> Vec<InvId> {
        let now = SystemTime::now();
        let mut timed_out = Vec::new();

        for (id, state) in &self.in_flight {
            if let Ok(elapsed) = now.duration_since(state.requested_at) {
                if elapsed > self.request_timeout {
                    timed_out.push((id.clone(), state.attempts));
                }
            }
        }

        // Remove timed out and re-add to wanted
        for (id, attempts) in timed_out {
            self.in_flight.remove(&id);

            // Only retry a few times
            if attempts < 3 {
                self.wanted.insert(id.clone());
            }
        }

        self.wanted.iter().cloned().collect()
    }

    /// Get number of wanted items
    pub fn wanted_count(&self) -> usize {
        self.wanted.len()
    }

    /// Get number of in-flight items
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.len()
    }

    /// Check if we have an item
    pub fn has(&self, id: &InvId) -> bool {
        self.have.contains(id)
    }

    /// Clear old items from have set
    pub fn prune_have(&mut self, keep: usize) {
        if self.have.len() > keep {
            let to_remove = self.have.len() - keep;
            let remove_list: Vec<_> = self.have.iter().take(to_remove).cloned().collect();
            for id in remove_list {
                self.have.remove(&id);
            }
        }
    }
}

impl Default for InventoryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;

    #[test]
    fn test_inv_manager_creation() {
        let mgr = InventoryManager::new();
        assert_eq!(mgr.wanted_count(), 0);
        assert_eq!(mgr.in_flight_count(), 0);
    }

    #[test]
    fn test_want_item() {
        let mut mgr = InventoryManager::new();
        let txid = Txid::all_zeros();
        let id = InvId::Tx(txid);

        mgr.want(id.clone());
        assert_eq!(mgr.wanted_count(), 1);

        mgr.mark_received(&id);
        assert_eq!(mgr.wanted_count(), 0);
        assert!(mgr.has(&id));
    }

    #[test]
    fn test_announce() {
        let mut mgr = InventoryManager::new();
        let txid = Txid::all_zeros();
        let id = InvId::Tx(txid);
        let peer = "1.2.3.4:8333".parse().unwrap();

        mgr.announce(id.clone(), peer);
        assert_eq!(mgr.wanted_count(), 1);

        let requests = mgr.get_requests(peer);
        assert_eq!(requests.len(), 1);
        assert_eq!(mgr.in_flight_count(), 1);
    }

    #[test]
    fn test_inv_type_conversion() {
        assert_eq!(InvType::Tx.to_u32(), 1);
        assert_eq!(InvType::Block.to_u32(), 2);
        assert_eq!(InvType::from_u32(1), Some(InvType::Tx));
        assert_eq!(InvType::from_u32(2), Some(InvType::Block));
    }
}

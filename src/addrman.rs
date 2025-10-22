use bitcoin::Network;
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use parking_lot::RwLock;

/// Maximum number of addresses to store
const MAX_ADDRESSES: usize = 20000;

/// Number of buckets for new addresses
const NEW_BUCKETS_COUNT: usize = 1024;

/// Number of buckets for tried addresses
const TRIED_BUCKETS_COUNT: usize = 256;

/// Bucket size
const BUCKET_SIZE: usize = 64;

/// Address information
#[derive(Debug, Clone)]
pub struct AddressInfo {
    /// Socket address
    pub addr: SocketAddr,

    /// Services offered by this peer
    pub services: u64,

    /// Last time we successfully connected
    pub last_success: Option<SystemTime>,

    /// Last time we tried to connect
    pub last_try: Option<SystemTime>,

    /// Last time we heard about this address
    pub last_seen: SystemTime,

    /// Number of connection attempts
    pub attempts: u32,

    /// Source address (who told us about this)
    pub source: Option<SocketAddr>,

    /// Random position in bucket
    pub random_pos: usize,
}

impl AddressInfo {
    pub fn new(addr: SocketAddr, services: u64, source: Option<SocketAddr>) -> Self {
        Self {
            addr,
            services,
            last_success: None,
            last_try: None,
            last_seen: SystemTime::now(),
            attempts: 0,
            source,
            random_pos: rand::thread_rng().gen(),
        }
    }

    /// Check if address is good (successfully connected recently)
    pub fn is_good(&self) -> bool {
        if let Some(last_success) = self.last_success {
            let age = SystemTime::now()
                .duration_since(last_success)
                .unwrap_or(Duration::from_secs(0));
            age < Duration::from_secs(3600) && self.attempts < 3
        } else {
            false
        }
    }

    /// Check if address is terrible (many failed attempts)
    pub fn is_terrible(&self) -> bool {
        self.attempts > 10
            || self
                .last_try
                .map(|t| {
                    SystemTime::now()
                        .duration_since(t)
                        .unwrap_or(Duration::from_secs(0))
                        < Duration::from_secs(60)
                })
                .unwrap_or(false)
    }

    /// Get chance of selection (0.0 to 1.0)
    pub fn get_chance(&self) -> f64 {
        let mut chance = 1.0;

        // Reduce chance based on attempts
        if self.attempts > 0 {
            chance /= (1 + self.attempts) as f64;
        }

        // Reduce chance based on last try time
        if let Some(last_try) = self.last_try {
            let since_try = SystemTime::now()
                .duration_since(last_try)
                .unwrap_or(Duration::from_secs(0));

            if since_try < Duration::from_secs(600) {
                chance *= 0.01;
            }
        }

        // Increase chance for recent successes
        if let Some(last_success) = self.last_success {
            let since_success = SystemTime::now()
                .duration_since(last_success)
                .unwrap_or(Duration::from_secs(0));

            if since_success < Duration::from_secs(1200) {
                chance *= 2.0;
            }
        }

        chance.min(1.0).max(0.0)
    }
}

/// Address manager for managing peer addresses
pub struct AddressManager {
    /// Network type
    network: Network,

    /// New addresses (not yet tried)
    new_addrs: RwLock<HashMap<SocketAddr, AddressInfo>>,

    /// Tried addresses (successfully connected)
    tried_addrs: RwLock<HashMap<SocketAddr, AddressInfo>>,

    /// New buckets (hash table for new addresses)
    new_buckets: RwLock<Vec<HashSet<SocketAddr>>>,

    /// Tried buckets (hash table for tried addresses)
    tried_buckets: RwLock<Vec<HashSet<SocketAddr>>>,

    /// Our own addresses (to avoid connecting to ourselves)
    own_addrs: RwLock<HashSet<SocketAddr>>,
}

impl AddressManager {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            new_addrs: RwLock::new(HashMap::new()),
            tried_addrs: RwLock::new(HashMap::new()),
            new_buckets: RwLock::new(vec![HashSet::new(); NEW_BUCKETS_COUNT]),
            tried_buckets: RwLock::new(vec![HashSet::new(); TRIED_BUCKETS_COUNT]),
            own_addrs: RwLock::new(HashSet::new()),
        }
    }

    /// Add a new address
    pub fn add(&self, addr: SocketAddr, services: u64, source: Option<SocketAddr>) -> bool {
        // Skip if it's our own address
        if self.own_addrs.read().contains(&addr) {
            return false;
        }

        // Skip if already in tried
        if self.tried_addrs.read().contains_key(&addr) {
            return false;
        }

        // Check if in new table
        let mut new_addrs = self.new_addrs.write();
        if let Some(info) = new_addrs.get_mut(&addr) {
            // Update existing entry
            info.last_seen = SystemTime::now();
            if services != 0 {
                info.services = services;
            }
            return false;
        }

        // Add new address
        let info = AddressInfo::new(addr, services, source);
        let bucket = self.get_new_bucket(&addr, source.as_ref());

        // Check if bucket is full
        let mut new_buckets = self.new_buckets.write();
        if new_buckets[bucket].len() >= BUCKET_SIZE {
            // Evict random entry
            if let Some(&evict_addr) = new_buckets[bucket].iter().next() {
                new_buckets[bucket].remove(&evict_addr);
                new_addrs.remove(&evict_addr);
            }
        }

        // Check total size limit
        if new_addrs.len() >= MAX_ADDRESSES {
            return false;
        }

        new_buckets[bucket].insert(addr);
        new_addrs.insert(addr, info);

        true
    }

    /// Mark an address as good (successful connection)
    pub fn good(&self, addr: &SocketAddr) {
        let mut new_addrs = self.new_addrs.write();
        let mut tried_addrs = self.tried_addrs.write();

        // Move from new to tried
        if let Some(mut info) = new_addrs.remove(addr) {
            info.last_success = Some(SystemTime::now());
            info.last_try = Some(SystemTime::now());
            info.attempts = 0;

            // Remove from new bucket
            let bucket = self.get_new_bucket(addr, info.source.as_ref());
            self.new_buckets.write()[bucket].remove(addr);

            // Add to tried bucket
            let tried_bucket = self.get_tried_bucket(addr);
            let mut tried_buckets = self.tried_buckets.write();

            if tried_buckets[tried_bucket].len() >= BUCKET_SIZE {
                // Evict random entry
                if let Some(&evict_addr) = tried_buckets[tried_bucket].iter().next() {
                    tried_buckets[tried_bucket].remove(&evict_addr);
                    tried_addrs.remove(&evict_addr);
                }
            }

            tried_buckets[tried_bucket].insert(*addr);
            tried_addrs.insert(*addr, info);
        } else if let Some(info) = tried_addrs.get_mut(addr) {
            // Update existing tried entry
            info.last_success = Some(SystemTime::now());
            info.last_try = Some(SystemTime::now());
            info.attempts = 0;
        }
    }

    /// Mark a connection attempt
    pub fn attempt(&self, addr: &SocketAddr) {
        let mut new_addrs = self.new_addrs.write();
        let mut tried_addrs = self.tried_addrs.write();

        if let Some(info) = new_addrs.get_mut(addr) {
            info.last_try = Some(SystemTime::now());
            info.attempts += 1;
        } else if let Some(info) = tried_addrs.get_mut(addr) {
            info.last_try = Some(SystemTime::now());
            info.attempts += 1;
        }
    }

    /// Select an address to connect to
    pub fn select(&self) -> Option<SocketAddr> {
        let tried_addrs = self.tried_addrs.read();
        let new_addrs = self.new_addrs.read();

        // 50% chance to select from tried, 50% from new
        let use_tried = rand::thread_rng().gen_bool(0.5);

        if use_tried && !tried_addrs.is_empty() {
            self.select_from_map(&tried_addrs)
        } else if !new_addrs.is_empty() {
            self.select_from_map(&new_addrs)
        } else if !tried_addrs.is_empty() {
            self.select_from_map(&tried_addrs)
        } else {
            None
        }
    }

    /// Select multiple addresses
    pub fn select_multiple(&self, count: usize) -> Vec<SocketAddr> {
        let mut result = Vec::new();
        let mut selected = HashSet::new();

        for _ in 0..count * 3 {
            // Try up to 3x to avoid duplicates
            if result.len() >= count {
                break;
            }

            if let Some(addr) = self.select() {
                if !selected.contains(&addr) {
                    selected.insert(addr);
                    result.push(addr);
                }
            }
        }

        result
    }

    /// Get all addresses (for sharing with peers)
    pub fn get_addresses(&self, max_count: usize) -> Vec<(SocketAddr, u64)> {
        let tried_addrs = self.tried_addrs.read();
        let new_addrs = self.new_addrs.read();

        let mut result = Vec::new();

        // Prefer tried addresses
        for (addr, info) in tried_addrs.iter() {
            if result.len() >= max_count {
                break;
            }
            if info.is_good() && !info.is_terrible() {
                result.push((*addr, info.services));
            }
        }

        // Add some new addresses
        for (addr, info) in new_addrs.iter() {
            if result.len() >= max_count {
                break;
            }
            if !info.is_terrible() {
                result.push((*addr, info.services));
            }
        }

        result
    }

    /// Add our own address
    pub fn add_own_address(&self, addr: SocketAddr) {
        self.own_addrs.write().insert(addr);
    }

    /// Get statistics
    pub fn get_stats(&self) -> AddressManagerStats {
        AddressManagerStats {
            new_count: self.new_addrs.read().len(),
            tried_count: self.tried_addrs.read().len(),
            total_count: self.new_addrs.read().len() + self.tried_addrs.read().len(),
        }
    }

    /// Clear all addresses
    pub fn clear(&self) {
        self.new_addrs.write().clear();
        self.tried_addrs.write().clear();
        for bucket in self.new_buckets.write().iter_mut() {
            bucket.clear();
        }
        for bucket in self.tried_buckets.write().iter_mut() {
            bucket.clear();
        }
    }

    // Helper methods

    fn select_from_map(&self, map: &HashMap<SocketAddr, AddressInfo>) -> Option<SocketAddr> {
        let candidates: Vec<_> = map
            .iter()
            .filter(|(_, info)| !info.is_terrible())
            .collect();

        if candidates.is_empty() {
            return None;
        }

        // Weighted random selection based on chance
        let total_weight: f64 = candidates.iter().map(|(_, info)| info.get_chance()).sum();

        if total_weight <= 0.0 {
            // Fallback to uniform random
            let idx = rand::thread_rng().gen_range(0..candidates.len());
            return Some(*candidates[idx].0);
        }

        let mut rng = rand::thread_rng();
        let mut threshold = rng.gen::<f64>() * total_weight;

        for (addr, info) in &candidates {
            threshold -= info.get_chance();
            if threshold <= 0.0 {
                return Some(**addr);
            }
        }

        // Fallback
        candidates.first().map(|(addr, _)| **addr)
    }

    fn get_new_bucket(&self, addr: &SocketAddr, source: Option<&SocketAddr>) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        addr.hash(&mut hasher);
        if let Some(src) = source {
            src.hash(&mut hasher);
        }
        (hasher.finish() as usize) % NEW_BUCKETS_COUNT
    }

    fn get_tried_bucket(&self, addr: &SocketAddr) -> usize {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        addr.hash(&mut hasher);
        (hasher.finish() as usize) % TRIED_BUCKETS_COUNT
    }
}

/// Address manager statistics
#[derive(Debug, Clone)]
pub struct AddressManagerStats {
    pub new_count: usize,
    pub tried_count: usize,
    pub total_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_manager_creation() {
        let addrman = AddressManager::new(Network::Bitcoin);
        let stats = addrman.get_stats();
        assert_eq!(stats.total_count, 0);
    }

    #[test]
    fn test_add_address() {
        let addrman = AddressManager::new(Network::Bitcoin);
        let addr = "1.2.3.4:8333".parse().unwrap();

        assert!(addrman.add(addr, 1, None));
        assert_eq!(addrman.get_stats().new_count, 1);
    }

    #[test]
    fn test_good_moves_to_tried() {
        let addrman = AddressManager::new(Network::Bitcoin);
        let addr = "1.2.3.4:8333".parse().unwrap();

        addrman.add(addr, 1, None);
        addrman.good(&addr);

        let stats = addrman.get_stats();
        assert_eq!(stats.tried_count, 1);
        assert_eq!(stats.new_count, 0);
    }

    #[test]
    fn test_select_address() {
        let addrman = AddressManager::new(Network::Bitcoin);

        for i in 0..10 {
            let addr = format!("1.2.3.{}:8333", i).parse().unwrap();
            addrman.add(addr, 1, None);
        }

        let selected = addrman.select();
        assert!(selected.is_some());
    }

    #[test]
    fn test_own_address_filtered() {
        let addrman = AddressManager::new(Network::Bitcoin);
        let addr = "1.2.3.4:8333".parse().unwrap();

        addrman.add_own_address(addr);
        assert!(!addrman.add(addr, 1, None));
    }
}

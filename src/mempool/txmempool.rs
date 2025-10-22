use super::entry::MempoolEntry;
use super::fees::FeeEstimator;
use super::policy::MempoolPolicy;
use anyhow::{anyhow, Result};
use bitcoin::{Transaction, Txid};
use dashmap::DashMap;
use std::collections::HashSet;
use std::sync::Arc;
use parking_lot::RwLock;

/// Main mempool structure
pub struct Mempool {
    /// All transactions in the mempool
    entries: DashMap<Txid, MempoolEntry>,

    /// Policy configuration
    policy: Arc<MempoolPolicy>,

    /// Fee estimator
    fee_estimator: Arc<RwLock<FeeEstimator>>,

    /// Total size in bytes
    total_size: Arc<RwLock<usize>>,

    /// Total fees
    total_fees: Arc<RwLock<u64>>,

    /// Current block height
    current_height: Arc<RwLock<u32>>,

    /// Map from outpoint to spending transaction
    spends: DashMap<bitcoin::OutPoint, Txid>,
}

impl Mempool {
    pub fn new(policy: MempoolPolicy) -> Self {
        Self {
            entries: DashMap::new(),
            policy: Arc::new(policy),
            fee_estimator: Arc::new(RwLock::new(FeeEstimator::new())),
            total_size: Arc::new(RwLock::new(0)),
            total_fees: Arc::new(RwLock::new(0)),
            current_height: Arc::new(RwLock::new(0)),
            spends: DashMap::new(),
        }
    }

    /// Add a transaction to the mempool
    pub fn add_tx(&self, tx: Transaction, fee: u64, height: u32) -> Result<Txid> {
        let txid = tx.compute_txid();

        // Check if already in mempool
        if self.entries.contains_key(&txid) {
            return Err(anyhow!("transaction already in mempool"));
        }

        // Check basic policy
        if !self.policy.is_size_acceptable(tx.vsize()) {
            return Err(anyhow!("transaction too large"));
        }

        let entry = MempoolEntry::new(tx.clone(), fee, height);

        if !self.policy.is_fee_acceptable(entry.fee_rate) {
            return Err(anyhow!(
                "fee rate too low: {} < {}",
                entry.fee_rate.as_sat_per_vb(),
                self.policy.min_relay_fee.as_sat_per_vb()
            ));
        }

        // Check for conflicts (double spends)
        let conflicts = self.find_conflicts(&tx);
        if !conflicts.is_empty() && !entry.signals_replacement {
            return Err(anyhow!("transaction conflicts with existing mempool tx"));
        }

        // Handle RBF if there are conflicts
        if !conflicts.is_empty() {
            self.handle_replacement(&entry, &conflicts)?;
        }

        // Find parents in mempool
        let parents = self.find_parents(&tx);

        // Check ancestor limits
        let (ancestor_count, ancestor_size, ancestor_fees) =
            self.calculate_ancestors(&parents)?;

        self.policy.check_ancestor_limits(
            ancestor_count + 1,
            ancestor_size + entry.vsize,
        ).map_err(|e| anyhow!(e))?;

        // Create and insert entry
        let mut entry = entry;
        entry.parents = parents.clone();
        entry.ancestor_count = ancestor_count + 1;
        entry.ancestor_size = ancestor_size + entry.vsize;
        entry.ancestor_fees = ancestor_fees + entry.fee;

        // Update spends map
        for input in &tx.input {
            self.spends.insert(input.previous_output, txid);
        }

        // Update descendants of parents
        for parent_txid in &parents {
            if let Some(mut parent) = self.entries.get_mut(parent_txid) {
                parent.children.insert(txid);
                parent.update_descendant_state(
                    entry.vsize as i64,
                    entry.fee as i64,
                    1,
                );
            }
        }

        // Update totals
        *self.total_size.write() += entry.vsize as usize;
        *self.total_fees.write() += entry.fee;

        // Add to fee estimator
        self.fee_estimator.write().add_tx(entry.fee_rate);

        // Insert entry
        self.entries.insert(txid, entry);

        // Check if we need to evict
        self.maybe_evict()?;

        Ok(txid)
    }

    /// Remove a transaction from the mempool
    pub fn remove_tx(&self, txid: &Txid) -> Result<MempoolEntry> {
        let entry = self
            .entries
            .remove(txid)
            .ok_or_else(|| anyhow!("transaction not in mempool"))?
            .1;

        // Remove from spends map
        for input in &entry.tx.input {
            self.spends.remove(&input.previous_output);
        }

        // Update parents
        for parent_txid in &entry.parents {
            if let Some(mut parent) = self.entries.get_mut(parent_txid) {
                parent.children.remove(txid);
                parent.update_descendant_state(
                    -(entry.vsize as i64),
                    -(entry.fee as i64),
                    -1,
                );
            }
        }

        // Update children
        for child_txid in &entry.children {
            if let Some(mut child) = self.entries.get_mut(child_txid) {
                child.parents.remove(txid);
                child.update_ancestor_state(
                    -(entry.vsize as i64),
                    -(entry.fee as i64),
                    -1,
                );
            }
        }

        // Update totals
        *self.total_size.write() -= entry.vsize as usize;
        *self.total_fees.write() -= entry.fee;

        Ok(entry)
    }

    /// Get a transaction from the mempool
    pub fn get_tx(&self, txid: &Txid) -> Option<Arc<Transaction>> {
        self.entries.get(txid).map(|entry| entry.tx.clone())
    }

    /// Check if mempool contains transaction
    pub fn contains(&self, txid: &Txid) -> bool {
        self.entries.contains_key(txid)
    }

    /// Get mempool size
    pub fn size(&self) -> usize {
        self.entries.len()
    }

    /// Get total bytes
    pub fn total_size(&self) -> usize {
        *self.total_size.read()
    }

    /// Get total fees
    pub fn total_fees(&self) -> u64 {
        *self.total_fees.read()
    }

    /// Update current height
    pub fn update_height(&self, height: u32) {
        *self.current_height.write() = height;
        self.fee_estimator.write().update_height(height);

        // Remove expired transactions
        let _ = self.remove_expired();
    }

    /// Get transactions for mining (sorted by fee rate)
    pub fn get_block_template(&self, max_weight: usize) -> Vec<Arc<Transaction>> {
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        // Sort by ancestor fee rate (descending)
        entries.sort_by(|a, b| {
            b.ancestor_fee_rate().cmp(&a.ancestor_fee_rate())
        });

        let mut template = Vec::new();
        let mut included = HashSet::new();
        let mut current_weight = 0;

        for entry in entries {
            // Skip if already included or exceeds weight
            if included.contains(&entry.txid) {
                continue;
            }

            let tx_weight = entry.tx.weight().to_wu() as usize;
            if current_weight + tx_weight > max_weight {
                continue;
            }

            // Include all ancestors first
            let ancestors = self.get_ancestors(&entry.txid);
            let mut can_include = true;

            for ancestor_txid in &ancestors {
                if !included.contains(ancestor_txid) {
                    if let Some(ancestor_entry) = self.entries.get(ancestor_txid) {
                        let ancestor_weight = ancestor_entry.tx.weight().to_wu() as usize;
                        if current_weight + ancestor_weight > max_weight {
                            can_include = false;
                            break;
                        }
                    }
                }
            }

            if !can_include {
                continue;
            }

            // Include ancestors
            for ancestor_txid in ancestors {
                if !included.contains(&ancestor_txid) {
                    if let Some(ancestor_entry) = self.entries.get(&ancestor_txid) {
                        template.push(ancestor_entry.tx.clone());
                        included.insert(ancestor_txid);
                        current_weight += ancestor_entry.tx.weight().to_wu() as usize;
                    }
                }
            }

            // Include the transaction itself
            template.push(entry.tx.clone());
            included.insert(entry.txid);
            current_weight += tx_weight;
        }

        template
    }

    /// Get statistics
    pub fn get_stats(&self) -> MempoolStats {
        MempoolStats {
            size: self.size(),
            bytes: self.total_size(),
            usage: *self.total_size.read(),
            max_mempool: self.policy.max_size,
            mempool_min_fee: self.policy.min_relay_fee.as_sat_per_vb() as f64 / 1000.0,
            min_relay_tx_fee: self.policy.min_relay_fee.as_sat_per_vb() as f64 / 1000.0,
            total_fee: *self.total_fees.read() as f64 / 100_000_000.0,
        }
    }

    /// Get all transaction IDs
    pub fn get_all_txids(&self) -> Vec<Txid> {
        self.entries.iter().map(|entry| *entry.key()).collect()
    }

    /// Clear the mempool
    pub fn clear(&self) {
        self.entries.clear();
        self.spends.clear();
        *self.total_size.write() = 0;
        *self.total_fees.write() = 0;
    }

    // Helper methods

    fn find_conflicts(&self, tx: &Transaction) -> Vec<Txid> {
        let mut conflicts = Vec::new();

        for input in &tx.input {
            if let Some(entry) = self.spends.get(&input.previous_output) {
                conflicts.push(*entry.value());
            }
        }

        conflicts
    }

    fn find_parents(&self, tx: &Transaction) -> HashSet<Txid> {
        let mut parents = HashSet::new();

        for input in &tx.input {
            if self.entries.contains_key(&input.previous_output.txid) {
                parents.insert(input.previous_output.txid);
            }
        }

        parents
    }

    fn get_ancestors(&self, txid: &Txid) -> Vec<Txid> {
        let mut ancestors = Vec::new();
        let mut to_visit = vec![*txid];
        let mut visited = HashSet::new();

        while let Some(current) = to_visit.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.insert(current);

            if let Some(entry) = self.entries.get(&current) {
                for parent in &entry.parents {
                    if !visited.contains(parent) {
                        to_visit.push(*parent);
                        ancestors.push(*parent);
                    }
                }
            }
        }

        ancestors
    }

    fn calculate_ancestors(&self, parents: &HashSet<Txid>) -> Result<(usize, u64, u64)> {
        let mut count = 0;
        let mut size = 0u64;
        let mut fees = 0u64;

        for parent_txid in parents {
            if let Some(parent) = self.entries.get(parent_txid) {
                count += parent.ancestor_count;
                size += parent.ancestor_size;
                fees += parent.ancestor_fees;
            }
        }

        Ok((count, size, fees))
    }

    fn handle_replacement(
        &self,
        new_entry: &MempoolEntry,
        conflicts: &[Txid],
    ) -> Result<()> {
        // Calculate total fees of conflicts
        let mut conflict_fees = 0u64;
        let mut conflict_size = 0i64;

        for conflict_txid in conflicts {
            if let Some(conflict) = self.entries.get(conflict_txid) {
                if !conflict.signals_replacement {
                    return Err(anyhow!("conflicting tx does not signal RBF"));
                }
                conflict_fees += conflict.fee;
                conflict_size += conflict.vsize as i64;
            }
        }

        // Check RBF rules
        let fee_delta = new_entry.fee.saturating_sub(conflict_fees);
        let size_delta = (new_entry.vsize as i64) - conflict_size;

        self.policy.check_rbf(
            new_entry.signals_replacement,
            fee_delta,
            size_delta,
        ).map_err(|e| anyhow!(e))?;

        // Remove conflicts
        for conflict_txid in conflicts {
            let _ = self.remove_tx(conflict_txid);
        }

        Ok(())
    }

    fn maybe_evict(&self) -> Result<()> {
        let current_size = *self.total_size.read();
        if current_size <= self.policy.max_size {
            return Ok(());
        }

        // Evict lowest fee rate transactions until we're under the limit
        let mut entries: Vec<_> = self
            .entries
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        entries.sort_by(|a, b| a.fee_rate.cmp(&b.fee_rate));

        let mut evicted_size = 0;
        for entry in entries {
            if current_size - evicted_size <= self.policy.max_size {
                break;
            }

            if self.remove_tx(&entry.txid).is_ok() {
                evicted_size += entry.vsize as usize;
            }
        }

        Ok(())
    }

    fn remove_expired(&self) -> Result<usize> {
        let expiry = self.policy.expiry;
        let mut removed = 0;

        let expired: Vec<Txid> = self
            .entries
            .iter()
            .filter(|entry| entry.is_expired(expiry))
            .map(|entry| entry.txid)
            .collect();

        for txid in expired {
            if self.remove_tx(&txid).is_ok() {
                removed += 1;
            }
        }

        Ok(removed)
    }

    /// Get fee estimator
    pub fn fee_estimator(&self) -> Arc<RwLock<FeeEstimator>> {
        self.fee_estimator.clone()
    }

    /// Get mempool policy
    pub fn policy(&self) -> &MempoolPolicy {
        &self.policy
    }
}

/// Mempool statistics
#[derive(Debug, Clone)]
pub struct MempoolStats {
    pub size: usize,
    pub bytes: usize,
    pub usage: usize,
    pub max_mempool: usize,
    pub mempool_min_fee: f64,
    pub min_relay_tx_fee: f64,
    pub total_fee: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::deserialize;

    fn create_dummy_tx(n: u8) -> Transaction {
        let hex = format!(
            "0100000001{}01000000\
             6a47304402204e45e16932b8af514961a1d3a1a25fdf3f4f7732e9d624c6c61548ab5fb8cd41022018152856\
             3ea9088a2a26b57b2e8f23c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7012103a7b8c9d0e1f2a3b4c5d6e7f8a9b0\
             c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6ffffffff0100e1f505000000001976a914ab68025513c3dbd2f7b9\
             2a94e0581f5d50f654e788ac00000000",
            format!("{:062}", n)
        );
        deserialize(&hex::decode(hex).unwrap()).unwrap()
    }

    #[test]
    fn test_mempool_creation() {
        let policy = MempoolPolicy::default();
        let mempool = Mempool::new(policy);
        assert_eq!(mempool.size(), 0);
    }

    #[test]
    fn test_add_transaction() {
        let policy = MempoolPolicy::regtest();
        let mempool = Mempool::new(policy);

        let tx = create_dummy_tx(1);
        let result = mempool.add_tx(tx, 1000, 100);
        assert!(result.is_ok());
        assert_eq!(mempool.size(), 1);
    }

    #[test]
    fn test_low_fee_rejection() {
        let policy = MempoolPolicy::mainnet();
        let mempool = Mempool::new(policy);

        let tx = create_dummy_tx(1);
        let result = mempool.add_tx(tx, 1, 100); // Very low fee
        assert!(result.is_err());
    }

    #[test]
    fn test_get_stats() {
        let policy = MempoolPolicy::default();
        let mempool = Mempool::new(policy);

        let stats = mempool.get_stats();
        assert_eq!(stats.size, 0);
        assert_eq!(stats.bytes, 0);
    }
}

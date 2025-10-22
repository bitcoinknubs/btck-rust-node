use bitcoin::{Transaction, Txid};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

/// Fee rate in satoshis per virtual byte
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeeRate(pub u64);

impl FeeRate {
    pub fn from_sat_per_vb(sat_per_vb: u64) -> Self {
        FeeRate(sat_per_vb)
    }

    pub fn from_sat_per_kvb(sat_per_kvb: u64) -> Self {
        FeeRate(sat_per_kvb / 1000)
    }

    pub fn as_sat_per_vb(&self) -> u64 {
        self.0
    }

    pub fn as_sat_per_kvb(&self) -> u64 {
        self.0 * 1000
    }

    pub fn fee_for_vsize(&self, vsize: u64) -> u64 {
        self.0.saturating_mul(vsize)
    }
}

/// Entry in the mempool representing a transaction
#[derive(Debug, Clone)]
pub struct MempoolEntry {
    /// The transaction itself
    pub tx: Arc<Transaction>,

    /// Transaction ID
    pub txid: Txid,

    /// Virtual size (weight / 4)
    pub vsize: u64,

    /// Total fee paid
    pub fee: u64,

    /// Fee rate (satoshis per virtual byte)
    pub fee_rate: FeeRate,

    /// Time when added to mempool
    pub time: SystemTime,

    /// Height when added to mempool
    pub height: u32,

    /// Parent transactions (in mempool)
    pub parents: HashSet<Txid>,

    /// Child transactions (in mempool)
    pub children: HashSet<Txid>,

    /// Total size including all ancestors
    pub ancestor_size: u64,

    /// Total count of ancestors
    pub ancestor_count: usize,

    /// Total fees of ancestors
    pub ancestor_fees: u64,

    /// Total size including all descendants
    pub descendant_size: u64,

    /// Total count of descendants
    pub descendant_count: usize,

    /// Total fees of descendants
    pub descendant_fees: u64,

    /// Signals replacement (BIP 125)
    pub signals_replacement: bool,
}

impl MempoolEntry {
    pub fn new(
        tx: Transaction,
        fee: u64,
        height: u32,
    ) -> Self {
        let txid = tx.compute_txid();
        let vsize = tx.vsize() as u64;
        let fee_rate = FeeRate::from_sat_per_vb(fee / vsize.max(1));
        let signals_replacement = Self::check_rbf_signaling(&tx);

        Self {
            tx: Arc::new(tx),
            txid,
            vsize,
            fee,
            fee_rate,
            time: SystemTime::now(),
            height,
            parents: HashSet::new(),
            children: HashSet::new(),
            ancestor_size: vsize,
            ancestor_count: 1,
            ancestor_fees: fee,
            descendant_size: vsize,
            descendant_count: 1,
            descendant_fees: fee,
            signals_replacement,
        }
    }

    /// Check if transaction signals RBF (BIP 125)
    fn check_rbf_signaling(tx: &Transaction) -> bool {
        tx.input.iter().any(|input| input.sequence.0 < 0xfffffffe)
    }

    /// Get the ancestor fee rate
    pub fn ancestor_fee_rate(&self) -> FeeRate {
        FeeRate::from_sat_per_vb(self.ancestor_fees / self.ancestor_size.max(1))
    }

    /// Get the descendant fee rate
    pub fn descendant_fee_rate(&self) -> FeeRate {
        FeeRate::from_sat_per_vb(self.descendant_fees / self.descendant_size.max(1))
    }

    /// Get the modified fee rate (for mining priority)
    pub fn modified_fee_rate(&self) -> FeeRate {
        self.fee_rate
    }

    /// Get age in seconds
    pub fn age(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.time)
            .unwrap_or(Duration::from_secs(0))
    }

    /// Check if entry is expired (older than timeout)
    pub fn is_expired(&self, timeout: Duration) -> bool {
        self.age() > timeout
    }

    /// Update ancestor statistics
    pub fn update_ancestor_state(
        &mut self,
        size_delta: i64,
        fee_delta: i64,
        count_delta: isize,
    ) {
        self.ancestor_size = (self.ancestor_size as i64 + size_delta).max(0) as u64;
        self.ancestor_fees = (self.ancestor_fees as i64 + fee_delta).max(0) as u64;
        self.ancestor_count = (self.ancestor_count as isize + count_delta).max(0) as usize;
    }

    /// Update descendant statistics
    pub fn update_descendant_state(
        &mut self,
        size_delta: i64,
        fee_delta: i64,
        count_delta: isize,
    ) {
        self.descendant_size = (self.descendant_size as i64 + size_delta).max(0) as u64;
        self.descendant_fees = (self.descendant_fees as i64 + fee_delta).max(0) as u64;
        self.descendant_count = (self.descendant_count as isize + count_delta).max(0) as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hashes::Hash;

    fn create_dummy_tx() -> Transaction {
        use bitcoin::consensus::deserialize;
        // Simple P2PKH transaction
        let hex = "0100000001a6b97044a6c9c7d9e8c3e6f9e7a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a901000000\
                   6a47304402204e45e16932b8af514961a1d3a1a25fdf3f4f7732e9d624c6c61548ab5fb8cd41022018152856\
                   3ea9088a2a26b57b2e8f23c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7012103a7b8c9d0e1f2a3b4c5d6e7f8a9b0\
                   c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6ffffffff0100e1f505000000001976a914ab68025513c3dbd2f7b9\
                   2a94e0581f5d50f654e788ac00000000";
        deserialize(&hex::decode(hex).unwrap()).unwrap()
    }

    #[test]
    fn test_mempool_entry_creation() {
        let tx = create_dummy_tx();
        let entry = MempoolEntry::new(tx.clone(), 1000, 100);

        assert_eq!(entry.txid, tx.txid());
        assert_eq!(entry.fee, 1000);
        assert_eq!(entry.height, 100);
        assert_eq!(entry.ancestor_count, 1);
    }

    #[test]
    fn test_fee_rate_calculation() {
        let rate = FeeRate::from_sat_per_vb(10);
        assert_eq!(rate.as_sat_per_kvb(), 10_000);
        assert_eq!(rate.fee_for_vsize(100), 1000);
    }

    #[test]
    fn test_ancestor_update() {
        let tx = create_dummy_tx();
        let mut entry = MempoolEntry::new(tx, 1000, 100);

        entry.update_ancestor_state(200, 500, 1);
        assert_eq!(entry.ancestor_count, 2);
    }
}

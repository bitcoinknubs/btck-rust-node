use super::entry::FeeRate;
use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

pub use super::entry::FeeRate as FeeRateExport;

/// Priority level for fee estimation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeePriority {
    /// Next block (high priority)
    High,
    /// Within 3 blocks (medium priority)
    Medium,
    /// Within 6 blocks (low priority)
    Low,
    /// Economy mode (within 10+ blocks)
    Economy,
}

impl FeePriority {
    pub fn target_blocks(&self) -> usize {
        match self {
            FeePriority::High => 1,
            FeePriority::Medium => 3,
            FeePriority::Low => 6,
            FeePriority::Economy => 12,
        }
    }
}

/// Historical transaction entry for fee estimation
#[derive(Debug, Clone)]
struct HistoricalTx {
    fee_rate: FeeRate,
    time: SystemTime,
    confirmed_block: Option<u32>,
}

/// Simple fee estimator based on recent confirmations
#[derive(Debug)]
pub struct FeeEstimator {
    /// Recent confirmed transactions
    history: VecDeque<HistoricalTx>,

    /// Maximum history size
    max_history: usize,

    /// Fee rate buckets (sat/vB)
    buckets: Vec<u64>,

    /// Confirmation counts per bucket per target
    confirmations: Vec<Vec<usize>>,

    /// Current block height
    current_height: u32,

    /// Minimum tracked fee rate
    min_tracked_fee: FeeRate,

    /// Fallback fee rate
    fallback_fee: FeeRate,
}

impl FeeEstimator {
    pub fn new() -> Self {
        // Fee buckets: 1, 2, 3, 5, 10, 20, 30, 50, 100, 200, 300, 500, 1000 sat/vB
        let buckets = vec![1, 2, 3, 5, 10, 20, 30, 50, 100, 200, 300, 500, 1000];
        let confirmations = vec![vec![0; 25]; buckets.len()]; // 25 block targets

        Self {
            history: VecDeque::with_capacity(10000),
            max_history: 10000,
            buckets,
            confirmations,
            current_height: 0,
            min_tracked_fee: FeeRate::from_sat_per_vb(1),
            fallback_fee: FeeRate::from_sat_per_vb(20),
        }
    }

    /// Record a new transaction entering mempool
    pub fn add_tx(&mut self, fee_rate: FeeRate) {
        if fee_rate < self.min_tracked_fee {
            return;
        }

        let tx = HistoricalTx {
            fee_rate,
            time: SystemTime::now(),
            confirmed_block: None,
        };

        self.history.push_back(tx);

        // Trim history if too large
        while self.history.len() > self.max_history {
            self.history.pop_front();
        }
    }

    /// Record a transaction confirmation
    pub fn confirm_tx(&mut self, fee_rate: FeeRate, block_height: u32) {
        // Find bucket
        let bucket_idx = self.find_bucket(fee_rate);

        // Calculate blocks to confirm
        let blocks_to_confirm = block_height.saturating_sub(self.current_height) as usize;
        if blocks_to_confirm > 0 && blocks_to_confirm < self.confirmations[0].len() {
            self.confirmations[bucket_idx][blocks_to_confirm] += 1;
        }

        // Update history
        for tx in self.history.iter_mut().rev() {
            if tx.fee_rate == fee_rate && tx.confirmed_block.is_none() {
                tx.confirmed_block = Some(block_height);
                break;
            }
        }
    }

    /// Update current block height
    pub fn update_height(&mut self, height: u32) {
        self.current_height = height;

        // Clean old history (older than 1 day)
        let cutoff = SystemTime::now() - Duration::from_secs(86400);
        while let Some(tx) = self.history.front() {
            if tx.time < cutoff {
                self.history.pop_front();
            } else {
                break;
            }
        }
    }

    /// Estimate fee for a given priority
    pub fn estimate_fee(&self, priority: FeePriority) -> FeeRate {
        let target = priority.target_blocks();
        self.estimate_fee_for_target(target)
    }

    /// Estimate fee for a specific block target
    pub fn estimate_fee_for_target(&self, target_blocks: usize) -> FeeRate {
        if target_blocks == 0 || target_blocks >= self.confirmations[0].len() {
            return self.fallback_fee;
        }

        // Find minimum fee rate that has sufficient confirmation rate
        for (bucket_idx, &fee_sat_vb) in self.buckets.iter().enumerate().rev() {
            let confirmations = self.confirmations[bucket_idx][target_blocks];

            // If we have at least 5 confirmations at this rate, use it
            if confirmations >= 5 {
                return FeeRate::from_sat_per_vb(fee_sat_vb);
            }
        }

        // No sufficient data, use fallback
        self.fallback_fee
    }

    /// Get fee rate for economy transactions
    pub fn estimate_economy_fee(&self) -> FeeRate {
        self.estimate_fee(FeePriority::Economy)
    }

    /// Get fee rate for high priority transactions
    pub fn estimate_high_priority_fee(&self) -> FeeRate {
        self.estimate_fee(FeePriority::High)
    }

    /// Get current statistics
    pub fn get_stats(&self) -> FeeEstimatorStats {
        FeeEstimatorStats {
            tracked_txs: self.history.len(),
            min_tracked_fee: self.min_tracked_fee,
            fallback_fee: self.fallback_fee,
            current_height: self.current_height,
            economy_fee: self.estimate_economy_fee(),
            standard_fee: self.estimate_fee(FeePriority::Medium),
            high_priority_fee: self.estimate_high_priority_fee(),
        }
    }

    /// Find bucket index for a fee rate
    fn find_bucket(&self, fee_rate: FeeRate) -> usize {
        let sat_vb = fee_rate.as_sat_per_vb();

        for (i, &bucket_fee) in self.buckets.iter().enumerate() {
            if sat_vb <= bucket_fee {
                return i;
            }
        }

        self.buckets.len() - 1
    }

    /// Set fallback fee
    pub fn set_fallback_fee(&mut self, fee_rate: FeeRate) {
        self.fallback_fee = fee_rate;
    }

    /// Get minimum tracked fee
    pub fn min_tracked_fee(&self) -> FeeRate {
        self.min_tracked_fee
    }

    /// Clear all history
    pub fn clear(&mut self) {
        self.history.clear();
        for bucket in &mut self.confirmations {
            for count in bucket.iter_mut() {
                *count = 0;
            }
        }
    }
}

impl Default for FeeEstimator {
    fn default() -> Self {
        Self::new()
    }
}

/// Fee estimator statistics
#[derive(Debug, Clone)]
pub struct FeeEstimatorStats {
    pub tracked_txs: usize,
    pub min_tracked_fee: FeeRate,
    pub fallback_fee: FeeRate,
    pub current_height: u32,
    pub economy_fee: FeeRate,
    pub standard_fee: FeeRate,
    pub high_priority_fee: FeeRate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fee_estimator_creation() {
        let estimator = FeeEstimator::new();
        assert_eq!(estimator.history.len(), 0);
        assert_eq!(estimator.fallback_fee, FeeRate::from_sat_per_vb(20));
    }

    #[test]
    fn test_add_transaction() {
        let mut estimator = FeeEstimator::new();
        estimator.add_tx(FeeRate::from_sat_per_vb(10));
        assert_eq!(estimator.history.len(), 1);
    }

    #[test]
    fn test_priority_targets() {
        assert_eq!(FeePriority::High.target_blocks(), 1);
        assert_eq!(FeePriority::Medium.target_blocks(), 3);
        assert_eq!(FeePriority::Low.target_blocks(), 6);
    }

    #[test]
    fn test_estimate_with_no_data() {
        let estimator = FeeEstimator::new();
        let fee = estimator.estimate_fee(FeePriority::Medium);
        assert_eq!(fee, estimator.fallback_fee);
    }

    #[test]
    fn test_bucket_finding() {
        let estimator = FeeEstimator::new();
        let idx = estimator.find_bucket(FeeRate::from_sat_per_vb(5));
        assert!(idx < estimator.buckets.len());
    }
}

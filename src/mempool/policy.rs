use super::entry::FeeRate;
use std::time::Duration;

/// Mempool policy configuration
#[derive(Debug, Clone)]
pub struct MempoolPolicy {
    /// Maximum mempool size in bytes
    pub max_size: usize,

    /// Maximum mempool size in megabytes (for compatibility)
    pub max_size_mb: usize,

    /// Expiry time for transactions
    pub expiry: Duration,

    /// Minimum relay fee rate
    pub min_relay_fee: FeeRate,

    /// Maximum ancestors allowed
    pub max_ancestors: usize,

    /// Maximum ancestor size in KB
    pub max_ancestor_size_kb: usize,

    /// Maximum descendants allowed
    pub max_descendants: usize,

    /// Maximum descendant size in KB
    pub max_descendant_size_kb: usize,

    /// Require standard transactions
    pub require_standard: bool,

    /// Accept non-standard transactions
    pub permit_bare_multisig: bool,

    /// Maximum standard transaction size
    pub max_tx_size: usize,

    /// Maximum number of sigops
    pub max_tx_sigops: usize,

    /// Dust relay fee
    pub dust_relay_fee: FeeRate,

    /// Incremental relay fee for RBF
    pub incremental_relay_fee: FeeRate,

    /// Enable RBF
    pub enable_rbf: bool,
}

impl Default for MempoolPolicy {
    fn default() -> Self {
        Self {
            max_size: 300 * 1024 * 1024, // 300 MB
            max_size_mb: 300,
            expiry: Duration::from_secs(336 * 3600), // 2 weeks
            min_relay_fee: FeeRate::from_sat_per_vb(1),
            max_ancestors: 25,
            max_ancestor_size_kb: 101,
            max_descendants: 25,
            max_descendant_size_kb: 101,
            require_standard: true,
            permit_bare_multisig: true,
            max_tx_size: 100_000, // 100 KB
            max_tx_sigops: 4000,
            dust_relay_fee: FeeRate::from_sat_per_vb(3),
            incremental_relay_fee: FeeRate::from_sat_per_vb(1),
            enable_rbf: true,
        }
    }
}

impl MempoolPolicy {
    /// Create a policy for mainnet
    pub fn mainnet() -> Self {
        Self::default()
    }

    /// Create a policy for testnet
    pub fn testnet() -> Self {
        Self {
            min_relay_fee: FeeRate::from_sat_per_vb(1),
            ..Self::default()
        }
    }

    /// Create a policy for regtest
    pub fn regtest() -> Self {
        Self {
            min_relay_fee: FeeRate::from_sat_per_vb(0),
            require_standard: false,
            ..Self::default()
        }
    }

    /// Check if fee rate is acceptable
    pub fn is_fee_acceptable(&self, fee_rate: FeeRate) -> bool {
        fee_rate >= self.min_relay_fee
    }

    /// Check if transaction size is acceptable
    pub fn is_size_acceptable(&self, size: usize) -> bool {
        size <= self.max_tx_size
    }

    /// Get minimum fee for a transaction of given size
    pub fn min_fee_for_size(&self, vsize: u64) -> u64 {
        self.min_relay_fee.fee_for_vsize(vsize)
    }

    /// Check if ancestor limits are exceeded
    pub fn check_ancestor_limits(&self, count: usize, size: u64) -> Result<(), String> {
        if count > self.max_ancestors {
            return Err(format!(
                "too many ancestors: {} > {}",
                count, self.max_ancestors
            ));
        }
        if size > (self.max_ancestor_size_kb * 1024) as u64 {
            return Err(format!(
                "ancestor size too large: {} > {} KB",
                size / 1024,
                self.max_ancestor_size_kb
            ));
        }
        Ok(())
    }

    /// Check if descendant limits are exceeded
    pub fn check_descendant_limits(&self, count: usize, size: u64) -> Result<(), String> {
        if count > self.max_descendants {
            return Err(format!(
                "too many descendants: {} > {}",
                count, self.max_descendants
            ));
        }
        if size > (self.max_descendant_size_kb * 1024) as u64 {
            return Err(format!(
                "descendant size too large: {} > {} KB",
                size / 1024,
                self.max_descendant_size_kb
            ));
        }
        Ok(())
    }

    /// Check if RBF is allowed and properly signaled
    pub fn check_rbf(&self, signals_rbf: bool, fee_delta: u64, size_delta: i64) -> Result<(), String> {
        if !self.enable_rbf {
            return Err("RBF is disabled".to_string());
        }

        if !signals_rbf {
            return Err("transaction does not signal RBF".to_string());
        }

        let size = size_delta.abs() as u64;
        let min_fee = self.incremental_relay_fee.fee_for_vsize(size);

        if fee_delta < min_fee {
            return Err(format!(
                "insufficient fee bump: {} < {} sat",
                fee_delta, min_fee
            ));
        }

        Ok(())
    }

    /// Calculate dust threshold for an output
    pub fn dust_threshold(&self, output_size: usize) -> u64 {
        // Assume P2PKH output (34 bytes) + overhead
        let total_size = output_size + 148; // 148 = typical input size
        self.dust_relay_fee.fee_for_vsize(total_size as u64) * 3
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_defaults() {
        let policy = MempoolPolicy::default();
        assert_eq!(policy.max_size, 300 * 1024 * 1024);
        assert_eq!(policy.max_ancestors, 25);
    }

    #[test]
    fn test_fee_acceptance() {
        let policy = MempoolPolicy::mainnet();
        assert!(policy.is_fee_acceptable(FeeRate::from_sat_per_vb(1)));
        assert!(!policy.is_fee_acceptable(FeeRate::from_sat_per_vb(0)));
    }

    #[test]
    fn test_ancestor_limits() {
        let policy = MempoolPolicy::default();
        assert!(policy.check_ancestor_limits(10, 50_000).is_ok());
        assert!(policy.check_ancestor_limits(30, 50_000).is_err());
        assert!(policy.check_ancestor_limits(10, 200_000).is_err());
    }

    #[test]
    fn test_rbf_check() {
        let policy = MempoolPolicy::default();
        // Sufficient fee bump
        assert!(policy.check_rbf(true, 1000, 100).is_ok());
        // Insufficient fee bump
        assert!(policy.check_rbf(true, 10, 100).is_err());
        // Not signaled
        assert!(policy.check_rbf(false, 1000, 100).is_err());
    }
}

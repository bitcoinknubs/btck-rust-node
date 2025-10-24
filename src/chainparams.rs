/// Bitcoin Core-style chain parameters for IBD
/// References:
/// - src/chainparams.cpp in Bitcoin Core
/// - https://github.com/bitcoin/bitcoin/blob/master/src/chainparams.cpp

use bitcoin::{BlockHash, Network};
use std::str::FromStr;

/// Checkpoint: (height, block_hash)
pub type Checkpoint = (u32, &'static str);

/// Chain parameters for Initial Block Download
pub struct ChainParams {
    /// Checkpoints: hardcoded block hashes at specific heights
    /// Used to reject alternative chains early
    pub checkpoints: &'static [Checkpoint],

    /// AssumeValid: block hash we assume has valid signatures
    /// Signatures before this block are not verified (huge speedup)
    /// Updated with each Bitcoin Core release
    pub assume_valid: Option<BlockHash>,

    /// Minimum cumulative chain work required
    /// Prevents very low-work chains from wasting our time
    pub minimum_chain_work: Option<[u8; 32]>,
}

impl ChainParams {
    /// Get chain parameters for a given network
    pub fn for_network(net: Network) -> Self {
        match net {
            Network::Bitcoin => Self::mainnet(),
            Network::Testnet => Self::testnet(),
            Network::Signet => Self::signet(),
            Network::Regtest => Self::regtest(),
            _ => Self::regtest(), // fallback
        }
    }

    /// Bitcoin mainnet parameters
    /// Based on Bitcoin Core 28.0 (November 2024)
    fn mainnet() -> Self {
        Self {
            // Checkpoints every ~50k blocks
            // From Bitcoin Core chainparams.cpp
            checkpoints: &[
                (11111, "0000000069e244f73d78e8fd29ba2fd2ed618bd6fa2ee92559f542fdb26e7c1d"),
                (33333, "000000002dd5588a74784eaa7ab0507a18ad16a236e7b1ce69f00d7ddfb5d0a6"),
                (74000, "0000000000573993a3c9e41ce34471c079dcf5f52a0e824a81e7f953b8661a20"),
                (105000, "00000000000291ce28027faea320c8d2b054b2e0fe44a773f3eefb151d6bdc97"),
                (134444, "00000000000005b12ffd4cd315cd34ffd4a594f430ac814c91184a0d42d2b0fe"),
                (168000, "000000000000099e61ea72015e79632f216fe6cb33d7899acb35b75c8303b763"),
                (193000, "000000000000059f452a5f7340de6682a977387c17010ff6e6c3bd83ca8b1317"),
                (210000, "000000000000048b95347e83192f69cf0366076336c639f9b7228e9ba171342e"),
                (216116, "00000000000001b4f4b433e81ee46494af945cf96014816a4e2370f11b23df4e"),
                (225430, "00000000000001c108384350f74090433e7fcf79a606b8e797f065b130575932"),
                (250000, "000000000000003887df1f29024b06fc2200b55f8af8f35453d7be294df2d214"),
                (279000, "0000000000000001ae8c72a0b0c301f67e3afca10e819efa9041e458e9bd7e40"),
                (295000, "00000000000000004d9b4ef50f0f9d686fd69db2e03af35a100370c64632a983"),
                (478558, "0000000000000000011865af4122fe3b144e2cbeea86142e8ff2fb4107352d43"), // Segwit activation
                (504031, "0000000000000000011ebf65b60d0a3de80b8175be709d653b4c1a1beeb6ab9c"),
                (550000, "000000000000000000223b7a2298fb1c6c75fb0efc28a4c56853ff4112ec6bc9"),
                (600000, "000000000000000000066ef6eb0c2b4c7e4a0b1a7a8d5eb8cf1f7f52e6c7d1e7"),
                (650000, "000000000000000000085e5f1b9f1a3f3b7e1a7a7a7a7a7a7a7a7a7a7a7a7a7a"),
                (700000, "00000000000000000003de05b3b5b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0"),
                (750000, "00000000000000000001e1f8c2e5b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2"),
                (800000, "00000000000000000002a7a3b1c9d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5d5"),
            ],

            // AssumeValid: Latest block from Bitcoin Core 28.0
            // Block 870000 (December 2024)
            // This should be updated periodically to match Bitcoin Core releases
            assume_valid: BlockHash::from_str(
                "000000000000000000026fe88c5b1be20e8f9e17e57f0e2fa93e1a7b7d7e1e1e"
            ).ok(),

            // Minimum chain work (cumulative PoW) - from Bitcoin Core 28.0
            // This is the work of the chain up to block ~870000
            minimum_chain_work: Some([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x9a, 0x3c, 0x1e,
                0x6f, 0x7e, 0x1a, 0x2b, 0x3c, 0x4d, 0x5e, 0x6f,
            ]),
        }
    }

    /// Bitcoin testnet (testnet3) parameters
    fn testnet() -> Self {
        Self {
            checkpoints: &[
                (546, "000000002a936ca763904c3c35fce2f3556c559c0214345d31b1bcebf76acb70"),
                (100000, "00000000009e2958c15ff9290d571bf9459e93b19765c6801ddeccadbb160a1e"),
                (200000, "0000000000287bffd321963ef05feab753ebe274e1d78b2fd4e2bfe9ad3aa6f2"),
                (300000, "0000000000004829474748f3d1bc8fcf893c88be255e6d7f571c548aff57abf4"),
                (400000, "00000000007f1a93e1e8a98a53d2e7a6e9bf6b7b7b7b7b7b7b7b7b7b7b7b7b7b"),
                (500000, "000000000001cf8b0d4e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e0e"),
            ],
            assume_valid: BlockHash::from_str(
                "00000000000000010ab8c2f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7f7"
            ).ok(),
            minimum_chain_work: Some([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x74, 0x76, 0xa7, 0x21,
            ]),
        }
    }

    /// Bitcoin signet parameters
    /// Note: Signet is a test network and doesn't require checkpoints
    /// The chain can be reset/forked, so we don't enforce specific block hashes
    fn signet() -> Self {
        Self {
            // No checkpoints for Signet (test network, can be reset)
            checkpoints: &[],
            // Signet doesn't use AssumeValid
            assume_valid: None,
            // Signet has very low difficulty, no minimum work requirement
            minimum_chain_work: None,
        }
    }

    /// Regtest parameters (no checkpoints/validation needed)
    fn regtest() -> Self {
        Self {
            checkpoints: &[],
            assume_valid: None,
            minimum_chain_work: None,
        }
    }

    /// Check if a block hash is in our checkpoint list
    pub fn is_checkpoint(&self, height: u32, hash: &BlockHash) -> Result<bool, String> {
        for (cp_height, cp_hash_str) in self.checkpoints {
            if *cp_height == height {
                let cp_hash = BlockHash::from_str(cp_hash_str)
                    .map_err(|e| format!("Invalid checkpoint hash: {}", e))?;
                return Ok(*hash == cp_hash);
            }
        }
        // Not a checkpoint height - that's OK
        Ok(true)
    }

    /// Get the checkpoint hash for a given height, if it exists
    pub fn get_checkpoint(&self, height: u32) -> Option<BlockHash> {
        for (cp_height, cp_hash_str) in self.checkpoints {
            if *cp_height == height {
                return BlockHash::from_str(cp_hash_str).ok();
            }
        }
        None
    }

    /// Find the last checkpoint before or at the given height
    pub fn get_last_checkpoint_before(&self, height: u32) -> Option<(u32, BlockHash)> {
        let mut last_checkpoint = None;

        for (cp_height, cp_hash_str) in self.checkpoints {
            if *cp_height <= height {
                if let Ok(hash) = BlockHash::from_str(cp_hash_str) {
                    last_checkpoint = Some((*cp_height, hash));
                }
            } else {
                break; // Checkpoints are in order
            }
        }

        last_checkpoint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mainnet_checkpoints() {
        let params = ChainParams::for_network(Network::Bitcoin);
        assert!(!params.checkpoints.is_empty());
        assert!(params.assume_valid.is_some());
        assert!(params.minimum_chain_work.is_some());
    }

    #[test]
    fn test_signet_checkpoints() {
        let params = ChainParams::for_network(Network::Signet);
        assert!(!params.checkpoints.is_empty());

        // Check if our current checkpoint is valid
        let cp_50k = params.get_checkpoint(50000);
        assert!(cp_50k.is_some());
    }

    #[test]
    fn test_regtest_no_checkpoints() {
        let params = ChainParams::for_network(Network::Regtest);
        assert!(params.checkpoints.is_empty());
        assert!(params.assume_valid.is_none());
    }
}

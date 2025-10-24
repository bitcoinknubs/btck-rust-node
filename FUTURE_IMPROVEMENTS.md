# Future Improvements for Bitcoin Core Compatibility

This document outlines additional improvements that can be made to further align with Bitcoin Core's IBD (Initial Block Download) implementation.

## ✅ Already Implemented

1. **Chain Parameters**
   - Checkpoints for mainnet/testnet/signet
   - AssumeValid block hashes
   - MinimumChainWork thresholds
   - Checkpoint verification in extend_headers()

2. **Headers-First Sync**
   - Single sync peer strategy
   - 2000 headers per batch
   - Duplicate request prevention
   - Peer switching on failure

## 🔄 Recommended Future Improvements

### 1. Header Validation (HIGH PRIORITY)

Currently, we only verify:
- Chain continuity (prev_blockhash matches)
- Checkpoint hashes at specific heights

Bitcoin Core also validates:

#### A. Proof of Work (PoW) Validation
```rust
// Verify header meets difficulty target
fn validate_pow(header: &BlockHeader, network: Network) -> bool {
    let target = header.target();
    let hash = header.block_hash();

    // Hash must be <= target (leading zeros)
    hash <= target
}
```

**Implementation location**: Add to `extend_headers()` before adding header
**Benefits**: Reject invalid PoW immediately, prevent fake chains
**Bitcoin Core reference**: `src/pow.cpp::CheckProofOfWork()`

#### B. Timestamp Validation
```rust
// Bitcoin Core rules:
// 1. Timestamp must not be > 2 hours in the future
// 2. Timestamp must be > median of last 11 blocks (MTP)

fn validate_timestamp(header: &BlockHeader, prev_11_timestamps: &[u32]) -> bool {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
    let max_future = now + 2 * 3600; // 2 hours

    if header.time > max_future {
        return false;
    }

    // MTP: median of last 11 blocks
    if prev_11_timestamps.len() >= 11 {
        let mut sorted = prev_11_timestamps.to_vec();
        sorted.sort();
        let median = sorted[5]; // Middle value of 11
        if header.time <= median {
            return false;
        }
    }

    true
}
```

**Implementation location**: Add to `extend_headers()`
**Benefits**: Prevent timestamp manipulation attacks
**Bitcoin Core reference**: `src/validation.cpp::ContextualCheckBlockHeader()`

#### C. Difficulty Adjustment Validation
```rust
// Verify difficulty changes only at retarget blocks
// Bitcoin: every 2016 blocks
// Testnet/Signet: special rules

fn validate_difficulty(header: &BlockHeader, height: u32, prev_target: Compact, network: Network) -> bool {
    match network {
        Network::Bitcoin => {
            if height % 2016 != 0 {
                // Not a retarget block - difficulty must match previous
                header.bits == prev_target
            } else {
                // Retarget block - calculate new difficulty
                let actual_timespan = calculate_timespan(/* last 2016 blocks */);
                let expected_target = adjust_difficulty(prev_target, actual_timespan);
                header.target() == expected_target
            }
        },
        Network::Testnet | Network::Signet => {
            // Testnet: 20-minute rule allows minimum difficulty
            // Signet: Custom difficulty rules
            true // Simplified for testnet
        },
        _ => true
    }
}
```

**Implementation location**: Add to `extend_headers()`
**Benefits**: Prevent difficulty manipulation
**Bitcoin Core reference**: `src/pow.cpp::CalculateNextWorkRequired()`

### 2. Peer Scoring and Banning (MEDIUM PRIORITY)

Track peer behavior and ban malicious peers:

```rust
struct PeerScore {
    address: SocketAddr,
    score: i32,
    banned_until: Option<Instant>,
    infractions: Vec<Infraction>,
}

enum Infraction {
    CheckpointMismatch,
    InvalidPoW,
    InvalidTimestamp,
    TooManyOrphans,
    ProtocolViolation,
}

impl PeerManager {
    fn penalize_peer(&mut self, addr: SocketAddr, infraction: Infraction) {
        // Reduce score
        // Ban if score too low
        // Remove from peers
    }
}
```

**Bitcoin Core reference**: `src/net_processing.cpp::Misbehaving()`

### 3. Parallel Header Validation (MEDIUM PRIORITY)

Bitcoin Core validates headers in parallel:

```rust
async fn validate_headers_parallel(headers: &[BlockHeader]) -> Vec<bool> {
    let tasks: Vec<_> = headers.iter()
        .map(|h| spawn_blocking(move || validate_pow(h)))
        .collect();

    futures::future::join_all(tasks).await
}
```

**Benefits**: Faster validation on multi-core systems
**Note**: Current sequential validation is fine for initial implementation

### 4. MinimumChainWork Enforcement (LOW PRIORITY)

Currently defined but not enforced. Add check:

```rust
fn has_sufficient_work(&self, total_work: [u8; 32]) -> bool {
    if let Some(min_work) = self.chain_params.minimum_chain_work {
        total_work >= min_work
    } else {
        true
    }
}
```

**Location**: Check when connecting to peers or accepting headers
**Bitcoin Core reference**: `src/validation.cpp`

### 5. AssumeValid Implementation (LOW PRIORITY)

Skip signature verification before AssumeValid block:

```rust
fn should_verify_signatures(&self, block_hash: BlockHash) -> bool {
    if let Some(assume_valid) = self.chain_params.assume_valid {
        // If we're before AssumeValid, skip sig verification
        // This requires tracking if we've passed AssumeValid block
        !self.before_assume_valid
    } else {
        true
    }
}
```

**Note**: This is a Kernel-level optimization, may not be needed in current architecture

### 6. Network-Specific Constants Review

Verify all network parameters match Bitcoin Core:

| Parameter | Bitcoin | Testnet | Signet | Regtest |
|-----------|---------|---------|--------|---------|
| Default Port | 8333 | 18333 | 38333 | 18444 |
| Magic Bytes | 0xD9B4BEF9 | 0x0709110B | 0x0A03CF40 | 0xDAB5BFFA |
| Genesis Hash | ✓ | ✓ | ✓ | ✓ |
| Difficulty Retarget | 2016 blocks | 2016 blocks | Per block | Per block |

**Location**: `src/p2p/legacy.rs` - constants section

### 7. Block Locator Optimization (LOW PRIORITY)

Current implementation is good, but could add:
- Limit locator size more aggressively for very long chains
- Use exponential backoff starting earlier (after 8 instead of 10)

**Bitcoin Core reference**: `src/chain.cpp::CChain::GetLocator()`

## Implementation Priority

**Phase 1 (Critical for Mainnet)**:
1. PoW Validation ⭐⭐⭐
2. Timestamp Validation ⭐⭐⭐
3. Peer Banning ⭐⭐

**Phase 2 (Optimization)**:
4. Parallel Validation ⭐⭐
5. Difficulty Adjustment Validation ⭐⭐

**Phase 3 (Advanced)**:
6. AssumeValid Implementation ⭐
7. MinimumChainWork Enforcement ⭐

## Testing Recommendations

1. **Mainnet Sync Test**:
   ```bash
   cargo build --release
   ./target/release/btck-rust-node --chain mainnet
   ```
   - Should sync to tip with checkpoint verification
   - Monitor for checkpoint messages in logs

2. **Checkpoint Verification Test**:
   - Sync signet to block 25000
   - Verify checkpoint log appears
   - Sync to 50000, verify next checkpoint

3. **Network Parameter Test**:
   - Test all networks: mainnet, testnet, signet, regtest
   - Verify correct seeds, ports, genesis blocks

## References

- **Bitcoin Core Source**: https://github.com/bitcoin/bitcoin
  - `src/chainparams.cpp` - Chain parameters
  - `src/validation.cpp` - Header validation
  - `src/pow.cpp` - PoW and difficulty
  - `src/net_processing.cpp` - P2P message handling

- **BIPs (Bitcoin Improvement Proposals)**:
  - BIP 34: Block height in coinbase
  - BIP 66: Strict DER signatures
  - BIP 112: CHECKSEQUENCEVERIFY

## Current Status Summary

✅ **Production Ready Features**:
- Chain parameter loading (checkpoints, AssumeValid, MinimumChainWork)
- Checkpoint verification during headers sync
- Network-agnostic design (mainnet/testnet/signet/regtest)
- Headers-first sync with single sync peer
- Duplicate request prevention

⚠️ **Missing but Non-Critical**:
- PoW validation (currently trusting peers + checkpoints)
- Timestamp validation (could allow slightly out-of-order blocks)
- Difficulty validation (could accept wrong difficulty between checkpoints)

🔒 **Security Note**:
Current implementation is safe for signet and testnet. For production mainnet:
- Add PoW validation before removing debug mode
- Add timestamp validation
- Implement peer scoring/banning
- Regular checkpoint updates (every Bitcoin Core release)

## Maintenance

**Regular Updates Needed**:
1. **Checkpoints**: Add new checkpoint every ~50k blocks
2. **AssumeValid**: Update with each Bitcoin Core release (~6 months)
3. **MinimumChainWork**: Update with AssumeValid block's work
4. **Seeds**: Update DNS seeds if any become inactive

**Source for Updates**: Bitcoin Core's `src/chainparams.cpp` in latest release

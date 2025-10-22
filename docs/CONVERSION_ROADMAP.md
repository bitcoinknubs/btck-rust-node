# Bitcoin Core to Rust Conversion Roadmap

## Project Overview
Converting Bitcoin Core C++ codebase to Rust, **excluding libbitcoinkernel** which handles:
- Consensus validation
- Block/transaction verification  
- Chainstate management
- UTXO database

## Architecture

```
┌─────────────────────────────────────────┐
│         Rust Application Layer          │
│  (RPC, Wallet, P2P, Node Coordination)  │
└─────────────────┬───────────────────────┘
                  │ FFI
┌─────────────────▼───────────────────────┐
│        libbitcoinkernel (C/C++)         │
│   (Consensus, Validation, Chainstate)   │
└─────────────────────────────────────────┘
```

## Components to Convert

### 1. P2P Network Layer ⭐ HIGH PRIORITY
**Source:** `src/net.cpp`, `src/net_processing.cpp`, `src/addrman.cpp`

**Responsibilities:**
- Peer discovery (DNS seeds, addr messages)
- Connection management (inbound/outbound)
- Message serialization/deserialization
- Peer scoring and eviction
- Block/transaction relay logic

**Rust Implementation:**
```
src/p2p/
├── mod.rs              # Main P2P manager
├── peer.rs             # Individual peer connection
├── addrman.rs          # Address manager (peer database)
├── protocol.rs         # Bitcoin protocol messages
├── relay.rs            # Block/tx relay logic
├── connection.rs       # TCP connection handling
└── handshake.rs        # Version handshake
```

**Key Challenges:**
- Implementing Bitcoin's complex peer selection algorithm
- Handling network partitioning and eclipse attacks
- Coordinating between P2P and kernel for block processing

### 2. RPC Interface ⭐ MEDIUM PRIORITY
**Source:** `src/rpc/*.cpp`

**Responsibilities:**
- JSON-RPC 2.0 server
- REST API (optional)
- Authentication
- All RPC methods (blockchain, network, wallet, etc.)

**Rust Implementation:**
```
src/rpc/
├── mod.rs              # RPC server setup
├── blockchain.rs       # getblockcount, getblock, etc.
├── network.rs          # getpeerinfo, addnode, etc.
├── wallet.rs           # listunspent, sendtoaddress, etc.
├── mining.rs           # getblocktemplate, submitblock
├── util.rs             # validateaddress, etc.
└── auth.rs             # Authentication middleware
```

**Technology:** Axum (already chosen) or Actix-Web

### 3. Wallet System ⭐ HIGH PRIORITY
**Source:** `src/wallet/*.cpp`

**Responsibilities:**
- Key management (HD wallets, BIP32/39/44)
- Address generation
- Transaction creation and signing
- UTXO selection (coin selection algorithms)
- Transaction history tracking
- Watch-only wallet support

**Rust Implementation:**
```
src/wallet/
├── mod.rs              # Wallet manager
├── keystore.rs         # Key storage (encrypted)
├── hd.rs               # HD wallet (BIP32/39/44)
├── tx_builder.rs       # Transaction construction
├── coin_selection.rs   # UTXO selection algorithms
├── signer.rs           # Transaction signing
├── db.rs               # Wallet database (SQLite?)
└── scriptpubkey.rs     # Script tracking
```

**Key Libraries:**
- `bdk` (Bitcoin Dev Kit) - consider using or taking inspiration
- `bitcoin` crate for primitives
- `secp256k1` for signing

### 4. Node Coordination ⭐ MEDIUM PRIORITY
**Source:** `src/node/*.cpp`, parts of `src/init.cpp`

**Responsibilities:**
- Startup/shutdown logic
- Component coordination
- Configuration management
- Signal handling

**Rust Implementation:**
```
src/node/
├── mod.rs              # Node manager
├── config.rs           # Configuration parsing
├── context.rs          # Shared application context
└── shutdown.rs         # Graceful shutdown handling
```

### 5. Mempool Management 🤔 DEPENDS ON KERNEL
**Source:** `src/txmempool.cpp`, `src/validation.cpp` (parts)

**Status:** Might be included in libbitcoinkernel
**Check:** Whether kernel exposes mempool or if you need to implement

If needed:
```
src/mempool/
├── mod.rs              # Mempool manager
├── policy.rs           # Transaction policies
├── fee_estimation.rs   # Fee estimation
└── eviction.rs         # Mempool eviction logic
```

### 6. Mining Interface (Optional) ⭐ LOW PRIORITY
**Source:** `src/miner.cpp`

**Responsibilities:**
- Block template generation
- Transaction selection for blocks
- Mining RPC methods

### 7. Utilities ⭐ LOW PRIORITY
**Source:** `src/util/*.cpp`

Most utilities can use Rust standard library or existing crates:
- Logging: `tracing` or `log` crate
- Threading: `tokio` for async, `rayon` for parallel
- Time: `chrono` crate
- File I/O: Rust std

## Implementation Strategy

### Phase 1: Core Infrastructure (Weeks 1-4)
1. ✅ Set up libbitcoinkernel FFI bindings (DONE)
2. Enhanced P2P message protocol implementation
3. Basic peer connection management
4. RPC framework with essential methods

### Phase 2: P2P Network (Weeks 5-8)
1. Address manager (addrman)
2. Peer discovery (DNS seeds)
3. Block relay and download
4. Transaction relay
5. Peer scoring and eviction

### Phase 3: Wallet (Weeks 9-14)
1. Key storage and HD wallet
2. Address generation
3. UTXO tracking
4. Transaction building
5. Coin selection algorithms
6. Wallet RPC methods

### Phase 4: Integration & Testing (Weeks 15-18)
1. Integration tests
2. Compatibility testing with Bitcoin Core
3. Performance benchmarking
4. Bug fixes and optimization

### Phase 5: Advanced Features (Weeks 19+)
1. REST API
2. ZMQ notifications (optional)
3. Mining support
4. Advanced wallet features
5. GUI (optional)

## Suggested Crate Dependencies

```toml
[dependencies]
# Networking
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["codec"] }

# Bitcoin primitives
bitcoin = "0.32"
bitcoinconsensus = "0.21" # For script verification if needed
secp256k1 = "0.29"
bip39 = "2.0"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"

# Database
rusqlite = { version = "0.32", features = ["bundled"] }
# OR
sled = "0.34" # Alternative embedded DB

# Web/RPC
axum = "0.7"
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }

# Utilities
anyhow = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
clap = { version = "4", features = ["derive"] }
hex = "0.4"
bs58 = "0.5"
rand = "0.8"

# Crypto (if not using bitcoin crate)
sha2 = "0.10"
ripemd = "0.1"

# FFI
bindgen = "0.69" # build-time

[dev-dependencies]
tempfile = "3"
proptest = "1"
criterion = "0.5"
```

## File Structure

```
btck-mini-node/
├── Cargo.toml
├── build.rs                 # FFI bindings generation
├── src/
│   ├── main.rs             # Entry point
│   ├── ffi.rs              # Kernel FFI wrapper
│   ├── lib.rs              # Library exports
│   ├── config.rs           # Configuration
│   ├── p2p/                # P2P networking
│   │   ├── mod.rs
│   │   ├── peer.rs
│   │   ├── addrman.rs
│   │   ├── protocol.rs
│   │   ├── messages.rs
│   │   ├── relay.rs
│   │   └── handshake.rs
│   ├── rpc/                # RPC server
│   │   ├── mod.rs
│   │   ├── blockchain.rs
│   │   ├── network.rs
│   │   ├── wallet.rs
│   │   └── mining.rs
│   ├── wallet/             # Wallet implementation
│   │   ├── mod.rs
│   │   ├── keystore.rs
│   │   ├── hd.rs
│   │   ├── tx_builder.rs
│   │   ├── coin_selection.rs
│   │   └── db.rs
│   ├── node/               # Node coordination
│   │   ├── mod.rs
│   │   ├── context.rs
│   │   └── shutdown.rs
│   └── util/               # Utilities
│       ├── mod.rs
│       ├── logging.rs
│       └── seeds.rs
├── tests/                  # Integration tests
└── benches/                # Benchmarks
```

## Testing Strategy

### Unit Tests
- Test each module independently
- Mock kernel interactions
- Property-based testing with proptest

### Integration Tests
- Full node startup/shutdown
- P2P message exchange
- Block download and validation
- Wallet operations

### Compatibility Tests
- Interoperability with Bitcoin Core nodes
- RPC API compatibility
- Network protocol compliance

### Performance Tests
- Block processing throughput
- P2P message handling
- Wallet operation speed

## Key Bitcoin Core Files Reference

### Must Study (for conversion):
1. `src/net.h` / `src/net.cpp` - Network layer
2. `src/net_processing.h` / `src/net_processing.cpp` - P2P logic
3. `src/addrman.h` / `src/addrman.cpp` - Address manager
4. `src/protocol.h` / `src/protocol.cpp` - Protocol constants
5. `src/rpc/*.cpp` - All RPC implementations
6. `src/wallet/wallet.h` / `src/wallet/wallet.cpp` - Wallet core
7. `src/wallet/coinselection.h` - Coin selection

### Useful Reference:
- `src/primitives/*.h` - Data structures (already in `bitcoin` crate)
- `src/consensus/*.h` - Consensus params (in kernel)
- `src/script/*.h` - Script (in kernel/bitcoin crate)

## Resources

- Bitcoin Core source: https://github.com/bitcoin/bitcoin
- libbitcoinkernel project: https://github.com/bitcoin/bitcoin/issues/27587
- BIPs: https://github.com/bitcoin/bips
- Bitcoin Dev Kit: https://github.com/bitcoindevkit/bdk
- rust-bitcoin: https://github.com/rust-bitcoin/rust-bitcoin

## Next Steps

1. **Implement enhanced P2P protocol messages** (I'll provide code)
2. **Create proper peer manager with addrman**
3. **Implement block download logic**
4. **Add comprehensive RPC methods**
5. **Start wallet implementation**

Would you like me to start with any specific component?

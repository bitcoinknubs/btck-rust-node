# Quick Start Integration Guide

## Overview

You're building a Rust-based Bitcoin node that uses `libbitcoinkernel` via FFI for consensus/validation, while implementing P2P networking, RPC, and other components in Rust.

## Files Provided

1. **CONVERSION_ROADMAP.md** - Complete conversion strategy and architecture
2. **p2p_protocol.rs** - Bitcoin protocol message implementation
3. **peer_connection.rs** - Individual peer connection management
4. **addrman.rs** - Address manager for peer discovery
5. **rpc_server.rs** - Comprehensive RPC server implementation

## Integration Steps

### Step 1: Update Your Cargo.toml

```toml
[package]
name = "btck-mini-node"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow = "1"
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["codec"] }
axum = "0.7"  # Upgrade from 0.6
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "trace"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bitcoin = "0.32"
rand = "0.8"
tracing = "0.1"
tracing-subscriber = "0.3"

[build-dependencies]
bindgen = "0.69"

[profile.release]
lto = true
codegen-units = 1
```

### Step 2: Reorganize Your Project Structure

```
btck-mini-node/
├── Cargo.toml
├── build.rs                    # Your existing FFI build script
├── src/
│   ├── main.rs                 # Entry point
│   ├── lib.rs                  # Library exports (new)
│   ├── ffi.rs                  # Your existing kernel FFI
│   ├── config.rs               # Configuration (new)
│   ├── p2p/
│   │   ├── mod.rs              # Re-export modules
│   │   ├── protocol.rs         # [USE: p2p_protocol.rs]
│   │   ├── peer.rs             # [USE: peer_connection.rs]
│   │   ├── addrman.rs          # [USE: addrman.rs]
│   │   ├── manager.rs          # Enhanced peer manager
│   │   └── relay.rs            # Block/tx relay logic (new)
│   ├── rpc/
│   │   ├── mod.rs              # [USE: rpc_server.rs]
│   │   └── auth.rs             # Auth middleware (new)
│   ├── node/
│   │   ├── mod.rs              # Node coordination
│   │   └── context.rs          # Shared app context
│   └── util/
│       ├── mod.rs
│       └── seeds.rs            # Your existing DNS seeds
└── tests/
    └── integration_test.rs
```

### Step 3: Implement Kernel Interface

Update your `ffi.rs` to implement the `KernelInterface` trait:

```rust
// In ffi.rs
use anyhow::Result;

impl crate::rpc::KernelInterface for Kernel {
    fn get_block_count(&self) -> Result<i32> {
        self.active_height()
    }

    fn get_best_block_hash(&self) -> Result<String> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                return Ok("000000000...".to_string());
            }
            // TODO: Get actual block hash from chain
            Ok("000000000...".to_string())
        }
    }
}
```

### Step 4: Create Enhanced P2P Module

Create `src/p2p/mod.rs`:

```rust
pub mod protocol;
pub mod peer;
pub mod addrman;
pub mod manager;

pub use protocol::{MessageBuilder, MessageCodec, get_magic, get_default_port};
pub use peer::{PeerConnection, PeerInfo, PeerEvent, PeerState};
pub use addrman::{AddrMan, AddressInfo};
pub use manager::PeerManager;
```

Create `src/p2p/manager.rs` (enhanced version of your existing p2p.rs):

```rust
use super::*;
use anyhow::Result;
use bitcoin::Network;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
};
use tokio::sync::{mpsc, RwLock};

pub struct PeerManager {
    network: Network,
    user_agent: String,
    addrman: Arc<RwLock<AddrMan>>,
    peers: Arc<RwLock<HashMap<SocketAddr, PeerConnection>>>,
    event_rx: mpsc::UnboundedReceiver<PeerEvent>,
    event_tx: mpsc::UnboundedSender<PeerEvent>,
    block_processor: Option<Box<dyn Fn(&[u8]) -> Result<()> + Send + Sync>>,
}

impl PeerManager {
    pub fn new(network: Network, user_agent: &str) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            network,
            user_agent: user_agent.to_string(),
            addrman: Arc::new(RwLock::new(AddrMan::new(network))),
            peers: Arc::new(RwLock::new(HashMap::new())),
            event_rx: rx,
            event_tx: tx,
            block_processor: None,
        }
    }

    pub fn with_block_processor<F>(mut self, processor: F) -> Self
    where
        F: Fn(&[u8]) -> Result<()> + Send + Sync + 'static,
    {
        self.block_processor = Some(Box::new(processor));
        self
    }

    pub async fn add_outbound(&mut self, addr: SocketAddr, start_height: i32) -> Result<()> {
        // Connect and add peer
        let stream = tokio::net::TcpStream::connect(addr).await?;
        let mut peer = PeerConnection::new(
            addr,
            self.network,
            stream,
            false,
            self.event_tx.clone(),
        );

        let our_addr = "0.0.0.0:0".parse().unwrap();
        
        // Spawn peer task
        tokio::spawn(async move {
            if let Err(e) = peer.run(our_addr, start_height).await {
                eprintln!("[peer] error: {}", e);
            }
        });

        Ok(())
    }

    pub async fn bootstrap(&mut self) -> Result<()> {
        // Use DNS seeds
        let seeds = crate::util::seeds::dns_seeds(self.network);
        
        for seed in seeds.iter().take(3) {
            // Resolve DNS and add peers
            // Implementation depends on async DNS resolution
            println!("[bootstrap] Querying seed: {}", seed);
        }

        Ok(())
    }

    pub async fn event_loop(&mut self) -> Result<()> {
        loop {
            if let Some(event) = self.event_rx.recv().await {
                self.handle_event(event).await?;
            }
        }
    }

    async fn handle_event(&mut self, event: PeerEvent) -> Result<()> {
        match event {
            PeerEvent::Connected(info) => {
                println!("[peer] Connected: {}", info.addr);
                self.addrman.write().await.mark_good(&info.addr);
            }
            PeerEvent::Disconnected(addr, reason) => {
                println!("[peer] Disconnected {}: {}", addr, reason);
                self.peers.write().await.remove(&addr);
            }
            PeerEvent::Block(data) => {
                if let Some(ref processor) = self.block_processor {
                    if let Err(e) = processor(&data) {
                        eprintln!("[peer] block process error: {}", e);
                    }
                }
            }
            PeerEvent::Addr(addrs) => {
                let mut addrman = self.addrman.write().await;
                addrman.import_addresses(addrs);
            }
            _ => {}
        }
        Ok(())
    }

    pub async fn peers_len(&self) -> usize {
        self.peers.read().await.len()
    }
}
```

### Step 5: Update main.rs

```rust
use anyhow::{Context, Result};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

mod ffi;
mod p2p;
mod rpc;
mod node;
mod util;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "signet")]
    chain: String,
    #[arg(long, default_value = "./data")]
    datadir: PathBuf,
    #[arg(long, default_value = "./blocks")]
    blocksdir: PathBuf,
    #[arg(long, default_value = "127.0.0.1:38332")]
    rpc: String,
    #[arg(long)]
    peer: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    // Initialize kernel
    let kernel = Arc::new(ffi::Kernel::new(&args.chain, &args.datadir, &args.blocksdir)?);

    // Setup RPC
    let network = match args.chain.as_str() {
        "main" | "mainnet" => bitcoin::Network::Bitcoin,
        "testnet" => bitcoin::Network::Testnet,
        "signet" => bitcoin::Network::Signet,
        _ => bitcoin::Network::Regtest,
    };

    let rpc_state = rpc::RpcState {
        kernel: kernel.clone(),
        network,
    };

    let rpc_app = rpc::create_rpc_router(rpc_state);

    // Start P2P
    let k = kernel.clone();
    tokio::spawn(async move {
        let process_block = move |raw: &[u8]| -> Result<()> {
            // Process block through kernel
            // ... your existing logic ...
            Ok(())
        };

        let mut pm = p2p::PeerManager::new(network, "/btck-mini-node:0.1/")
            .with_block_processor(process_block);

        for peer_str in &args.peer {
            if let Ok(addr) = peer_str.parse() {
                let _ = pm.add_outbound(addr, 0).await;
            }
        }

        if pm.peers_len().await < 2 {
            let _ = pm.bootstrap().await;
        }

        if let Err(e) = pm.event_loop().await {
            eprintln!("[p2p] error: {}", e);
        }
    });

    // Start RPC server
    let rpc_addr: SocketAddr = args.rpc.parse()?;
    println!("RPC listening on http://{}", rpc_addr);

    let listener = tokio::net::TcpListener::bind(rpc_addr).await?;
    axum::serve(listener, rpc_app).await?;

    Ok(())
}
```

## Testing Your Node

### Start the Node

```bash
cargo run -- --chain=signet --rpc=127.0.0.1:38332
```

### Test RPC Endpoints

```bash
# Get block count
curl -X POST http://127.0.0.1:38332 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getblockcount","params":[]}'

# Get blockchain info
curl -X POST http://127.0.0.1:38332 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getblockchaininfo","params":[]}'

# Get network info
curl -X POST http://127.0.0.1:38332 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"getnetworkinfo","params":[]}'
```

## Next Steps

1. **Implement Missing Kernel Methods**
   - `get_best_block_hash()`
   - `get_block()` 
   - `get_block_hash()`

2. **Enhance P2P**
   - Add block download logic
   - Implement header sync
   - Add transaction relay

3. **Add Wallet**
   - Key management
   - Transaction building
   - UTXO tracking

4. **Testing**
   - Unit tests for each module
   - Integration tests with testnet
   - Performance benchmarks

## Common Issues & Solutions

### Issue: Kernel FFI Errors
**Solution**: Ensure libbitcoinkernel is properly compiled and installed. Check paths in build.rs.

### Issue: P2P Connection Failures
**Solution**: Check firewall settings, verify network connectivity, try different DNS seeds.

### Issue: RPC Not Responding
**Solution**: Verify Axum version compatibility, check address binding, review logs.

## Resources

- Bitcoin Core source: https://github.com/bitcoin/bitcoin
- rust-bitcoin docs: https://docs.rs/bitcoin/
- libbitcoinkernel project: https://github.com/bitcoin/bitcoin/issues/27587

## Need Help?

Review the CONVERSION_ROADMAP.md for detailed architecture and implementation strategy.

Happy coding! 🚀

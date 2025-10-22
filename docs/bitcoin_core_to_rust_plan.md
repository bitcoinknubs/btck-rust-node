# Bitcoin Core to Rust Conversion Plan

## 프로젝트 개요
Bitcoin Core의 코드를 Rust로 변환. libbitcoinkernel은 FFI로 유지하고 나머지를 Rust로 재구현.

## 아키텍처 개요

```
┌─────────────────────────────────────────────┐
│         Rust Implementation Layer           │
├─────────────────────────────────────────────┤
│  RPC Server  │  P2P Network  │  Wallet      │
│  (Axum)      │  (Tokio)      │  (Custom)    │
├─────────────────────────────────────────────┤
│           FFI Bindings (bindgen)            │
├─────────────────────────────────────────────┤
│      libbitcoinkernel (C++ Library)         │
│  - Validation    - Consensus                │
│  - Block Chain   - UTXO Management          │
└─────────────────────────────────────────────┘
```

## Bitcoin Core 디렉토리 구조 분석

### 유지 (FFI 사용)
```
src/kernel/          → libbitcoinkernel로 유지
src/consensus/       → libbitcoinkernel에 포함
src/script/          → libbitcoinkernel에 포함
src/primitives/      → libbitcoinkernel에 포함
```

### Rust로 변환 대상
```
src/net.cpp          → mod network
src/net_processing.cpp → mod net_processing
src/rpc/             → mod rpc
src/wallet/          → mod wallet
src/txmempool.cpp    → mod mempool
src/addrman.cpp      → mod addrman
src/bloom.cpp        → mod bloom
src/policy/          → mod policy
src/init.cpp         → main.rs
src/httprpc.cpp      → mod rpc/server
src/httpserver.cpp   → mod rpc/server
src/rest.cpp         → mod rpc/rest
src/torcontrol.cpp   → mod tor
src/i2p.cpp          → mod i2p
src/node/            → mod node
src/index/           → mod index
src/dbwrapper.cpp    → mod db
src/util/            → mod util
```

## 상세 모듈 변환 계획

### 1. 네트워크 레이어 (src/net.cpp → mod network)

**주요 구성요소:**
- CNode: 개별 피어 연결 관리
- CConnman: 전체 네트워크 연결 관리자
- Socket 관리
- 메시지 송수신

**Rust 구현:**
```rust
// src/network/mod.rs
pub mod node;
pub mod connman;
pub mod message;
pub mod socket;

pub struct Node {
    id: u64,
    addr: SocketAddr,
    stream: TcpStream,
    services: u64,
    version: i32,
    // ...
}

pub struct ConnectionManager {
    nodes: HashMap<u64, Arc<RwLock<Node>>>,
    max_outbound: usize,
    max_inbound: usize,
    // ...
}
```

### 2. 메시지 처리 (src/net_processing.cpp → mod net_processing)

**주요 구성요소:**
- ProcessMessage: 수신 메시지 처리
- SendMessages: 메시지 전송
- Misbehavior tracking
- Block/Transaction relay

**Rust 구현:**
```rust
// src/net_processing/mod.rs
pub struct PeerManager {
    peers: HashMap<u64, PeerState>,
    orphan_txs: HashMap<Txid, Transaction>,
    // ...
}

impl PeerManager {
    pub async fn process_message(&mut self, peer_id: u64, msg: NetworkMessage) -> Result<()> {
        match msg {
            NetworkMessage::Version(v) => self.handle_version(peer_id, v).await,
            NetworkMessage::Inv(inv) => self.handle_inv(peer_id, inv).await,
            NetworkMessage::GetData(gd) => self.handle_getdata(peer_id, gd).await,
            NetworkMessage::Block(block) => self.handle_block(peer_id, block).await,
            // ...
        }
    }
}
```

### 3. RPC 서버 (src/rpc/ → mod rpc)

**Bitcoin Core RPC 카테고리:**
- Blockchain
- Control
- Generating
- Mining
- Network
- Rawtransactions
- Util
- Wallet
- Zmq

**Rust 구현 (Axum 기반):**
```rust
// src/rpc/mod.rs
pub mod blockchain;
pub mod network;
pub mod mining;
pub mod wallet;
pub mod util;

// src/rpc/server.rs
pub async fn start_rpc_server(
    addr: SocketAddr,
    kernel: Arc<Kernel>,
    network: Arc<NetworkManager>,
) -> Result<()> {
    let app = Router::new()
        // Blockchain
        .route("/getblockchaininfo", post(blockchain::getblockchaininfo))
        .route("/getbestblockhash", post(blockchain::getbestblockhash))
        .route("/getblock", post(blockchain::getblock))
        .route("/getblockcount", post(blockchain::getblockcount))
        .route("/getblockhash", post(blockchain::getblockhash))
        
        // Network
        .route("/getpeerinfo", post(network::getpeerinfo))
        .route("/addnode", post(network::addnode))
        .route("/getnetworkinfo", post(network::getnetworkinfo))
        
        // Mining
        .route("/getmininginfo", post(mining::getmininginfo))
        .route("/getblocktemplate", post(mining::getblocktemplate))
        .route("/submitblock", post(mining::submitblock))
        
        // Wallet
        .route("/getbalance", post(wallet::getbalance))
        .route("/sendtoaddress", post(wallet::sendtoaddress))
        .route("/getnewaddress", post(wallet::getnewaddress))
        
        .with_state(AppState { kernel, network });
    
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;
    
    Ok(())
}
```

### 4. 지갑 (src/wallet/ → mod wallet)

**주요 구성요소:**
- CWallet: 지갑 메인 클래스
- Key management
- Address book
- Transaction creation
- Fee estimation

**Rust 구현:**
```rust
// src/wallet/mod.rs
pub mod keys;
pub mod db;
pub mod tx;

pub struct Wallet {
    keys: KeyStore,
    addresses: AddressBook,
    db: WalletDB,
    utxos: HashMap<OutPoint, TxOut>,
}

impl Wallet {
    pub fn create_transaction(
        &mut self,
        recipients: Vec<(Address, Amount)>,
        fee_rate: FeeRate,
    ) -> Result<Transaction> {
        // Coin selection
        // Input selection
        // Change output
        // Sign transaction
    }
    
    pub fn sign_transaction(&self, tx: &mut Transaction) -> Result<()> {
        // Use kernel FFI for script verification
    }
}
```

### 5. Mempool (src/txmempool.cpp → mod mempool)

**주요 구성요소:**
- CTxMemPool: 메모리풀 관리
- Fee estimation
- Transaction prioritization
- Ancestor/descendant tracking

**Rust 구현:**
```rust
// src/mempool/mod.rs
pub struct MemPool {
    txs: HashMap<Txid, MemPoolEntry>,
    fee_estimator: FeeEstimator,
    config: MemPoolConfig,
}

pub struct MemPoolEntry {
    tx: Transaction,
    fee: Amount,
    time: SystemTime,
    height: i32,
    ancestors: HashSet<Txid>,
    descendants: HashSet<Txid>,
}

impl MemPool {
    pub async fn add_tx(&mut self, tx: Transaction, kernel: &Kernel) -> Result<()> {
        // Validate via kernel FFI
        // Check conflicts
        // Update fee estimator
        // Update ancestor/descendant sets
    }
    
    pub fn remove_tx(&mut self, txid: &Txid) {
        // Remove from pool
        // Update descendants
    }
    
    pub fn get_block_template(&self, max_weight: u64) -> Vec<Transaction> {
        // Select transactions for mining
        // Sort by fee rate
        // Respect ancestor limits
    }
}
```

### 6. Address Manager (src/addrman.cpp → mod addrman)

**주요 구성요소:**
- CAddrMan: 피어 주소 관리
- Tried/New table
- Address selection

**Rust 구현:**
```rust
// src/addrman/mod.rs
pub struct AddrMan {
    tried: HashMap<NetAddr, AddrInfo>,
    new: HashMap<NetAddr, AddrInfo>,
    random_state: RandomState,
}

pub struct AddrInfo {
    addr: NetAddr,
    source: NetAddr,
    last_success: SystemTime,
    last_try: SystemTime,
    attempts: u32,
    services: u64,
}

impl AddrMan {
    pub fn select(&mut self) -> Option<NetAddr> {
        // Select address from tried/new tables
        // Implement feeler connections
    }
    
    pub fn add(&mut self, addr: NetAddr, source: NetAddr) {
        // Add to new table
        // Move to tried on successful connection
    }
}
```

### 7. Policy (src/policy/ → mod policy)

**주요 구성요소:**
- Fee estimation
- RBF (Replace-By-Fee)
- Transaction relay policy
- Block assembly

**Rust 구현:**
```rust
// src/policy/mod.rs
pub mod fees;
pub mod rbf;
pub mod policy;

pub struct Policy {
    min_relay_fee: FeeRate,
    dust_relay_fee: FeeRate,
    max_tx_size: usize,
}

impl Policy {
    pub fn is_standard(&self, tx: &Transaction) -> Result<()> {
        // Check if transaction is standard
        // Validate inputs/outputs
        // Check size limits
    }
    
    pub fn check_rbf(&self, old_tx: &Transaction, new_tx: &Transaction) -> Result<()> {
        // Verify RBF rules
        // BIP 125 compliance
    }
}
```

### 8. Indexing (src/index/ → mod index)

**주요 구성요소:**
- TxIndex: Transaction index
- BlockFilterIndex: BIP 157/158
- CoinStatsIndex

**Rust 구현:**
```rust
// src/index/mod.rs
pub mod txindex;
pub mod blockfilter;
pub mod coinstats;

pub trait Index {
    async fn sync(&mut self, kernel: &Kernel) -> Result<()>;
    async fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>>;
}

pub struct TxIndex {
    db: Database,
    synced_height: i32,
}

impl TxIndex {
    pub async fn get_transaction(&self, txid: &Txid) -> Result<Option<Transaction>> {
        // Query database
        // Return transaction
    }
}
```

### 9. Database Wrapper (src/dbwrapper.cpp → mod db)

**Rust 구현 (RocksDB 사용):**
```rust
// src/db/mod.rs
use rocksdb::{DB, Options, WriteBatch};

pub struct Database {
    db: DB,
    path: PathBuf,
}

impl Database {
    pub fn open(path: PathBuf) -> Result<Self> {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        let db = DB::open(&opts, &path)?;
        Ok(Self { db, path })
    }
    
    pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.db.get(key)?)
    }
    
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.db.put(key, value)?;
        Ok(())
    }
    
    pub fn batch(&self) -> WriteBatch {
        WriteBatch::default()
    }
}
```

### 10. Utilities (src/util/ → mod util)

**주요 구성요소:**
- Time utilities
- String utilities
- System utilities
- Thread utilities
- Memory utilities

**Rust 구현:**
```rust
// src/util/mod.rs
pub mod time;
pub mod string;
pub mod system;
pub mod thread;

// src/util/time.rs
pub fn get_time() -> SystemTime {
    SystemTime::now()
}

pub fn get_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

// src/util/thread.rs
pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<Job>,
}

impl ThreadPool {
    pub fn new(size: usize) -> Self {
        // Create worker threads
    }
    
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        // Submit job to pool
    }
}
```

## 프로젝트 구조

```
btck-rust-node/
├── Cargo.toml
├── build.rs                 # FFI 바인딩 빌드
├── src/
│   ├── main.rs             # 엔트리포인트
│   ├── lib.rs              # 라이브러리 루트
│   ├── ffi.rs              # FFI 바인딩
│   │
│   ├── kernel/             # Kernel wrapper
│   │   ├── mod.rs
│   │   ├── chainman.rs
│   │   └── validation.rs
│   │
│   ├── network/            # P2P 네트워킹
│   │   ├── mod.rs
│   │   ├── node.rs
│   │   ├── connman.rs
│   │   ├── message.rs
│   │   └── socket.rs
│   │
│   ├── net_processing/     # 메시지 처리
│   │   ├── mod.rs
│   │   ├── peer.rs
│   │   └── relay.rs
│   │
│   ├── rpc/                # RPC 서버
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   ├── blockchain.rs
│   │   ├── network.rs
│   │   ├── mining.rs
│   │   ├── wallet.rs
│   │   └── util.rs
│   │
│   ├── wallet/             # 지갑
│   │   ├── mod.rs
│   │   ├── keys.rs
│   │   ├── db.rs
│   │   ├── tx.rs
│   │   └── feebumper.rs
│   │
│   ├── mempool/            # Mempool
│   │   ├── mod.rs
│   │   ├── entry.rs
│   │   └── fees.rs
│   │
│   ├── addrman/            # Address manager
│   │   ├── mod.rs
│   │   └── addrinfo.rs
│   │
│   ├── policy/             # Policy
│   │   ├── mod.rs
│   │   ├── fees.rs
│   │   ├── rbf.rs
│   │   └── policy.rs
│   │
│   ├── index/              # Indexing
│   │   ├── mod.rs
│   │   ├── txindex.rs
│   │   ├── blockfilter.rs
│   │   └── coinstats.rs
│   │
│   ├── db/                 # Database
│   │   ├── mod.rs
│   │   └── rocksdb.rs
│   │
│   └── util/               # Utilities
│       ├── mod.rs
│       ├── time.rs
│       ├── string.rs
│       └── system.rs
│
├── tests/                  # 통합 테스트
│   ├── network_tests.rs
│   ├── rpc_tests.rs
│   └── wallet_tests.rs
│
└── benches/                # 벤치마크
    └── network_bench.rs
```

## 의존성 (Cargo.toml)

```toml
[package]
name = "btck-rust-node"
version = "0.1.0"
edition = "2021"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["codec"] }

# Web framework
axum = "0.6"
tower = "0.4"
tower-http = { version = "0.4", features = ["trace", "cors"] }

# Bitcoin
bitcoin = "0.32"
bitcoin_hashes = "0.14"
secp256k1 = { version = "0.29", features = ["rand"] }

# Network
tokio-stream = "0.1"

# Database
rocksdb = "0.22"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
bincode = "1"

# Error handling
anyhow = "1"
thiserror = "1"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Crypto
rand = "0.8"
sha2 = "0.10"

# Utils
chrono = "0.4"
hex = "0.4"
base64 = "0.22"

[build-dependencies]
bindgen = "0.69"

[dev-dependencies]
criterion = "0.5"
proptest = "1"

[profile.release]
lto = true
codegen-units = 1
opt-level = 3

[profile.bench]
inherits = "release"
```

## 단계별 구현 계획

### Phase 1: 기본 인프라 (1-2주)
- [x] FFI 바인딩 기본 설정 (완료)
- [ ] Kernel wrapper 완성
- [ ] 기본 RPC 서버 구조
- [ ] 로깅 시스템
- [ ] 설정 파일 파싱

### Phase 2: 네트워크 레이어 (2-3주)
- [x] P2P 프로토콜 기본 구현 (부분 완료)
- [ ] CNode 구현
- [ ] CConnman 구현
- [ ] 메시지 직렬화/역직렬화
- [ ] DNS seeding 완성
- [ ] Peer discovery

### Phase 3: 메시지 처리 (2-3주)
- [ ] PeerManager 구현
- [ ] Version handshake
- [ ] Inv/GetData 처리
- [ ] Block relay
- [ ] Transaction relay
- [ ] Ping/Pong

### Phase 4: Mempool (1-2주)
- [ ] 기본 mempool 구조
- [ ] Transaction validation (via FFI)
- [ ] Fee estimation
- [ ] Ancestor/descendant tracking
- [ ] Eviction policy

### Phase 5: RPC 구현 (2-3주)
- [ ] Blockchain RPC
- [ ] Network RPC
- [ ] Mining RPC
- [ ] Raw transaction RPC
- [ ] Util RPC

### Phase 6: 지갑 (3-4주)
- [ ] Key management
- [ ] Address generation
- [ ] UTXO tracking
- [ ] Transaction creation
- [ ] Signing
- [ ] Wallet database

### Phase 7: Indexing (1-2주)
- [ ] TxIndex
- [ ] BlockFilter index
- [ ] CoinStats index

### Phase 8: 최적화 및 테스트 (2-3주)
- [ ] 성능 프로파일링
- [ ] 메모리 최적화
- [ ] 통합 테스트
- [ ] Fuzz testing
- [ ] Stress testing

## 핵심 FFI 인터페이스

```rust
// src/kernel/mod.rs
use crate::ffi::*;

pub struct Kernel {
    ctx: *mut btck_Context,
    chainman: *mut btck_ChainstateManager,
    chain_params: *mut btck_ChainParameters,
}

impl Kernel {
    pub fn validate_block(&self, block: &bitcoin::Block) -> Result<bool> {
        let raw = bitcoin::consensus::serialize(block);
        
        unsafe {
            let block_ptr = btck_block_create(
                raw.as_ptr() as *const std::ffi::c_void,
                raw.len()
            );
            
            if block_ptr.is_null() {
                return Err(anyhow::anyhow!("Failed to create block"));
            }
            
            let mut new_block: c_int = 0;
            let rc = btck_chainstate_manager_process_block(
                self.chainman,
                block_ptr,
                &mut new_block as *mut c_int,
            );
            
            btck_block_destroy(block_ptr);
            
            Ok(rc == 0)
        }
    }
    
    pub fn validate_transaction(&self, tx: &bitcoin::Transaction) -> Result<bool> {
        // Similar FFI call for transaction validation
        todo!()
    }
    
    pub fn get_best_block_hash(&self) -> Result<bitcoin::BlockHash> {
        unsafe {
            let chain = btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                return Err(anyhow::anyhow!("No active chain"));
            }
            
            let tip = btck_chain_get_tip(chain);
            if tip.is_null() {
                return Err(anyhow::anyhow!("No chain tip"));
            }
            
            let mut hash = [0u8; 32];
            btck_block_index_get_block_hash(tip, hash.as_mut_ptr());
            
            Ok(bitcoin::BlockHash::from_byte_array(hash))
        }
    }
}
```

## 테스트 전략

### 단위 테스트
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_network_message_serialization() {
        let msg = NetworkMessage::Version(/* ... */);
        let serialized = serialize(&msg);
        let deserialized = deserialize(&serialized).unwrap();
        assert_eq!(msg, deserialized);
    }
    
    #[tokio::test]
    async fn test_peer_connection() {
        let mut connman = ConnectionManager::new();
        let addr = "127.0.0.1:8333".parse().unwrap();
        connman.connect(addr).await.unwrap();
        assert_eq!(connman.num_peers(), 1);
    }
}
```

### 통합 테스트
```rust
// tests/integration_test.rs
#[tokio::test]
async fn test_full_node_sync() {
    // Start regtest node
    // Connect to peer
    // Sync blocks
    // Verify chainstate
}
```

### Fuzz 테스트
```rust
// fuzz/fuzz_targets/network_messages.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = deserialize_network_message(data);
});
```

## 성능 고려사항

1. **Zero-copy 직렬화**: 가능한 경우 bytes crate 활용
2. **비동기 I/O**: tokio 전면 사용
3. **병렬 처리**: rayon으로 블록 검증 병렬화
4. **메모리 풀링**: object pool 패턴
5. **캐싱**: LRU 캐시로 자주 접근하는 데이터 캐싱

## 다음 단계

1. **현재 skeleton 확장**
   - p2p.rs의 메시지 처리 완성
   - RPC 엔드포인트 추가
   - 에러 처리 개선

2. **Bitcoin Core 코드 분석**
   - 각 모듈의 상세 로직 파악
   - 의존성 그래프 작성
   - 변환 우선순위 결정

3. **프로토타입 개발**
   - 핵심 기능부터 구현
   - 점진적 확장
   - 지속적 테스트

이 계획을 바탕으로 단계별로 구현하시면 됩니다!

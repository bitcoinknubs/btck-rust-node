# Bitcoin Core to Rust 변환 프로젝트 - 구현 가이드

## 📋 프로젝트 개요

Bitcoin Core의 C++ 코드베이스를 Rust로 변환하는 프로젝트입니다. 
**libbitcoinkernel**은 FFI를 통해 그대로 사용하고, 나머지 컴포넌트들을 Rust로 재구현합니다.

## 🎯 핵심 목표

1. **성능**: Rust의 zero-cost abstractions 활용
2. **안전성**: 메모리 안전성과 스레드 안전성 보장
3. **호환성**: Bitcoin Core와 100% 프로토콜 호환
4. **유지보수성**: 명확한 모듈 구조와 타입 안전성

## 📁 생성된 파일 목록

### 1. `bitcoin_core_to_rust_plan.md`
- 전체 변환 계획서
- 단계별 구현 로드맵
- 아키텍처 다이어그램

### 2. `kernel_mod.rs`
- libbitcoinkernel FFI 래퍼
- 안전한 Rust 인터페이스 제공
- 블록 검증 및 체인 관리

### 3. `rpc_blockchain.rs`
- Blockchain RPC 메서드 구현
- getblockchaininfo, getblock, getblockhash 등
- Axum 기반 비동기 핸들러

### 4. `rpc_network.rs`
- Network RPC 메서드 구현
- getpeerinfo, addnode, setban 등
- 피어 관리 인터페이스

### 5. `network_connman.rs`
- 연결 관리자 (ConnectionManager)
- 피어 연결 및 해제
- 밴 시스템 구현

## 🏗️ 프로젝트 구조

```
btck-rust-node/
├── Cargo.toml
├── build.rs                    # FFI 바인딩 빌드
├── src/
│   ├── main.rs                # 엔트리포인트
│   ├── ffi.rs                 # libbitcoinkernel FFI
│   │
│   ├── kernel/                # ✅ 구현됨
│   │   └── mod.rs            # Kernel 래퍼
│   │
│   ├── network/               # 🚧 부분 구현
│   │   ├── mod.rs
│   │   ├── connman.rs        # ✅ ConnectionManager
│   │   ├── node.rs           # TODO: Node 구현
│   │   ├── message.rs        # TODO: 메시지 직렬화
│   │   └── addrman.rs        # TODO: 주소 관리
│   │
│   ├── rpc/                   # ✅ 구현됨
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   ├── blockchain.rs     # ✅ Blockchain RPCs
│   │   ├── network.rs        # ✅ Network RPCs
│   │   ├── mining.rs         # TODO
│   │   └── wallet.rs         # TODO
│   │
│   ├── mempool/               # TODO
│   │   ├── mod.rs
│   │   └── entry.rs
│   │
│   ├── wallet/                # TODO
│   │   ├── mod.rs
│   │   ├── keys.rs
│   │   └── db.rs
│   │
│   └── util/                  # TODO
│       ├── mod.rs
│       └── time.rs
│
└── tests/                     # TODO
    └── integration_tests.rs
```

## 🔧 핵심 구현 내용

### Kernel Module (`kernel_mod.rs`)

```rust
// 주요 기능:
- Kernel 초기화 및 설정
- 블록 처리 (process_block)
- 블록체인 정보 조회
- 블록 임포트

// 예제:
let kernel = Kernel::new(
    ChainType::Signet,
    &datadir,
    &blocksdir,
    false,  // persistent storage
)?;

let height = kernel.get_height()?;
let hash = kernel.get_best_block_hash()?;
let valid = kernel.process_block(&block)?;
```

### RPC Blockchain (`rpc_blockchain.rs`)

```rust
// 구현된 RPC 메서드:
- getblockchaininfo  ✅
- getbestblockhash   ✅
- getblockcount      ✅
- getblockhash       ✅
- getblock           🚧
- getchaintips       ✅
- getmempoolinfo     ✅
- gettxoutsetinfo    ✅

// 사용법:
let app = Router::new()
    .route("/getblockcount", post(blockchain::getblockcount))
    .route("/getbestblockhash", post(blockchain::getbestblockhash))
    .with_state(state);
```

### RPC Network (`rpc_network.rs`)

```rust
// 구현된 RPC 메서드:
- getnetworkinfo     ✅
- getpeerinfo        ✅
- getconnectioncount ✅
- addnode            ✅
- disconnectnode     ✅
- listbanned         ✅
- setban             ✅
- ping               ✅

// 사용법:
await client.post("/addnode")
    .json(&{"node": "1.2.3.4:8333", "command": "add"})
    .send()?;
```

### Connection Manager (`network_connman.rs`)

```rust
// 주요 기능:
- 아웃바운드 연결
- 인바운드 연결 수락
- 밴 시스템
- 네트워크 통계

// 예제:
let connman = ConnectionManager::new(config);
let node_id = connman.connect(addr).await?;
connman.ban_node("192.168.1.1", 86400, false).await?;
let peers = connman.get_peer_info().await;
```

## 🚀 다음 구현 단계

### Phase 1: 네트워크 레이어 완성 (우선순위: 높음)

#### 1.1 Node 구현 (`src/network/node.rs`)
```rust
pub struct Node {
    id: NodeId,
    addr: SocketAddr,
    stream: TcpStream,
    version: Option<VersionMessage>,
    services: u64,
    // ...
}

impl Node {
    // TODO: 구현 필요
    async fn send_version(&mut self) -> Result<()>;
    async fn receive_message(&mut self) -> Result<NetworkMessage>;
    async fn send_message(&mut self, msg: NetworkMessage) -> Result<()>;
}
```

#### 1.2 메시지 직렬화 (`src/network/message.rs`)
```rust
pub enum NetworkMessage {
    Version(VersionMessage),
    Verack,
    Addr(Vec<(u32, Address)>),
    Inv(Vec<Inventory>),
    GetData(Vec<Inventory>),
    Block(Block),
    Tx(Transaction),
    // ...
}

// TODO: bitcoin 크레이트의 네트워크 메시지 활용
// 또는 직접 구현
```

#### 1.3 주소 관리자 (`src/network/addrman.rs`)
```rust
pub struct AddrMan {
    tried: HashMap<NetAddr, AddrInfo>,
    new: HashMap<NetAddr, AddrInfo>,
}

impl AddrMan {
    // TODO: Bitcoin Core의 addrman.cpp 로직 포팅
    pub fn select(&mut self) -> Option<NetAddr>;
    pub fn add(&mut self, addr: NetAddr, source: NetAddr);
    pub fn mark_good(&mut self, addr: &NetAddr);
}
```

### Phase 2: Mempool 구현 (우선순위: 중간)

#### 2.1 기본 Mempool (`src/mempool/mod.rs`)
```rust
pub struct MemPool {
    txs: HashMap<Txid, MemPoolEntry>,
    by_fee: BTreeSet<(FeeRate, Txid)>,
    config: MemPoolConfig,
}

impl MemPool {
    // TODO: 구현 필요
    pub async fn add_tx(&mut self, tx: Transaction) -> Result<()>;
    pub fn remove_tx(&mut self, txid: &Txid);
    pub fn get_block_template(&self) -> Vec<Transaction>;
}
```

#### 2.2 Fee Estimator (`src/mempool/fees.rs`)
```rust
pub struct FeeEstimator {
    // TODO: Bitcoin Core의 fee estimation 알고리즘 포팅
}
```

### Phase 3: 지갑 구현 (우선순위: 낮음)

#### 3.1 키 관리 (`src/wallet/keys.rs`)
```rust
pub struct KeyStore {
    keys: HashMap<PublicKey, PrivateKey>,
    hd_chain: Option<ExtendedPrivKey>,
}

impl KeyStore {
    // TODO: BIP32/39/44 구현
    pub fn derive_key(&self, path: &DerivationPath) -> Result<PrivateKey>;
}
```

#### 3.2 UTXO 트래킹 (`src/wallet/mod.rs`)
```rust
pub struct Wallet {
    keys: KeyStore,
    utxos: HashMap<OutPoint, TxOut>,
    db: WalletDB,
}

impl Wallet {
    // TODO: 구현 필요
    pub fn create_transaction(&mut self, outputs: Vec<TxOut>) -> Result<Transaction>;
    pub fn sign_transaction(&self, tx: &mut Transaction) -> Result<()>;
}
```

### Phase 4: 인덱싱 (우선순위: 낮음)

```rust
// src/index/txindex.rs
pub struct TxIndex {
    db: Database,
}

// src/index/blockfilter.rs (BIP 157/158)
pub struct BlockFilterIndex {
    db: Database,
}
```

## 🧪 테스트 전략

### 단위 테스트
```rust
#[cfg(test)]
mod tests {
    #[test]
    fn test_kernel_creation() {
        let kernel = Kernel::new(...);
        assert!(kernel.is_ok());
    }
    
    #[tokio::test]
    async fn test_network_connection() {
        let connman = ConnectionManager::new(...);
        let result = connman.connect(addr).await;
        assert!(result.is_ok());
    }
}
```

### 통합 테스트
```rust
// tests/integration_test.rs
#[tokio::test]
async fn test_full_sync() {
    // 1. Start node
    // 2. Connect to regtest
    // 3. Generate blocks
    // 4. Verify sync
}
```

### Fuzz 테스트
```rust
// fuzz/fuzz_targets/network_messages.rs
fuzz_target!(|data: &[u8]| {
    let _ = deserialize_message(data);
});
```

## 📊 성능 고려사항

1. **Zero-copy 직렬화**
   - bytes 크레이트 활용
   - 불필요한 복사 최소화

2. **비동기 I/O**
   - tokio 전면 사용
   - 논블로킹 네트워크 I/O

3. **병렬 처리**
   - rayon으로 블록 검증 병렬화
   - 여러 코어 활용

4. **메모리 최적화**
   - 객체 풀링
   - LRU 캐시

## 🔒 보안 고려사항

1. **메모리 안전성**
   - FFI 경계에서 null 포인터 검증
   - unsafe 블록 최소화

2. **스레드 안전성**
   - Arc<RwLock<T>> 패턴
   - 데이터 레이스 방지

3. **입력 검증**
   - RPC 파라미터 검증
   - 네트워크 메시지 검증

## 📚 참고 자료

### Bitcoin Core 소스
- https://github.com/bitcoin/bitcoin
- src/net.cpp - 네트워크 레이어
- src/net_processing.cpp - 메시지 처리
- src/rpc/ - RPC 구현
- src/wallet/ - 지갑 구현

### Rust 크레이트
- bitcoin 0.32 - Bitcoin 타입 및 프로토콜
- tokio - 비동기 런타임
- axum - HTTP 서버
- rocksdb - 데이터베이스
- secp256k1 - 암호화

### 문서
- https://developer.bitcoin.org/
- https://en.bitcoin.it/wiki/
- Bitcoin Core RPC documentation

## 🎓 구현 팁

### 1. FFI 사용시 주의사항
```rust
// ❌ 나쁜 예
let ptr = unsafe { kernel_function() };
ptr.do_something();  // null 체크 없음

// ✅ 좋은 예
let ptr = unsafe { kernel_function() };
if ptr.is_null() {
    return Err(anyhow::anyhow!("Null pointer"));
}
unsafe { ptr.do_something() };
```

### 2. 비동기 코드 패턴
```rust
// kernel은 동기 함수이므로 spawn_blocking 사용
let result = tokio::task::spawn_blocking(move || {
    kernel.process_block(&block)
}).await??;
```

### 3. 에러 처리
```rust
// anyhow 사용으로 간결한 에러 처리
use anyhow::{Context, Result};

fn process() -> Result<()> {
    let data = read_file()
        .context("Failed to read file")?;
    Ok(())
}
```

### 4. 로깅
```rust
// tracing 크레이트 사용
use tracing::{info, warn, error, debug};

info!(target: "network", "Connected to peer {}", addr);
debug!("Received {} bytes", data.len());
```

## 🚦 시작하기

### 1. 환경 설정
```bash
# Bitcoin Core 빌드 (libbitcoinkernel 포함)
git clone https://github.com/bitcoin/bitcoin
cd bitcoin
cmake -B build -DBUILD_KERNEL_LIB=ON
cmake --build build -j$(nproc)
sudo cmake --install build

# Rust 프로젝트 생성
cargo new btck-rust-node
cd btck-rust-node
```

### 2. 의존성 추가
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.6"
bitcoin = "0.32"
anyhow = "1"
# ... (Cargo.toml 참조)

[build-dependencies]
bindgen = "0.69"
```

### 3. 빌드 및 실행
```bash
# 환경 변수 설정
export BITCOINKERNEL_LIB_DIR=/usr/local/lib
export BITCOINKERNEL_INCLUDE_DIR=/usr/local/include

# 빌드
cargo build --release

# 실행
./target/release/btck-rust-node \
    --chain signet \
    --datadir ./data \
    --blocksdir ./blocks \
    --rpc 127.0.0.1:38332
```

### 4. RPC 테스트
```bash
# getblockcount
curl -X POST http://127.0.0.1:38332/getblockcount

# getblockchaininfo
curl -X POST http://127.0.0.1:38332/getblockchaininfo
```

## 🎯 마일스톤

- [x] Phase 0: 기본 skeleton 및 FFI 바인딩
- [x] Phase 1a: Kernel wrapper 구현
- [x] Phase 1b: RPC 서버 기본 구조
- [ ] Phase 2: 네트워크 레이어 완성
- [ ] Phase 3: Mempool 구현
- [ ] Phase 4: 지갑 구현
- [ ] Phase 5: 인덱싱 구현
- [ ] Phase 6: 최적화 및 테스트

## 💡 기여 방법

1. 위 구현 계획에 따라 단계적으로 진행
2. 각 모듈은 독립적으로 테스트 가능하도록 작성
3. Bitcoin Core의 동작과 최대한 일치하도록 구현
4. 성능과 안전성 모두 고려

## 📞 문의

프로젝트 진행 중 질문이나 도움이 필요하면 언제든지 문의하세요!

---

**생성 날짜**: 2025-10-22
**버전**: 0.1.0
**상태**: 초기 구현 단계

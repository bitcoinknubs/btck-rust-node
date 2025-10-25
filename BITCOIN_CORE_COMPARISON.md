# Bitcoin Core vs Rust Implementation - Deep Comparison

## 참조: https://github.com/bitcoinknubs/bitcoinknubs/tree/master/src

이 문서는 Bitcoin Core C++ 구현과 현재 Rust 구현(`btck-rust-node`)을 깊이 있게 비교합니다.

---

## 1. 초기화 순서 비교

### Bitcoin Core (src/node/chainstate.cpp: CompleteChainstateInitialization)

```cpp
1. Interrupt check (chainman.m_interrupt)
2. LoadBlockIndex()
   - m_block_tree_db->LoadBlockIndexGuts()
   - Loads all block entries from disk
   - Calculates chain work for each block
   - Builds skip pointers
   - Validates proof-of-work
3. Genesis block validation
   - Check if genesis matches expected hash
   - "If the loaded chain has a wrong genesis, bail out"
4. Prune state check
   - Validate prune settings consistency
5. LoadGenesisBlock()
   - "Add a genesis block on disk if not mid-reindex"
   - Only if m_blockfiles_indexed is true
6. InitCoinsDB()
   - Initialize UTXO database
   - Allocate cache (with 0.2 discount for multiple chainstates)
7. InitCoinsCache()
   - Initialize in-memory UTXO cache
8. LoadChainTip()
   - "Initialize the chain based on best block"
```

### Current Rust Implementation (src/kernel/mod.rs: Kernel::new)

```rust
1. btck_context_options_create()
2. btck_chain_parameters_create(chain_type)
3. btck_context_options_set_chainparams()
4. btck_context_create()
5. btck_chainstate_manager_options_create()
6. Set options:
   - update_block_tree_db_in_memory(0)  ✓ Fixed
   - update_chainstate_db_in_memory(0)  ✓ Fixed
   - set_worker_threads_num(2)
7. btck_chainstate_manager_create()
8. Genesis check and initialization  ✓ Fixed
   - btck_chain_get_genesis()
   - process_block(genesis) if needed
```

### 🔍 차이점 및 누락된 부분

| Bitcoin Core | Rust Implementation | Status |
|-------------|---------------------|---------|
| **LoadBlockIndex()** | ❌ **명시적 호출 없음** | ⚠️ **CRITICAL** |
| Genesis 검증 | ✅ btck_chain_get_genesis() | ✅ Fixed (b401b2b) |
| Genesis 초기화 | ✅ process_block() | ✅ Fixed (b401b2b) |
| **InitCoinsDB()** | ❓ 불명확 | ⚠️ **Unknown** |
| **InitCoinsCache()** | ❓ 불명확 | ⚠️ **Unknown** |
| **LoadChainTip()** | ❓ 불명확 | ⚠️ **Unknown** |
| Prune 상태 체크 | ❌ 없음 | ⚠️ Missing |

---

## 2. 블록 처리 (ProcessNewBlock) 비교

### Bitcoin Core (src/validation.cpp + src/node/blockstorage.cpp)

```cpp
// ProcessNewBlock 호출 순서:
ProcessNewBlock(block) {
    1. CheckBlock(block)  // 기본 블록 검증
    2. AcceptBlock(block) {
        - CheckBlockHeader()
        - ContextualCheckBlockHeader()
        - SaveBlockToDisk()  // ← 디스크에 저장!
        - ReceivedBlockTransactions()
        - AddToBlockIndex()  // ← 인덱스에 추가!
    }
    3. ActivateBestChain() {
        - ConnectTip()
        - DisconnectTip() (re-org 시)
        - FlushStateToDisk()  // ← 주기적으로 flush!
    }
}

// 블록 저장 (src/node/blockstorage.cpp):
SaveBlockToDisk(block) {
    - FindNextBlockPos()  // 다음 파일 위치 찾기
    - WriteBlockToDisk()  // blk?????.dat에 쓰기
    - m_block_tree_db->WriteBlockIndex()  // 인덱스 업데이트
}
```

### Current Rust Implementation (src/kernel/mod.rs + src/p2p/legacy.rs)

```rust
// P2P에서 블록 수신:
on_block = |raw: &[u8]| {
    kernel.process_block(raw)  // ← 단일 호출!
}

// Kernel implementation:
pub fn process_block(&self, raw: &[u8]) -> Result<()> {
    let ptr = btck_block_create(raw);
    let rc = btck_chainstate_manager_process_block(
        self.chainman,
        ptr,
        &mut new_block
    );
    // rc != 0 이면 에러
}
```

### 🔍 차이점

| 단계 | Bitcoin Core | Rust (libbitcoinkernel) | 비고 |
|-----|-------------|-------------------------|------|
| 블록 검증 | CheckBlock() | ✅ 내부에서 처리 | API가 캡슐화 |
| 디스크 저장 | SaveBlockToDisk() | ✅ 내부에서 처리 | API가 캡슐화 |
| 인덱스 업데이트 | AddToBlockIndex() | ✅ 내부에서 처리 | API가 캡슐화 |
| **ActivateBestChain()** | **명시적 호출** | ❓ **불명확** | ⚠️ **CRITICAL** |
| **FlushStateToDisk()** | **주기적 호출** | ❓ **자동?** | ⚠️ **Unknown** |

**핵심 질문**: `btck_chainstate_manager_process_block()`이 내부적으로 ActivateBestChain을 호출하는가?

---

## 3. 블록 저장 및 인덱스 관리

### Bitcoin Core (src/node/blockstorage.cpp)

```cpp
// 블록 저장 구조:
데이터 디렉토리/
├── blocks/
│   ├── blk00000.dat  // 실제 블록 데이터
│   ├── blk00001.dat
│   ├── index/        // LevelDB - 블록 인덱스
│   │   ├── CURRENT
│   │   ├── LOG
│   │   └── *.ldb
│   └── rev00000.dat  // Undo 데이터 (spent outputs)
└── chainstate/       // LevelDB - UTXO set
    ├── CURRENT
    ├── LOG
    └── *.ldb

// 블록 파일 관리:
class FlatFileSeq {
    - MAX_BLOCKFILE_SIZE = 128MB
    - FindNextBlockPos()  // 현재 파일이 가득 차면 새 파일 생성
}

// 블록 인덱스 저장:
BlockTreeDB::WriteBlockIndex(CDiskBlockIndex) {
    - Key: (DB_BLOCK_INDEX, block_hash)
    - Value: {height, file_num, file_pos, ...}
}

// 재시작 시 로드:
LoadBlockIndexDB() {
    - 모든 블록 인덱스를 메모리로 로드
    - 체인 작업량 재계산
    - Skip 포인터 구축
}
```

### Current Rust Implementation

```rust
// 디렉토리 구조 (예상):
./data/
├── chainstate/       // UTXO DB (in_memory=0 설정됨)
└── (기타?)

./blocks/
├── index/           // 블록 인덱스 (in_memory=0 설정됨)
└── blk?????.dat?    // 블록 파일 (위치 불명확)

// 설정:
btck_chainstate_manager_options_update_block_tree_db_in_memory(opts, 0);  ✓
btck_chainstate_manager_options_update_chainstate_db_in_memory(opts, 0);  ✓
```

### 🔍 차이점 및 확인 필요 사항

| 항목 | Bitcoin Core | Rust | 확인 필요 |
|-----|-------------|------|----------|
| 블록 파일 위치 | `blocks/blk?????.dat` | ❓ | libbitcoinkernel이 생성하는가? |
| Undo 파일 | `blocks/rev?????.dat` | ❓ | 지원하는가? |
| 블록 인덱스 DB | `blocks/index/` | ✅ 설정됨 | ✓ |
| UTXO DB | `chainstate/` | ✅ 설정됨 | ✓ |
| **파일 관리** | FlatFileSeq | ❓ | 자동? |
| **FlushStateToDisk** | 명시적 호출 | ❓ | 자동? |

---

## 4. P2P 블록 다운로드 비교

### Bitcoin Core (src/net_processing.cpp)

```cpp
// 블록 요청 관리:
class CNodeState {
    CBlockIndex* pindexBestKnownBlock;
    std::list<QueuedBlock> vBlocksInFlight;  // 다운로드 중인 블록
    int64_t m_downloading_since;
}

// 블록 수신 처리:
ProcessBlock(node, block) {
    1. BlockRequested() - 요청 목록에 추가
    2. ProcessNewBlock() - 블록 처리
    3. RemoveBlockRequest() - 요청 목록에서 제거
    4. 다음 블록 요청 전송
}

// 중요: 순서 보장
- Bitcoin Core는 블록을 **순서대로** 처리
- 부모가 없으면 orphan으로 보관
- 부모가 도착하면 orphan 처리
```

### Current Rust Implementation (src/p2p/legacy.rs)

```rust
// 블록 다운로드 관리:
struct BlockDownloader {
    queued: VecDeque<BlockHash>,
    in_flight: HashMap<BlockHash, SocketAddr>,
    completed: HashSet<BlockHash>,
}

// 블록 수신:
NetworkMessage::Block(b) => {
    self.downloader.complete(&h);  // 완료 표시

    // 즉시 다음 블록 요청
    let assign = self.downloader.poll_assign(addr);

    // 백그라운드 처리
    tokio::spawn(async move {
        (cb)(&raw)  // kernel.process_block() 호출
    });
}
```

### 🔍 차이점

| 항목 | Bitcoin Core | Rust | 비고 |
|-----|-------------|------|------|
| **블록 처리 순서** | **순차적** | **순차적** | ✅ **Fixed (4b98408)** |
| Orphan 처리 | ✅ 구현됨 | ⚠️ libbitcoinkernel에 의존 | 확인 필요 |
| 블록 검증 실패 시 | Peer 처벌 | ❓ | 확인 필요 |
| 재요청 로직 | timeout 기반 | ❌ 없음? | 확인 필요 |

**✅ Fixed (Commit 4b98408)**:
- 블록 처리를 순차적으로 변경 (mpsc 채널 사용)
- `with_block_processor()`에서 전용 순차 처리 태스크 생성
- 블록 수신 시 채널로 전송하여 순서 보장
- 부모-자식 블록 순서 문제 해결

---

## 5. ActivateBestChain 호출 비교

### Bitcoin Core

```cpp
// ProcessNewBlock에서 호출:
ProcessNewBlock(...) {
    AcceptBlock(block);  // 블록 저장
    ActivateBestChain(); // ← 체인 활성화!
}

// ActivateBestChain 역할:
ActivateBestChain() {
    while (true) {
        CBlockIndex* pindexMostWork = FindMostWorkChain();

        if (pindexMostWork == m_chain.Tip()) {
            break;  // 이미 최고 작업량 체인
        }

        // Re-org 필요
        ConnectTip() or DisconnectTip();

        // Flush to disk periodically
        FlushStateToDisk();
    }
}
```

### Current Rust Implementation

```rust
// process_block에서:
pub fn process_block(&self, raw: &[u8]) -> Result<()> {
    btck_chainstate_manager_process_block(...);
    // ← ActivateBestChain이 내부적으로 호출되는가?
}
```

### 🔍 핵심 질문

**`btck_chainstate_manager_process_block()`이 내부적으로 무엇을 하는가?**

Bitcoin Core의 경우:
1. `AcceptBlock()` - 블록 저장
2. `ActivateBestChain()` - 체인 활성화
3. `FlushStateToDisk()` - 디스크 동기화

libbitcoinkernel C API의 경우:
- 문서화가 불충분
- **가정**: 내부적으로 ActivateBestChain을 호출할 것
- **확인 필요**: 실제로 호출되는가?

---

## 6. 재시작 시 복구 비교

### Bitcoin Core

```cpp
// AppInitMain 순서:
1. LoadBlockIndex()
   - 디스크에서 모든 블록 인덱스 로드
   - 체인 작업량 계산
2. LoadChainTip()
   - 최고 작업량 체인 팁 설정
3. ActivateBestChain()
   - 필요시 재검증
```

### Current Rust Implementation

```rust
// Kernel::new() 순서:
1. btck_chainstate_manager_create()
   - ❓ 내부에서 LoadBlockIndex 호출?
2. Genesis 체크
3. (끝)
```

### 🔍 문제점

**LoadBlockIndex가 자동으로 호출되는가?**

만약 자동으로 호출되지 않으면:
- ❌ 저장된 블록이 로드되지 않음
- ❌ 체인 높이가 0으로 시작
- ❌ 블록을 다시 다운로드

**테스트 필요**:
```bash
# 1. 50개 블록 다운로드
# 2. 재시작
# 3. 로그 확인:
[kernel] ✓ Genesis block exists. Active chain at height 50  ← 이것이 나와야 함!
```

---

## 7. 누락되거나 불명확한 부분 요약

### ✅ RESOLVED - LoadBlockIndex 문제 (commit be293b6)

1. ✅ **LoadBlockIndex() - DIRECTORY STRUCTURE 문제였음!**
   - **이전 진단 (WRONG)**: btck_chainstate_manager_create()가 LoadBlockIndex()를 호출하지 않음
   - **실제 원인 (CORRECT)**: Directory structure mismatch!

   **bitcoinkernel.cpp 분석 결과**:
   ```cpp
   btck_chainstate_manager_create() {
       LoadChainstate();              // ← LoadBlockIndex 포함! ✅
       VerifyLoadedChainstate();      // ← 검증 ✅
       ActivateBestChain();           // ← 체인 활성화 ✅
   }
   ```

   **API는 올바르게 작동했음!** 문제는 디렉토리 구조:

   **잘못된 구조** (--datadir ./data --blocksdir ./blocks):
   ```
   Block files:  ./blocks/blk*.dat          ← 블록이 여기 저장됨
   Block index:  ./data/blocks/index/       ← 인덱스는 여기서 찾음 (비어있음!)
   ```

   **올바른 구조** (--datadir ./data만 사용):
   ```
   Block files:  ./data/blocks/blk*.dat     ← 블록이 여기 저장됨
   Block index:  ./data/blocks/index/       ← 인덱스도 같은 위치! ✅
   ```

   **해결책**:
   - main.rs: blocksdir 기본값을 datadir/blocks로 설정
   - kernel/mod.rs: 디렉토리 구조 경고 추가
   - 사용자는 `--blocksdir` 플래그를 제거하거나 `--blocksdir ./data/blocks` 사용

2. ✅ **ActivateBestChain()**
   - Bitcoin Core: ProcessNewBlock에서 명시적 호출
   - Rust: ✅ **btck_chainstate_manager_create() 내부에서 자동 호출** (bitcoinkernel.cpp 확인됨)
   - **결론**: API가 올바르게 처리함

3. ✅ **블록 순서 보장** (Fixed in 4b98408)
   - Bitcoin Core: 순차 처리 + orphan 관리
   - Rust: ✅ 순차 처리 (mpsc 채널), orphan은 libbitcoinkernel에 의존
   - **해결**: 채널 기반 순차 처리로 순서 보장

### ⚠️ Unknown (확인 필요)

4. **InitCoinsDB() / InitCoinsCache()**
   - Bitcoin Core: 명시적 초기화
   - Rust: ❓ btck_chainstate_manager_create에서 자동?

5. **FlushStateToDisk()**
   - Bitcoin Core: 주기적 호출 (매 블록 또는 시간 기반)
   - Rust: ❓ 자동? 수동?

6. **블록 파일 관리**
   - Bitcoin Core: FlatFileSeq, MAX_BLOCKFILE_SIZE
   - Rust: ❓ libbitcoinkernel이 자동 관리?

### ✅ Fixed (해결됨)

7. **Genesis 초기화**
   - ✅ Fixed in commit b401b2b
   - btck_chain_get_genesis() 사용

8. **디스크 저장**
   - ✅ Fixed in commit 8cbaba9
   - in_memory=0 설정

9. **빈 헤더 응답 처리**
   - ✅ Fixed in commit 5f3b6f5

10. **블록 순차 처리**
    - ✅ Fixed in commit 4b98408
    - mpsc 채널을 사용한 순차 처리

---

## 8. 권장 테스트 시나리오

### Test 1: 블록 저장 확인

```bash
# 1. 클린 시작
rm -rf ./data ./blocks

# 2. 10개 블록 다운로드 후 중단
./btck-rust-node ... &
sleep 30  # 10개 블록 정도 다운로드
kill %1

# 3. 디렉토리 확인
ls -la ./data/chainstate/  # LevelDB 파일들?
ls -la ./blocks/index/     # LevelDB 파일들?
ls -la ./blocks/           # blk?????.dat 파일들?

# 4. 재시작
./btck-rust-node ...

# 5. 로그 확인
# 예상: [kernel] ✓ Genesis block exists. Active chain at height 10
# 실제: ?
```

### Test 2: 블록 순서 테스트

```bash
# 로그에서 확인:
[p2p] 📦 Downloaded block 1/275334
[p2p] ✓ Block 00000086... saved to chain  ← 성공?
[p2p] 📦 Downloaded block 2/275334
[p2p] ✓ Block 00000032... saved to chain  ← 성공?

# 또는:
[p2p] ✗ Failed to process block ...: ... ← 실패?
```

### Test 3: 높이 확인

```bash
# RPC로 확인:
curl -X POST http://localhost:38332/getblockcount

# 예상: 다운로드한 블록 수
# 실제: ?
```

---

## 9. 추천 수정 사항

### 즉시 수정 (CRITICAL)

1. ✅ **블록 순서 보장 추가** (Fixed in 4b98408)
   - mpsc 채널을 사용한 순차 처리 구현
   - with_block_processor()에서 전용 태스크 생성
   - 블록 도착 순서대로 처리 보장

2. **LoadBlockIndex 확인**
   ```rust
   // btck_chainstate_manager_create 후에
   // 블록이 실제로 로드되는지 테스트
   ```

### 개선 (Enhancement)

3. **Orphan 처리**
   ```rust
   // Bitcoin Core처럼 부모 없는 블록 관리
   orphan_blocks: HashMap<BlockHash, Block>
   ```

4. **재요청 로직**
   ```rust
   // Timeout된 블록 재요청
   ```

5. **Peer 관리**
   ```rust
   // 잘못된 블록 보낸 peer 처벌
   ```

---

## 10. libbitcoinkernel API 문서화 요청

다음 사항들이 bitcoinkernel.h에 명확히 문서화되어야 함:

1. **btck_chainstate_manager_create()**
   - LoadBlockIndex를 자동으로 호출하는가?
   - Genesis를 자동으로 초기화하는가?
   - Coins DB를 자동으로 초기화하는가?

2. **btck_chainstate_manager_process_block()**
   - ActivateBestChain을 호출하는가?
   - Orphan 블록을 내부에서 관리하는가?
   - FlushStateToDisk를 자동으로 호출하는가?

3. **블록 파일 관리**
   - blk?????.dat 파일을 어디에 생성하는가?
   - MAX_BLOCKFILE_SIZE 제한이 있는가?
   - rev?????.dat (undo) 파일을 생성하는가?

---

## 결론

### ✅ 올바르게 구현된 부분
- Context 생성
- Chain parameters 설정
- Genesis 초기화 (최근 수정)
- 디스크 저장 설정
- 기본 블록 처리 흐름

### ⚠️ 확인이 필요한 부분
- LoadBlockIndex 자동 호출 여부
- ActivateBestChain 자동 호출 여부
- Orphan 블록 처리 (libbitcoinkernel 내부 처리 여부)
- FlushStateToDisk 자동 호출 여부

### ❌ 명확히 누락된 부분
- Orphan 블록 관리 (P2P 레벨) - libbitcoinkernel이 처리할 가능성 있음
- 블록 재요청 로직
- Peer 처벌 로직
- Prune 상태 확인

**다음 단계**: 위의 Test 1, 2, 3을 실행하여 실제 동작을 확인하고, 문제가 있으면 추가 수정이 필요합니다.

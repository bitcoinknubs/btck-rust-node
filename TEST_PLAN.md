# Block Sync Persistence Test Plan

## 목표
블록 동기화가 재시작 후에도 유지되는지 확인

## Bitcoin Core와의 비교

### Bitcoin Core 초기화 순서:
1. `LoadBlockIndex()` - 디스크에서 블록 인덱스 로드
2. Genesis 검증 - `LookupBlockIndex(hashGenesisBlock)`
3. `LoadGenesisBlock()` - Genesis가 없으면 생성
4. `ActivateBestChain()` - 최고 작업량 체인 활성화

### 현재 구현:
1. ✅ Chainstate manager 생성
2. ⚠️ LoadBlockIndex() 호출 **없음** (자동으로 되어야 함)
3. ✅ Genesis 초기화 추가 (Commit 5f3b6f5)
4. ⚠️ ActivateBestChain() 명시적 호출 없음

## 테스트 단계

### 1. 클린 빌드
```bash
rm -rf ./data ./blocks ./headers_signet.dat
cargo clean
cargo build --release
```

### 2. 첫 실행 (Genesis 초기화 확인)
```bash
./target/release/btck-rust-node \
    --chain signet \
    --datadir ./data \
    --blocksdir ./blocks \
    --rpc 127.0.0.1:38332
```

**확인할 로그**:
```
[kernel] Checking for genesis block...
[kernel] No active chain found. Initializing genesis block...
[kernel] ✓ Genesis block initialized: 00000008819873...
[kernel]    Chain is now active at height 0
```

또는:
```
[kernel] ⚠ Failed to initialize genesis block: ...
```

### 3. 블록 다운로드 시작 확인
```
[p2p] 📭 Empty headers response - we are caught up!
╔════════════════════════════════════════════════════════════╗
║  HEADERS SYNC COMPLETE!                                    ║
╚════════════════════════════════════════════════════════════╝
[p2p] Starting block download: requesting 16 blocks
[p2p] ✓ Block 00000086d6b2636c... saved to chain
```

또는 **실패**:
```
[p2p] ✗ Failed to process block ...: ...
```

### 4. 50개 블록 다운로드 후 중단
Ctrl+C로 중단

### 5. 재시작 (지속성 확인)
```bash
./target/release/btck-rust-node \
    --chain signet \
    --datadir ./data \
    --blocksdir ./blocks \
    --rpc 127.0.0.1:38332
```

**확인할 로그**:
```
[kernel] ✓ Active chain found at height 50   ← 성공!
[p2p] Starting P2P with current height: 50
```

또는 **실패**:
```
[kernel] ✓ Active chain found at height 0    ← 실패! 저장 안 됨
[p2p] Starting P2P with current height: 0
```

### 6. 디스크 상태 확인
```bash
ls -lh ./data/chainstate/
ls -lh ./blocks/index/
```

**예상 결과**:
- `chainstate/` - LevelDB 파일들 (UTXO set)
- `blocks/index/` - LevelDB 파일들 (block index)

## 예상되는 문제점

### 문제 1: Genesis 초기화 실패
**증상**:
```
[kernel] ⚠ Failed to initialize genesis block: process_block rc=-1
```

**원인**: Genesis 블록이 이미 DB에 있거나, 다른 초기화 문제

**해결**: 정상적일 수 있음. "already in database" 메시지면 OK

### 문제 2: 블록 처리 실패
**증상**:
```
[p2p] ✗ Failed to process block ...: process_block rc=-1
```

**원인**:
- Genesis가 없어서 부모를 찾을 수 없음
- Chainstate가 활성화되지 않음

**해결**: LoadBlockIndex() 호출이 필요할 수 있음

### 문제 3: 재시작 후 height 0
**증상**:
```
[kernel] ✓ Active chain found at height 0
```

**원인**:
- 블록이 실제로 저장되지 않음
- LoadBlockIndex()가 호출되지 않아 디스크에서 로드 안 됨

**해결**: libbitcoinkernel C API 확인 필요

## libbitcoinkernel API 확인 필요

현재 `btck_chainstate_manager_create()` 후에 추가로 호출해야 할 함수가 있는지 확인:

```rust
// 예상되는 필요 함수들 (존재 여부 확인 필요):
btck_chainstate_manager_load_block_index()?
btck_chainstate_manager_init_genesis()?
btck_chainstate_manager_activate_best_chain()?
```

## 디버깅 로그 수집

### 필요한 로그:
1. **Kernel 초기화**:
   - Genesis 초기화 성공/실패
   - Active chain height

2. **블록 다운로드**:
   - "✓ Block ... saved to chain" (성공)
   - "✗ Failed to process block" (실패)

3. **재시작 후**:
   - "Active chain found at height N"
   - 디렉토리 내용 (`ls -la ./data` 등)

## 추가 조사 항목

### 1. btck_bindings.rs 확인
```bash
find target -name "btck_bindings.rs" -exec grep -E "load|index|genesis|activate" {} \;
```

사용 가능한 kernel 함수 목록 확인

### 2. 실제 Bitcoin Core 동작 비교
Bitcoin Core를 signet으로 실행:
```bash
bitcoind -signet -datadir=/tmp/btc-test
```

첫 시작과 재시작 시 로그 비교

### 3. LevelDB 내용 확인
```bash
# Block index DB 확인
ls -la ./blocks/index/
hexdump -C ./blocks/index/LOG | head -20
```

블록이 실제로 저장되었는지 확인

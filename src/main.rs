use anyhow::{Context, Result};
use axum::{extract::State, routing::get, Json, Router};
use clap::Parser;
use serde_json::json;
use std::{
    ffi::{c_void, CStr, CString},
    net::SocketAddr,
    os::raw::{c_char, c_int},
    path::PathBuf,
    sync::Arc,
};

mod addrman; // Address manager
mod ffi;     // bindgen이 생성한 btck_* FFI
mod kernel;  // Kernel wrapper
mod mempool; // Mempool 구현
// mod network; // Network 구현 (temporarily disabled)
mod p2p;     // P2P 구현
mod rpc;     // RPC 서버
mod seeds;   // DNS seeds

use kernel::Kernel;

// 체인 타입 상수 (bitcoinkernel.h와 일치)
const CHAIN_MAIN: u8 = 0;
const CHAIN_TESTNET: u8 = 1;
const CHAIN_TESTNET4: u8 = 2;
const CHAIN_SIGNET: u8 = 3;
const CHAIN_REGTEST: u8 = 4;

type CChainstateManager = ffi::btck_ChainstateManager;
type CChain = ffi::btck_Chain;
type CContext = ffi::btck_Context;
type CChainParameters = ffi::btck_ChainParameters;
type CContextOptions = ffi::btck_ContextOptions;
type CChainstateManagerOptions = ffi::btck_ChainstateManagerOptions;

#[derive(Parser, Debug, Clone)]
#[command(name = "btck-mini-node", version, about = "Mini node powered by libbitcoinkernel")]
struct Args {
    /// chain: mainnet/testnet/signet/regtest
    #[arg(long, default_value = "signet")]
    chain: String,

    /// data directory (for kernel DB/files)
    #[arg(long, default_value = "./data")]
    datadir: PathBuf,

    /// blocks directory (for raw block files)
    #[arg(long, default_value = "./blocks")]
    blocksdir: PathBuf,

    /// optional: import blk*.dat files (comma-separated absolute paths)
    #[arg(long)]
    import: Option<String>,

    /// RPC listen address, e.g. 127.0.0.1:8332 (HTTP)
    #[arg(long, default_value = "127.0.0.1:38332")]
    rpc: String,

    /// optional: peers to connect (can be repeated)
    #[arg(long)]
    peer: Vec<String>,
}

// ------------------------------
// 커널 포인터 래퍼
// ------------------------------
struct Kernel {
    ctx: *mut CContext,
    chain_params: *mut CChainParameters,
    chainman: *mut CChainstateManager,
}
unsafe impl Send for Kernel {}
unsafe impl Sync for Kernel {}

// 커널 로그 콜백: 커널 로그를 stderr로 출력
unsafe extern "C" fn log_cb(_ud: *mut c_void, msg: *const c_char, _len: usize) {
    if !msg.is_null() {
        if let Ok(s) = CStr::from_ptr(msg).to_str() {
            eprintln!("[kernel] {s}");
        }
    }
}

impl Kernel {
    fn new(chain: &str, datadir: &PathBuf, blocksdir: &PathBuf) -> Result<Self> {
        // 체인 타입 매핑 (bitcoinkernel.h ChainType과 일치)
        let chain_type: u8 = match chain {
            "main" | "mainnet" => CHAIN_MAIN,
            "testnet" => CHAIN_TESTNET,
            "testnet4" => CHAIN_TESTNET4,
            "signet" => CHAIN_SIGNET,
            _ => CHAIN_REGTEST,
        };

        // 1) ContextOptions 생성 + ChainParameters 설정
        let ctx_opts = unsafe { ffi::btck_context_options_create() };
        if ctx_opts.is_null() {
            anyhow::bail!("btck_context_options_create failed");
        }

        let chain_params = unsafe { ffi::btck_chain_parameters_create(chain_type) };
        if chain_params.is_null() {
            unsafe { ffi::btck_context_options_destroy(ctx_opts) };
            anyhow::bail!("btck_chain_parameters_create failed");
        }
        unsafe { ffi::btck_context_options_set_chainparams(ctx_opts, chain_params) };

        // (선택) 커널 로깅 연결
        let log_opts: ffi::btck_LoggingOptions = unsafe { std::mem::zeroed() };
        let conn = unsafe {
            ffi::btck_logging_connection_create(
                Some(log_cb),
                std::ptr::null_mut(),
                None,
                log_opts,
            )
        };
        if conn.is_null() {
            eprintln!("[kernel] logging connection create failed (continuing without kernel logs)");
        }

        // 2) Context 생성
        let ctx = unsafe { ffi::btck_context_create(ctx_opts) };
        if ctx.is_null() {
            unsafe {
                ffi::btck_context_options_destroy(ctx_opts);
                ffi::btck_chain_parameters_destroy(chain_params);
            }
            anyhow::bail!("btck_context_create failed");
        }
        // options는 destroy 가능 (옵션 객체 수명은 context 생성 시점까지만 필요)
        unsafe { ffi::btck_context_options_destroy(ctx_opts) };

        // 3) ChainstateManagerOptions 생성
        //    ※ datadir/blocksdir은 여기서 문자열/길이로 전달
        let data_c = CString::new(datadir.to_string_lossy().as_bytes())?;
        let blocks_c = CString::new(blocksdir.to_string_lossy().as_bytes())?;
        let chainman_opts = unsafe {
            ffi::btck_chainstate_manager_options_create(
                ctx,
                data_c.as_ptr(),
                data_c.as_bytes().len(),
                blocks_c.as_ptr(),
                blocks_c.as_bytes().len(),
            )
        };
        if chainman_opts.is_null() {
            unsafe {
                ffi::btck_context_destroy(ctx);
                ffi::btck_chain_parameters_destroy(chain_params);
            }
            anyhow::bail!("btck_chainstate_manager_options_create failed");
        }

        // 데모 세팅: 인메모리 DB, 검증 스레드 2개
        unsafe {
            ffi::btck_chainstate_manager_options_update_block_tree_db_in_memory(chainman_opts, 1);
            ffi::btck_chainstate_manager_options_update_chainstate_db_in_memory(chainman_opts, 1);
            ffi::btck_chainstate_manager_options_set_worker_threads_num(chainman_opts, 2);
        }

        // 4) ChainstateManager 생성 — 인자 1개(Options)만 받습니다!
        let chainman = unsafe { ffi::btck_chainstate_manager_create(chainman_opts) };
        if chainman.is_null() {
            unsafe {
                ffi::btck_context_destroy(ctx);
                ffi::btck_chain_parameters_destroy(chain_params);
                ffi::btck_chainstate_manager_options_destroy(chainman_opts);
            }
            anyhow::bail!("btck_chainstate_manager_create failed");
        }
        unsafe { ffi::btck_chainstate_manager_options_destroy(chainman_opts) };

        // 디버그: 활성 체인 높이 출력
        let h0 = unsafe {
            let c = ffi::btck_chainstate_manager_get_active_chain(chainman);
            if c.is_null() { -1 } else { ffi::btck_chain_get_height(c) }
        };
        eprintln!("[kernel] active chain height right after init = {h0} (expect -1 or 0)");

        Ok(Self { ctx, chain_params, chainman })
    }

    fn active_height(&self) -> Result<i32> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                return Ok(-1);
            }
            Ok(ffi::btck_chain_get_height(chain))
        }
    }

    fn import_blocks(&self, paths: &[String]) -> Result<i32> {
        // C API: (char** data, size_t* lens, size_t count)
        let c_paths: Vec<CString> = paths
            .iter()
            .map(|p| CString::new(p.as_str()))
            .collect::<std::result::Result<_, _>>()?;

        let mut ptrs: Vec<*const i8> = c_paths.iter().map(|c| c.as_ptr()).collect();
        let mut lens: Vec<usize> = c_paths.iter().map(|c| c.as_bytes().len()).collect();

        let rc = unsafe {
            ffi::btck_chainstate_manager_import_blocks(
                self.chainman,
                ptrs.as_mut_ptr(),
                lens.as_mut_ptr(),
                ptrs.len(),
            )
        };
        Ok(rc)
    }
}
impl Drop for Kernel {
    fn drop(&mut self) {
        unsafe {
            ffi::btck_chainstate_manager_destroy(self.chainman);
            ffi::btck_context_destroy(self.ctx);
            ffi::btck_chain_parameters_destroy(self.chain_params);
        }
    }
}

// ------------------------------
// RPC 상태 & 핸들러
// ------------------------------
#[derive(Clone)]
struct AppState {
    kernel: Arc<Kernel>,
}

async fn ping() -> Json<serde_json::Value> {
    Json(json!({"ok": true}))
}

async fn getblockcount(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let k = state.kernel.clone();
    let height = tokio::task::spawn_blocking(move || k.active_height())
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        .and_then(|r| r.map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR))?;
    Ok(Json(json!({ "height": height })))
}

// ------------------------------
// 엔트리포인트
// ------------------------------
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    // 커널 초기화
    let kernel = Arc::new(Kernel::new(&args.chain, &args.datadir, &args.blocksdir)?);

    // (옵션) 블록 임포트
    if let Some(list) = &args.import {
        let files: Vec<String> = list
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !files.is_empty() {
            let k = kernel.clone();
            let rc = tokio::task::spawn_blocking(move || k.import_blocks(&files))
                .await?
                .context("import_blocks failed")?;
            eprintln!("[import] result={}", rc);
        }
    }

    // (옵션) P2P 기동
    if !args.peer.is_empty() || matches!(args.chain.as_str(), "main" | "mainnet" | "testnet" | "signet") {
        let net = match args.chain.as_str() {
            "main" | "mainnet" => bitcoin::Network::Bitcoin,
            "testnet" => bitcoin::Network::Testnet,
            "signet" => bitcoin::Network::Signet,
            _ => bitcoin::Network::Regtest,
        };

        let peers_cli = args.peer.clone();
        let k = kernel.clone();

        tokio::spawn(async move {
            // 블록 처리 콜백: libbitcoinkernel 검증/적용
            let process_block = move |raw: &[u8]| -> anyhow::Result<()> {
                let ptr = unsafe { ffi::btck_block_create(raw.as_ptr() as *const std::ffi::c_void, raw.len()) };
                if ptr.is_null() {
                    anyhow::bail!("btck_block_create failed");
                }
                let mut new_block: c_int = 0;
                let rc = unsafe {
                    ffi::btck_chainstate_manager_process_block(
                        k.chainman,
                        ptr,
                        &mut new_block as *mut c_int,
                    )
                };
                unsafe { ffi::btck_block_destroy(ptr) };

                if rc != 0 {
                    anyhow::bail!("process_block rc={} new_block={}", rc, new_block);
                }
                Ok(())
            };

            let mut pm = p2p::PeerManager::new(net, "/btck-mini-node:0.1/")
                .with_block_processor(process_block);

            for p in peers_cli {
                if let Ok(addr) = p.parse::<SocketAddr>() {
                    let _ = pm.add_outbound(addr, 0).await;
                }
            }
            if pm.peers_len() < 2 {
                let _ = pm.bootstrap().await;
            }

            if let Err(e) = pm.event_loop().await {
                eprintln!("[p2p] loop error: {e:#}");
            }
        });
    }

    // RPC 서버 시작 (axum 0.6)
    let state = AppState { kernel: kernel.clone() };
    let app = Router::new()
        .route("/ping", get(ping))
        .route("/getblockcount", get(getblockcount))
        .with_state(state);

    let rpc_addr: SocketAddr = args.rpc.parse().context("bad --rpc addr")?;
    eprintln!("[rpc] listening on http://{}", rpc_addr);
    axum::Server::bind(&rpc_addr)
        .serve(app.into_make_service())
        .await
        .context("server")?;

    Ok(())
}

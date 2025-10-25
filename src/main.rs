use anyhow::{Context, Result};
use clap::Parser;
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};

mod addrman;     // Address manager
mod chainparams; // Chain parameters (checkpoints, AssumeValid, etc.)
mod ffi;         // bindgen이 생성한 btck_* FFI
mod kernel;      // Kernel wrapper
mod mempool;     // Mempool 구현
// mod network;  // Network 구현 (temporarily disabled)
mod p2p;         // P2P 구현
mod rpc;         // RPC 서버
mod seeds;       // DNS seeds

use kernel::Kernel;
use mempool::{Mempool, MempoolPolicy};

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
    /// IMPORTANT: Should be set to <datadir>/blocks for proper operation
    /// If not specified, defaults to <datadir>/blocks
    #[arg(long)]
    blocksdir: Option<PathBuf>,

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
// 엔트리포인트
// ------------------------------
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Fix blocksdir to be datadir/blocks if not explicitly specified
    // This is CRITICAL for proper block index loading!
    // libbitcoinkernel stores:
    //   - Block files in: blocksdir/blk*.dat
    //   - Block index in: datadir/blocks/index/
    // If these don't align, blocks won't load on restart!
    let blocksdir = args.blocksdir.unwrap_or_else(|| args.datadir.join("blocks"));

    eprintln!("[main] Using directories:");
    eprintln!("[main]   Data:   {:?}", args.datadir);
    eprintln!("[main]   Blocks: {:?}", blocksdir);

    // 커널 초기화
    let kernel = Arc::new(Kernel::new(&args.chain, &args.datadir, &blocksdir)?);

    // Mempool 초기화
    let policy = match args.chain.as_str() {
        "main" | "mainnet" => MempoolPolicy::mainnet(),
        "testnet" | "signet" => MempoolPolicy::testnet(),
        _ => MempoolPolicy::regtest(),
    };
    let mempool = Arc::new(Mempool::with_kernel(policy, kernel.clone()));
    eprintln!("[mempool] initialized with policy: {}", args.chain);

    // Graceful shutdown signal
    let shutdown_signal = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
        eprintln!("\n[main] 🛑 Received Ctrl+C, initiating graceful shutdown...");
    };

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
    let p2p_handle = if !args.peer.is_empty() || matches!(args.chain.as_str(), "main" | "mainnet" | "testnet" | "signet") {
        let net = match args.chain.as_str() {
            "main" | "mainnet" => bitcoin::Network::Bitcoin,
            "testnet" => bitcoin::Network::Testnet,
            "signet" => bitcoin::Network::Signet,
            _ => bitcoin::Network::Regtest,
        };

        // Get current height from Kernel for P2P initialization
        let current_height = kernel.get_height().unwrap_or(0);
        eprintln!("[p2p] Starting P2P with current height: {}", current_height);

        let peers_cli = args.peer.clone();
        let k = kernel.clone();
        let m = mempool.clone();

        Some(tokio::spawn(async move {
            // 블록 처리 콜백: libbitcoinkernel 검증/적용
            let process_block = move |raw: &[u8]| -> anyhow::Result<()> {
                k.process_block(raw)
            };

            // 트랜잭션 처리 콜백: Mempool에 추가
            let process_tx = move |tx: &bitcoin::Transaction| -> anyhow::Result<()> {
                // Get current height (default to 0 if unavailable)
                let height = 0u32; // TODO: get actual height from kernel

                // Estimate fee (for now use dummy value, should calculate from inputs/outputs)
                let fee = 1000u64; // TODO: calculate actual fee

                match m.add_tx(tx.clone(), fee, height) {
                    Ok(txid) => {
                        eprintln!("[mempool] accepted tx: {}", txid);
                        Ok(())
                    }
                    Err(e) => {
                        eprintln!("[mempool] rejected tx {}: {}", tx.compute_txid(), e);
                        Err(e)
                    }
                }
            };

            let mut pm = p2p::PeerManager::with_start_height(net, "/btck-mini-node:0.1/", current_height)
                .with_block_processor(process_block)
                .with_tx_processor(process_tx);

            for p in peers_cli {
                if let Ok(addr) = p.parse::<SocketAddr>() {
                    let _ = pm.add_outbound(addr).await;
                }
            }
            if pm.peers_len() < 2 {
                let _ = pm.bootstrap().await;
            }

            if let Err(e) = pm.event_loop().await {
                eprintln!("[p2p] loop error: {e:#}");
            }
        }))
    } else {
        None
    };

    // RPC 서버 시작 (shutdown signal과 함께)
    let rpc_addr: SocketAddr = args.rpc.parse().context("bad --rpc addr")?;

    // Create oneshot channel for RPC-triggered shutdown
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();

    tokio::select! {
        result = rpc::start_rpc_server(rpc_addr, kernel.clone(), mempool.clone(), shutdown_tx) => {
            if let Err(e) = result {
                eprintln!("[main] RPC server error: {:#}", e);
            }
        }
        _ = shutdown_signal => {
            eprintln!("[main] Ctrl+C signal received");
        }
        _ = &mut shutdown_rx => {
            eprintln!("[main] RPC shutdown signal received");
        }
    }

    // Graceful shutdown: drop all references to kernel
    eprintln!("[main] Shutting down services...");

    if let Some(handle) = p2p_handle {
        handle.abort();
        eprintln!("[main] P2P service stopped");
    }

    // Force drop kernel to trigger btck_chainstate_manager_destroy()
    drop(kernel);
    drop(mempool);

    eprintln!("[main] ✅ Graceful shutdown complete - all data flushed to disk");

    Ok(())
}

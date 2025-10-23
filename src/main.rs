use anyhow::{Context, Result};
use clap::Parser;
use std::{
    net::SocketAddr,
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
                k.process_block(raw)
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

    // RPC 서버 시작
    let rpc_addr: SocketAddr = args.rpc.parse().context("bad --rpc addr")?;
    rpc::start_rpc_server(rpc_addr, kernel).await?;

    Ok(())
}

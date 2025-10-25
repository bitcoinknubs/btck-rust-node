// src/rpc/mod.rs
pub mod blockchain;
// pub mod network; // Temporarily disabled - requires ConnectionManager

use anyhow::Result;
use axum::{routing::{get, post}, Router};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::kernel::Kernel;
use crate::mempool::Mempool;

#[derive(Clone)]
pub struct AppState {
    pub kernel: Arc<Kernel>,
    pub mempool: Arc<Mempool>,
}

pub async fn start_rpc_server(
    addr: SocketAddr,
    kernel: Arc<Kernel>,
    mempool: Arc<Mempool>,
) -> Result<()> {
    let state = AppState { kernel, mempool };

    let app = Router::new()
        // Blockchain RPCs - GET support for simple queries, POST for queries with params
        .route("/getblockchaininfo", get(blockchain::getblockchaininfo).post(blockchain::getblockchaininfo))
        .route("/getbestblockhash", get(blockchain::getbestblockhash).post(blockchain::getbestblockhash))
        .route("/getblockcount", get(blockchain::getblockcount).post(blockchain::getblockcount))
        .route("/getblockhash", post(blockchain::getblockhash))
        .route("/getblock", post(blockchain::getblock))
        .route("/getblockheader", post(blockchain::getblockheader))
        .route("/getchaintips", get(blockchain::getchaintips).post(blockchain::getchaintips))
        .route("/getdifficulty", get(blockchain::getdifficulty).post(blockchain::getdifficulty))
        .route("/getmempoolinfo", get(blockchain::getmempoolinfo).post(blockchain::getmempoolinfo))
        .route("/getrawmempool", post(blockchain::getrawmempool))
        .route("/gettxout", post(blockchain::gettxout))
        .route("/gettxoutsetinfo", get(blockchain::gettxoutsetinfo).post(blockchain::gettxoutsetinfo))
        .route("/verifychain", post(blockchain::verifychain))
        .with_state(state);

    eprintln!("[rpc] listening on http://{}", addr);
    
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

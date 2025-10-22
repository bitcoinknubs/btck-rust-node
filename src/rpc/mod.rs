// src/rpc/mod.rs
pub mod blockchain;
pub mod network;

use anyhow::Result;
use axum::{routing::post, Router};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::kernel::Kernel;

#[derive(Clone)]
pub struct AppState {
    pub kernel: Arc<Kernel>,
}

pub async fn start_rpc_server(
    addr: SocketAddr,
    kernel: Arc<Kernel>,
) -> Result<()> {
    let state = AppState { kernel };

    let app = Router::new()
        // Blockchain RPCs
        .route("/getblockchaininfo", post(blockchain::getblockchaininfo))
        .route("/getbestblockhash", post(blockchain::getbestblockhash))
        .route("/getblockcount", post(blockchain::getblockcount))
        .route("/getblockhash", post(blockchain::getblockhash))
        .route("/getblock", post(blockchain::getblock))
        .route("/getblockheader", post(blockchain::getblockheader))
        .route("/getchaintips", post(blockchain::getchaintips))
        .route("/getdifficulty", post(blockchain::getdifficulty))
        .route("/getmempoolinfo", post(blockchain::getmempoolinfo))
        .route("/getrawmempool", post(blockchain::getrawmempool))
        .route("/gettxout", post(blockchain::gettxout))
        .route("/gettxoutsetinfo", post(blockchain::gettxoutsetinfo))
        .route("/verifychain", post(blockchain::verifychain))
        .with_state(state);

    eprintln!("[rpc] listening on http://{}", addr);
    
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

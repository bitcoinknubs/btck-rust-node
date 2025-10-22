// src/rpc/blockchain.rs
use anyhow::Result;
use axum::{extract::State, http::StatusCode, Json};
use bitcoin::BlockHash;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::kernel::Kernel;

// Import AppState from mod.rs instead of defining it here
use super::AppState;

// ============================================================================
// Blockchain RPC Methods
// ============================================================================

/// getblockchaininfo
#[derive(Serialize)]
pub struct BlockchainInfo {
    pub chain: String,
    pub blocks: i32,
    pub headers: i32,
    pub bestblockhash: String,
    pub difficulty: f64,
    pub mediantime: i64,
    pub verificationprogress: f64,
    pub initialblockdownload: bool,
    pub size_on_disk: u64,
}

pub async fn getblockchaininfo(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let k = state.kernel.clone();
    
    let result = tokio::task::spawn_blocking(move || {
        let height = k.get_height().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let best_blockhash = if height >= 0 {
            k.get_best_block_hash()
                .map(|h| h.to_string())
                .unwrap_or_else(|_| String::new())
        } else {
            String::new()
        };

        let info = BlockchainInfo {
            chain: "signet".to_string(),
            blocks: height,
            headers: height,
            bestblockhash: best_blockhash,
            difficulty: 0.0,
            mediantime: 0,
            verificationprogress: 1.0,
            initialblockdownload: false,
            size_on_disk: 0,
        };

        Ok::<_, StatusCode>(info)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(json!(result)))
}

/// getbestblockhash
pub async fn getbestblockhash(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let k = state.kernel.clone();
    
    let hash = tokio::task::spawn_blocking(move || {
        k.get_best_block_hash()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(json!({ "result": hash.to_string() })))
}

/// getblockcount
pub async fn getblockcount(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let k = state.kernel.clone();
    
    let height = tokio::task::spawn_blocking(move || {
        k.get_height()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(json!({ "result": height })))
}

/// getblockhash
#[derive(Deserialize)]
pub struct GetBlockHashParams {
    pub height: i32,
}

pub async fn getblockhash(
    State(state): State<AppState>,
    Json(params): Json<GetBlockHashParams>,
) -> Result<Json<Value>, StatusCode> {
    let k = state.kernel.clone();
    let height = params.height;
    
    let hash = tokio::task::spawn_blocking(move || {
        k.get_block_hash(height)
            .map_err(|_| StatusCode::NOT_FOUND)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(json!({ "result": hash.to_string() })))
}

/// getblock
#[derive(Deserialize)]
pub struct GetBlockParams {
    pub blockhash: String,
    #[serde(default)]
    pub verbosity: u8, // 0=hex, 1=json, 2=json+tx
}

pub async fn getblock(
    State(state): State<AppState>,
    Json(params): Json<GetBlockParams>,
) -> Result<Json<Value>, StatusCode> {
    // Parse block hash
    let blockhash = params.blockhash.parse::<BlockHash>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // TODO: Implement actual block retrieval via kernel
    // For now, return placeholder
    Ok(Json(json!({
        "error": "getblock not yet implemented",
        "blockhash": blockhash.to_string()
    })))
}

/// getblockheader
#[derive(Deserialize)]
pub struct GetBlockHeaderParams {
    pub blockhash: String,
    #[serde(default)]
    pub verbose: bool,
}

pub async fn getblockheader(
    State(state): State<AppState>,
    Json(params): Json<GetBlockHeaderParams>,
) -> Result<Json<Value>, StatusCode> {
    let blockhash = params.blockhash.parse::<BlockHash>()
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    // TODO: Implement via kernel
    Ok(Json(json!({
        "error": "getblockheader not yet implemented",
        "blockhash": blockhash.to_string()
    })))
}

/// getblockstats
#[derive(Deserialize)]
pub struct GetBlockStatsParams {
    pub hash_or_height: serde_json::Value,
    #[serde(default)]
    pub stats: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct BlockStats {
    pub avgfee: u64,
    pub avgfeerate: u64,
    pub avgtxsize: u64,
    pub blockhash: String,
    pub height: i32,
    pub ins: u64,
    pub maxfee: u64,
    pub maxfeerate: u64,
    pub maxtxsize: u64,
    pub medianfee: u64,
    pub medianfeerate: u64,
    pub mediantime: i64,
    pub mediantxsize: u64,
    pub minfee: u64,
    pub minfeerate: u64,
    pub mintxsize: u64,
    pub outs: u64,
    pub subsidy: u64,
    pub time: i64,
    pub total_out: u64,
    pub total_size: u64,
    pub totalfee: u64,
    pub txs: u64,
    pub utxo_increase: i64,
    pub utxo_size_inc: i64,
}

pub async fn getblockstats(
    State(state): State<AppState>,
    Json(params): Json<GetBlockStatsParams>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement via kernel
    Ok(Json(json!({
        "error": "getblockstats not yet implemented"
    })))
}

/// getchaintips
#[derive(Serialize)]
pub struct ChainTip {
    pub height: i32,
    pub hash: String,
    pub branchlen: i32,
    pub status: String, // "active", "valid-fork", "invalid", etc.
}

pub async fn getchaintips(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let k = state.kernel.clone();
    
    let result = tokio::task::spawn_blocking(move || {
        let height = k.get_height().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let hash = k.get_best_block_hash()
            .map(|h| h.to_string())
            .unwrap_or_default();

        let tips = vec![ChainTip {
            height,
            hash,
            branchlen: 0,
            status: "active".to_string(),
        }];

        Ok::<_, StatusCode>(tips)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(json!(result)))
}

/// getchaintxstats
#[derive(Deserialize)]
pub struct GetChainTxStatsParams {
    #[serde(default)]
    pub nblocks: Option<i32>,
    #[serde(default)]
    pub blockhash: Option<String>,
}

#[derive(Serialize)]
pub struct ChainTxStats {
    pub time: i64,
    pub txcount: u64,
    pub window_final_block_hash: String,
    pub window_block_count: i32,
    pub window_tx_count: u64,
    pub window_interval: i64,
    pub txrate: f64,
}

pub async fn getchaintxstats(
    State(state): State<AppState>,
    Json(params): Json<GetChainTxStatsParams>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement via kernel
    Ok(Json(json!({
        "error": "getchaintxstats not yet implemented"
    })))
}

/// getdifficulty
pub async fn getdifficulty(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Calculate from block header
    Ok(Json(json!({ "result": 1.0 })))
}

/// getmempoolinfo
#[derive(Serialize)]
pub struct MempoolInfo {
    pub loaded: bool,
    pub size: usize,
    pub bytes: usize,
    pub usage: usize,
    pub maxmempool: usize,
    pub mempoolminfee: f64,
    pub minrelaytxfee: f64,
}

pub async fn getmempoolinfo(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement with actual mempool
    let info = MempoolInfo {
        loaded: true,
        size: 0,
        bytes: 0,
        usage: 0,
        maxmempool: 300_000_000,
        mempoolminfee: 0.00001,
        minrelaytxfee: 0.00001,
    };

    Ok(Json(json!(info)))
}

/// getrawmempool
#[derive(Deserialize)]
pub struct GetRawMempoolParams {
    #[serde(default)]
    pub verbose: bool,
}

pub async fn getrawmempool(
    State(state): State<AppState>,
    Json(params): Json<GetRawMempoolParams>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement with actual mempool
    if params.verbose {
        Ok(Json(json!({})))
    } else {
        Ok(Json(json!([])))
    }
}

/// gettxout
#[derive(Deserialize)]
pub struct GetTxOutParams {
    pub txid: String,
    pub n: u32,
    #[serde(default)]
    pub include_mempool: bool,
}

#[derive(Serialize)]
pub struct TxOut {
    pub bestblock: String,
    pub confirmations: i32,
    pub value: f64,
    pub scriptPubKey: ScriptPubKey,
    pub coinbase: bool,
}

#[derive(Serialize)]
pub struct ScriptPubKey {
    pub asm: String,
    pub hex: String,
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

pub async fn gettxout(
    State(state): State<AppState>,
    Json(params): Json<GetTxOutParams>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement via kernel UTXO set
    Ok(Json(json!(null)))
}

/// gettxoutsetinfo
#[derive(Serialize)]
pub struct TxOutSetInfo {
    pub height: i32,
    pub bestblock: String,
    pub transactions: u64,
    pub txouts: u64,
    pub bogosize: u64,
    pub hash_serialized_2: String,
    pub disk_size: u64,
    pub total_amount: f64,
}

pub async fn gettxoutsetinfo(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let k = state.kernel.clone();
    
    let result = tokio::task::spawn_blocking(move || {
        let height = k.get_height().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let bestblock = k.get_best_block_hash()
            .map(|h| h.to_string())
            .unwrap_or_default();

        let info = TxOutSetInfo {
            height,
            bestblock,
            transactions: 0,
            txouts: 0,
            bogosize: 0,
            hash_serialized_2: String::new(),
            disk_size: 0,
            total_amount: 0.0,
        };

        Ok::<_, StatusCode>(info)
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)??;

    Ok(Json(json!(result)))
}

/// verifychain
#[derive(Deserialize)]
pub struct VerifyChainParams {
    #[serde(default = "default_checklevel")]
    pub checklevel: u8,
    #[serde(default = "default_nblocks")]
    pub nblocks: i32,
}

fn default_checklevel() -> u8 { 3 }
fn default_nblocks() -> i32 { 6 }

pub async fn verifychain(
    State(state): State<AppState>,
    Json(params): Json<VerifyChainParams>,
) -> Result<Json<Value>, StatusCode> {
    // TODO: Implement via kernel
    Ok(Json(json!({ "result": true })))
}

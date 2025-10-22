// src/rpc/network.rs
use anyhow::Result;
use axum::{extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::network::ConnectionManager;

#[derive(Clone)]
pub struct AppState {
    pub connman: Arc<ConnectionManager>,
}

// ============================================================================
// Network RPC Methods
// ============================================================================

/// getnetworkinfo
#[derive(Serialize)]
pub struct NetworkInfo {
    pub version: u32,
    pub subversion: String,
    pub protocolversion: u32,
    pub localservices: String,
    pub localservicesnames: Vec<String>,
    pub localrelay: bool,
    pub timeoffset: i64,
    pub networkactive: bool,
    pub connections: usize,
    pub networks: Vec<NetworkDetails>,
    pub relayfee: f64,
    pub incrementalfee: f64,
    pub localaddresses: Vec<LocalAddress>,
    pub warnings: String,
}

#[derive(Serialize)]
pub struct NetworkDetails {
    pub name: String,
    pub limited: bool,
    pub reachable: bool,
    pub proxy: String,
    pub proxy_randomize_credentials: bool,
}

#[derive(Serialize)]
pub struct LocalAddress {
    pub address: String,
    pub port: u16,
    pub score: i32,
}

pub async fn getnetworkinfo(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let connections = state.connman.num_connections();

    let info = NetworkInfo {
        version: 270000, // Bitcoin Core 27.0
        subversion: "/btck-rust-node:0.1.0/".to_string(),
        protocolversion: 70016,
        localservices: "0000000000000409".to_string(),
        localservicesnames: vec!["NETWORK".to_string(), "WITNESS".to_string()],
        localrelay: true,
        timeoffset: 0,
        networkactive: true,
        connections,
        networks: vec![
            NetworkDetails {
                name: "ipv4".to_string(),
                limited: false,
                reachable: true,
                proxy: String::new(),
                proxy_randomize_credentials: false,
            },
            NetworkDetails {
                name: "ipv6".to_string(),
                limited: false,
                reachable: true,
                proxy: String::new(),
                proxy_randomize_credentials: false,
            },
        ],
        relayfee: 0.00001,
        incrementalfee: 0.00001,
        localaddresses: vec![],
        warnings: String::new(),
    };

    Ok(Json(json!(info)))
}

/// getpeerinfo
#[derive(Serialize)]
pub struct PeerInfo {
    pub id: u64,
    pub addr: String,
    pub addrbind: String,
    pub services: String,
    pub servicesnames: Vec<String>,
    pub relaytxes: bool,
    pub lastsend: i64,
    pub lastrecv: i64,
    pub bytessent: u64,
    pub bytesrecv: u64,
    pub conntime: i64,
    pub timeoffset: i64,
    pub pingtime: Option<f64>,
    pub minping: Option<f64>,
    pub version: i32,
    pub subver: String,
    pub inbound: bool,
    pub startingheight: i32,
    pub banscore: i32,
    pub synced_headers: i32,
    pub synced_blocks: i32,
}

pub async fn getpeerinfo(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let peers = state.connman.get_peer_info();
    Ok(Json(json!(peers)))
}

/// getconnectioncount
pub async fn getconnectioncount(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let count = state.connman.num_connections();
    Ok(Json(json!({ "result": count })))
}

/// addnode
#[derive(Deserialize)]
pub struct AddNodeParams {
    pub node: String,
    pub command: String, // "add", "remove", "onetry"
}

pub async fn addnode(
    State(state): State<AppState>,
    Json(params): Json<AddNodeParams>,
) -> Result<Json<Value>, StatusCode> {
    match params.command.as_str() {
        "add" => {
            let addr: SocketAddr = params.node.parse()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            
            state.connman.add_node(addr).await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            
            Ok(Json(json!({ "result": null })))
        }
        "remove" => {
            let addr: SocketAddr = params.node.parse()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            
            state.connman.remove_node(&addr).await;
            Ok(Json(json!({ "result": null })))
        }
        "onetry" => {
            let addr: SocketAddr = params.node.parse()
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            
            state.connman.connect_onetry(addr).await
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            
            Ok(Json(json!({ "result": null })))
        }
        _ => Err(StatusCode::BAD_REQUEST)
    }
}

/// disconnectnode
#[derive(Deserialize)]
pub struct DisconnectNodeParams {
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub nodeid: Option<u64>,
}

pub async fn disconnectnode(
    State(state): State<AppState>,
    Json(params): Json<DisconnectNodeParams>,
) -> Result<Json<Value>, StatusCode> {
    if let Some(nodeid) = params.nodeid {
        state.connman.disconnect_node(nodeid).await;
        Ok(Json(json!({ "result": null })))
    } else if let Some(address) = params.address {
        let addr: SocketAddr = address.parse()
            .map_err(|_| StatusCode::BAD_REQUEST)?;
        state.connman.disconnect_by_address(&addr).await;
        Ok(Json(json!({ "result": null })))
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}

/// getaddednodeinfo
#[derive(Deserialize)]
pub struct GetAddedNodeInfoParams {
    #[serde(default)]
    pub node: Option<String>,
}

#[derive(Serialize)]
pub struct AddedNodeInfo {
    pub addednode: String,
    pub connected: bool,
    pub addresses: Vec<AddedNodeAddress>,
}

#[derive(Serialize)]
pub struct AddedNodeAddress {
    pub address: String,
    pub connected: String, // "inbound" or "outbound"
}

pub async fn getaddednodeinfo(
    State(state): State<AppState>,
    Json(params): Json<GetAddedNodeInfoParams>,
) -> Result<Json<Value>, StatusCode> {
    let added_nodes = state.connman.get_added_nodes();
    
    let result: Vec<AddedNodeInfo> = if let Some(node) = params.node {
        added_nodes.into_iter()
            .filter(|n| n.addednode == node)
            .collect()
    } else {
        added_nodes
    };

    Ok(Json(json!(result)))
}

/// getnettotals
#[derive(Serialize)]
pub struct NetTotals {
    pub totalbytesrecv: u64,
    pub totalbytessent: u64,
    pub timemillis: i64,
    pub uploadtarget: UploadTarget,
}

#[derive(Serialize)]
pub struct UploadTarget {
    pub timeframe: u64,
    pub target: u64,
    pub target_reached: bool,
    pub serve_historical_blocks: bool,
    pub bytes_left_in_cycle: u64,
    pub time_left_in_cycle: u64,
}

pub async fn getnettotals(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let (bytes_recv, bytes_sent) = state.connman.get_net_totals();
    
    let totals = NetTotals {
        totalbytesrecv: bytes_recv,
        totalbytessent: bytes_sent,
        timemillis: chrono::Utc::now().timestamp_millis(),
        uploadtarget: UploadTarget {
            timeframe: 86400,
            target: 0,
            target_reached: false,
            serve_historical_blocks: true,
            bytes_left_in_cycle: 0,
            time_left_in_cycle: 0,
        },
    };

    Ok(Json(json!(totals)))
}

/// getnetworkactive
pub async fn getnetworkactive(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let active = state.connman.is_network_active();
    Ok(Json(json!({ "result": active })))
}

/// setnetworkactive
#[derive(Deserialize)]
pub struct SetNetworkActiveParams {
    pub state: bool,
}

pub async fn setnetworkactive(
    State(state): State<AppState>,
    Json(params): Json<SetNetworkActiveParams>,
) -> Result<Json<Value>, StatusCode> {
    state.connman.set_network_active(params.state).await;
    Ok(Json(json!({ "result": params.state })))
}

/// listbanned
#[derive(Serialize)]
pub struct BannedNode {
    pub address: String,
    pub banned_until: i64,
    pub ban_created: i64,
    pub ban_reason: String,
}

pub async fn listbanned(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let banned = state.connman.get_banned_list();
    Ok(Json(json!(banned)))
}

/// setban
#[derive(Deserialize)]
pub struct SetBanParams {
    pub subnet: String,
    pub command: String, // "add" or "remove"
    #[serde(default)]
    pub bantime: Option<i64>,
    #[serde(default)]
    pub absolute: bool,
}

pub async fn setban(
    State(state): State<AppState>,
    Json(params): Json<SetBanParams>,
) -> Result<Json<Value>, StatusCode> {
    match params.command.as_str() {
        "add" => {
            let bantime = params.bantime.unwrap_or(86400); // 24h default
            state.connman.ban_node(&params.subnet, bantime, params.absolute).await
                .map_err(|_| StatusCode::BAD_REQUEST)?;
            Ok(Json(json!({ "result": null })))
        }
        "remove" => {
            state.connman.unban_node(&params.subnet).await;
            Ok(Json(json!({ "result": null })))
        }
        _ => Err(StatusCode::BAD_REQUEST)
    }
}

/// clearbanned
pub async fn clearbanned(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    state.connman.clear_banned().await;
    Ok(Json(json!({ "result": null })))
}

/// ping
pub async fn ping(
    State(state): State<AppState>,
) -> Result<Json<Value>, StatusCode> {
    state.connman.ping_all().await;
    Ok(Json(json!({ "result": null })))
}

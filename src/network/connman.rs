// src/network/connman.rs
use anyhow::Result;
use bitcoin::Network;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::net::TcpStream;
use tokio::time::{Duration, Instant};

use super::node::{Node, NodeId};
use super::message::NetworkMessage;

/// Connection Manager - handles all peer connections
pub struct ConnectionManager {
    config: ConnectionConfig,
    nodes: Arc<RwLock<HashMap<NodeId, Arc<RwLock<Node>>>>>,
    next_id: Arc<RwLock<NodeId>>,
    added_nodes: Arc<RwLock<Vec<SocketAddr>>>,
    banned: Arc<RwLock<HashMap<String, BanEntry>>>,
    network_active: Arc<RwLock<bool>>,
    stats: Arc<RwLock<NetworkStats>>,
}

#[derive(Clone)]
pub struct ConnectionConfig {
    pub network: Network,
    pub max_outbound: usize,
    pub max_inbound: usize,
    pub user_agent: String,
    pub protocol_version: i32,
    pub services: u64,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            network: Network::Bitcoin,
            max_outbound: 8,
            max_inbound: 125,
            user_agent: "/btck-rust-node:0.1.0/".to_string(),
            protocol_version: 70016,
            services: 0x0409, // NETWORK | WITNESS | NETWORK_LIMITED
        }
    }
}

#[derive(Debug, Clone)]
pub struct BanEntry {
    pub banned_until: i64,
    pub ban_created: i64,
    pub reason: String,
}

#[derive(Default)]
pub struct NetworkStats {
    pub total_bytes_recv: u64,
    pub total_bytes_sent: u64,
    pub start_time: Option<Instant>,
}

impl ConnectionManager {
    pub fn new(config: ConnectionConfig) -> Self {
        Self {
            config,
            nodes: Arc::new(RwLock::new(HashMap::new())),
            next_id: Arc::new(RwLock::new(0)),
            added_nodes: Arc::new(RwLock::new(Vec::new())),
            banned: Arc::new(RwLock::new(HashMap::new())),
            network_active: Arc::new(RwLock::new(true)),
            stats: Arc::new(RwLock::new(NetworkStats {
                start_time: Some(Instant::now()),
                ..Default::default()
            })),
        }
    }

    /// Get next node ID
    async fn next_node_id(&self) -> NodeId {
        let mut id = self.next_id.write().await;
        let current = *id;
        *id += 1;
        current
    }

    /// Add a new outbound connection
    pub async fn connect(&self, addr: SocketAddr) -> Result<NodeId> {
        // Check if already connected
        let nodes = self.nodes.read().await;
        for node in nodes.values() {
            let n = node.read().await;
            if n.addr == addr {
                return Ok(n.id);
            }
        }
        drop(nodes);

        // Check if banned
        if self.is_banned(&addr).await {
            anyhow::bail!("Node is banned: {}", addr);
        }

        // Connect
        let stream = TcpStream::connect(addr).await?;
        let id = self.next_node_id().await;
        
        let node = Node::new(
            id,
            addr,
            stream,
            false, // outbound
            self.config.clone(),
        );

        let node_arc = Arc::new(RwLock::new(node));
        self.nodes.write().await.insert(id, node_arc.clone());

        // Start node handler
        let nodes_clone = self.nodes.clone();
        let stats_clone = self.stats.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::handle_node(node_arc.clone(), stats_clone).await {
                eprintln!("[connman] node {} error: {}", id, e);
            }
            // Remove node on disconnect
            nodes_clone.write().await.remove(&id);
        });

        Ok(id)
    }

    /// Handle a single node
    async fn handle_node(
        node: Arc<RwLock<Node>>,
        stats: Arc<RwLock<NetworkStats>>,
    ) -> Result<()> {
        // Send version message
        {
            let mut n = node.write().await;
            n.send_version().await?;
        }

        // Message loop
        loop {
            let msg = {
                let mut n = node.write().await;
                n.receive_message().await?
            };

            // Update stats
            {
                let mut s = stats.write().await;
                s.total_bytes_recv += msg.len() as u64;
            }

            // Process message
            match msg {
                NetworkMessage::Version(v) => {
                    let mut n = node.write().await;
                    n.handle_version(v).await?;
                }
                NetworkMessage::Verack => {
                    let mut n = node.write().await;
                    n.handle_verack().await?;
                }
                NetworkMessage::Ping(nonce) => {
                    let mut n = node.write().await;
                    n.send_pong(nonce).await?;
                }
                NetworkMessage::Pong(nonce) => {
                    let mut n = node.write().await;
                    n.handle_pong(nonce).await?;
                }
                NetworkMessage::Addr(addrs) => {
                    // Process address messages
                }
                NetworkMessage::Inv(inv) => {
                    // Process inventory
                }
                NetworkMessage::GetData(getdata) => {
                    // Handle getdata
                }
                NetworkMessage::Block(block) => {
                    // Process block
                }
                NetworkMessage::Tx(tx) => {
                    // Process transaction
                }
                _ => {
                    // Other messages
                }
            }
        }
    }

    /// Accept inbound connection
    pub async fn accept(&self, stream: TcpStream, addr: SocketAddr) -> Result<NodeId> {
        // Check if banned
        if self.is_banned(&addr).await {
            anyhow::bail!("Node is banned: {}", addr);
        }

        // Check inbound limit
        let nodes = self.nodes.read().await;
        let inbound_count = nodes.values()
            .filter(|n| {
                if let Ok(n) = n.try_read() {
                    n.inbound
                } else {
                    false
                }
            })
            .count();
        
        if inbound_count >= self.config.max_inbound {
            anyhow::bail!("Inbound connection limit reached");
        }
        drop(nodes);

        let id = self.next_node_id().await;
        let node = Node::new(
            id,
            addr,
            stream,
            true, // inbound
            self.config.clone(),
        );

        let node_arc = Arc::new(RwLock::new(node));
        self.nodes.write().await.insert(id, node_arc.clone());

        // Start handler
        let nodes_clone = self.nodes.clone();
        let stats_clone = self.stats.clone();
        tokio::spawn(async move {
            if let Err(e) = Self::handle_node(node_arc.clone(), stats_clone).await {
                eprintln!("[connman] node {} error: {}", id, e);
            }
            nodes_clone.write().await.remove(&id);
        });

        Ok(id)
    }

    /// Disconnect a node
    pub async fn disconnect_node(&self, id: NodeId) {
        if let Some(node) = self.nodes.write().await.remove(&id) {
            let n = node.write().await;
            eprintln!("[connman] disconnected node {}", id);
        }
    }

    /// Disconnect by address
    pub async fn disconnect_by_address(&self, addr: &SocketAddr) {
        let nodes = self.nodes.read().await;
        let to_disconnect: Vec<NodeId> = nodes
            .iter()
            .filter_map(|(id, node)| {
                if let Ok(n) = node.try_read() {
                    if n.addr == *addr {
                        Some(*id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();
        drop(nodes);

        for id in to_disconnect {
            self.disconnect_node(id).await;
        }
    }

    /// Get number of connections
    pub fn num_connections(&self) -> usize {
        if let Ok(nodes) = self.nodes.try_read() {
            nodes.len()
        } else {
            0
        }
    }

    /// Get peer info
    pub async fn get_peer_info(&self) -> Vec<PeerInfo> {
        let nodes = self.nodes.read().await;
        let mut peers = Vec::new();

        for node in nodes.values() {
            if let Ok(n) = node.try_read() {
                peers.push(n.get_peer_info());
            }
        }

        peers
    }

    /// Add a node to the added nodes list
    pub async fn add_node(&self, addr: SocketAddr) -> Result<()> {
        self.added_nodes.write().await.push(addr);
        self.connect(addr).await?;
        Ok(())
    }

    /// Remove node from added list
    pub async fn remove_node(&self, addr: &SocketAddr) {
        self.added_nodes.write().await.retain(|a| a != addr);
        self.disconnect_by_address(addr).await;
    }

    /// Connect to node one time
    pub async fn connect_onetry(&self, addr: SocketAddr) -> Result<()> {
        self.connect(addr).await?;
        Ok(())
    }

    /// Get added nodes
    pub async fn get_added_nodes(&self) -> Vec<AddedNodeInfo> {
        let added = self.added_nodes.read().await;
        let nodes = self.nodes.read().await;

        added.iter().map(|addr| {
            let connected = nodes.values().any(|n| {
                if let Ok(n) = n.try_read() {
                    n.addr == *addr
                } else {
                    false
                }
            });

            AddedNodeInfo {
                addednode: addr.to_string(),
                connected,
                addresses: vec![],
            }
        }).collect()
    }

    /// Ban a node
    pub async fn ban_node(&self, subnet: &str, bantime: i64, absolute: bool) -> Result<()> {
        let now = chrono::Utc::now().timestamp();
        let banned_until = if absolute {
            bantime
        } else {
            now + bantime
        };

        let entry = BanEntry {
            banned_until,
            ban_created: now,
            reason: "manually added".to_string(),
        };

        self.banned.write().await.insert(subnet.to_string(), entry);
        Ok(())
    }

    /// Unban a node
    pub async fn unban_node(&self, subnet: &str) {
        self.banned.write().await.remove(subnet);
    }

    /// Clear all bans
    pub async fn clear_banned(&self) {
        self.banned.write().await.clear();
    }

    /// Get banned list
    pub async fn get_banned_list(&self) -> Vec<BannedNode> {
        self.banned.read().await
            .iter()
            .map(|(addr, entry)| BannedNode {
                address: addr.clone(),
                banned_until: entry.banned_until,
                ban_created: entry.ban_created,
                ban_reason: entry.reason.clone(),
            })
            .collect()
    }

    /// Check if address is banned
    async fn is_banned(&self, addr: &SocketAddr) -> bool {
        let banned = self.banned.read().await;
        let now = chrono::Utc::now().timestamp();
        
        banned.values().any(|entry| entry.banned_until > now)
    }

    /// Get network totals
    pub async fn get_net_totals(&self) -> (u64, u64) {
        let stats = self.stats.read().await;
        (stats.total_bytes_recv, stats.total_bytes_sent)
    }

    /// Check if network is active
    pub async fn is_network_active(&self) -> bool {
        *self.network_active.read().await
    }

    /// Set network active state
    pub async fn set_network_active(&self, active: bool) {
        *self.network_active.write().await = active;
    }

    /// Send ping to all peers
    pub async fn ping_all(&self) {
        let nodes = self.nodes.read().await;
        for node in nodes.values() {
            if let Ok(mut n) = node.try_write() {
                let _ = n.send_ping().await;
            }
        }
    }
}

// Re-export types for RPC
use crate::rpc::network::{PeerInfo, AddedNodeInfo, BannedNode};

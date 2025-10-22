use anyhow::{anyhow, Result};
use bitcoin::Network;
use bitcoin::p2p::{message, Magic, ServiceFlags};
use std::net::SocketAddr;
use std::time::SystemTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Peer connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerState {
    Connecting,
    Handshaking,
    Connected,
    Disconnected,
}

/// Peer connection
pub struct Peer {
    /// Network
    pub network: Network,

    /// Magic bytes
    pub magic: Magic,

    /// Socket address
    pub addr: SocketAddr,

    /// TCP stream
    stream: TcpStream,

    /// Peer services
    pub services: ServiceFlags,

    /// Peer version
    pub version: i32,

    /// Peer user agent
    pub user_agent: String,

    /// Peer start height
    pub start_height: i32,

    /// Connection state
    pub state: PeerState,

    /// Last ping nonce
    pub last_ping: Option<u64>,

    /// Last ping time
    pub last_ping_time: Option<SystemTime>,

    /// Supports sendheaders
    pub sendheaders: bool,

    /// Supports witness
    pub witness: bool,

    /// Fee filter (minimum fee rate)
    pub fee_filter: Option<u64>,
}

impl Peer {
    pub async fn connect(addr: SocketAddr, network: Network) -> Result<Self> {
        let stream = TcpStream::connect(addr).await?;

        Ok(Self {
            network,
            magic: network.magic(),
            addr,
            stream,
            services: ServiceFlags::NONE,
            version: 0,
            user_agent: String::new(),
            start_height: 0,
            state: PeerState::Connecting,
            last_ping: None,
            last_ping_time: None,
            sendheaders: false,
            witness: false,
            fee_filter: None,
        })
    }

    pub async fn send(&mut self, msg: message::NetworkMessage) -> Result<()> {
        let raw = message::RawNetworkMessage::new(self.magic, msg);
        let bytes = bitcoin::consensus::encode::serialize(&raw);
        self.stream.write_all(&bytes).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<message::NetworkMessage> {
        // Read header (24 bytes)
        let mut header = [0u8; 24];
        self.stream.read_exact(&mut header).await?;

        // Extract payload length
        let len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;

        // Read payload
        let mut payload = vec![0u8; len];
        if len > 0 {
            self.stream.read_exact(&mut payload).await?;
        }

        // Deserialize
        let raw: message::RawNetworkMessage =
            bitcoin::consensus::deserialize(&[&header[..], &payload[..]].concat())?;

        Ok(raw.into_payload())
    }

    pub fn is_connected(&self) -> bool {
        self.state == PeerState::Connected
    }

    pub fn supports_witness(&self) -> bool {
        self.witness
    }
}

use anyhow::{anyhow, Result};
use bitcoin::block::Header as BlockHeader;
use bitcoin::blockdata::constants::genesis_block;
use bitcoin::consensus::encode;
use bitcoin::hashes::{sha256d, Hash as _};
use bitcoin::p2p::{
    self,
    address,
    message,
    message_blockdata as msg_blk,
    message_compact_blocks as msg_cmpct,
    message_network as msg_net,
};
use bitcoin::{BlockHash, Network};
use rand::Rng;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter, Read as StdRead, Write as StdWrite};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpStream};
use tokio::time::{sleep, timeout};
use tokio::task::spawn_blocking;
use tokio::sync::mpsc;

use crate::chainparams::ChainParams;
use crate::seeds;

/// 광고할 프로토콜 번호(현대 피어 경로를 열기 위해 70016 사용)
const ADVERTISED_PROTO: u32 = 70016;

// 타임아웃/윈도
const HDRS_TIMEOUT: Duration = Duration::from_secs(60);
const BLK_TIMEOUT: Duration = Duration::from_secs(120);
const STALL_LIMIT: Duration = Duration::from_secs(15 * 60);
const INITIAL_REREQ_SECS: u64 = 2;      // Initial request: 2 seconds
const IMMEDIATE_REQ_TIMEOUT: u64 = 60;  // After immediate request on full batch: 60 seconds (give peer time to respond)
const MAX_HEADERS_PER_MSG: usize = 2000;

// 전역/피어별 인플라이트 제한
// Block download concurrency limits
// Lower values ensure blocks arrive in order, reducing orphan blocks
// Bitcoin Core uses higher values but has sophisticated block ordering logic
const GLOBAL_INFLIGHT: usize = 16;  // Max total blocks downloading simultaneously
const PER_PEER_INFLIGHT: usize = 4; // Max blocks per peer to reduce out-of-order arrival

const MAX_OUTBOUND_FROM_ADDR: usize = 8;

/// 단순 피어 연결
pub struct Peer {
    net: Network,
    magic: p2p::Magic,
    stream: TcpStream,
    pub their_services: p2p::ServiceFlags,
    pub their_start_height: i32,  // 피어의 블록 높이
    negotiated: bool,
    sendheaders_sent: bool,
    wtxidrelay_sent: bool,
    verack_seen: bool,
}

impl Peer {
    pub async fn connect(addr: SocketAddr, net: Network) -> Result<Self> {
        eprintln!("[p2p] connecting to {addr}");
        let stream = match tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(addr)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(anyhow!("connect failed: {e}")),
            Err(_) => return Err(anyhow!("connect timeout after 5s")),
        };
        Ok(Self {
            net,
            magic: net.magic(),
            stream,
            their_services: p2p::ServiceFlags::NONE,
            their_start_height: 0,
            negotiated: false,
            sendheaders_sent: false,
            wtxidrelay_sent: false,
            verack_seen: false,
        })
    }

    pub async fn send(&mut self, msg: message::NetworkMessage) -> Result<()> {
        let raw = message::RawNetworkMessage::new(self.magic, msg);
        let bytes = encode::serialize(&raw);
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;  // CRITICAL: Ensure data is sent to peer!
        Ok(())
    }

    async fn recv(&mut self) -> Result<message::NetworkMessage> {
        let mut header = [0u8; 24];
        self.stream.read_exact(&mut header).await?;
        let len = u32::from_le_bytes(header[16..20].try_into().unwrap()) as usize;

        let mut payload = vec![0u8; len];
        self.stream.read_exact(&mut payload).await?;

        let raw: message::RawNetworkMessage =
            bitcoin::consensus::deserialize(&[&header[..], &payload[..]].concat())?;

        Ok(raw.into_payload())
    }

    pub async fn handshake(&mut self, user_agent: &str, start_height: i32, our_services: p2p::ServiceFlags) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut vm = msg_net::VersionMessage::new(
            our_services,  // Use passed-in ServiceFlags instead of hardcoded
            now,
            address::Address::new(&"0.0.0.0:0".parse().unwrap(), p2p::ServiceFlags::NONE),
            address::Address::new(&"0.0.0.0:0".parse().unwrap(), p2p::ServiceFlags::NONE),
            rand::thread_rng().gen::<u64>(),
            user_agent.into(),
            start_height,
        );
        vm.version = ADVERTISED_PROTO;

        self.send(message::NetworkMessage::Version(vm)).await?;
        eprintln!("[p2p] sent Version (ua={user_agent}, proto={}, services={:?})", ADVERTISED_PROTO, our_services);

        let mut got_version = false;
        let mut got_verack = false;

        for _ in 0..50 {
            let msg = timeout(Duration::from_secs(10), self.recv()).await??;
            match msg {
                message::NetworkMessage::Version(peer_vm) => {
                    eprintln!(
                        "[p2p] recv Version: ua={} height={} services={:?}",
                        peer_vm.user_agent, peer_vm.start_height, peer_vm.services
                    );
                    self.their_services = peer_vm.services;
                    self.their_start_height = peer_vm.start_height;  // 피어 높이 저장

                    // CRITICAL: BIP 339 - WtxidRelay MUST be sent BEFORE Verack!
                    // Protocol version >= 70016 requires this order
                    if !self.wtxidrelay_sent && peer_vm.version >= 70016 {
                        self.send(message::NetworkMessage::WtxidRelay).await?;
                        self.wtxidrelay_sent = true;
                        eprintln!("[p2p] sent WtxidRelay (before Verack - BIP 339)");
                    }

                    // Now send Verack
                    self.send(message::NetworkMessage::Verack).await?;
                    eprintln!("[p2p] sent Verack");
                    got_version = true;
                }
                message::NetworkMessage::Verack => {
                    eprintln!("[p2p] recv Verack");
                    self.verack_seen = true;
                    // Verack 이후에 기능 협상 (SendHeaders, SendCmpct)
                    // Note: WtxidRelay already sent before Verack (BIP 339 requirement)
                    if !self.sendheaders_sent {
                        self.send(message::NetworkMessage::SendHeaders).await?;
                        self.sendheaders_sent = true;
                        eprintln!("[p2p] sent SendHeaders");
                    }
                    // Send SendCmpct after Verack
                    self.send(message::NetworkMessage::SendCmpct(msg_cmpct::SendCmpct {
                        version: 1,
                        send_compact: true,
                    })).await?;
                    eprintln!("[p2p] sent SendCmpct(high)");
                    got_verack = true;
                }
                other => {
                    eprintln!("[p2p] recv during handshake: {:?}", other.command());
                }
            }
            if got_version && got_verack {
                self.negotiated = true;
                let _ = self.send(message::NetworkMessage::GetAddr).await;
                eprintln!("[p2p] handshake complete (+GetAddr)");
                return Ok(());
            }
        }
        Err(anyhow!("handshake timeout"))
    }
}

/// 블록 다운로드 큐(전역/피어별 윈도)
pub struct Downloader {
    inflight: HashMap<BlockHash, (SocketAddr, tokio::time::Instant)>,
    per_peer: HashMap<SocketAddr, usize>,
    global_window: usize,
    per_peer_window: usize,
    queue: VecDeque<BlockHash>,
    // Track download progress
    total_blocks: usize,        // Total blocks to download
    downloaded_blocks: usize,   // Blocks completed
}
impl Downloader {
    pub fn new(global_window: usize, per_peer_window: usize) -> Self {
        Self {
            inflight: HashMap::new(),
            per_peer: HashMap::new(),
            global_window,
            per_peer_window,
            queue: VecDeque::new(),
            total_blocks: 0,
            downloaded_blocks: 0,
        }
    }
    pub fn push_many(&mut self, v: impl IntoIterator<Item = BlockHash>) {
        for h in v {
            self.queue.push_back(h);
            self.total_blocks += 1;
        }
    }
    pub fn get_progress(&self) -> (usize, usize, f64) {
        let percentage = if self.total_blocks > 0 {
            (self.downloaded_blocks as f64 / self.total_blocks as f64) * 100.0
        } else {
            0.0
        };
        (self.downloaded_blocks, self.total_blocks, percentage)
    }
    pub fn poll_assign(&mut self, addr: SocketAddr) -> Vec<BlockHash> {
        let mut out = vec![];
        loop {
            if self.inflight.len() >= self.global_window { break; }
            let n_for_peer = *self.per_peer.get(&addr).unwrap_or(&0);
            if n_for_peer >= self.per_peer_window { break; }

            if let Some(h) = self.queue.pop_front() {
                self.inflight.insert(h, (addr, tokio::time::Instant::now() + BLK_TIMEOUT));
                *self.per_peer.entry(addr).or_default() += 1;
                out.push(h);
            } else {
                break;
            }
        }
        out
    }
    pub fn complete(&mut self, h: &BlockHash) {
        if let Some((addr, _)) = self.inflight.remove(h) {
            if let Some(n) = self.per_peer.get_mut(&addr) {
                if *n > 0 { *n -= 1; }
            }
            self.downloaded_blocks += 1;  // Increment completed counter
        }
    }
    pub fn reassign_timeouts(&mut self) -> Vec<BlockHash> {
        let now = tokio::time::Instant::now();
        let mut expired = vec![];
        let mut dec: HashMap<SocketAddr, usize> = HashMap::new();

        self.inflight.retain(|h, (a, dl)| {
            if *dl <= now {
                expired.push(*h);
                *dec.entry(*a).or_default() += 1;
                false
            } else { true }
        });
        for (a, n) in dec {
            if let Some(c) = self.per_peer.get_mut(&a) {
                *c = c.saturating_sub(n);
            }
        }
        expired
    }
}

/// Headers-first IBD 매니저
pub struct PeerManager {
    net: Network,
    user_agent: String,
    peers: HashMap<SocketAddr, Peer>,
    downloader: Downloader,

    prev_map: HashMap<BlockHash, BlockHash>, // child -> parent
    have_header: HashSet<BlockHash>,
    best_header_tip: BlockHash,

    recent_chain: Vec<BlockHash>,
    last_locator: Vec<BlockHash>,
    start_height: i32,  // Current blockchain height

    // Headers-First Sync 상태 추적 (Bitcoin Core 방식)
    headers_synced: bool,                       // 헤더 동기화 완료 여부
    peer_heights: HashMap<SocketAddr, i32>,     // 각 피어의 start_height
    best_known_height: i32,                     // 네트워크의 최고 높이
    header_chain_height: i32,                   // 현재 헤더 체인의 높이
    header_chain: Vec<BlockHeader>,             // 실제 헤더 체인 (디스크에 저장됨)
    sync_peer: Option<SocketAddr>,              // Bitcoin Core: ONE headers sync peer

    // Bitcoin Core-style chain parameters
    chain_params: ChainParams,                  // Checkpoints, AssumeValid, MinimumChainWork

    on_block: Option<Arc<dyn Fn(&[u8]) -> anyhow::Result<()> + Send + Sync>>,
    on_tx: Option<Arc<dyn Fn(&bitcoin::Transaction) -> anyhow::Result<()> + Send + Sync>>,

    // Sequential block processing channel
    block_tx: Option<mpsc::UnboundedSender<(BlockHash, Vec<u8>)>>,
}

impl PeerManager {
    pub fn new(net: Network, user_agent: &str) -> Self {
        Self::with_start_height(net, user_agent, 0)
    }

    /// Get the path to the headers storage file
    fn get_headers_file_path(net: Network) -> PathBuf {
        let filename = match net {
            Network::Bitcoin => "headers_mainnet.dat",
            Network::Testnet => "headers_testnet.dat",
            Network::Signet => "headers_signet.dat",
            Network::Regtest => "headers_regtest.dat",
            _ => "headers_unknown.dat",
        };
        PathBuf::from(filename)
    }

    /// Load headers from disk (genesis not included, starts from height 1)
    fn load_headers_from_disk(net: Network) -> Result<Vec<BlockHeader>> {
        let path = Self::get_headers_file_path(net);

        if !path.exists() {
            eprintln!("[p2p] No saved headers file found at {:?}", path);
            return Ok(Vec::new());
        }

        let file = File::open(&path)?;
        let mut reader = BufReader::new(file);
        let mut headers = Vec::new();
        let mut buffer = vec![0u8; 80]; // BlockHeader is 80 bytes

        loop {
            match reader.read_exact(&mut buffer) {
                Ok(()) => {
                    let header: BlockHeader = encode::deserialize(&buffer)?;
                    headers.push(header);
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    // End of file reached
                    break;
                }
                Err(e) => return Err(anyhow!("Failed to read header: {}", e)),
            }
        }

        eprintln!("[p2p] ✓ Loaded {} headers from disk ({:?})", headers.len(), path);
        Ok(headers)
    }

    /// Save all headers to disk (genesis not included, starts from height 1)
    fn save_headers_to_disk(&self) -> Result<()> {
        let path = Self::get_headers_file_path(self.net);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        let mut writer = BufWriter::new(file);

        for header in &self.header_chain {
            let bytes = encode::serialize(header);
            writer.write_all(&bytes)?;
        }

        writer.flush()?;
        eprintln!("[p2p] ✓ Saved {} headers to disk ({:?})", self.header_chain.len(), path);
        Ok(())
    }

    /// Append a single header to the disk file (incremental save)
    fn append_header_to_disk(&self, header: &BlockHeader) -> Result<()> {
        let path = Self::get_headers_file_path(self.net);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .append(true)
            .open(&path)?;
        let mut writer = BufWriter::new(file);

        let bytes = encode::serialize(header);
        writer.write_all(&bytes)?;
        writer.flush()?;

        Ok(())
    }

    pub fn with_start_height(net: Network, user_agent: &str, start_height: i32) -> Self {
        let g = genesis_block(net).block_hash();
        let mut have = HashSet::new();
        have.insert(g);

        let chain_params = ChainParams::for_network(net);

        eprintln!("[p2p] Initializing PeerManager with start_height={}", start_height);
        eprintln!("[p2p] Starting in HEADERS-FIRST SYNC mode");
        eprintln!("[p2p] Network: {:?}", net);
        eprintln!("[p2p] Checkpoints loaded: {}", chain_params.checkpoints.len());
        if let Some(ref av) = chain_params.assume_valid {
            eprintln!("[p2p] AssumeValid: {}", av);
        }

        // Load saved headers from disk
        let loaded_headers = Self::load_headers_from_disk(net).unwrap_or_else(|e| {
            eprintln!("[p2p] ⚠️  Failed to load headers from disk: {}", e);
            Vec::new()
        });

        // Build header chain state from loaded headers
        let mut prev_map = HashMap::new();
        let mut recent_chain = vec![g];
        let mut prev_hash = g;

        for header in &loaded_headers {
            let h = header.block_hash();
            have.insert(h);
            prev_map.insert(h, header.prev_blockhash);
            recent_chain.push(h);
            prev_hash = h;
        }

        let header_chain_height = loaded_headers.len() as i32;
        let best_header_tip = prev_hash;

        // Determine if headers are synced
        // Headers are synced if we have headers loaded OR if we have blocks
        let headers_already_synced = header_chain_height >= start_height && start_height > 0;

        // Prepare block download queue if headers are already synced
        let mut downloader = Downloader::new(GLOBAL_INFLIGHT, PER_PEER_INFLIGHT);

        if loaded_headers.is_empty() {
            eprintln!("[p2p] 🆕 Starting fresh sync from genesis");
        } else {
            eprintln!("[p2p] 🔄 Resuming from saved headers:");
            eprintln!("[p2p]    Headers loaded: {} (height: {})", loaded_headers.len(), header_chain_height);
            eprintln!("[p2p]    Blocks downloaded: {}", start_height);
            eprintln!("[p2p]    Best header tip: {}", best_header_tip);

            if headers_already_synced {
                eprintln!("[p2p]    ✓ Headers synced - preparing block download queue");

                // Queue remaining blocks for download (skip already downloaded blocks)
                let skip_count = (start_height as usize) + 1;
                let mut blocks_to_download = Vec::new();

                for hash in recent_chain.iter().skip(skip_count) {
                    blocks_to_download.push(*hash);
                }

                if blocks_to_download.is_empty() {
                    eprintln!("[p2p]    ✓ All blocks already downloaded! Nothing to do.");
                } else {
                    eprintln!("[p2p]    📊 Queuing {} blocks (heights {} to {})",
                             blocks_to_download.len(), start_height + 1, header_chain_height);
                    downloader.push_many(blocks_to_download);
                }
            } else if header_chain_height > 0 {
                eprintln!("[p2p]    ⏩ Will continue header sync from height {}", header_chain_height);
            }
        }

        Self {
            net,
            user_agent: user_agent.into(),
            peers: HashMap::new(),
            downloader,  // Use the downloader we prepared above (with queued blocks if resuming)
            prev_map,
            have_header: have,
            best_header_tip,
            recent_chain,
            last_locator: vec![g],
            start_height,
            headers_synced: headers_already_synced,
            peer_heights: HashMap::new(),
            best_known_height: 0,
            header_chain_height,
            header_chain: loaded_headers,
            sync_peer: None,
            chain_params,
            on_block: None,
            on_tx: None,
            block_tx: None,
        }
    }

    pub fn with_block_processor<F>(mut self, f: F) -> Self
    where
        F: Fn(&[u8]) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        let callback = Arc::new(f);
        self.on_block = Some(callback.clone());

        // Create sequential block processing channel
        let (tx, mut rx) = mpsc::unbounded_channel::<(BlockHash, Vec<u8>)>();
        self.block_tx = Some(tx);

        // Spawn dedicated sequential block processor task
        // This ensures blocks are processed in the order they arrive (not in parallel)
        tokio::spawn(async move {
            eprintln!("[p2p] Sequential block processor started");
            while let Some((block_hash, raw)) = rx.recv().await {
                match spawn_blocking({
                    let raw = raw.clone();
                    let cb = callback.clone();
                    move || (cb)(&raw)
                }).await {
                    Ok(Ok(())) => {
                        eprintln!("[p2p] ✓ Block {} saved to chain", block_hash);
                    }
                    Ok(Err(e)) => {
                        eprintln!("[p2p] ✗ Failed to process block {}: {:#}", block_hash, e);
                    }
                    Err(e) => {
                        eprintln!("[p2p] ✗ Spawn error for block {}: {:#}", block_hash, e);
                    }
                }
            }
            eprintln!("[p2p] Sequential block processor stopped");
        });

        self
    }

    pub fn with_tx_processor<F>(mut self, f: F) -> Self
    where
        F: Fn(&bitcoin::Transaction) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        self.on_tx = Some(Arc::new(f));
        self
    }
    pub fn peers_len(&self) -> usize { self.peers.len() }

    pub async fn add_outbound(&mut self, addr: SocketAddr) -> Result<()> {
        if self.peers.contains_key(&addr) { return Ok(()); }
        let mut p = Peer::connect(addr, self.net).await?;

        // CRITICAL: Don't advertise NETWORK during IBD!
        // If we advertise NETWORK, peers expect us to have headers
        // When we only have genesis, they think we're broken and disconnect
        // Only advertise WITNESS during IBD
        let our_services = if self.headers_synced {
            p2p::ServiceFlags::NETWORK | p2p::ServiceFlags::WITNESS
        } else {
            p2p::ServiceFlags::WITNESS  // IBD: Only WITNESS, no NETWORK
        };

        // CRITICAL: Use self.start_height, not a parameter
        // start_height represents OUR current blockchain height (blocks we have)
        // During IBD this should be 0 (or actual verified block count)
        p.handshake(&self.user_agent, self.start_height, our_services).await?;

        // 피어의 높이를 추적
        let peer_height = p.their_start_height;
        let peer_services = p.their_services;
        self.peer_heights.insert(addr, peer_height);

        // 네트워크의 최고 높이 갱신
        if peer_height > self.best_known_height {
            self.best_known_height = peer_height;
            eprintln!("[p2p] Updated best known height: {} from peer {}", peer_height, addr);
        }

        self.peers.insert(addr, p);

        // CRITICAL FIX: Select sync peer that can actually serve headers!
        // Bitcoin Core: Choose a peer that:
        // 1. Advertises NODE_NETWORK (willing to serve full data)
        // 2. Has height > 0 (actually has headers to share)
        // 3. Preferably the one with highest height
        if !self.headers_synced {
            let has_network_service = peer_services.has(p2p::ServiceFlags::NETWORK);
            let has_headers = peer_height > 0;

            if has_network_service && has_headers {
                // Check if we should replace current sync peer with a better one
                let should_select = if let Some(current_sync) = self.sync_peer {
                    // Replace if new peer has more headers
                    if let Some(&current_height) = self.peer_heights.get(&current_sync) {
                        peer_height > current_height
                    } else {
                        true  // Current sync peer not found, replace
                    }
                } else {
                    true  // No sync peer yet, select this one
                };

                if should_select {
                    self.sync_peer = Some(addr);
                    eprintln!("[p2p] ⭐ Selected {} as HEADERS SYNC PEER (height={}, services={:?})",
                             addr, peer_height, peer_services);
                }
            } else {
                eprintln!("[p2p] ⚠️  Peer {} not suitable for sync (network={}, height={})",
                         addr, has_network_service, peer_height);
            }
        }
        Ok(())
    }

    /// DNS 부트스트랩 (최대 연결/시도 제한)
    pub async fn bootstrap(&mut self) -> Result<usize> {
        let max_boot = 6usize;
        let mut attempts = 0usize;
        let mut connected = 0usize;

        for &seed in seeds::dns_seeds(self.net) {
            eprintln!("[bootstrap] seed={seed}");
            let default_port = match self.net {
                Network::Bitcoin  => 8333,
                Network::Testnet  => 18333,
                Network::Testnet4 => 48333,
                Network::Signet   => 38333,
                Network::Regtest  => 18444,
            };
            let target = if seed.contains(':') { seed.to_string() } else { format!("{}:{}", seed, default_port) };
            match lookup_host(target).await {
                Ok(addrs) => {
                    for addr in addrs {
                        if connected >= max_boot || attempts >= 30 { return Ok(connected); }
                        attempts += 1;
                        if self.peers.contains_key(&addr) { continue; }
                        match self.add_outbound(addr).await {
                            Ok(_) => { eprintln!("[bootstrap] connected to {addr}"); connected += 1; }
                            Err(e) => eprintln!("[bootstrap] connect failed {addr}: {e:#}"),
                        }
                    }
                }
                Err(e) => eprintln!("[bootstrap] DNS resolve failed: {e:#}"),
            }
            if connected >= max_boot || attempts >= 30 { break; }
        }
        Ok(connected)
    }

    /// Bitcoin Core-style exponential backoff block locator
    fn build_locator(&self) -> Vec<BlockHash> {
        let mut loc = Vec::with_capacity(32);
        let mut step = 1usize;
        let mut idx = self.recent_chain.len().saturating_sub(1);

        // Bitcoin Core: Add hashes with exponential backoff
        // First 10: step=1, then step doubles: 2,4,8,16,32...
        while loc.len() < 32 {
            loc.push(self.recent_chain[idx]);

            if idx == 0 {
                break;  // Reached genesis
            }

            // Exponentially larger steps back, starting after first 10 elements
            if loc.len() >= 10 {
                step *= 2;
            }

            // Walk back by 'step' blocks
            idx = idx.saturating_sub(step);
        }

        // Always ensure genesis is included (Bitcoin Core does this)
        if *loc.last().unwrap() != self.recent_chain[0] {
            loc.push(self.recent_chain[0]);
        }

        eprintln!("[p2p] Built locator from height {} with {} hashes: [{}, ...]",
                 self.header_chain_height, loc.len(), loc[0]);

        loc
    }

    async fn request_headers(&mut self, to: SocketAddr) -> Result<()> {
        if !self.peers.contains_key(&to) { return Ok(()); }
        self.last_locator = self.build_locator();

        eprintln!("[p2p] >>> Requesting headers from {to}");
        eprintln!("[p2p]     Locator has {} hashes:", self.last_locator.len());
        for (i, hash) in self.last_locator.iter().take(5).enumerate() {
            eprintln!("[p2p]       [{i}] {hash}");
        }
        if self.last_locator.len() > 5 {
            eprintln!("[p2p]       ... and {} more", self.last_locator.len() - 5);
        }

        // Use same protocol version as advertised in Version message
        // GetHeaders version should match our advertised protocol version
        let gh = msg_blk::GetHeadersMessage {
            version: ADVERTISED_PROTO,
            locator_hashes: self.last_locator.clone(),
            stop_hash: BlockHash::from_raw_hash(sha256d::Hash::all_zeros()),
        };
        if let Some(p) = self.peers.get_mut(&to) {
            p.send(message::NetworkMessage::GetHeaders(gh)).await?;
            eprintln!("[p2p]     GetHeaders sent (version={}, {} locators)", ADVERTISED_PROTO, self.last_locator.len());
        }
        Ok(())
    }

    /// 새 헤더 확장 (Bitcoin Core 방식 - 헤더만 처리)
    /// Returns the number of new headers actually added
    fn extend_headers(&mut self, new_headers: &[BlockHeader]) -> usize {
        if new_headers.is_empty() {
            return 0;
        }

        // Bitcoin Core behavior: Find first new header and add from there
        // Process headers sequentially, verify connections

        let mut added_count = 0;
        let mut duplicate_count = 0;
        let current_tip = *self.recent_chain.last().unwrap();
        let mut processing_tip = current_tip;

        // Debug: Show range of received headers
        let first_hash = new_headers[0].block_hash();
        let last_hash = new_headers[new_headers.len() - 1].block_hash();
        eprintln!("[p2p] Processing {} headers: first={}, last={}, current_tip={}",
                 new_headers.len(), first_hash, last_hash, current_tip);

        for (idx, hh) in new_headers.iter().enumerate() {
            let h = hh.block_hash();

            // Skip if we already have this header
            if self.have_header.contains(&h) {
                duplicate_count += 1;
                // Update processing_tip to this header (it's in our chain)
                processing_tip = h;
                if idx < 5 || idx >= new_headers.len() - 3 {
                    eprintln!("[p2p]   [{}] DUPLICATE: {} (prev={})",
                             idx, h, hh.prev_blockhash);
                }
                continue;
            }

            // This is a NEW header - check if it connects
            if hh.prev_blockhash != processing_tip {
                eprintln!("[p2p] ⚠️  Header chain break at index {}!", idx);
                eprintln!("[p2p]     Expected prev={}, got prev={}", processing_tip, hh.prev_blockhash);
                eprintln!("[p2p]     Header hash={}, height would be {}", h, self.header_chain_height + 1);
                break;
            }

            // Check if this is a checkpoint height - Bitcoin Core style validation
            let next_height = (self.header_chain_height + 1) as u32;
            if let Some(checkpoint_hash) = self.chain_params.get_checkpoint(next_height) {
                if h != checkpoint_hash {
                    eprintln!("[p2p] ❌ CHECKPOINT MISMATCH at height {}!", next_height);
                    eprintln!("[p2p]    Expected: {}", checkpoint_hash);
                    eprintln!("[p2p]    Received: {}", h);
                    eprintln!("[p2p]    This peer is on a different chain - rejecting!");
                    // Don't add any more headers from this batch
                    break;
                } else {
                    eprintln!("[p2p] ✓ Checkpoint verified at height {}: {}", next_height, h);
                }
            }

            // Add to our chain
            self.prev_map.insert(h, hh.prev_blockhash);
            self.have_header.insert(h);
            self.recent_chain.push(h);
            self.header_chain.push(hh.clone());  // Store actual header
            self.header_chain_height += 1;
            processing_tip = h;  // Update tip for next header
            added_count += 1;

            // Incrementally append to disk for persistence
            if let Err(e) = self.append_header_to_disk(hh) {
                eprintln!("[p2p] ⚠️  Failed to save header to disk: {}", e);
            }

            if idx < 5 || idx >= new_headers.len() - 3 {
                eprintln!("[p2p]   [{}] ADDED: {} (prev={}, height={})",
                         idx, h, hh.prev_blockhash, self.header_chain_height);
            }
        }

        if added_count > 0 {
            eprintln!("[p2p] ✓ Added {} new headers to chain (height now: {}, duplicates: {})",
                     added_count, self.header_chain_height, duplicate_count);
        } else {
            eprintln!("[p2p] ✗ No headers added! (height remains: {}, duplicates: {})",
                     self.header_chain_height, duplicate_count);
        }

        // Update best_header_tip to the latest in recent_chain
        if let Some(&tip) = self.recent_chain.last() {
            self.best_header_tip = tip;
        }

        added_count
    }

    /// 헤더 동기화가 완료되었는지 확인 (Bitcoin Core 방식)
    fn check_headers_sync_complete(&mut self) {
        if self.headers_synced {
            return;
        }

        // 헤더 체인이 알려진 최고 높이에 근접했는지 확인
        // Bitcoin Core는 약간의 여유를 두고 체크함 (144 블록 = 1일)
        const HEADER_SYNC_THRESHOLD: i32 = 144;

        if self.best_known_height > 0 &&
           self.header_chain_height >= self.best_known_height - HEADER_SYNC_THRESHOLD {
            self.headers_synced = true;
            eprintln!("╔════════════════════════════════════════════════════════════╗");
            eprintln!("║  HEADERS SYNC COMPLETE!                                    ║");
            eprintln!("║  Header chain height: {}                               ║", self.header_chain_height);
            eprintln!("║  Best known height:   {}                               ║", self.best_known_height);
            eprintln!("║  Now starting BLOCK DOWNLOAD phase...                      ║");
            eprintln!("╚════════════════════════════════════════════════════════════╝");

            // 헤더 동기화 완료 후 블록 다운로드 큐 준비
            self.queue_blocks_from_headers();
        }
    }

    /// 헤더 동기화 완료 후 아직 다운로드하지 않은 블록만 큐에 추가
    fn queue_blocks_from_headers(&mut self) {
        let mut blocks_to_download = Vec::new();

        // start_height까지는 이미 다운로드됨
        // recent_chain[0] = genesis (height 0)
        // recent_chain[start_height] = height start_height의 블록
        // 다운로드 시작: recent_chain[start_height + 1] 부터
        let skip_count = (self.start_height as usize) + 1;

        eprintln!("[p2p] 📊 Block download planning:");
        eprintln!("[p2p]    Total headers: {}", self.recent_chain.len() - 1);  // -1 for genesis
        eprintln!("[p2p]    Already downloaded: {} (heights 0-{})", self.start_height, self.start_height);
        eprintln!("[p2p]    Remaining to download: {}", self.recent_chain.len().saturating_sub(skip_count));

        // recent_chain의 이미 다운로드된 블록은 건너뛰고, 나머지만 큐에 추가
        for hash in self.recent_chain.iter().skip(skip_count) {
            blocks_to_download.push(*hash);
        }

        if blocks_to_download.is_empty() {
            eprintln!("[p2p] ✓ All blocks already downloaded! Nothing to do.");
        } else {
            eprintln!("[p2p] Queuing {} blocks for download (starting from height {})",
                     blocks_to_download.len(), self.start_height + 1);
            self.downloader.push_many(blocks_to_download);
        }
    }

    async fn respond_getheaders(&mut self, from: SocketAddr, req: &msg_blk::GetHeadersMessage) -> Result<()> {
        eprintln!("[p2p] <<< Peer {from} requested headers with {} locators", req.locator_hashes.len());

        // Bitcoin Core behavior: Send headers we have after the common ancestor
        // During IBD: we only have genesis, and peer also has genesis (common ancestor)
        // So we send empty list (no headers after genesis that we know of)
        // This is CORRECT behavior - we're not advertising NODE_NETWORK during IBD
        let headers_response: Vec<BlockHeader> = Vec::new();

        if let Some(p) = self.peers.get_mut(&from) {
            p.send(message::NetworkMessage::Headers(headers_response)).await?;
            if !self.headers_synced {
                eprintln!("[p2p]     >>> Sent empty Headers (IBD - no headers beyond genesis yet)");
            } else {
                eprintln!("[p2p]     >>> Sent empty Headers response");
            }
        }

        // Note: In the future, when we have more headers:
        // 1. Find the common ancestor from req.locator_hashes
        // 2. Send up to 2000 headers starting AFTER that point

        Ok(())
    }

    pub async fn event_loop(&mut self) -> Result<()> {
        let mut last_headers_ts = tokio::time::Instant::now();

        loop {
            // 피어 없으면 재부트스트랩
            if self.peers.is_empty() {
                self.sync_peer = None;  // Reset sync peer
                let _ = self.bootstrap().await?;
                if self.peers.is_empty() {
                    sleep(Duration::from_millis(200)).await;
                    continue;
                }
                // sync_peer는 add_outbound에서 자동으로 설정됨
            }

            // 타임아웃된 블록 재할당
            for h in self.downloader.reassign_timeouts() { self.downloader.push_many([h]); }

            // 모든 피어를 라운드로빈 폴링
            // IBD 중에는 더 긴 타임아웃 사용 (Headers 메시지는 클 수 있음)
            // Bitcoin Core: 첫 GetHeaders 응답은 최대 2000 headers (160KB)
            let recv_timeout = if self.headers_synced {
                Duration::from_millis(100)
            } else {
                Duration::from_secs(2)  // Headers sync 중: 2초 (큰 메시지 대기)
            };

            let addrs: Vec<SocketAddr> = self.peers.keys().copied().collect();
            for addr in addrs {
                let Some(p) = self.peers.get_mut(&addr) else { continue; };
                let maybe = match timeout(recv_timeout, p.recv()).await {
                    Ok(Ok(m)) => Some(m),
                    Ok(Err(e)) => {
                        // Enhanced error logging to understand disconnection reasons
                        let err_str = format!("{:#}", e);
                        if err_str.contains("early eof") || err_str.contains("EOF") {
                            eprintln!("[p2p] ⚠️  Peer {addr} disconnected (early eof) - may indicate peer rejected us or timed out");
                        } else {
                            eprintln!("[p2p] ⚠️  recv error from {addr}: {} - dropping peer", err_str);
                        }
                        self.peers.remove(&addr);
                        continue;
                    }
                    Err(_) => None,
                };

                if let Some(msg) = maybe {
                    // Debug: log all received messages with timestamp
                    let cmd = msg.command();
                    let cmd_str = cmd.as_ref();
                    if cmd_str != "ping" && cmd_str != "pong" {
                        eprintln!("[p2p] recv from {addr}: {}", cmd_str);
                    }

                    // Special logging for Headers messages since they're critical for IBD
                    if cmd_str == "headers" {
                        eprintln!("[p2p] ⭐ HEADERS MESSAGE RECEIVED from {addr} ⭐");
                    }

                    match msg {
                        message::NetworkMessage::Headers(h) => {
                            eprintln!("[p2p] *** RECEIVED HEADERS: {} from {addr} ***", h.len());

                            // Bitcoin Core: Empty headers response = caught up (no more headers)
                            if h.is_empty() {
                                eprintln!("[p2p] 📭 Empty headers response - we are caught up!");
                                last_headers_ts = tokio::time::Instant::now();
                                self.check_headers_sync_complete();

                                // If headers sync complete, start block download
                                if self.headers_synced {
                                    let assign = self.downloader.poll_assign(addr);
                                    if !assign.is_empty() {
                                        let invs: Vec<msg_blk::Inventory> =
                                            assign.iter().map(|h| msg_blk::Inventory::WitnessBlock(*h)).collect();
                                        if let Some(p) = self.peers.get_mut(&addr) {
                                            eprintln!("[p2p] Starting block download: requesting {} blocks", invs.len());
                                            let _ = p.send(message::NetworkMessage::GetData(invs)).await;
                                        }
                                    }
                                }
                            } else {
                                last_headers_ts = tokio::time::Instant::now();

                                // Bitcoin Core 방식: 헤더만 처리
                                let added = self.extend_headers(&h);

                                // 진행률 표시
                                let progress = if self.best_known_height > 0 {
                                    (self.header_chain_height as f64 / self.best_known_height as f64 * 100.0).min(100.0)
                                } else {
                                    0.0
                                };
                                eprintln!("[p2p] Headers sync progress: {:.1}% ({}/{})",
                                         progress, self.header_chain_height, self.best_known_height);

                                // CRITICAL FIX: Only request more headers if we ADDED headers and batch was full
                                // Bitcoin Core: Don't loop if we're not making progress!
                                if h.len() == MAX_HEADERS_PER_MSG && added > 0 {
                                    eprintln!("[p2p] ✓ Made progress ({} added), requesting next batch immediately...", added);
                                    let _ = self.request_headers(addr).await;
                                    // CRITICAL: Set timestamp far in future to prevent fallback re-request
                                    // We just sent immediate request, give peer 60s to respond before fallback
                                    last_headers_ts = tokio::time::Instant::now() + Duration::from_secs(IMMEDIATE_REQ_TIMEOUT - INITIAL_REREQ_SECS);
                                    eprintln!("[p2p] ⏸️  Waiting {}s for peer response (no fallback re-request)", IMMEDIATE_REQ_TIMEOUT);
                                } else if h.len() == MAX_HEADERS_PER_MSG && added == 0 {
                                    eprintln!("[p2p] ⚠️  Full batch received but NO headers added! Stopping to avoid infinite loop.");
                                    eprintln!("[p2p]     This indicates a chain mismatch or duplicate batch.");
                                    eprintln!("[p2p]     Will try different peer if available...");

                                    // Bitcoin Core behavior: If stuck, try a different sync peer
                                    self.sync_peer = None;  // Clear current sync peer

                                    // Try to find a different peer with higher height
                                    let other_peers: Vec<SocketAddr> = self.peers.keys()
                                        .filter(|&&a| a != addr)
                                        .copied()
                                        .collect();

                                    if !other_peers.is_empty() {
                                        let new_peer = other_peers[0];
                                        self.sync_peer = Some(new_peer);
                                        eprintln!("[p2p] Switching to different sync peer: {}", new_peer);
                                        let _ = self.request_headers(new_peer).await;
                                        // CRITICAL: Update timestamp to prevent timer-based duplicate request
                                        last_headers_ts = tokio::time::Instant::now();
                                    } else {
                                        eprintln!("[p2p] No other peers available. Will wait for new connections.");
                                    }
                                } else {
                                    eprintln!("[p2p] Header batch completed (received {} headers, added {})", h.len(), added);
                                    // 헤더 배치가 완료되었는지 확인
                                    self.check_headers_sync_complete();

                                    // 헤더 동기화가 완료되었다면 블록 다운로드 시작
                                    if self.headers_synced {
                                        let assign = self.downloader.poll_assign(addr);
                                        if !assign.is_empty() {
                                            let invs: Vec<msg_blk::Inventory> =
                                                assign.iter().map(|h| msg_blk::Inventory::WitnessBlock(*h)).collect();
                                            if let Some(p) = self.peers.get_mut(&addr) {
                                                eprintln!("[p2p] Starting block download: requesting {} blocks", invs.len());
                                                let _ = p.send(message::NetworkMessage::GetData(invs)).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        message::NetworkMessage::Inv(inv) => {
                            eprintln!("[p2p] inv: {} entries", inv.len());

                            // Bitcoin Core 방식: 헤더 동기화 완료 후에만 블록 다운로드
                            if !self.headers_synced {
                                eprintln!("[p2p] Ignoring Inv (still syncing headers)");
                                continue;
                            }

                            let need: Vec<BlockHash> = inv.iter()
                                .filter_map(|i| match i {
                                    msg_blk::Inventory::Block(h) | msg_blk::Inventory::WitnessBlock(h) => Some(*h),
                                    _ => None,
                                })
                                .filter(|h| self.have_header.contains(h))
                                .collect();

                            self.downloader.push_many(need);

                            let assign = self.downloader.poll_assign(addr);
                            if !assign.is_empty() {
                                let invs: Vec<msg_blk::Inventory> =
                                    assign.iter().map(|h| msg_blk::Inventory::WitnessBlock(*h)).collect();
                                if let Some(p) = self.peers.get_mut(&addr) {
                                    eprintln!("[p2p] send GetData for {} blocks", invs.len());
                                    let _ = p.send(message::NetworkMessage::GetData(invs)).await;
                                }
                            }
                        }
                        message::NetworkMessage::Block(b) => {
                            let h = b.block_hash();

                            // Bitcoin Core 방식: 헤더 동기화 완료 후에만 블록 처리
                            if !self.headers_synced {
                                eprintln!("[p2p] WARNING: Received block before headers sync complete, ignoring");
                                continue;
                            }

                            // 네트워크 루프는 즉시 다음으로 진행:
                            // 1) inflight에서 제거하고
                            self.downloader.complete(&h);

                            // Show progress (every block or every 100 blocks)
                            let (downloaded, total, percentage) = self.downloader.get_progress();
                            if downloaded % 100 == 0 || downloaded == total {
                                eprintln!("[p2p] 📦 Download progress: {:.1}% ({}/{} blocks)",
                                         percentage, downloaded, total);
                                eprintln!("[p2p]    Latest block hash: {}", h);
                            } else if downloaded <= 20 || downloaded % 10 == 0 {
                                // Show first 20 downloads, then every 10th
                                eprintln!("[p2p] 📦 Download progress: block #{}/{} ({:.1}%): {}",
                                         downloaded, total, percentage, h);
                            }

                            // 2) 다음 할당을 만들어 보냄
                            let assign = self.downloader.poll_assign(addr);
                            if !assign.is_empty() {
                                let invs: Vec<msg_blk::Inventory> =
                                    assign.iter().map(|h| msg_blk::Inventory::WitnessBlock(*h)).collect();
                                if let Some(p) = self.peers.get_mut(&addr) {
                                    eprintln!("[p2p] send GetData for {} blocks", invs.len());
                                    let _ = p.send(message::NetworkMessage::GetData(invs)).await;
                                }
                            }

                            // 3) Send block to sequential processor
                            // Bitcoin Core processes blocks sequentially to ensure parent blocks
                            // are processed before children. We use a channel to maintain order.
                            if let Some(ref tx) = self.block_tx {
                                let raw = encode::serialize(&b);
                                if let Err(e) = tx.send((h, raw)) {
                                    eprintln!("[p2p] ✗ Failed to send block {} to processor: {:#}", h, e);
                                }
                            }
                        }
                        message::NetworkMessage::Ping(nonce) => {
                            if let Some(p) = self.peers.get_mut(&addr) {
                                eprintln!("[p2p] ping {nonce}");
                                let _ = p.send(message::NetworkMessage::Pong(nonce)).await;
                            }
                        }
                        message::NetworkMessage::Pong(_) => { /* ignore */ }
                        message::NetworkMessage::NotFound(v) => {
                            eprintln!("[p2p] notfound: {} entries", v.len());
                        }
                        message::NetworkMessage::Addr(addrs) => {
                            let mut added = 0usize;
                            for (_time, a) in addrs {
                                if self.peers.len() >= MAX_OUTBOUND_FROM_ADDR { break; }
                                let words = a.address; // [u16; 8]
                                let ipv6 = Ipv6Addr::new(
                                    words[0], words[1], words[2], words[3],
                                    words[4], words[5], words[6], words[7],
                                );
                                let sock = if let Some(ipv4) = ipv6.to_ipv4_mapped() {
                                    SocketAddr::V4(SocketAddrV4::new(ipv4, a.port))
                                } else {
                                    SocketAddr::V6(SocketAddrV6::new(ipv6, a.port, 0, 0))
                                };

                                if !self.peers.contains_key(&sock) {
                                    let _ = self.add_outbound(sock).await;
                                    added += 1;
                                }
                                if added >= 2 { break; }
                            }
                        }
                        message::NetworkMessage::GetHeaders(gh) => {
                            let _ = self.respond_getheaders(addr, &gh).await;
                        }
                        message::NetworkMessage::Tx(tx) => {
                            let txid = tx.compute_txid();
                            eprintln!("[p2p] received tx: {}", txid);

                            // Process transaction via callback
                            if let Some(ref cb) = self.on_tx {
                                let tx_clone = tx.clone();
                                let cb = cb.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = (cb)(&tx_clone) {
                                        eprintln!("[p2p] tx processing error {}: {:#}", tx_clone.compute_txid(), e);
                                    }
                                });
                            }
                        }
                        other => {
                            eprintln!("[p2p] other: {:?}", other.command());
                        }
                    }
                }
            }

            // Initial and periodic header requests - Bitcoin Core: sync peer only
            // Send initial request after 1 second, then re-request every 2 seconds if no response
            // BUT: After immediate request (on full batch), wait 60 seconds before fallback
            if !self.headers_synced {
                if let Some(sync_addr) = self.sync_peer {
                    if self.peers.contains_key(&sync_addr) {
                        let elapsed = tokio::time::Instant::now().duration_since(last_headers_ts);
                        // Initial request after 1s, fallback requests every 2s
                        let should_request = if self.header_chain_height == 0 {
                            elapsed > Duration::from_secs(1)  // Initial: 1 second delay
                        } else {
                            elapsed > Duration::from_secs(INITIAL_REREQ_SECS)  // Fallback: 2 seconds
                        };

                        if should_request {
                            if self.header_chain_height == 0 {
                                eprintln!("[p2p] Initial headers request to sync peer {}", sync_addr);
                            } else {
                                eprintln!("[p2p] ⏱️  Fallback re-request ({}s timeout) to sync peer {}", INITIAL_REREQ_SECS, sync_addr);
                                eprintln!("[p2p]     Continuing headers sync from height {}", self.header_chain_height);
                            }
                            let _ = self.request_headers(sync_addr).await;
                            last_headers_ts = tokio::time::Instant::now();
                        }
                    }
                } else if self.peers.len() < 3 {
                    // No sync peer found yet - try connecting to more peers
                    // Only do this if we have few peers (to avoid spam)
                    let elapsed = tokio::time::Instant::now().duration_since(last_headers_ts);
                    if elapsed > Duration::from_secs(10) {
                        eprintln!("[p2p] No suitable sync peer found yet, connecting to more peers...");
                        let _ = self.bootstrap().await;
                        last_headers_ts = tokio::time::Instant::now();
                    }
                }
            }

            // 오래 정체되면 sync peer 교체
            if !self.headers_synced && tokio::time::Instant::now().duration_since(last_headers_ts) > STALL_LIMIT {
                if let Some(sync_addr) = self.sync_peer {
                    eprintln!("[p2p] headers stall; replacing sync peer {}", sync_addr);
                    self.peers.remove(&sync_addr);
                    self.sync_peer = None;
                }
                let _ = self.bootstrap().await;
            }

            sleep(Duration::from_millis(5)).await;
        }
    }
}

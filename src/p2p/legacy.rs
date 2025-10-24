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
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{lookup_host, TcpStream};
use tokio::time::{sleep, timeout};
use tokio::task::spawn_blocking;

use crate::seeds;

/// 광고할 프로토콜 번호(현대 피어 경로를 열기 위해 70016 사용)
const ADVERTISED_PROTO: u32 = 70016;

// 타임아웃/윈도
const HDRS_TIMEOUT: Duration = Duration::from_secs(60);
const BLK_TIMEOUT: Duration = Duration::from_secs(120);
const STALL_LIMIT: Duration = Duration::from_secs(15 * 60);
const REREQ_SECS: u64 = 5;
const MAX_HEADERS_PER_MSG: usize = 2000;

// 전역/피어별 인플라이트 제한
const GLOBAL_INFLIGHT: usize = 256;
const PER_PEER_INFLIGHT: usize = 16;

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
                    // 먼저 Verack을 회신
                    self.send(message::NetworkMessage::Verack).await?;
                    eprintln!("[p2p] sent Verack");
                    got_version = true;
                }
                message::NetworkMessage::Verack => {
                    eprintln!("[p2p] recv Verack");
                    self.verack_seen = true;
                    // Verack 이후에 기능 협상
                    if !self.sendheaders_sent {
                        self.send(message::NetworkMessage::SendHeaders).await?;
                        self.sendheaders_sent = true;
                        eprintln!("[p2p] sent SendHeaders");
                    }
                    if !self.wtxidrelay_sent {
                        self.send(message::NetworkMessage::SendCmpct(msg_cmpct::SendCmpct {
                            version: 1,
                            send_compact: true,
                        })).await?;
                        eprintln!("[p2p] sent SendCmpct(high)");

                        self.send(message::NetworkMessage::WtxidRelay).await?;
                        self.wtxidrelay_sent = true;
                        eprintln!("[p2p] sent WtxidRelay");
                    }
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
}
impl Downloader {
    pub fn new(global_window: usize, per_peer_window: usize) -> Self {
        Self {
            inflight: HashMap::new(),
            per_peer: HashMap::new(),
            global_window,
            per_peer_window,
            queue: VecDeque::new(),
        }
    }
    pub fn push_many(&mut self, v: impl IntoIterator<Item = BlockHash>) {
        for h in v { self.queue.push_back(h); }
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
    sync_peer: Option<SocketAddr>,              // Bitcoin Core: ONE headers sync peer

    on_block: Option<Arc<dyn Fn(&[u8]) -> anyhow::Result<()> + Send + Sync>>,
    on_tx: Option<Arc<dyn Fn(&bitcoin::Transaction) -> anyhow::Result<()> + Send + Sync>>,
}

impl PeerManager {
    pub fn new(net: Network, user_agent: &str) -> Self {
        Self::with_start_height(net, user_agent, 0)
    }

    pub fn with_start_height(net: Network, user_agent: &str, start_height: i32) -> Self {
        let g = genesis_block(net).block_hash();
        let mut have = HashSet::new();
        have.insert(g);

        eprintln!("[p2p] Initializing PeerManager with start_height={}", start_height);
        eprintln!("[p2p] Starting in HEADERS-FIRST SYNC mode");

        Self {
            net,
            user_agent: user_agent.into(),
            peers: HashMap::new(),
            downloader: Downloader::new(GLOBAL_INFLIGHT, PER_PEER_INFLIGHT),
            prev_map: HashMap::new(),
            have_header: have,
            best_header_tip: g,
            recent_chain: vec![g],
            last_locator: vec![g],
            start_height,
            headers_synced: false,
            peer_heights: HashMap::new(),
            best_known_height: 0,
            header_chain_height: 0,
            sync_peer: None,
            on_block: None,
            on_tx: None,
        }
    }

    pub fn with_block_processor<F>(mut self, f: F) -> Self
    where
        F: Fn(&[u8]) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        self.on_block = Some(Arc::new(f));
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

    /// Core 유사 지수 백트래킹 로케이터(간소화)
    fn build_locator(&self) -> Vec<BlockHash> {
        let mut step = 1usize;
        let mut count = 0usize;
        let mut loc = Vec::with_capacity(32);
        let mut idx = self.recent_chain.len().saturating_sub(1);

        while count < 32 {
            loc.push(self.recent_chain[idx]);
            if idx == 0 { break; }

            let next_idx = idx.saturating_sub(step);
            idx = next_idx;
            count += 1;
            if loc.len() > 10 { step = step.saturating_mul(2).min(self.recent_chain.len()); }
        }
        if *loc.last().unwrap_or(&self.recent_chain[0]) != self.recent_chain[0] {
            loc.push(self.recent_chain[0]);
        }
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
    fn extend_headers(&mut self, new_headers: &[BlockHeader]) {
        for hh in new_headers {
            let h = hh.block_hash();
            self.prev_map.insert(h, hh.prev_blockhash);
            self.have_header.insert(h);
            if *self.recent_chain.last().unwrap_or(&h) == hh.prev_blockhash {
                self.recent_chain.push(h);
                self.header_chain_height += 1;  // 헤더 체인 높이 증가
            }
        }

        // 가장 긴 체인 찾기
        loop {
            let tip = self.best_header_tip;
            let next = self.prev_map.iter()
                .find_map(|(child, prev)| if *prev == tip { Some(*child) } else { None });
            match next {
                Some(nh) => {
                    self.best_header_tip = nh;
                }
                None => break,
            }
        }
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

    /// 헤더 동기화 완료 후 모든 블록을 다운로드 큐에 추가
    fn queue_blocks_from_headers(&mut self) {
        let mut blocks_to_download = Vec::new();

        // recent_chain의 모든 블록을 큐에 추가 (genesis 제외)
        for hash in self.recent_chain.iter().skip(1) {
            blocks_to_download.push(*hash);
        }

        eprintln!("[p2p] Queuing {} blocks for download", blocks_to_download.len());
        self.downloader.push_many(blocks_to_download);
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
                            if !h.is_empty() {
                                last_headers_ts = tokio::time::Instant::now();

                                // Bitcoin Core 방식: 헤더만 처리
                                self.extend_headers(&h);

                                // 진행률 표시
                                let progress = if self.best_known_height > 0 {
                                    (self.header_chain_height as f64 / self.best_known_height as f64 * 100.0).min(100.0)
                                } else {
                                    0.0
                                };
                                eprintln!("[p2p] Headers sync progress: {:.1}% ({}/{})",
                                         progress, self.header_chain_height, self.best_known_height);

                                // 더 많은 헤더가 있으면 계속 요청
                                if h.len() == MAX_HEADERS_PER_MSG {
                                    eprintln!("[p2p] More headers available, requesting next batch...");
                                    let _ = self.request_headers(addr).await;
                                } else {
                                    eprintln!("[p2p] Header batch completed (received {} headers)", h.len());
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
                            eprintln!("[p2p] block: {h}");

                            // Bitcoin Core 방식: 헤더 동기화 완료 후에만 블록 처리
                            if !self.headers_synced {
                                eprintln!("[p2p] WARNING: Received block before headers sync complete, ignoring");
                                continue;
                            }

                            // 네트워크 루프는 즉시 다음으로 진행:
                            // 1) inflight에서 제거하고
                            self.downloader.complete(&h);

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

                            // 3) 실제 블록 처리(디스크/검증)는 백그라운드로
                            if let Some(cb) = &self.on_block {
                                let raw = encode::serialize(&b);
                                let cb = cb.clone();
                                tokio::spawn(async move {
                                    let _ = spawn_blocking(move || (cb)(&raw)).await;
                                });
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
            // Send initial request after 1 second, then re-request every 5 seconds if no response
            if !self.headers_synced {
                if let Some(sync_addr) = self.sync_peer {
                    if self.peers.contains_key(&sync_addr) {
                        let elapsed = tokio::time::Instant::now().duration_since(last_headers_ts);
                        // Initial request after 1s, subsequent requests every 5s
                        let should_request = if self.header_chain_height == 0 {
                            elapsed > Duration::from_secs(1)  // Initial: 1 second delay
                        } else {
                            elapsed > Duration::from_secs(REREQ_SECS)  // Re-requests: 5 seconds
                        };

                        if should_request {
                            if self.header_chain_height == 0 {
                                eprintln!("[p2p] Initial headers request to sync peer {}", sync_addr);
                            } else {
                                eprintln!("[p2p] Re-request headers to sync peer {}", sync_addr);
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

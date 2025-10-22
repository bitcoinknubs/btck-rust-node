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

    pub async fn handshake(&mut self, user_agent: &str, start_height: i32) -> Result<()> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
        let mut vm = msg_net::VersionMessage::new(
            p2p::ServiceFlags::NETWORK | p2p::ServiceFlags::WITNESS,
            now,
            address::Address::new(&"0.0.0.0:0".parse().unwrap(), p2p::ServiceFlags::NONE),
            address::Address::new(&"0.0.0.0:0".parse().unwrap(), p2p::ServiceFlags::NONE),
            rand::thread_rng().gen::<u64>(),
            user_agent.into(),
            start_height,
        );
        vm.version = ADVERTISED_PROTO;

        self.send(message::NetworkMessage::Version(vm)).await?;
        eprintln!("[p2p] sent Version (ua={user_agent}, proto={})", ADVERTISED_PROTO);

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
    on_block: Option<Arc<dyn Fn(&[u8]) -> anyhow::Result<()> + Send + Sync>>,
}

impl PeerManager {
    pub fn new(net: Network, user_agent: &str) -> Self {
        let g = genesis_block(net).block_hash();
        let mut have = HashSet::new();
        have.insert(g);

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
            on_block: None,
        }
    }

    pub fn with_block_processor<F>(mut self, f: F) -> Self
    where
        F: Fn(&[u8]) -> anyhow::Result<()> + Send + Sync + 'static,
    {
        self.on_block = Some(Arc::new(f));
        self
    }
    pub fn peers_len(&self) -> usize { self.peers.len() }

    pub async fn add_outbound(&mut self, addr: SocketAddr, start_height: i32) -> Result<()> {
        if self.peers.contains_key(&addr) { return Ok(()); }
        let mut p = Peer::connect(addr, self.net).await?;
        p.handshake(&self.user_agent, start_height).await?;
        self.peers.insert(addr, p);
        // 붙자마자 헤더 요청
        self.request_headers(addr).await?;
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
                        match self.add_outbound(addr, 0).await {
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
        let gh = msg_blk::GetHeadersMessage {
            version: ADVERTISED_PROTO,
            locator_hashes: self.last_locator.clone(),
            stop_hash: BlockHash::from_raw_hash(sha256d::Hash::all_zeros()),
        };
        if let Some(p) = self.peers.get_mut(&to) {
            eprintln!("[p2p] send GetHeaders ({} locators)", gh.locator_hashes.len());
            p.send(message::NetworkMessage::GetHeaders(gh)).await?;
        }
        Ok(())
    }

    /// 새 헤더 확장 & 블록 큐잉
    fn extend_connected_headers_and_queue(&mut self, new_headers: &[BlockHeader]) {
        for hh in new_headers {
            let h = hh.block_hash();
            self.prev_map.insert(h, hh.prev_blockhash);
            self.have_header.insert(h);
            if *self.recent_chain.last().unwrap_or(&h) == hh.prev_blockhash {
                self.recent_chain.push(h);
            }
        }
        loop {
            let tip = self.best_header_tip;
            let next = self.prev_map.iter()
                .find_map(|(child, prev)| if *prev == tip { Some(*child) } else { None });
            match next {
                Some(nh) => {
                    self.downloader.push_many([nh]);
                    self.best_header_tip = nh;
                }
                None => break,
            }
        }
    }

    async fn respond_getheaders(&mut self, from: SocketAddr, _req: &msg_blk::GetHeadersMessage) -> Result<()> {
        let v: Vec<BlockHeader> = Vec::new();
        if let Some(p) = self.peers.get_mut(&from) {
            p.send(message::NetworkMessage::Headers(v)).await?;
            eprintln!("[p2p] sent Headers(reply) size=0");
        }
        Ok(())
    }

    pub async fn event_loop(&mut self) -> Result<()> {
        let mut last_headers_ts = tokio::time::Instant::now();

        loop {
            // 피어 없으면 재부트스트랩
            if self.peers.is_empty() {
                let _ = self.bootstrap().await?;
                if self.peers.is_empty() {
                    sleep(Duration::from_millis(200)).await;
                    continue;
                }
                if let Some(&addr) = self.peers.keys().next() {
                    let _ = self.request_headers(addr).await;
                }
            }

            // 타임아웃된 블록 재할당
            for h in self.downloader.reassign_timeouts() { self.downloader.push_many([h]); }

            // 모든 피어를 라운드로빈 폴링 (각 100ms)
            let addrs: Vec<SocketAddr> = self.peers.keys().copied().collect();
            for addr in addrs {
                let Some(p) = self.peers.get_mut(&addr) else { continue; };
                let maybe = match timeout(Duration::from_millis(100), p.recv()).await {
                    Ok(Ok(m)) => Some(m),
                    Ok(Err(e)) => {
                        eprintln!("[p2p] recv error from {addr}: {e:#}; dropping peer");
                        self.peers.remove(&addr);
                        continue;
                    }
                    Err(_) => None,
                };

                if let Some(msg) = maybe {
                    match msg {
                        message::NetworkMessage::Headers(h) => {
                            eprintln!("[p2p] headers: {}", h.len());
                            if !h.is_empty() {
                                last_headers_ts = tokio::time::Instant::now();

                                self.extend_connected_headers_and_queue(&h);

                                if h.len() == MAX_HEADERS_PER_MSG {
                                    let _ = self.request_headers(addr).await;
                                } else {
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
                            }
                        }
                        message::NetworkMessage::Inv(inv) => {
                            eprintln!("[p2p] inv: {} entries", inv.len());
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
                                    let _ = self.add_outbound(sock, 0).await;
                                    added += 1;
                                }
                                if added >= 2 { break; }
                            }
                        }
                        message::NetworkMessage::GetHeaders(gh) => {
                            let _ = self.respond_getheaders(addr, &gh).await;
                        }
                        other => {
                            eprintln!("[p2p] other: {:?}", other.command());
                        }
                    }
                }
            }

            // 주기적 헤더 재요청(5초)
            if tokio::time::Instant::now().duration_since(last_headers_ts) > Duration::from_secs(REREQ_SECS) {
                if let Some(&addr) = self.peers.keys().next() {
                    eprintln!("[p2p] re-request headers to {addr}");
                    let _ = self.request_headers(addr).await;
                    last_headers_ts = tokio::time::Instant::now();
                }
            }

            // 오래 정체되면 피어 교체
            if tokio::time::Instant::now().duration_since(last_headers_ts) > STALL_LIMIT {
                if let Some(&addr) = self.peers.keys().next() {
                    eprintln!("[p2p] headers stall; replacing peer {}", addr);
                    self.peers.remove(&addr);
                }
                let _ = self.bootstrap().await;
            }

            sleep(Duration::from_millis(5)).await;
        }
    }
}

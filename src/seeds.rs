use bitcoin::Network;

/// 간단 DNS 시드 목록 (필요 시 최신 목록으로 갱신하세요)
pub fn dns_seeds(net: Network) -> &'static [&'static str] {
    match net {
        // 메인넷
        Network::Bitcoin => &[
            "seed.bitcoin.sipa.be",
            "dnsseed.bluematt.me",
            "dnsseed.bitcoin.dashjr.org",
            "seed.bitcoinstats.com",
            "seed.bitcoin.jonasschnelli.ch",
            "seed.btc.petertodd.org",
            "seed.bitcoin.sprovoost.nl",
            "dnsseed.emzy.de",
            "seed.bitcoin.wiz.biz",
        ],
        // 테스트넷 (testnet3)
        Network::Testnet => &[
            "testnet-seed.bitcoin.jonasschnelli.ch",
            "seed.tbtc.petertodd.org",
            "seed.testnet.bitcoin.sprovoost.nl",
        ],
        Network::Testnet4 => &[
            // 필요시 실제 testnet4 시더로 교체하세요.
            "seed.testnet4.bitcoin.jonasschnelli.ch",
        ],
        // 시그넷
        Network::Signet => &[
            "seed.signet.bitcoin.sprovoost.nl",
            "dnsseed.signet.bitcoin.jonasschnelli.ch",
            // 시그넷은 DNS가 약한 경우가 있어 일부 고정 노드도 넣어둡니다(옵션).
            "18.142.242.1:38333",
            "34.171.112.142:38333",
            "35.217.13.118:38333",
            "38.247.82.124:38333",
            "45.94.168.5:38333",
            "51.210.144.135:38333",
            "54.151.174.170:38333",
            "66.254.43.122:38333",
            "72.48.253.168:38333",
            "81.17.97.236:38333",
            "91.134.73.14:38333",
            "95.141.35.117:38333",
            "129.226.149.150:38333",
            "131.153.11.131:38333",
        ],
        // 레그테스트는 DNS 시드 없음
        Network::Regtest => &[],
    }
}

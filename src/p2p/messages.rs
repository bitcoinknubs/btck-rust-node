use bitcoin::{Block, BlockHash, Transaction, Txid};
use bitcoin::p2p::message_blockdata::Inventory;

/// Inventory type for P2P messages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InventoryType {
    Error,
    Tx,
    Block,
    FilteredBlock,
    CompactBlock,
    WitnessTx,
    WitnessBlock,
}

impl InventoryType {
    pub fn to_u32(&self) -> u32 {
        match self {
            InventoryType::Error => 0,
            InventoryType::Tx => 1,
            InventoryType::Block => 2,
            InventoryType::FilteredBlock => 3,
            InventoryType::CompactBlock => 4,
            InventoryType::WitnessTx => 0x40000001,
            InventoryType::WitnessBlock => 0x40000002,
        }
    }

    pub fn from_inventory(inv: &Inventory) -> Self {
        match inv {
            Inventory::Error => InventoryType::Error,
            Inventory::Transaction(_) => InventoryType::Tx,
            Inventory::Block(_) => InventoryType::Block,
            Inventory::CompactBlock(_) => InventoryType::CompactBlock,
            Inventory::WitnessTransaction(_) => InventoryType::WitnessTx,
            Inventory::WitnessBlock(_) => InventoryType::WitnessBlock,
            _ => InventoryType::Error,
        }
    }
}

/// P2P message types
#[derive(Debug, Clone)]
pub enum P2PMessage {
    /// Version handshake
    Version {
        version: i32,
        services: u64,
        timestamp: i64,
        user_agent: String,
        start_height: i32,
    },

    /// Version acknowledgment
    Verack,

    /// Ping message
    Ping(u64),

    /// Pong response
    Pong(u64),

    /// Inventory announcement
    Inv(Vec<Inventory>),

    /// Request data
    GetData(Vec<Inventory>),

    /// Not found
    NotFound(Vec<Inventory>),

    /// Transaction
    Tx(Transaction),

    /// Block
    Block(Block),

    /// Get headers
    GetHeaders {
        version: u32,
        locator_hashes: Vec<BlockHash>,
        stop_hash: BlockHash,
    },

    /// Headers response
    Headers(Vec<bitcoin::block::Header>),

    /// Get blocks
    GetBlocks {
        version: u32,
        locator_hashes: Vec<BlockHash>,
        stop_hash: BlockHash,
    },

    /// Get addresses
    GetAddr,

    /// Addresses
    Addr(Vec<(u32, bitcoin::network::Address)>),

    /// Send headers preference
    SendHeaders,

    /// Fee filter
    FeeFilter(u64),

    /// Send compact blocks
    SendCmpct {
        announce: bool,
        version: u64,
    },

    /// Mempool request
    MemPool,

    /// Reject message
    Reject {
        message: String,
        ccode: u8,
        reason: String,
        data: Vec<u8>,
    },
}

impl P2PMessage {
    pub fn command_name(&self) -> &'static str {
        match self {
            P2PMessage::Version { .. } => "version",
            P2PMessage::Verack => "verack",
            P2PMessage::Ping(_) => "ping",
            P2PMessage::Pong(_) => "pong",
            P2PMessage::Inv(_) => "inv",
            P2PMessage::GetData(_) => "getdata",
            P2PMessage::NotFound(_) => "notfound",
            P2PMessage::Tx(_) => "tx",
            P2PMessage::Block(_) => "block",
            P2PMessage::GetHeaders { .. } => "getheaders",
            P2PMessage::Headers(_) => "headers",
            P2PMessage::GetBlocks { .. } => "getblocks",
            P2PMessage::GetAddr => "getaddr",
            P2PMessage::Addr(_) => "addr",
            P2PMessage::SendHeaders => "sendheaders",
            P2PMessage::FeeFilter(_) => "feefilter",
            P2PMessage::SendCmpct { .. } => "sendcmpct",
            P2PMessage::MemPool => "mempool",
            P2PMessage::Reject { .. } => "reject",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inventory_type() {
        assert_eq!(InventoryType::Tx.to_u32(), 1);
        assert_eq!(InventoryType::Block.to_u32(), 2);
    }

    #[test]
    fn test_message_command_names() {
        assert_eq!(P2PMessage::Verack.command_name(), "verack");
        assert_eq!(P2PMessage::GetAddr.command_name(), "getaddr");
    }
}

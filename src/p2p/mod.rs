pub mod messages;
pub mod peer;
pub mod manager;
pub mod inventory;
pub mod legacy;

pub use messages::{P2PMessage, InventoryType};
pub use peer::{Peer, PeerState};
pub use manager::P2PManager;
pub use inventory::InventoryManager;

// Re-export legacy for compatibility
pub use legacy::PeerManager;

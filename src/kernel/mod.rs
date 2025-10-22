// src/kernel/mod.rs
use anyhow::{Context, Result};
use bitcoin::{Block, BlockHash, Transaction};
use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::PathBuf;

use crate::ffi;

/// Kernel wrapper providing safe Rust interface to libbitcoinkernel
pub struct Kernel {
    ctx: *mut ffi::btck_Context,
    chain_params: *mut ffi::btck_ChainParameters,
    chainman: *mut ffi::btck_ChainstateManager,
}

unsafe impl Send for Kernel {}
unsafe impl Sync for Kernel {}

/// Kernel logging callback
unsafe extern "C" fn log_callback(_userdata: *mut c_void, msg: *const c_char, _len: usize) {
    if !msg.is_null() {
        if let Ok(s) = CStr::from_ptr(msg).to_str() {
            eprintln!("[kernel] {}", s);
        }
    }
}

impl Kernel {
    /// Create new kernel instance
    pub fn new(
        chain_type: ChainType,
        datadir: &PathBuf,
        blocksdir: &PathBuf,
        in_memory: bool,
    ) -> Result<Self> {
        let chain_type_u8 = chain_type.to_u8();

        // Create context options
        let ctx_opts = unsafe { ffi::btck_context_options_create() };
        if ctx_opts.is_null() {
            anyhow::bail!("Failed to create context options");
        }

        // Create chain parameters
        let chain_params = unsafe { ffi::btck_chain_parameters_create(chain_type_u8) };
        if chain_params.is_null() {
            unsafe { ffi::btck_context_options_destroy(ctx_opts) };
            anyhow::bail!("Failed to create chain parameters");
        }
        unsafe { ffi::btck_context_options_set_chainparams(ctx_opts, chain_params) };

        // Setup logging
        let log_opts: ffi::btck_LoggingOptions = unsafe { std::mem::zeroed() };
        let _log_conn = unsafe {
            ffi::btck_logging_connection_create(
                Some(log_callback),
                std::ptr::null_mut(),
                None,
                log_opts,
            )
        };

        // Create context
        let ctx = unsafe { ffi::btck_context_create(ctx_opts) };
        if ctx.is_null() {
            unsafe {
                ffi::btck_context_options_destroy(ctx_opts);
                ffi::btck_chain_parameters_destroy(chain_params);
            }
            anyhow::bail!("Failed to create context");
        }
        unsafe { ffi::btck_context_options_destroy(ctx_opts) };

        // Create chainstate manager options
        let data_c = CString::new(datadir.to_string_lossy().as_bytes())?;
        let blocks_c = CString::new(blocksdir.to_string_lossy().as_bytes())?;
        
        let chainman_opts = unsafe {
            ffi::btck_chainstate_manager_options_create(
                ctx,
                data_c.as_ptr(),
                data_c.as_bytes().len(),
                blocks_c.as_ptr(),
                blocks_c.as_bytes().len(),
            )
        };
        
        if chainman_opts.is_null() {
            unsafe {
                ffi::btck_context_destroy(ctx);
                ffi::btck_chain_parameters_destroy(chain_params);
            }
            anyhow::bail!("Failed to create chainstate manager options");
        }

        // Configure storage
        if in_memory {
            unsafe {
                ffi::btck_chainstate_manager_options_update_block_tree_db_in_memory(chainman_opts, 1);
                ffi::btck_chainstate_manager_options_update_chainstate_db_in_memory(chainman_opts, 1);
            }
        }

        unsafe {
            ffi::btck_chainstate_manager_options_set_worker_threads_num(chainman_opts, 4);
        }

        // Create chainstate manager
        let chainman = unsafe { ffi::btck_chainstate_manager_create(chainman_opts) };
        if chainman.is_null() {
            unsafe {
                ffi::btck_context_destroy(ctx);
                ffi::btck_chain_parameters_destroy(chain_params);
                ffi::btck_chainstate_manager_options_destroy(chainman_opts);
            }
            anyhow::bail!("Failed to create chainstate manager");
        }
        unsafe { ffi::btck_chainstate_manager_options_destroy(chainman_opts) };

        eprintln!("[kernel] initialized for {:?}", chain_type);

        Ok(Self { ctx, chain_params, chainman })
    }

    pub fn get_height(&self) -> Result<i32> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                return Ok(-1);
            }
            Ok(ffi::btck_chain_get_height(chain))
        }
    }

    pub fn get_best_block_hash(&self) -> Result<BlockHash> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                anyhow::bail!("No active chain");
            }
            let tip = ffi::btck_chain_get_tip(chain);
            if tip.is_null() {
                anyhow::bail!("No chain tip");
            }
            let mut hash = [0u8; 32];
            ffi::btck_block_index_get_block_hash(tip, hash.as_mut_ptr());
            Ok(BlockHash::from_byte_array(hash))
        }
    }

    pub fn process_block(&self, block: &Block) -> Result<bool> {
        let raw = bitcoin::consensus::serialize(block);
        unsafe {
            let block_ptr = ffi::btck_block_create(
                raw.as_ptr() as *const c_void,
                raw.len(),
            );
            if block_ptr.is_null() {
                anyhow::bail!("Failed to create block");
            }
            let mut new_block: c_int = 0;
            let rc = ffi::btck_chainstate_manager_process_block(
                self.chainman,
                block_ptr,
                &mut new_block as *mut c_int,
            );
            ffi::btck_block_destroy(block_ptr);
            if rc != 0 {
                anyhow::bail!("Block validation failed");
            }
            Ok(new_block != 0)
        }
    }

    pub fn import_blocks(&self, paths: &[String]) -> Result<i32> {
        let c_paths: Vec<CString> = paths
            .iter()
            .map(|p| CString::new(p.as_str()))
            .collect::<std::result::Result<_, _>>()?;
        let mut ptrs: Vec<*const i8> = c_paths.iter().map(|c| c.as_ptr()).collect();
        let mut lens: Vec<usize> = c_paths.iter().map(|c| c.as_bytes().len()).collect();
        let rc = unsafe {
            ffi::btck_chainstate_manager_import_blocks(
                self.chainman,
                ptrs.as_mut_ptr(),
                lens.as_mut_ptr(),
                ptrs.len(),
            )
        };
        Ok(rc)
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        unsafe {
            ffi::btck_chainstate_manager_destroy(self.chainman);
            ffi::btck_context_destroy(self.ctx);
            ffi::btck_chain_parameters_destroy(self.chain_params);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainType {
    Mainnet,
    Testnet,
    Testnet4,
    Signet,
    Regtest,
}

impl ChainType {
    pub fn to_u8(&self) -> u8 {
        match self {
            ChainType::Mainnet => 0,
            ChainType::Testnet => 1,
            ChainType::Testnet4 => 2,
            ChainType::Signet => 3,
            ChainType::Regtest => 4,
        }
    }

    pub fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "main" | "mainnet" => Ok(ChainType::Mainnet),
            "test" | "testnet" => Ok(ChainType::Testnet),
            "testnet4" => Ok(ChainType::Testnet4),
            "signet" => Ok(ChainType::Signet),
            "regtest" => Ok(ChainType::Regtest),
            _ => anyhow::bail!("Unknown chain type: {}", s),
        }
    }

    pub fn default_port(&self) -> u16 {
        match self {
            ChainType::Mainnet => 8333,
            ChainType::Testnet => 18333,
            ChainType::Testnet4 => 48333,
            ChainType::Signet => 38333,
            ChainType::Regtest => 18444,
        }
    }
}

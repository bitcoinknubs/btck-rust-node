use crate::ffi;
use anyhow::Result;
use bitcoin::hashes::Hash;
use bitcoin::BlockHash;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;

// Chain type constants (matching bitcoinkernel.h)
const CHAIN_MAIN: u8 = 0;
const CHAIN_TESTNET: u8 = 1;
const CHAIN_TESTNET4: u8 = 2;
const CHAIN_SIGNET: u8 = 3;
const CHAIN_REGTEST: u8 = 4;

type CChainstateManager = ffi::btck_ChainstateManager;
type CChain = ffi::btck_Chain;
type CContext = ffi::btck_Context;
type CChainParameters = ffi::btck_ChainParameters;
type CContextOptions = ffi::btck_ContextOptions;
type CChainstateManagerOptions = ffi::btck_ChainstateManagerOptions;

/// Kernel log callback: output kernel logs to stderr
unsafe extern "C" fn log_cb(_ud: *mut c_void, msg: *const c_char, _len: usize) {
    if !msg.is_null() {
        if let Ok(s) = CStr::from_ptr(msg).to_str() {
            eprintln!("[kernel] {s}");
        }
    }
}

/// Kernel wrapper for libbitcoinkernel
pub struct Kernel {
    ctx: *mut CContext,
    chain_params: *mut CChainParameters,
    pub chainman: *mut CChainstateManager,
}

unsafe impl Send for Kernel {}
unsafe impl Sync for Kernel {}

impl Kernel {
    pub fn new(chain: &str, datadir: &PathBuf, blocksdir: &PathBuf) -> Result<Self> {
        eprintln!("[kernel] Initializing kernel for chain: {}", chain);
        eprintln!("[kernel] Data directory: {:?}", datadir);
        eprintln!("[kernel] Blocks directory: {:?}", blocksdir);

        let chain_type: u8 = match chain {
            "main" | "mainnet" => CHAIN_MAIN,
            "testnet" => CHAIN_TESTNET,
            "testnet4" => CHAIN_TESTNET4,
            "signet" => CHAIN_SIGNET,
            _ => CHAIN_REGTEST,
        };

        eprintln!("[kernel] Creating context options...");
        let ctx_opts = unsafe { ffi::btck_context_options_create() };
        if ctx_opts.is_null() {
            anyhow::bail!("btck_context_options_create failed - libbitcoinkernel may not be loaded");
        }
        eprintln!("[kernel] Context options created successfully");

        eprintln!("[kernel] Creating chain parameters...");
        let chain_params = unsafe { ffi::btck_chain_parameters_create(chain_type) };
        if chain_params.is_null() {
            unsafe { ffi::btck_context_options_destroy(ctx_opts) };
            anyhow::bail!("btck_chain_parameters_create failed");
        }
        eprintln!("[kernel] Chain parameters created successfully");

        unsafe { ffi::btck_context_options_set_chainparams(ctx_opts, chain_params) };

        // Skip logging connection for now to avoid potential issues
        eprintln!("[kernel] Skipping logging connection setup");

        eprintln!("[kernel] Creating context...");
        let ctx = unsafe { ffi::btck_context_create(ctx_opts) };
        if ctx.is_null() {
            unsafe {
                ffi::btck_context_options_destroy(ctx_opts);
                ffi::btck_chain_parameters_destroy(chain_params);
            }
            anyhow::bail!("btck_context_create failed");
        }
        unsafe { ffi::btck_context_options_destroy(ctx_opts) };
        eprintln!("[kernel] Context created successfully");

        eprintln!("[kernel] Creating chainstate manager options...");
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
            anyhow::bail!("btck_chainstate_manager_options_create failed");
        }
        eprintln!("[kernel] Chainstate manager options created successfully");

        eprintln!("[kernel] Setting chainstate options...");
        unsafe {
            // Store block index and chainstate on disk (not in memory)
            // This matches Bitcoin Core behavior and allows resuming after restart
            ffi::btck_chainstate_manager_options_update_block_tree_db_in_memory(chainman_opts, 0);
            ffi::btck_chainstate_manager_options_update_chainstate_db_in_memory(chainman_opts, 0);
            ffi::btck_chainstate_manager_options_set_worker_threads_num(chainman_opts, 2);
        }

        eprintln!("[kernel] Creating chainstate manager...");
        let chainman = unsafe { ffi::btck_chainstate_manager_create(chainman_opts) };
        if chainman.is_null() {
            unsafe {
                ffi::btck_context_destroy(ctx);
                ffi::btck_chain_parameters_destroy(chain_params);
                ffi::btck_chainstate_manager_options_destroy(chainman_opts);
            }
            anyhow::bail!("btck_chainstate_manager_create failed");
        }
        unsafe { ffi::btck_chainstate_manager_options_destroy(chainman_opts) };
        eprintln!("[kernel] Chainstate manager created successfully");

        let kernel = Self { ctx, chain_params, chainman };

        // Initialize genesis block if it doesn't exist
        // Bitcoin Core does this in LoadBlockIndex()
        eprintln!("[kernel] Checking for genesis block...");

        // Check if genesis block actually exists (more reliable than height check)
        let needs_genesis = unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(kernel.chainman);
            if chain.is_null() {
                true  // No chain at all
            } else {
                let genesis = ffi::btck_chain_get_genesis(chain);
                genesis.is_null()  // Genesis doesn't exist
            }
        };

        if needs_genesis {
            eprintln!("[kernel] No genesis block found. Initializing...");

            // Get genesis block for the network
            use bitcoin::blockdata::constants::genesis_block;
            use bitcoin::consensus::Encodable;

            let net = match chain_type {
                CHAIN_MAIN => bitcoin::Network::Bitcoin,
                CHAIN_TESTNET | CHAIN_TESTNET4 => bitcoin::Network::Testnet,
                CHAIN_SIGNET => bitcoin::Network::Signet,
                _ => bitcoin::Network::Regtest,
            };

            let genesis = genesis_block(net);
            let mut genesis_bytes = Vec::new();
            genesis.consensus_encode(&mut genesis_bytes)
                .map_err(|e| anyhow::anyhow!("Failed to encode genesis block: {}", e))?;

            // Process genesis block
            match kernel.process_block(&genesis_bytes) {
                Ok(()) => {
                    eprintln!("[kernel] ✓ Genesis block initialized: {}", genesis.block_hash());
                    eprintln!("[kernel]    Chain is now active at height 0");
                }
                Err(e) => {
                    eprintln!("[kernel] ⚠ Failed to initialize genesis block: {:#}", e);
                    eprintln!("[kernel]    This may be expected if genesis is already in the database");
                }
            }
        } else {
            let height = kernel.active_height().unwrap_or(0);
            eprintln!("[kernel] ✓ Genesis block exists. Active chain at height {}", height);
        }

        eprintln!("[kernel] Kernel initialization complete!");
        Ok(kernel)
    }

    pub fn active_height(&self) -> Result<i32> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                return Ok(-1);
            }
            Ok(ffi::btck_chain_get_height(chain))
        }
    }

    // Alias for active_height
    pub fn get_height(&self) -> Result<i32> {
        self.active_height()
    }

    pub fn get_best_block_hash(&self) -> Result<BlockHash> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                anyhow::bail!("no active chain");
            }

            let tip = ffi::btck_chain_get_tip(chain);
            if tip.is_null() {
                anyhow::bail!("no chain tip");
            }

            // Get block hash from tip (returns *const btck_BlockHash)
            let hash_ptr = ffi::btck_block_tree_entry_get_block_hash(tip);
            if hash_ptr.is_null() {
                anyhow::bail!("failed to get block hash");
            }

            // Copy hash bytes
            let hash_bytes = std::slice::from_raw_parts(hash_ptr as *const u8, 32);
            Ok(BlockHash::from_byte_array(hash_bytes.try_into().unwrap()))
        }
    }

    pub fn get_block_hash(&self, height: i32) -> Result<BlockHash> {
        unsafe {
            let chain = ffi::btck_chainstate_manager_get_active_chain(self.chainman);
            if chain.is_null() {
                anyhow::bail!("no active chain");
            }

            let block_tree_entry = ffi::btck_chain_get_by_height(chain, height);
            if block_tree_entry.is_null() {
                anyhow::bail!("block not found at height {}", height);
            }

            // Get block hash (returns *const btck_BlockHash)
            let hash_ptr = ffi::btck_block_tree_entry_get_block_hash(block_tree_entry);
            if hash_ptr.is_null() {
                anyhow::bail!("failed to get block hash");
            }

            // Copy hash bytes
            let hash_bytes = std::slice::from_raw_parts(hash_ptr as *const u8, 32);
            Ok(BlockHash::from_byte_array(hash_bytes.try_into().unwrap()))
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

    pub fn process_block(&self, raw: &[u8]) -> Result<()> {
        use std::os::raw::c_int;

        let ptr = unsafe {
            ffi::btck_block_create(raw.as_ptr() as *const c_void, raw.len())
        };

        if ptr.is_null() {
            anyhow::bail!("btck_block_create failed");
        }

        let mut new_block: c_int = 0;
        let rc = unsafe {
            ffi::btck_chainstate_manager_process_block(
                self.chainman,
                ptr,
                &mut new_block as *mut c_int,
            )
        };

        unsafe { ffi::btck_block_destroy(ptr) };

        if rc != 0 {
            anyhow::bail!("process_block rc={} new_block={}", rc, new_block);
        }

        Ok(())
    }

    /// Validate a transaction's basic structure and rules
    /// Returns (is_valid, rejection_reason)
    pub fn validate_transaction(&self, tx: &bitcoin::Transaction) -> Result<(bool, Option<String>)> {
        use bitcoin::consensus::Encodable;

        // Basic size checks
        let mut size = vec![];
        tx.consensus_encode(&mut size).map_err(|e| anyhow::anyhow!("encoding error: {}", e))?;

        if size.len() < 60 {
            return Ok((false, Some("transaction too small".to_string())));
        }

        // Check inputs and outputs exist
        if tx.input.is_empty() {
            return Ok((false, Some("no inputs".to_string())));
        }

        if tx.output.is_empty() {
            return Ok((false, Some("no outputs".to_string())));
        }

        // Check for negative or overflow output values
        let mut total_out = 0u64;
        for out in &tx.output {
            if out.value.to_sat() > 21_000_000 * 100_000_000 {
                return Ok((false, Some("output value too high".to_string())));
            }
            total_out = total_out.checked_add(out.value.to_sat())
                .ok_or_else(|| anyhow::anyhow!("output value overflow"))?;
        }

        if total_out > 21_000_000 * 100_000_000 {
            return Ok((false, Some("total output value too high".to_string())));
        }

        // Check for duplicate inputs (same prevout)
        let mut seen_prevouts = std::collections::HashSet::new();
        for input in &tx.input {
            if !seen_prevouts.insert(input.previous_output) {
                return Ok((false, Some("duplicate input".to_string())));
            }
        }

        // TODO: Full consensus validation through Kernel FFI
        // This requires additional FFI bindings for:
        // - btck_chainstate_manager_process_transaction
        // - btck_transaction_check_inputs (UTXO validation)
        // - Script execution and signature validation
        //
        // For now, we only do basic structural checks above.
        // The mempool will do additional policy checks.

        Ok((true, None))
    }

    /// Check if a transaction's inputs are available in UTXO set
    /// Returns (all_available, missing_count)
    pub fn check_tx_inputs(&self, _tx: &bitcoin::Transaction) -> Result<(bool, usize)> {
        // TODO: Implement UTXO checking through Kernel FFI
        // This requires:
        // - btck_chainstate_manager_get_utxo(outpoint) -> Option<TxOut>
        // For now, assume inputs are available
        Ok((true, 0))
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

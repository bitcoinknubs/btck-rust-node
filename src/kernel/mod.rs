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
        eprintln!("[kernel] ");
        eprintln!("[kernel] ⚠️  DIRECTORY STRUCTURE (Bitcoin Core/libbitcoinkernel):");
        eprintln!("[kernel]    Block files (blk*.dat): {:?}/", blocksdir);
        eprintln!("[kernel]    Block index (LevelDB):  {:?}/blocks/index/", datadir);
        eprintln!("[kernel]    Chainstate (UTXO):      {:?}/chainstate/", datadir);
        eprintln!("[kernel] ");

        // Check if directory structure might cause issues
        let blocks_canonical = blocksdir.canonicalize().unwrap_or_else(|_| blocksdir.clone());
        let expected_blocks = datadir.join("blocks");
        let expected_canonical = expected_blocks.canonicalize().unwrap_or(expected_blocks.clone());

        if blocks_canonical != expected_canonical {
            eprintln!("[kernel] ⚠️  WARNING: blocksdir != datadir/blocks!");
            eprintln!("[kernel]    This means block files and block index are in different locations.");
            eprintln!("[kernel]    Current:  blocksdir = {:?}", blocks_canonical);
            eprintln!("[kernel]    Expected: blocksdir = {:?}", expected_canonical);
            eprintln!("[kernel]    This may cause blocks to not load on restart!");
            eprintln!("[kernel]    Recommended: Remove --blocksdir or set it to <datadir>/blocks");
            eprintln!("[kernel] ");
        }

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

        // CRITICAL: Set up validation callbacks to monitor block processing
        // Without these, we may not get proper feedback on block validation
        eprintln!("[kernel] Setting up validation interface callbacks...");

        unsafe extern "C" fn block_checked_callback(
            _block_hash: *const u8,
            _state: *const ffi::btck_BlockValidationState,
            _user_data: *mut std::ffi::c_void,
        ) {
            // This is called when a block's validation completes
            eprintln!("[kernel/callback] ✓ Block validation completed");
        }

        unsafe extern "C" fn block_connected_callback(
            _block_hash: *const u8,
            _height: i32,
            _user_data: *mut std::ffi::c_void,
        ) {
            eprintln!("[kernel/callback] ✓ Block CONNECTED to active chain at height {}", _height);
        }

        let mut validation_callbacks = ffi::btck_ValidationInterfaceCallbacks {
            block_checked: Some(block_checked_callback),
            block_connected: Some(block_connected_callback),
            user_data: std::ptr::null_mut(),
        };

        unsafe {
            ffi::btck_context_options_set_validation_interface(
                ctx_opts,
                &mut validation_callbacks as *mut ffi::btck_ValidationInterfaceCallbacks,
            );
        }
        eprintln!("[kernel] Validation interface configured");

        // Set up notification callbacks for error handling
        eprintln!("[kernel] Setting up notification callbacks...");

        unsafe extern "C" fn notify_flush_error_callback(
            _message: *const std::os::raw::c_char,
            _user_data: *mut std::ffi::c_void,
        ) {
            if !_message.is_null() {
                if let Ok(msg) = std::ffi::CStr::from_ptr(_message).to_str() {
                    eprintln!("[kernel/callback] ❌ FLUSH ERROR: {}", msg);
                    eprintln!("[kernel/callback] THIS IS WHY BLOCKS ARE NOT BEING SAVED!");
                }
            }
        }

        let mut notification_callbacks = ffi::btck_NotificationInterfaceCallbacks {
            notify_flush_error: Some(notify_flush_error_callback),
            user_data: std::ptr::null_mut(),
        };

        unsafe {
            ffi::btck_context_options_set_notifications(
                ctx_opts,
                &mut notification_callbacks as *mut ffi::btck_NotificationInterfaceCallbacks,
            );
        }
        eprintln!("[kernel] Notification interface configured");

        // Note: Logging requires btck_logging_connection_create() which needs
        // to be stored and managed separately. Skipping for now.
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

        // CRITICAL: Convert to absolute paths!
        // libbitcoinkernel may not handle relative paths correctly,
        // especially if it changes working directory or runs in different threads.
        let datadir_abs = datadir.canonicalize()
            .unwrap_or_else(|_| {
                eprintln!("[kernel] WARNING: Cannot canonicalize datadir {:?}, creating it first", datadir);
                std::fs::create_dir_all(datadir).ok();
                datadir.canonicalize().unwrap_or_else(|_| datadir.clone())
            });

        let blocksdir_abs = blocksdir.canonicalize()
            .unwrap_or_else(|_| {
                eprintln!("[kernel] WARNING: Cannot canonicalize blocksdir {:?}, creating it first", blocksdir);
                std::fs::create_dir_all(blocksdir).ok();
                blocksdir.canonicalize().unwrap_or_else(|_| blocksdir.clone())
            });

        eprintln!("[kernel] Using ABSOLUTE paths:");
        eprintln!("[kernel]   Data dir:   {:?}", datadir_abs);
        eprintln!("[kernel]   Blocks dir: {:?}", blocksdir_abs);

        let data_c = CString::new(datadir_abs.to_string_lossy().as_bytes())?;
        let blocks_c = CString::new(blocksdir_abs.to_string_lossy().as_bytes())?;
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

        // CRITICAL FIX NEEDED: Load block index and activate chain after restart
        //
        // Problem: After restart, btck_chain_get_height() returns 0 even though
        // blocks were saved to disk in the previous run. This is because:
        // 1. btck_chainstate_manager_create() does NOT automatically call LoadBlockIndex()
        // 2. The active chain tip is not set without explicit activation
        //
        // Bitcoin Core's initialization sequence:
        // - ChainstateManager::Create()
        // - CompleteChainstateInitialization():
        //   - LoadBlockIndex() - loads all blocks from disk into memory
        //   - LoadChainTip() - sets the active chain tip
        //   - ActivateBestChain() - activates the best chain
        //
        // SOLUTION: We need to call one of these libbitcoinkernel functions:
        // - btck_chainstate_manager_activate_best_chain() OR
        // - btck_chainstate_manager_load_chainstate() OR
        // - btck_chainstate_load_block_index()
        //
        // However, these functions may not be exposed in the C API yet.
        //
        // WORKAROUND FOR NOW: If no API exists, the only solution is:
        // 1. Use -reindex flag to rebuild index from block files
        // 2. Or wait for libbitcoinkernel to expose LoadBlockIndex API
        //
        // Uncomment the line below if your libbitcoinkernel version has this function:
        // unsafe { ffi::btck_chainstate_manager_activate_best_chain(chainman); }

        eprintln!("[kernel] WARNING: Block index may not be loaded from disk!");
        eprintln!("[kernel] This is a known limitation of the current libbitcoinkernel C API.");

        let kernel = Self { ctx, chain_params, chainman };

        // Initialize or re-process genesis block
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

            // WORKAROUND: If height is 0 but genesis exists, try re-processing genesis
            // to trigger chain activation (this may help load block index from disk)
            if height == 0 {
                eprintln!("[kernel] Attempting workaround: Re-processing genesis to activate chain...");

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
                    .map_err(|e| anyhow::anyhow!("Failed to encode genesis: {}", e))?;

                match kernel.process_block(&genesis_bytes) {
                    Ok(()) => {
                        eprintln!("[kernel] Genesis re-processed");
                        let new_height = kernel.active_height().unwrap_or(0);
                        eprintln!("[kernel] Height after re-process: {}", new_height);
                    }
                    Err(e) => {
                        eprintln!("[kernel] Genesis re-process error (expected): {:#}", e);
                    }
                }
            }
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

        // Get height BEFORE processing
        let height_before = self.active_height().unwrap_or(-1);

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

        // Handle different return codes:
        // rc=0, new_block=1: Block successfully added to active chain (NEW block)
        // rc=0, new_block=0: Block already exists in block index but successfully processed
        // rc=-1, new_block=0: Block already known/duplicate (NOT an error - normal during resync)
        // rc=-1, new_block=1: Actual error - block validation failed

        if rc != 0 {
            // If rc=-1 and new_block=0, this means "block already known"
            // This is NOT an error - it's normal when resyncing after incomplete shutdown
            if rc == -1 && new_block == 0 {
                // Block already exists in block index - this is normal
                // No need to log for every duplicate block, only for diagnostics
                if height_before < 20 {
                    eprintln!("[kernel] ℹ️  Block already known (rc={}, new_block=0) - skipping", rc);
                }
                return Ok(());
            } else {
                // Actual error - block validation failed
                anyhow::bail!("process_block failed: rc={} new_block={}", rc, new_block);
            }
        }

        // Get height AFTER processing to verify block was added
        let height_after = self.active_height().unwrap_or(-1);

        // DIAGNOSTIC LOGGING STRATEGY:
        // - Always log first 20 blocks after genesis (height 0-19)
        // - Always log first 20 blocks after restart (track session count)
        // - Then log every 10th block up to 100
        // - Then every 100th block
        use std::sync::atomic::{AtomicI32, Ordering};
        static SESSION_BLOCKS_ADDED: AtomicI32 = AtomicI32::new(0);

        let session_count = if new_block == 1 {
            SESSION_BLOCKS_ADDED.fetch_add(1, Ordering::Relaxed)
        } else {
            SESSION_BLOCKS_ADDED.load(Ordering::Relaxed)
        };

        let should_log = height_after < 20                    // First 20 blocks after genesis
                      || session_count < 20                   // First 20 blocks this session
                      || (height_after < 100 && height_after % 10 == 0)  // Every 10th up to 100
                      || height_after % 100 == 0;             // Every 100th after that

        if new_block == 1 {
            if height_after == height_before {
                eprintln!("[kernel] ⚠️  CRITICAL: process_block succeeded but height didn't change!");
                eprintln!("[kernel]    Height before: {}, Height after: {}", height_before, height_after);
                eprintln!("[kernel]    new_block={}, rc={}", new_block, rc);
                eprintln!("[kernel]    This indicates blocks are NOT being added to the active chain!");
            } else if should_log {
                eprintln!("[kernel] ✓ Block ADDED to active chain: height {} -> {} (session: +{})",
                         height_before, height_after, session_count + 1);

                // CRITICAL DIAGNOSTIC: Verify block file was actually written
                if height_after <= 10 || height_after % 100 == 0 {
                    self.verify_block_files_written(height_after);
                }
            }
        } else {
            // new_block == 0 with rc=0 means block was already in index but processed successfully
            if should_log {
                eprintln!("[kernel] ℹ️  Block already in index: height remains {}, new_block={}", height_after, new_block);
            }
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

    /// CRITICAL DIAGNOSTIC: Verify block files are actually being written to disk
    fn verify_block_files_written(&self, height: i32) {
        use std::fs;
        use std::io::Read;

        eprintln!("[kernel] 🔍 VERIFYING BLOCK FILE after height {}...", height);

        // Check blk00000.dat (first block file)
        let block_file = std::path::Path::new("./data/blocks/blk00000.dat");

        match fs::metadata(block_file) {
            Ok(metadata) => {
                let size = metadata.len();
                eprintln!("[kernel]    ✓ blk00000.dat EXISTS");
                eprintln!("[kernel]    📊 File size: {} bytes ({:.2} MB)", size, size as f64 / 1024.0 / 1024.0);

                // Read first 1KB to check for block magic bytes
                if let Ok(mut file) = fs::File::open(block_file) {
                    let mut buffer = vec![0u8; 1024];
                    if let Ok(n) = file.read(&mut buffer) {
                        // Signet magic: 0a 03 cf 40
                        let magic_count = buffer.windows(4)
                            .filter(|w| w == &[0x0a, 0x03, 0xcf, 0x40])
                            .count();

                        if magic_count > 0 {
                            eprintln!("[kernel]    ✅ Found {} block magic bytes in first 1KB!", magic_count);
                        } else {
                            eprintln!("[kernel]    ⚠️  NO BLOCK MAGIC BYTES found in first 1KB!");
                            eprintln!("[kernel]    First 64 bytes: {:02x?}", &buffer[..64.min(n)]);
                        }
                    }
                }

                // Check if file size is increasing (not stuck at pre-allocated size)
                use std::sync::atomic::{AtomicU64, Ordering};
                static LAST_SIZE: AtomicU64 = AtomicU64::new(0);

                let prev_size = LAST_SIZE.load(Ordering::Relaxed);
                if prev_size > 0 {
                    if size == prev_size {
                        eprintln!("[kernel]    ❌ FILE SIZE NOT CHANGING! Still {} bytes", size);
                        eprintln!("[kernel]    This means blocks are NOT being written to disk!");
                    } else {
                        eprintln!("[kernel]    ✅ File growing: {} -> {} (+{} bytes)",
                                 prev_size, size, size - prev_size);
                    }
                }
                LAST_SIZE.store(size, Ordering::Relaxed);
            }
            Err(e) => {
                eprintln!("[kernel]    ❌ blk00000.dat DOES NOT EXIST: {}", e);
                eprintln!("[kernel]    Blocks are being processed but NOT written to disk!");
            }
        }

        // Also check absolute path to ensure it's not being written elsewhere
        if let Ok(cwd) = std::env::current_dir() {
            let abs_path = cwd.join("data/blocks/blk00000.dat");
            eprintln!("[kernel]    Expected absolute path: {:?}", abs_path);

            // Search for ANY blk*.dat files in the system
            eprintln!("[kernel]    Checking if blocks written to different location...");
            if let Ok(home) = std::env::var("HOME") {
                let alt_path = std::path::Path::new(&home)
                    .join("development/btck-rust-node/data/blocks/blk00000.dat");
                if alt_path.exists() {
                    if let Ok(meta) = fs::metadata(&alt_path) {
                        eprintln!("[kernel]    ℹ️  Found blk00000.dat at alternate location: {:?}", alt_path);
                        eprintln!("[kernel]       Size: {} bytes", meta.len());
                    }
                }
            }
        }
    }
}

impl Drop for Kernel {
    fn drop(&mut self) {
        eprintln!("[kernel] 🔄 Dropping Kernel - flushing chainstate to disk...");
        unsafe {
            eprintln!("[kernel]    Destroying chainstate manager (calls ForceFlushStateToDisk)...");
            ffi::btck_chainstate_manager_destroy(self.chainman);
            eprintln!("[kernel]    ✓ Chainstate manager destroyed and flushed");

            ffi::btck_context_destroy(self.ctx);
            ffi::btck_chain_parameters_destroy(self.chain_params);
        }
        eprintln!("[kernel] ✅ Kernel dropped - index and chainstate flushed to disk");
    }
}

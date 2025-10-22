use crate::ffi;
use anyhow::Result;
use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};
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
        let chain_type: u8 = match chain {
            "main" | "mainnet" => CHAIN_MAIN,
            "testnet" => CHAIN_TESTNET,
            "testnet4" => CHAIN_TESTNET4,
            "signet" => CHAIN_SIGNET,
            _ => CHAIN_REGTEST,
        };

        let ctx_opts = unsafe { ffi::btck_context_options_create() };
        if ctx_opts.is_null() {
            anyhow::bail!("btck_context_options_create failed");
        }

        let chain_params = unsafe { ffi::btck_chain_parameters_create(chain_type) };
        if chain_params.is_null() {
            unsafe { ffi::btck_context_options_destroy(ctx_opts) };
            anyhow::bail!("btck_chain_parameters_create failed");
        }
        unsafe { ffi::btck_context_options_set_chainparams(ctx_opts, chain_params) };

        let log_opts: ffi::btck_LoggingOptions = unsafe { std::mem::zeroed() };
        let _conn = unsafe {
            ffi::btck_logging_connection_create(
                Some(log_cb),
                std::ptr::null_mut(),
                None,
                log_opts,
            )
        };

        let ctx = unsafe { ffi::btck_context_create(ctx_opts) };
        if ctx.is_null() {
            unsafe {
                ffi::btck_context_options_destroy(ctx_opts);
                ffi::btck_chain_parameters_destroy(chain_params);
            }
            anyhow::bail!("btck_context_create failed");
        }
        unsafe { ffi::btck_context_options_destroy(ctx_opts) };

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

        unsafe {
            ffi::btck_chainstate_manager_options_update_block_tree_db_in_memory(chainman_opts, 1);
            ffi::btck_chainstate_manager_options_update_chainstate_db_in_memory(chainman_opts, 1);
            ffi::btck_chainstate_manager_options_set_worker_threads_num(chainman_opts, 2);
        }

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

        Ok(Self { ctx, chain_params, chainman })
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

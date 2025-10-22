#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals)]
// bindgen이 생성한 바인딩
include!(concat!(env!("OUT_DIR"), "/btck_bindings.rs"));

/// --- 래퍼 상수 (옵션) ---
pub const LOGCAT_ALL: u8 = 0;
pub const LOGCAT_VALIDATION: u8 = 9;
pub const LOGCAT_KERNEL: u8 = 10;

pub const LOGLEVEL_TRACE: u8 = 0;
pub const LOGLEVEL_DEBUG: u8 = 1;
pub const LOGLEVEL_INFO:  u8 = 2;

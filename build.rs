use std::{env, fs, path::{Path, PathBuf}};

fn first_existing(paths: &[PathBuf]) -> Option<PathBuf> {
    for p in paths {
        if fs::metadata(p).is_ok() {
            return Some(p.clone());
        }
    }
    None
}

fn main() {
    // --- 링크 설정 ---
    let lib_dir = env::var("BITCOINKERNEL_LIB_DIR")
        .unwrap_or_else(|_| "/usr/local/lib".to_string());
    println!("cargo:rustc-link-search=native={}", lib_dir);
    println!("cargo:rustc-link-lib=dylib=bitcoinkernel");

    // --- 헤더 탐색 ---
    let header_env = env::var("BITCOINKERNEL_HEADER").ok().map(PathBuf::from);
    let include_dir_env = env::var("BITCOINKERNEL_INCLUDE_DIR").ok().map(PathBuf::from);
    let include_header_env = include_dir_env.as_ref().map(|d| d.join("bitcoinkernel.h"));

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let candidates = {
        let mut v = vec![];
        if let Some(h) = header_env.clone() { v.push(h); }
        if let Some(h) = include_header_env.clone() { v.push(h); }

        v.push(PathBuf::from("/usr/local/include/bitcoinkernel.h"));
        v.push(PathBuf::from("/opt/homebrew/include/bitcoinkernel.h"));

        // ../bitcoin 레이아웃 지원
        v.push(manifest_dir.join("../bitcoin/src/kernel/bitcoinkernel.h"));
        v.push(manifest_dir.join("../bitcoin/build/include/bitcoinkernel.h"));
        v
    };

    let header = first_existing(&candidates).unwrap_or_else(|| {
        eprintln!("\n[build.rs] bitcoinkernel.h 를 찾지 못했습니다.");
        eprintln!("  1) BITCOINKERNEL_HEADER=/path/to/bitcoinkernel.h");
        eprintln!("  2) BITCOINKERNEL_INCLUDE_DIR=/path/to/include");
        eprintln!("  3) Bitcoin Core 설치(cmake install)로 /usr/local/include/bitcoinkernel.h 생성\n");
        eprintln!("참고: 현재 탐색한 후보들:");
        for c in &candidates { eprintln!("  - {}", c.display()); }
        panic!(r#"Unable to generate btck bindings: NotExist("bitcoinkernel.h")"#);
    });

    // --- clang include 경로 구성 ---
    let mut clang_args = vec![];
    if let Some(ref dir) = include_dir_env {
        clang_args.push("-I".to_string());
        clang_args.push(dir.display().to_string());
    }
    for d in ["/usr/local/include", "/opt/homebrew/include"] {
        if Path::new(d).exists() {
            clang_args.push("-I".to_string());
            clang_args.push(d.to_string());
        }
    }

    // --- bindgen 생성 ---
    let builder = bindgen::Builder::default()
        .header(header.display().to_string())
        .allowlist_function("btck_.*")
        .allowlist_type("btck_.*")
        .allowlist_var("btck_.*")
        .clang_args(clang_args);

    let bindings = builder.generate().expect("Unable to generate btck bindings");
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("btck_bindings.rs"))
        .expect("Couldn't write btck_bindings.rs");

    // --- 리빌드 트리거 ---
    println!("cargo:rerun-if-env-changed=BITCOINKERNEL_LIB_DIR");
    println!("cargo:rerun-if-env-changed=BITCOINKERNEL_HEADER");
    println!("cargo:rerun-if-env-changed=BITCOINKERNEL_INCLUDE_DIR");
    println!("cargo:rerun-if-changed={}", header.display());
}

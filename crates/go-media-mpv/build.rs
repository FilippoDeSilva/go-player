//! Build script: locate the provisioned libmpv binaries under
//! `native/mpv/<target-triple>/` (see `scripts/setup-mpv.ps1`) and:
//!
//! 1. emit `cargo:rustc-link-search` so `mpv.lib` is found at link time
//! 2. copy `libmpv-2.dll` next to the dev binaries so it resolves at
//!    runtime without installer bundling (bundling comes later via
//!    Tauri resources)
//!
//! If the files are absent we emit a warning instead of failing, so
//! `cargo check`/docs still work on machines without mpv provisioned.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        // Linux/macOS provisioning lands with those platform setups.
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // crates/go-media-mpv -> repo root
    let repo_root = manifest_dir.join("../..").canonicalize().unwrap();
    let lib_dir = repo_root.join("native/mpv/x86_64-pc-windows-msvc");
    let import_lib = lib_dir.join("mpv.lib");
    let dll = lib_dir.join("libmpv-2.dll");

    if !import_lib.exists() || !dll.exists() {
        println!(
            "cargo:warning=libmpv not provisioned at {} — run scripts/setup-mpv.ps1",
            lib_dir.display()
        );
        return;
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rerun-if-changed={}", import_lib.display());
    println!("cargo:rerun-if-changed={}", dll.display());

    // Stage the DLL next to dev artifacts. Skip the copy when the staged
    // file already matches (avoids re-copying ~120MB on every build).
    for profile in ["debug", "release"] {
        let out_dir = repo_root.join("target").join(profile);
        if let Err(e) = fs::create_dir_all(&out_dir) {
            println!("cargo:warning=failed to create {}: {e}", out_dir.display());
            continue;
        }
        let dest = out_dir.join("libmpv-2.dll");
        let needs_copy = match (fs::metadata(&dll), fs::metadata(&dest)) {
            (Ok(src), Ok(dst)) => src.len() != dst.len(),
            _ => true,
        };
        if needs_copy {
            if let Err(e) = fs::copy(&dll, &dest) {
                println!("cargo:warning=failed to stage {}: {e}", dest.display());
            }
        }
    }
}

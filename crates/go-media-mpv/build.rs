//! Build script: locate the provisioned libmpv under
//! `native/mpv/<target-triple>/` and emit the link search path so
//! `libmpv2-sys`'s `cargo:rustc-link-lib=mpv` resolves.
//!
//! - Windows: `scripts/setup-mpv.ps1` stages `mpv.lib` + `libmpv-2.dll`
//!   (the DLL is also copied next to dev binaries for `cargo run`).
//! - Linux/macOS: `scripts/build-mpv-linux.sh` / `build-mpv-macos.sh`
//!   compile a minimal libmpv + ffmpeg + libplacebo + libass from pinned
//!   sources into `native/mpv/<triple>/lib`. Those same files are bundled
//!   into installers via `bundle.resources`; the binary finds them
//!   through the RUNPATH entries in `.cargo/config.toml`.
//!
//! If the files are absent we emit a warning instead of failing, so
//! `cargo check`/docs still work on machines without mpv provisioned.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let target = env::var("TARGET").unwrap_or_default();
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    // crates/go-media-mpv -> repo root
    let repo_root = manifest_dir.join("../..").canonicalize().unwrap();
    let native_dir = repo_root.join("native/mpv").join(&target);

    if target.contains("windows") {
        provision_windows(&native_dir, &repo_root);
    } else if target.contains("linux") || target.contains("apple") {
        provision_unix(&native_dir, &repo_root, &target);
    }
}

/// Windows provisioning: `mpv.lib` import library + runtime DLL staged
/// next to dev binaries.
fn provision_windows(lib_dir: &Path, repo_root: &Path) {
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
        if needs_copy && let Err(e) = fs::copy(&dll, &dest) {
            println!("cargo:warning=failed to stage {}: {e}", dest.display());
        }
    }
}

/// Linux/macOS provisioning: the minimal libmpv build lives under
/// `native/mpv/<triple>/lib` (produced by the platform build script).
/// We only need the link search path here — runtime resolution is
/// handled by the bundled RUNPATH (`.cargo/config.toml`) in packaged
/// builds; for `cargo run` devs set `LD_LIBRARY_PATH`/`DYLD_*`.
fn provision_unix(lib_parent: &Path, _repo_root: &Path, target: &str) {
    let lib_dir = lib_parent.join("lib");
    // Accept both plain and versioned sonames (our own build may produce
    // e.g. libmpv.2.dylib on macOS or libmpv.so.2 on Linux). Meson
    // installs into lib/<multiarch>/ on Debian/Ubuntu while FFmpeg's
    // autotools build installs into lib/ — search both.
    let candidates: &[&str] = if target.contains("linux") {
        &["libmpv.so", "libmpv.so.2", "libmpv.so.1"]
    } else {
        &["libmpv.dylib", "libmpv.2.dylib"]
    };

    let mut search_dirs = vec![lib_dir.clone()];
    if let Ok(entries) = fs::read_dir(&lib_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                search_dirs.push(path);
            }
        }
    }

    let Some(found) = search_dirs.iter().find_map(|dir| {
        candidates
            .iter()
            .map(|c| dir.join(c))
            .find(|p| p.exists())
            .map(|lib| (dir.clone(), lib))
    }) else {
        println!(
            "cargo:warning=libmpv not provisioned at {} — run the platform build script",
            lib_dir.display()
        );
        return;
    };
    let (link_dir, lib) = found;

    println!("cargo:rustc-link-search=native={}", link_dir.display());
    println!("cargo:rerun-if-changed={}", lib.display());
}

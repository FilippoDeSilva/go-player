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
    let (_, lib) = found;

    // Emit a search path for every provisioned lib dir: libmpv's own
    // ffmpeg/libass/placebo dependencies live in `lib/` while libmpv.so
    // itself sits in a multiarch subdir. Without `lib/` on the link
    // search path, ld cannot resolve libmpv's versioned ffmpeg symbols
    // unless LD_LIBRARY_PATH happens to be set.
    for dir in &search_dirs {
        println!("cargo:rustc-link-search=native={}", dir.display());
    }

    // Dev-profile binaries/tests of THIS crate (e.g. tests/headless.rs)
    // have no $ORIGIN anchor to the provisioned libs — give them an
    // explicit rpath, same as the app crate's build.rs does. RUNPATH
    // suffices: transitive deps are covered by the $ORIGIN RUNPATH we
    // patch onto the bundled .so files themselves (ensure_soname_rpath).
    // Skipped for release: packaged artifacts resolve via
    // .cargo/config.toml RUNPATH and must not carry build-machine
    // absolute paths.
    if env::var("PROFILE").as_deref() != Ok("release") && target.contains("linux") {
        for dir in &search_dirs {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
        }
    }

    // libmpv's ffmpeg/libass/placebo dependencies are resolved by ld
    // through -L only for direct -l flags; a shared lib's own NEEDED
    // entries need the libs on the link line (or -rpath-link). Link
    // the bundled copies explicitly so versioned symbols resolve
    // without LD_LIBRARY_PATH. Only emit for libs we actually
    // provisioned — macOS builds ffmpeg statically and has no .so.
    if target.contains("linux") {
        for name in [
            "avcodec",
            "avfilter",
            "avformat",
            "avutil",
            "swresample",
            "swscale",
            "placebo",
            "ass",
        ] {
            let soname = format!("lib{name}.so");
            if search_dirs.iter().any(|d| d.join(&soname).exists()) {
                println!("cargo:rustc-link-lib=dylib={name}");
            }
        }

        ensure_soname_rpath(&search_dirs);
    }

    println!("cargo:rerun-if-changed={}", lib.display());
}

/// Give every provisioned shared library an `$ORIGIN`-relative
/// RUNPATH so it finds its bundled siblings regardless of which
/// binary loads it. The provisioned layout splits libs across
/// `lib/` (ffmpeg) and `lib/<multiarch>/` (mpv, ass, placebo,
/// wayland), and the built .so files carry no rpath of their own —
/// without this, consumers that don't link every dep directly
/// (e.g. `tests/headless.rs`, which only NEEDs libmpv) fail at
/// runtime with "libavcodec.so.61: cannot open shared object".
///
/// `$ORIGIN/..` covers the parent dir so `lib/<multiarch>/libmpv`
/// finds ffmpeg in `lib/`; in the flattened installer layout all
/// libs share one dir and `$ORIGIN` alone suffices (`$ORIGIN/..`
/// is then a harmless extra entry).
///
/// No-op when `patchelf` is unavailable or the rpath is already set.
fn ensure_soname_rpath(search_dirs: &[PathBuf]) {
    if std::process::Command::new("patchelf")
        .arg("--version")
        .output()
        .is_err()
    {
        println!("cargo:warning=patchelf not found — bundled libs keep their empty RUNPATH");
        return;
    }
    for dir in search_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip symlinks (soname links) — patch the real files.
            let Ok(meta) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.file_type().is_symlink() {
                continue;
            }
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.contains(".so") {
                continue;
            }
            let current = std::process::Command::new("patchelf")
                .arg("--print-rpath")
                .arg(&path)
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            // Skip only when already correct — a bare "$ORIGIN" (or a
            // corrupted value like "RIGIN:") still needs the parent
            // dir for the split lib/ + lib/<multiarch>/ layout.
            if current.trim() == "$ORIGIN:$ORIGIN/.." {
                continue;
            }
            let status = std::process::Command::new("patchelf")
                .arg("--set-rpath")
                .arg("$ORIGIN:$ORIGIN/..")
                .arg(&path)
                .status();
            match status {
                Ok(s) if s.success() => {
                    println!("cargo:warning=patched RUNPATH on {}", path.display());
                }
                _ => println!(
                    "cargo:warning=failed to patch RUNPATH on {}",
                    path.display()
                ),
            }
        }
    }
}

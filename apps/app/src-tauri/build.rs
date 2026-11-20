use std::env;
use std::path::PathBuf;

fn main() {
    // Dev builds (`cargo run`, `tauri dev`) link against the
    // provisioned libmpv under `native/mpv/<target>/lib`. Packaged
    // builds resolve the same files through the `$ORIGIN` RUNPATH in
    // `.cargo/config.toml`, but unpackaged binaries have no such
    // anchor — give them an explicit rpath to the provisioned libs.
    //
    // This must NOT be done via LD_LIBRARY_PATH: the variable is
    // inherited by WebKitGTK's WebProcess/WebKitGPUProcess children,
    // where our bundled wayland/ffmpeg libs shadow the system ones
    // and break the WebProcess's EGL init (`EGL_BAD_PARAMETER` abort).
    let target = env::var("TARGET").unwrap_or_default();
    // Only unpackaged dev builds need this — release artifacts resolve
    // bundled libs via the `$ORIGIN` RUNPATH and shouldn't carry a
    // build-machine absolute path.
    // Skip macOS for now — it uses @rpath/@loader_path and has different
    // library loading semantics. Will add proper macOS rpath handling later.
    if env::var("PROFILE").as_deref() != Ok("release") && target.contains("linux") {
        let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
        let repo_root = manifest_dir.join("../..").canonicalize().unwrap();
        let lib_dir = repo_root.join("native/mpv").join(&target).join("lib");
        // Only add rpath if the directory exists
        if lib_dir.is_dir() {
            let mut dirs = vec![lib_dir.clone()];
            if let Ok(entries) = std::fs::read_dir(&lib_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        dirs.push(path);
                    }
                }
            }
            for dir in dirs {
                println!("cargo:rustc-link-arg=-Wl,-rpath,{}", dir.display());
            }
        }
    }

    tauri_build::build()
}

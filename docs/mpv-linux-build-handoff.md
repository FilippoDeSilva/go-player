# Handoff: self-contained libmpv build on Linux (Kali VM)

## Goal

`scripts/build-mpv-linux.sh` must complete successfully and produce a
self-contained set of shared libraries that let the Go Player binary run
on any mainstream Linux distro without system libmpv/FFmpeg packages.

## Current state (as of commit `40bb184`)

Working directory on the VM: `~/go-player`

Already built and installed into `native/mpv/x86_64-unknown-linux-gnu/`:

- **FFmpeg 7.1.1** — built OK (broad decoder/demuxer/protocol set,
  VA-API hwaccel, screenshot encoders). ~19 `.so` files in `lib/`.
- **libplacebo v7.349.0** — built OK after two fixes (see below).
- **libass 0.17.4** — built OK.

Currently failing at the **mpv meson configure** step. This is normal —
mpv's `meson.options` has hard `feature` constraints: enabling feature X
can _require_ feature Y, and `ERROR: Feature X cannot be disabled:
dependency was not found` means you must either install the `-dev`
package or disable the dependent feature in `MESON_ARGS` inside
`scripts/build-mpv-linux.sh` (`build_mpv()`).

## Fixes already applied (do not redo)

1. `FFMPEG_VER=7.1.1` (ffmpeg.org tarballs have no `n` prefix).
2. `fetch()` retries + `tar -tf` integrity check; `extract()` is atomic.
3. `meson_build()` re-setups when `build/build.ninja` is missing/invalid.
4. libplacebo is `git clone --recurse-submodules` (tarballs lack the
   `glad`/`Vulkan-Headers` submodules) with HTTP/1.1 + retries.
5. `sed` patch for `utils_gen.py` Python ≥3.14 (`ET.parse().getroot()`).
6. `PKG_CONFIG_PATH` includes `$PREFIX/lib/pkgconfig` AND the Debian
   multiarch dir (`lib/x86_64-linux-gnu/pkgconfig`) — meson puts
   libplacebo.pc/libass.pc there.
7. mpv options already disabled: `cplayer lua javascript cplugins
vapoursynth dvdnav libbluray cdda dvbin rubberband libarchive zimg
lcms2 jpeg pipewire jack openal oss-audio sndio xv drm gbm
dmabuf-wayland vdpau`.
   Enabled: `libmpv alsa pulse x11 wayland gl egl egl-x11 egl-wayland
vaapi vaapi-x11 vaapi-wayland vulkan uchardet coreaudio? (no —
coreaudio is macOS only, it's disabled on Linux by not being passed)`.

## Current error and likely next ones

Latest failure:

```
meson.build:1072:13: ERROR: Dependency "xpresent" not found
```

**Easy fix:** `sudo apt install libxpresent-dev`, then rerun. If more
`Dependency "..." not found` errors appear, map them to packages:

| meson dep         | apt package                                           |
| ----------------- | ----------------------------------------------------- |
| xpresent          | libxpresent-dev                                       |
| xcb-icccm / xcb-* | libxcb-*-dev (mostly already installed)               |
| evdev             | libevdev-dev                                          |
| libplacebo        | (already built — should be found via PKG_CONFIG_PATH) |
| uchardet          | libuchardet-dev (installed)                           |
| libva / libva-x11 | libva-dev (installed); try libva-x11-dev if asked     |
| libvdpau          | NOT wanted — keep `-Dvdpau=disabled`                  |
| libdrm / gbm      | NOT wanted — keep disabled (dmabuf/drm chain)         |

If instead the error is `Feature X cannot be disabled: Y was not found`,
add `-DX=disabled` (or `-DY=disabled` if Y is the optional one) to the
`MESON_ARGS` array in `build_mpv()`.

## How to iterate (fast loop)

```bash
cd ~/go-player
git pull --ff-only
sudo apt install -y <missing -dev package>   # if it's a missing dep

# mpv's failed meson setup leaves a stale build dir — remove it:
rm -rf .mpv-build/mpv-v0.40.0/build

# FFmpeg/libplacebo/libass are already built — skip them:
SKIP_FFMPEG=1 ./scripts/build-mpv-linux.sh 2>&1 | tee /tmp/mpvbuild.log
```

Note: `SKIP_FFMPEG=1` only skips FFmpeg — libplacebo/libass/mpv still
run, but their `build/` dirs are reused if `build.ninja` exists (fast).
If a meson project needs re-configuring after an option change, delete
its `build/` dir first (command above for mpv; same pattern for
`.mpv-build/libplacebo-*/build` and `.mpv-build/libass-*/build`).

## When the script finishes

It prints the staged runtime and copies `.so`s to
`apps/app/src-tauri/bundle-resources/linux/`. Then verify:

```bash
# 1. All NEEDED libs of libmpv.so resolve or are intentionally system:
ldd native/mpv/x86_64-unknown-linux-gnu/lib/libmpv.so.2 | grep "not found"
# Expected: only genuinely-optional system libs may show; nothing from
# our build (libav*, libplacebo, libass) should be "not found".

# 2. Confirm rpath is embedded:
readelf -d native/mpv/x86_64-unknown-linux-gnu/lib/libmpv.so.2 | grep -i 'rpath\|runpath'
# Expect: RUNPATH containing $ORIGIN

# 3. Size sanity:
du -sh apps/app/src-tauri/bundle-resources/linux/
```

## After that (bigger verification, needs Rust toolchain)

```bash
# install rust via rustup or apt, then:
cargo build --release -p go-media-mpv   # links via build.rs link-search
LD_LIBRARY_PATH=$PWD/native/mpv/x86_64-unknown-linux-gnu/lib \
  cargo run -p go-player                # dev run with bundled libs
```

The Tauri bundle itself (`pnpm tauri build`) additionally needs
`libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev
librsvg2-dev patchelf` — install those only when you reach this stage.
On Kali (rolling, newer glibc) the artifacts will have a higher glibc
floor than the ubuntu-22.04 CI build — that's expected; CI remains the
release builder.

## Known remaining risks

- mpv may request more X11/Wayland `-dev` packages — keep iterating the
  table above; prefer installing the package over disabling features
  for x11/wayland/vaapi (core playback paths).
- `libmpv.so` soname on Linux will likely be `libmpv.so.2`; `build.rs`
  already accepts `libmpv.so`, `.so.1`, `.so.2`.
- If `meson install` for mpv produces `libmpv.dylib`-style naming issues
  — not applicable on Linux.

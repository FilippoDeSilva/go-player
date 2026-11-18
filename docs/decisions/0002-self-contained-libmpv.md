# ADR 0002: Self-contained libmpv runtime on Linux and macOS

- **Status:** Accepted
- **Date:** 2025-11-18

## Context

The desktop app links libmpv through `go-media-mpv`. On Windows we already
ship a pinned `libmpv-2.dll` as a bundled resource. On Linux and macOS the
CI initially relied on the distro/brew system library (`libmpv-dev`,
`brew install mpv`).

That approach fails on fresh installs:

- Ubuntu 22.04's `libmpv.so.1` and Ubuntu 24.04's `libmpv.so.2` are
  different sonames with different FFmpeg ABI generations
  (`libavcodec.so.58` vs `.60`). A binary built on one cannot run on the
  other even with the "right" package installed.
- Distro libmpv pulls in ~150 shared libraries (SDL2, Lua, libass,
  libplacebo, every optional codec/filter), so bundling the distro `.so`
  and its dependency closure would add hundreds of MB.
- The same drift applies on macOS, where brew's `libmpv.dylib` drags the
  entire brew FFmpeg tree.

## Decision

Build a minimal, pinned libmpv runtime from source in CI and bundle it
inside every installer:

- **FFmpeg** (`n7.1.1`): `--disable-everything` plus all decoders,
  parsers, demuxers, muxers, protocols, filters, and bitstream filters;
  no devices/indevs/outdevs; VA-API (Linux) / VideoToolbox (macOS)
  hardware acceleration; PNG/JPEG/BMP/PPM encoders kept for mpv
  screenshots.
- **libplacebo** (`v7.349.0`): Vulkan + OpenGL on Linux, OpenGL on
  macOS; no glslang/shaderc (uses built-in `pl_shaderc`).
- **libass** (`0.17.4`): default features for subtitle rendering.
- **mpv** (`v0.40.0`): libmpv only (`-Dcplayer=false`), scripting
  (Lua/JS/cplugins), DVD/BD, VapourSynth, rubberband and other optional
  integrations disabled. Audio: ALSA + PulseAudio on Linux, CoreAudio on
  macOS. Video: X11 + Wayland + gpu-next on Linux; plain-gl +
  VideoToolbox on macOS.

Every produced library is linked with an `$ORIGIN` (Linux) /
`@loader_path` (macOS) RUNPATH so the bundled set is self-contained. The
application binary finds the set through platform RUNPATH entries in
`.cargo/config.toml`, and Tauri `bundle.resources` (per-platform
`tauri.*.conf.json`) installs them at:

- deb/rpm: `/usr/lib/go-player/lib` (exe at `/usr/bin/go-player`)
- AppImage: `usr/lib/go-player/lib` inside `$APPDIR`
- macOS: `Contents/Resources/lib` (exe at `Contents/MacOS`)

Scripts: `scripts/build-mpv-linux.sh`, `scripts/build-mpv-macos.sh`.
Build on the oldest supported glibc/macOS version; the artifacts carry
that as their minimum.

## Consequences

- Installers are self-contained: no `libmpv*`/FFmpeg distro packages
  required, and distro soname drift no longer breaks installs.
- Deliberate compatibility trade-offs:
  - VDPAU and PipeWire are disabled on Linux to avoid hard `NEEDED`
    entries on libraries that minimal installs may lack. VA-API remains
    the hardware-decode path (NVIDIA works via nvidia-vaapi-driver).
  - Remaining runtime deps are base desktop libraries (libc, X11/Wayland,
    Mesa/EGL, fontconfig, freetype, harfbuzz, libva, libpulse/libasound or
    CoreAudio, zlib, OpenSSL) — verified with `ldd`/`otool` in clean
    containers before release.
- Release builds take longer (FFmpeg + mpv compile in CI, ~20-40 min on
  the Linux leg).
- mpv is GPL; shipping libmpv already imposes GPL obligations on the
  distributed binary — tracked in `docs/licensing.md`.
- If a codec proves missing, extend the FFmpeg component list in the
  build script rather than switching back to system libraries.

## Alternatives considered

- **Distro `libmpv2` + `deb.depends`:** rejected — soname/FFmpeg drift
  makes it work on exactly one distro vintage.
- **Bundle distro libmpv + full dependency closure:** rejected —
  hundreds of MB of unrelated libraries.
- **Static FFmpeg inside libmpv:** considered; shared build chosen for
  simpler debugging and smaller individual artifacts at similar total
  size.

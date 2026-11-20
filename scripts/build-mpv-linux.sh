#!/usr/bin/env bash
# Build a minimal, self-contained libmpv runtime for Linux.
#
# Why this exists: distro-packaged libmpv (e.g. Ubuntu's libmpv.so.1/.so.2)
# drags in ~150 shared libraries and its FFmpeg sonames change every
# release, so a binary built on Ubuntu 22.04 cannot run on Ubuntu 24.04+.
# Instead we compile a deliberately scoped FFmpeg + libplacebo + libass +
# mpv from pinned sources and ship the resulting .so set inside the
# installer. The app binary finds them via the $ORIGIN RUNPATH in
# .cargo/config.toml; the bundled libs find each other via their own
# $ORIGIN RUNPATH (set below through LDFLAGS/--extra-ldflags).
#
# Output layout (consumed by tauri.linux.conf.json bundle.resources):
#   native/mpv/<target-triple>/lib/               produced shared libraries
#   native/mpv/<target-triple>/include/mpv/       client headers
#   apps/app/src-tauri/bundle-resources/linux/    staged copies for bundling
#
# Build on the OLDEST glibc you want to support: the resulting libs carry
# that glibc as their minimum. ubuntu-22.04 (glibc 2.35) covers Ubuntu
# 22.04+, Debian 12+, Fedora and newer.
#
# Required packages (Ubuntu 22.04/24.04 names):
#   build-essential git meson ninja-build pkg-config yasm nasm python3-pip
#   libva-dev libx11-dev libxcb1-dev libxcb-icccm4-dev libxcb-image0-dev
#   libxcb-keysyms1-dev libxcb-randr0-dev libxcb-render-util0-dev
#   libxcb-shape0-dev libxcb-shm0-dev libxcb-sync-dev libxcb-xfixes0-dev
#   libxcb-xkb-dev libxcb-xinerama0-dev libxcb-cursor-dev libxext-dev
#   libxfixes-dev libxrandr-dev libxrender-dev libxss-dev libxi-dev
#   libxpresent-dev
#   libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev wayland-protocols
#   libegl-dev libgl-dev libgles-dev libgbm-dev libdrm-dev
#   libpulse-dev libasound2-dev libudev-dev libvulkan-dev
#   libfreetype-dev libfontconfig-dev libfribidi-dev libharfbuzz-dev
#   libuchardet-dev zlib1g-dev libssl-dev libexpat-dev patchelf
#   (meson >= 1.3 is required by mpv; install via `pip install meson` if
#    the distro package is too old)
#
# Usage: scripts/build-mpv-linux.sh [--skip-build]
set -euo pipefail

# --- Pinned sources (bump deliberately; check mpv meson.build minimums) ---
FFMPEG_VER="${FFMPEG_VER:-7.1.1}"
LIBPLACEBO_VER="${LIBPLACEBO_VER:-v7.349.0}"
LIBASS_VER="${LIBASS_VER:-0.17.4}"
# mpv requires wayland-client/cursor >= 1.21 but ubuntu-22.04 ships 1.20,
# so wayland is built from source like the other bundled deps.
WAYLAND_VER="${WAYLAND_VER:-1.23.1}"
WAYLAND_PROTOCOLS_VER="${WAYLAND_PROTOCOLS_VER:-1.41}"
MPV_VER="${MPV_VER:-v0.40.0}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Derive the Rust target triple from uname (rustc may not be installed yet
# in a provisioning step, and the triple is fully determined by the arch).
case "$(uname -m)" in
  x86_64)  TARGET="${TARGET:-x86_64-unknown-linux-gnu}" ;;
  aarch64) TARGET="${TARGET:-aarch64-unknown-linux-gnu}" ;;
  *)       TARGET="${TARGET:-}" ;;
esac
case "$TARGET" in
  x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu) ;;
  *) echo "error: unsupported Linux arch '$(uname -m)'" >&2; exit 1 ;;
esac

PREFIX="$REPO_ROOT/native/mpv/$TARGET"
BUILD_ROOT="${BUILD_ROOT:-$REPO_ROOT/.mpv-build}"
STAGE_DIR="$REPO_ROOT/apps/app/src-tauri/bundle-resources/linux"
JOBS="$(nproc 2>/dev/null || echo 4)"

log() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1 (see header comment for required packages)"; }
for t in curl tar meson ninja pkg-config gcc make patchelf; do need "$t"; done
command -v yasm >/dev/null || command -v nasm >/dev/null || die "missing yasm/nasm"

# Every produced library gets $ORIGIN rpath so the bundled set resolves
# internally no matter where the installer puts it.
ORIGIN_RPATH='-Wl,-rpath,$ORIGIN'
export LDFLAGS="${LDFLAGS:-} $ORIGIN_RPATH"
# meson may install .pc files under the Debian multiarch dir
MULTIARCH="$(dpkg-architecture -qDEB_HOST_MULTIARCH 2>/dev/null || true)"
export PKG_CONFIG_PATH="$PREFIX/lib/pkgconfig${MULTIARCH:+:$PREFIX/lib/$MULTIARCH/pkgconfig}:$PREFIX/share/pkgconfig:${PKG_CONFIG_PATH:-}"
# wayland-scanner (built from source below) must be on PATH for mpv's
# meson configure.
export PATH="$PREFIX/bin:$PATH"

fetch() { # url dest — retries until the archive passes a tar listing check
  local url="$1" dest="$2" attempt
  [ -f "$dest" ] && tar -tf "$dest" >/dev/null 2>&1 && return 0
  for attempt in 1 2 3; do
    log "Downloading $url (attempt $attempt)"
    rm -f "$dest"
    if curl -fL --retry 3 --retry-all-errors -o "$dest" "$url" && tar -tf "$dest" >/dev/null 2>&1; then
      return 0
    fi
    echo "download failed or archive corrupt, retrying..." >&2
    sleep 2
  done
  die "failed to download $url"
}

extract() { # tarball dir — atomic: a failed extract can't poison retries
  local tmp="$2.tmp-extract"
  rm -rf "$tmp" "$2"
  mkdir -p "$tmp"
  tar -xf "$1" -C "$tmp" --strip-components=1
  mv "$tmp" "$2"
}

meson_build() { # srcdir  — idempotent setup + compile + install
  local src="$1"
  if [ ! -f "$src/build/build.ninja" ]; then
    rm -rf "$src/build"
    meson setup "$src/build" "$src" "${MESON_ARGS[@]}"
  fi
  meson compile -C "$src/build" -j "$JOBS"
  meson install -C "$src/build"
}

# ---------------------------------------------------------------- FFmpeg ---
build_ffmpeg() {
  local ver="$FFMPEG_VER" src="$BUILD_ROOT/ffmpeg-$FFMPEG_VER"
  fetch "https://ffmpeg.org/releases/ffmpeg-$ver.tar.xz" "$BUILD_ROOT/ffmpeg-$ver.tar.xz"
  [ -d "$src" ] || { mkdir -p "$src"; extract "$BUILD_ROOT/ffmpeg-$ver.tar.xz" "$src"; }

  log "Configuring FFmpeg $ver (broad playback component set)"
  (
    cd "$src"
    ./configure \
      --prefix="$PREFIX" \
      --enable-shared --disable-static --enable-pic \
      --disable-doc --disable-programs --disable-debug \
      --disable-everything \
      --enable-avcodec --enable-avformat --enable-avutil \
      --enable-avfilter --enable-swresample --enable-swscale \
      --disable-avdevice \
      --enable-decoder='*' --enable-parser='*' --enable-demuxer='*' \
      --enable-muxer='*' --enable-protocol='*' --enable-filter='*' \
      --enable-bsf='*' \
      --disable-indev='*' --disable-outdev='*' \
      --enable-vaapi \
      --enable-encoder=png --enable-encoder=mjpeg \
      --enable-encoder=bmp --enable-encoder=ppm \
      --extra-cflags='-O2' \
      --extra-ldflags="$ORIGIN_RPATH"
    make -j "$JOBS"
    make install
  )
}

# --------------------------------------------------------------- libplacebo
build_libplacebo() {
  local ver="$LIBPLACEBO_VER" src="$BUILD_ROOT/libplacebo-$LIBPLACEBO_VER"
  # NOTE: tarball downloads lack git submodules — libplacebo's OpenGL
  # backend needs 3rdparty/glad, so clone with submodules instead.
  if [ ! -d "$src/.git" ]; then
    rm -rf "$src"
    git -c http.version=HTTP/1.1 clone --depth 1 --branch "$ver"       https://github.com/haasn/libplacebo "$src"
    # Submodules (glad, Vulkan-Headers, ...) are required; retry because
    # github occasionally resets HTTP/2 streams mid-clone.
    for _ in 1 2 3; do
      git -C "$src" -c http.version=HTTP/1.1 submodule update --init --recursive --depth 1 && break
      sleep 3
    done
  fi

  # Python >=3.14 compat: ElementTree() now requires an Element, not an
  # ElementTree — pass the root element explicitly.
  perl -pi -e 's/VkXML\(ET\.parse\(xmlfile\)\)/VkXML(ET.parse(xmlfile).getroot())/' \
    "$src/src/vulkan/utils_gen.py"

  log "Building libplacebo $ver"
  MESON_ARGS=(
    --prefix="$PREFIX" --buildtype=release -Ddefault_library=shared
    -Dvulkan=disabled -Dopengl=enabled
    -Dglslang=disabled -Dshaderc=disabled
    -Dlcms=disabled -Dxxhash=disabled -Dunwind=disabled
    -Ddemos=false -Dtests=false -Dbench=false -Dfuzz=false
  )
  meson_build "$src"
}

# ------------------------------------------------------------------ libass
build_libass() {
  local ver="$LIBASS_VER" src="$BUILD_ROOT/libass-$LIBASS_VER"
  fetch "https://github.com/libass/libass/archive/refs/tags/$ver.tar.gz" \
        "$BUILD_ROOT/libass-$ver.tar.gz"
  [ -d "$src" ] || { mkdir -p "$src"; extract "$BUILD_ROOT/libass-$ver.tar.gz" "$src"; }

  log "Building libass $ver"
  MESON_ARGS=(
    --prefix="$PREFIX" --buildtype=release -Ddefault_library=shared
  )
  meson_build "$src"
}

# ------------------------------------------------------ wayland + protocols
build_wayland() {
  local ver="$WAYLAND_VER" src="$BUILD_ROOT/wayland-$WAYLAND_VER"
  fetch "https://github.com/wayland-mirror/wayland/archive/refs/tags/$ver.tar.gz" \
        "$BUILD_ROOT/wayland-$ver.tar.gz"
  [ -d "$src" ] || { mkdir -p "$src"; extract "$BUILD_ROOT/wayland-$ver.tar.gz" "$src"; }

  log "Building wayland $ver (wayland-client/cursor/scanner)"
  MESON_ARGS=(
    --prefix="$PREFIX" --buildtype=release -Ddefault_library=shared
    -Dtests=false -Ddocumentation=false
  )
  meson_build "$src"
}

build_wayland_protocols() {
  local ver="$WAYLAND_PROTOCOLS_VER" src="$BUILD_ROOT/wayland-protocols-$WAYLAND_PROTOCOLS_VER"
  fetch "https://github.com/wayland-mirror/wayland-protocols/archive/refs/tags/$ver.tar.gz" \
        "$BUILD_ROOT/wayland-protocols-$ver.tar.gz"
  [ -d "$src" ] || { mkdir -p "$src"; extract "$BUILD_ROOT/wayland-protocols-$ver.tar.gz" "$src"; }

  log "Building wayland-protocols $ver"
  MESON_ARGS=(
    --prefix="$PREFIX" --buildtype=release -Ddefault_library=shared
  )
  meson_build "$src"
}

# --------------------------------------------------------------------- mpv
build_mpv() {
  local ver="$MPV_VER" src="$BUILD_ROOT/mpv-$MPV_VER"
  fetch "https://github.com/mpv-player/mpv/archive/refs/tags/$ver.tar.gz" \
        "$BUILD_ROOT/mpv-$ver.tar.gz"
  [ -d "$src" ] || { mkdir -p "$src"; extract "$BUILD_ROOT/mpv-$ver.tar.gz" "$src"; }

  log "Building mpv $ver (libmpv only)"
  MESON_ARGS=(
    --prefix="$PREFIX" --buildtype=release -Ddefault_library=shared
    -Dcplayer=false -Dlibmpv=true
    # scripting / filters we don't use
    -Dlua=disabled -Djavascript=disabled -Dcplugins=disabled
    -Dvapoursynth=disabled -Ddvdnav=disabled -Dlibbluray=disabled
    -Dcdda=disabled -Ddvbin=disabled -Drubberband=disabled
    -Dlibarchive=disabled -Dzimg=disabled -Dlcms2=disabled -Djpeg=disabled
    # keep uchardet: it powers subtitle charset detection
    -Duchardet=enabled
    # audio outputs: ALSA + PulseAudio cover virtually every Linux desktop.
    # PipeWire is deliberately disabled: it would add a hard NEEDED on
    # libpipewire.so.0 which older distros lack; pipewire-pulse provides
    # libpulse on PipeWire systems anyway.
    -Dalsa=enabled -Dpulse=enabled -Dpipewire=disabled
    -Djack=disabled -Dopenal=disabled -Doss-audio=disabled -Dsndio=disabled
    # video outputs / contexts
    -Dx11=enabled -Dwayland=enabled -Dgl=enabled -Dgbm=disabled
    -Degl=enabled -Degl-x11=enabled -Degl-wayland=enabled -Degl-drm=disabled
    -Ddrm=disabled -Ddmabuf-wayland=disabled -Dxv=disabled
    # hw decode: VA-API is the common denominator (Intel/AMD/NVIDIA-via-
    # nvidia-vaapi-driver). VDPAU is disabled: it would hard-require
    # libvdpau.so.1 which pure Intel/AMD systems do not ship.
    -Dvaapi=enabled -Dvaapi-x11=enabled -Dvaapi-wayland=enabled \
    -Dvdpau=disabled -Dvulkan=disabled
    -Dtests=false -Dfuzzers=false -Dbuild-date=false
  )
  meson_build "$src"
}

# ------------------------------------------------------------- stage bundle
stage_bundle() {
  log "Staging shared libraries into $STAGE_DIR"
  mkdir -p "$STAGE_DIR"
  # FFmpeg (autotools) installs into lib/, while meson projects land in
  # lib/<multiarch>/ on Debian/Ubuntu — collect .so files from both.
  # Strip real files first, then copy preserving symlinks
  # (libfoo.so -> libfoo.so.X).
  find "$PREFIX/lib" -type f -name '*.so*' \
    -exec strip --strip-unneeded {} + 2>/dev/null || true
  # shellcheck disable=SC2046
  cp -a $(find "$PREFIX/lib" -name '*.so*' | sort -u) "$STAGE_DIR"/

  # FFmpeg's configure does not propagate --extra-ldflags into shared
  # library links, and where it does the $ORIGIN token can get mangled
  # by configure's eval — staged .so files have been observed with a
  # corrupted "RIGIN:" RUNPATH. Force a correct rpath on EVERYTHING
  # staged, recursively: libs live split across lib/ (ffmpeg) and
  # lib/<multiarch>/ (mpv/ass/placebo/wayland), so each needs $ORIGIN
  # for siblings plus $ORIGIN/.. to reach the other dir. In the
  # flattened installer layout $ORIGIN alone suffices and the extra
  # entry is harmless.
  find "$STAGE_DIR" -type f -name '*.so*' \
    -exec patchelf --set-rpath '$ORIGIN:$ORIGIN/..' {} +
  # mpv client headers (useful for debugging / future bindgen)
  mkdir -p "$REPO_ROOT/native/mpv/$TARGET/include"
  cp -r "$PREFIX/include/mpv" "$REPO_ROOT/native/mpv/$TARGET/include/" 2>/dev/null || true

  echo
  log "Produced runtime ($TARGET):"
  du -sh "$STAGE_DIR" | awk '{print "  total size: " $1}'
  ls -lh "$STAGE_DIR"/*.so* 2>/dev/null | awk '{print "  " $5, $9}' | sort -u
}

main() {
  mkdir -p "$BUILD_ROOT" "$PREFIX"

  if [ "${1:-}" != "--skip-build" ]; then
    # FFmpeg is the slowest stage; SKIP_FFMPEG=1 reuses an installed prefix.
    [ "${SKIP_FFMPEG:-}" = "1" ] || build_ffmpeg
    build_libplacebo
    build_libass
    # wayland first: it provides wayland-scanner, which
    # wayland-protocols' meson build requires (>= the wayland version).
    build_wayland
    build_wayland_protocols
    build_mpv
  fi

  stage_bundle

}

main "$@"

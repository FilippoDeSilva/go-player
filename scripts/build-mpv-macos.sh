#!/usr/bin/env bash
# Build a minimal, self-contained libmpv runtime for macOS.
#
# Same rationale as build-mpv-linux.sh: instead of linking the brew mpv
# bottle (which drags in the entire brew ffmpeg dependency tree), we
# compile a scoped FFmpeg + libplacebo + libass + mpv from pinned sources
# and bundle the resulting dylibs inside the .app. The binary finds them
# via the @executable_path RUNPATH in .cargo/config.toml; bundled libs
# find each other via @loader_path.
#
# Output layout (consumed by tauri.macos.conf.json bundle.resources):
#   native/mpv/<target-triple>/lib/               produced dylibs
#   native/mpv/<target-triple>/include/mpv/       client headers
#   apps/app/src-tauri/bundle-resources/macos/    staged copies for bundling
#
# Requires: brew. Xcode CLT. Run on the oldest macOS you support — the
# resulting binaries carry that as their minimum.
#
# Usage: scripts/build-mpv-macos.sh [--skip-build]
set -euo pipefail

FFMPEG_VER="${FFMPEG_VER:-7.1.1}"
LIBPLACEBO_VER="${LIBPLACEBO_VER:-v7.349.0}"
LIBASS_VER="${LIBASS_VER:-0.17.4}"
MPV_VER="${MPV_VER:-v0.40.0}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Rust target triple from uname -m (x86_64 -> x86_64-apple-darwin,
# arm64 -> aarch64-apple-darwin).
case "$(uname -m)" in
  arm64)  TARGET="${TARGET:-aarch64-apple-darwin}" ;;
  x86_64) TARGET="${TARGET:-x86_64-apple-darwin}" ;;
  *)      TARGET="${TARGET:-}" ;;
esac
case "$TARGET" in
  aarch64-apple-darwin|x86_64-apple-darwin) ;;
  *) echo "error: unsupported macOS arch '$(uname -m)'" >&2; exit 1 ;;
esac

PREFIX="$REPO_ROOT/native/mpv/$TARGET"
BUILD_ROOT="${BUILD_ROOT:-$REPO_ROOT/.mpv-build}"
STAGE_DIR="$REPO_ROOT/apps/app/src-tauri/bundle-resources/macos"
JOBS="$(sysctl -n hw.ncpu 2>/dev/null || echo 4)"
BREW_PREFIX="$(brew --prefix 2>/dev/null || echo /opt/homebrew)"

log() { printf '\n\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "missing tool: $1"; }
for t in curl tar meson ninja pkg-config make; do need "$t"; done
[ "$(uname -s)" = "Darwin" ] || die "this script must run on macOS"

# Bundled dylibs must find each other: @loader_path = the directory
# containing the loading image (works for both the exe and sibling dylibs).
ORIGIN_RPATH='-Wl,-rpath,@loader_path'
export LDFLAGS="${LDFLAGS:-} $ORIGIN_RPATH"
export PKG_CONFIG_PATH="$BREW_PREFIX/lib/pkgconfig:${PKG_CONFIG_PATH:-}"

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

meson_build() { # srcdir
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

  log "Configuring FFmpeg $ver"
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
      --enable-videotoolbox \
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
    -Dlua=disabled -Djavascript=disabled -Dcplugins=disabled
    -Dvapoursynth=disabled -Ddvdnav=disabled -Dlibbluray=disabled
    -Dcdda=disabled -Ddvbin=disabled -Drubberband=disabled
    -Dlibarchive=disabled -Dzimg=disabled -Dlcms2=disabled -Djpeg=disabled
    -Duchardet=enabled
    # audio: CoreAudio is the only output we want on macOS
    -Dalsa=disabled -Dpulse=disabled -Dpipewire=disabled \
    -Djack=disabled -Dopenal=disabled -Doss-audio=disabled -Dsndio=disabled \
    -Dcoreaudio=enabled
    # video: libmpv embeds via the render API / plain-gl; no X11/Wayland.
    # VideoToolbox gives hardware decode; plain-gl + libplacebo's OpenGL
    # backend is the gpu-next path used by macOS libmpv clients.
    -Dx11=disabled -Dwayland=disabled -Dgbm=disabled -Ddrm=disabled \
    -Ddmabuf-wayland=disabled -Dxv=disabled -Dcocoa=disabled \
    -Dvaapi=disabled -Dvdpau=disabled -Dvulkan=disabled \
    -Dplain-gl=enabled -Dgl-cocoa=disabled \
    -Dvideotoolbox-pl=enabled -Dvideotoolbox-gl=enabled \
    -Dtests=false -Dfuzzers=false -Dbuild-date=false
  )
  meson_build "$src"
}

# ------------------------------------------------------------- stage bundle
stage_bundle() {
  log "Staging dylibs into $STAGE_DIR"
  mkdir -p "$STAGE_DIR"
  find "$PREFIX/lib" -maxdepth 1 -type f \( -name '*.dylib' -o -name '*.so' \) \
    -exec strip -x {} + 2>/dev/null || true
  # shellcheck disable=SC2086
  cp -a "$PREFIX"/lib/*.dylib "$STAGE_DIR"/ 2>/dev/null || true
  mkdir -p "$REPO_ROOT/native/mpv/$TARGET/include"
  cp -r "$PREFIX/include/mpv" "$REPO_ROOT/native/mpv/$TARGET/include/" 2>/dev/null || true

  echo
  log "Produced runtime ($TARGET):"
  du -sh "$STAGE_DIR" | awk '{print "  total size: " $1}'
  ls -lh "$STAGE_DIR"/*.dylib 2>/dev/null | awk '{print "  " $5, $9}' | sort -u
}

main() {
  mkdir -p "$BUILD_ROOT" "$PREFIX"

  if [ "${1:-}" != "--skip-build" ]; then
    brew install meson ninja nasm pkg-config \
      freetype fontconfig fribidi harfbuzz uchardet
    [ "${SKIP_FFMPEG:-}" = "1" ] || build_ffmpeg
    build_libplacebo
    build_libass
    build_mpv
  fi

  stage_bundle

}

main "$@"

#!/usr/bin/env bash
# Provision libmpv for go-media-mpv on Linux/macOS.
#
# Unlike Windows (where we download an official libmpv zip and generate an
# import library), Linux and macOS use the system package manager:
#   - Linux:  libmpv-dev  (apt)  — provides libmpv.so for linking + runtime
#   - macOS:  mpv          (brew) — provides libmpv.dylib
#
# Idempotent: safe to re-run. Requires sudo on Linux.
set -euo pipefail

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    if ldconfig -p 2>/dev/null | grep -q 'libmpv\.so'; then
      echo "libmpv already installed."
    else
      echo "Installing libmpv-dev via apt..."
      sudo apt-get update
      sudo apt-get install -y libmpv-dev
    fi
    ;;
  Darwin)
    if brew list --versions mpv >/dev/null 2>&1; then
      echo "mpv already installed via brew."
    else
      echo "Installing mpv via brew (provides libmpv.dylib)..."
      brew install mpv
    fi
    ;;
  *)
    echo "Unsupported OS: $OS" >&2
    exit 1
    ;;
esac

echo "libmpv provisioning complete ($OS/$ARCH)."
echo "Note: release bundling of libmpv + its dependency tree on this OS is"
echo "tracked as follow-up work; builds link against the system library."

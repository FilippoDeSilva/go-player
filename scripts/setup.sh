#!/usr/bin/env bash
# Idempotent dev environment setup for Go Player (Linux/macOS).
# Windows users: run scripts/setup.ps1 instead (it also handles MSVC Build Tools).
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "== Go Player setup =="

# --- Rust ------------------------------------------------------------------
if ! command -v rustc >/dev/null 2>&1; then
    echo "[rust] not found — installing rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    # shellcheck disable=SC1091
    source "$HOME/.cargo/env"
fi
rustup default stable >/dev/null
rustup target add "$(rustc -vV | awk '/^host:/{print $2}')" >/dev/null 2>&1 || true
echo "[rust] $(rustc --version)"

# --- Node.js ----------------------------------------------------------------
if ! command -v node >/dev/null 2>&1; then
    echo "[node] not found — install Node.js 22+ LTS manually, then re-run."
    exit 1
fi
echo "[node] $(node --version)"

# --- pnpm -------------------------------------------------------------------
if ! command -v pnpm >/dev/null 2>&1; then
    npm install -g pnpm
fi
echo "[pnpm] $(pnpm --version)"

# --- System libs for Tauri (Linux only) -------------------------------------
if [[ "$(uname -s)" == "Linux" ]]; then
    if command -v apt-get >/dev/null 2>&1; then
        sudo apt-get update
        sudo apt-get install -y \
            libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
            librsvg2-dev patchelf
    elif command -v dnf >/dev/null 2>&1; then
        sudo dnf install -y \
            webkit2gtk4.1-devel gtk3-devel libappindicator-gtk3-devel \
            librsvg2-devel patchelf
    fi
fi

# --- Workspace dependencies ---------------------------------------------------
cd "$REPO_ROOT"
pnpm install

echo ""
echo "Setup complete. Run:"
echo "    pnpm dev        # tauri dev (desktop app)"
echo "    pnpm build      # production frontend build"

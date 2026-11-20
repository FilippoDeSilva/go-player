# Keep a Changelog — https://keepachangelog.com

# Version headings are generated from git tags; do not edit released sections.

## [0.1.0] - 2026-11-20

### Added

- **Desktop shell:** Tauri 2 with Solid + TypeScript + Tailwind CSS v4
- **Cross-platform playback:** libmpv backend with native video rendering
  - Windows: HWND embedding (no flicker)
  - Linux: GTK GLArea + OpenGL render API (X11/Wayland agnostic)
  - macOS: audio-only (native video surface pending)
- **Media playback:**
  - Video (H.264, VP9, AV1, etc.) with hardware decode fallback
  - Audio (MP3, FLAC, WAV, OGG, M4A, AAC, WMA, Opus, AIFF)
  - Subtitle support (SRT, VTT, ASS/SSA)
  - Cover art extraction from tags
  - Playback controls: play/pause, seek, volume, mute, fullscreen
- **UI/UX:**
  - Auto-hiding controls (3s idle)
  - Keyboard shortcuts (Space, ←/→, ↑/↓, M, F, Ctrl+O)
  - Dark-first theme (Midnight/OLED/Light)
  - Audio mode with cover art display
- **Linux packaging:**
  - Bundled libmpv + FFmpeg + libass + libplacebo
  - Debian package (.deb) with proper RPATH
  - AppImage support (experimental)
- **Infrastructure:**
  - GitHub Actions CI/CD for Windows/Linux/macOS
  - Pre-commit hooks (rustfmt, prettier)
  - Pre-push hooks (typecheck, build, test)
- **Crates:**
  - `go-media`: PlaybackEngine trait + media domain types
  - `go-media-mpv`: libmpv implementation
  - `go-subtitles`: subtitle parsing (SRT, VTT)
  - `go-tags`: metadata + cover art extraction
- **TypeScript contracts:** `@go-player/contracts` for typed IPC
- **ADR:** `docs/decisions/0001-playback-engine.md` (libmpv decision)

### Fixed

- **Linux:** Bundled wayland libraries caused WebKitGTK EGL failures → removed from bundle
- **Linux:** mkv/avi/asf/mpeg container detection
- **Linux:** mpv locale initialization (`LC_ALL=C`)
- **Linux:** GTK GLArea render deadlock (async open_media)
- **Linux:** libmpv RUNPATH for transitive dependencies
- **Cross-platform:** Media open now stops previous playback (no simultaneous audio+video)

### Known Issues

- **Linux VM (VMware svga):** WebKitGTK GPU compositing fails → falls back to software compositing; bars may flicker on weak GPUs
- **macOS:** Video surface not yet implemented (audio-only)
- **Linux AppImage:** `linuxdeploy` bundling fails on some systems; use .deb instead

### Platform Support

| Platform | Status        | Notes                                          |
| -------- | ------------- | ---------------------------------------------- |
| Windows  | ✅ Working    | Video + audio, native HWND embedding           |
| Linux    | ✅ Working    | Video + audio, GTK GLArea; .deb package tested |
| macOS    | ⚠️ Audio-only | Video surface pending; builds succeed          |

## [Unreleased]

### Added

- (Future features go here)

# ADR-0001: Playback engine — libmpv over GStreamer

- **Status:** Accepted
- **Date:** 2026-11-17

## Context

Go Player needs a playback engine that runs on Windows, Linux, and macOS
desktop first, with Android/iOS as a later target. AGENTS.md listed
GStreamer (`gstreamer-rs`) as the *candidate*, not a decided engine.

Candidate engines evaluated:

- **GStreamer** — powerful pipeline framework, fine-grained control.
- **libmpv** — the engine inside mpv; ffmpeg + libass + hardware decode
  and hardware-accelerated rendering (D3D11/Vulkan/Metal/OpenGL) in one
  package.
- **FFmpeg direct (`av`)** — maximum control but rebuilds demux/decode/
  sync/render that mpv and GStreamer already solve.
- **Platform-native (MediaFoundation / AVFoundation / ExoPlayer)** —
  fragmented; rejected for a shared-core architecture.

## Decision

Use **libmpv** as the v1 playback engine, behind the `go-media`
`PlaybackEngine` trait so the engine remains replaceable.

## Rationale

- **Playback quality:** mpv is the reference "plays everything" engine —
  container/codec/subtitle coverage out of the box, mature A/V sync and
  seeking semantics.
- **Size:** one shared library vs. a GStreamer plugin set that must be
  trimmed and shipped per-platform (hundreds of MB unpruned on Windows).
- **Dev speed:** fewer pipeline/threading bugs; GStreamer's GLib
  main-context and bus/thread-affinity model is a known source of
  subtle defects in Rust bindings.
- **Licensing:** LGPL-2.1+ is compatible with dynamic linking from a
  proprietary or open-source app; we will link dynamically, not
  statically embed.
- **Mobile path:** libmpv has an Android port (`libmpv-android`); iOS
  is possible via libmpv builds. GStreamer's mobile packaging is
  heavier. Neither is validated yet — mobile remains a later spike.

## Consequences

- `crates/go-media` owns the `PlaybackEngine` trait and domain types;
  no libmpv types leak into it.
- A future `crates/go-media-mpv` implements the trait (not yet written).
- Video rendering will use mpv's render API (OpenGL/Vulkan/Metal/D3D11)
  into a native surface integrated with the Tauri window — the
  webview-overlay approach from AGENTS.md §4.5 still applies.
- If libmpv fails a concrete requirement (e.g., a codec, a render
  integration, or mobile packaging), revisit this ADR — the trait
  boundary exists precisely to allow that.

## Alternatives rejected

- **GStreamer:** retained as fallback if libmpv hits a wall; not
  implemented in v1.
- **FFmpeg-only:** too much undifferentiated work for a player.

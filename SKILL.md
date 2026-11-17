---
name: go-player-engineering
description: Use when designing, implementing, reviewing, or debugging Go Player, a cross-platform desktop/mobile media player with local and cloud AI subtitle generation and translation.
---

# Go Player Engineering Skill

## Purpose

Act as a senior cross-platform media, Rust, and application engineer for Go Player. Help the project owner make sound architectural decisions and deliver production-quality features without compromising playback performance, offline capability, privacy, or maintainability.

Read the repository's `AGENTS.md` first and follow it. This skill supplements that file; it does not override it.

## Product context

Go Player is a standalone media player targeting desktop and mobile. It must provide excellent playback independently of AI features.

Its differentiator is integrated subtitle intelligence:

- Detect and load embedded/external subtitles.
- Generate subtitles from speech locally or through an explicitly selected cloud provider.
- Translate subtitle documents locally or through cloud providers.
- Support offline use after required models are installed.
- Edit, synchronize, display, and export subtitles.
- Support original and translated subtitle tracks simultaneously.

The intended architecture is a shared Rust core, Solid + TypeScript UI, and Tauri 2 as the application shell. libmpv is the playback engine (decided in `docs/decisions/0001-playback-engine.md`); FFmpeg may be used selectively. Whisper.cpp is the initial speech recognition candidate. Translation is pluggable and must be selected after benchmarking.

These are design directions. Inspect the repository and verify what is actually implemented before making claims.

## Operating procedure

For every task:

1. **Understand:** restate the concrete behavior or problem internally; inspect relevant code and docs.
2. **Locate boundaries:** identify whether the work belongs to UI, application orchestration, media, subtitle domain, AI, storage, model management, or a platform adapter.
3. **Check contracts:** reuse existing interfaces and domain types. Do not bypass abstractions for convenience.
4. **Plan:** for non-trivial work, outline files/modules to change, risks, and verification.
5. **Implement:** make the smallest complete, maintainable change.
6. **Verify:** run focused tests, formatting, linting, and relevant platform builds.
7. **Report:** summarize changes, actual commands/results, limitations, and follow-up work.

Do not invent tool output, build results, codec support, model benchmarks, or platform compatibility.

## Architectural boundaries

### Application core

Owns use cases, orchestration, job lifecycle, app state, and coordination between engines. It should not contain playback-engine-specific or Whisper-specific implementation details.

### Media subsystem

Owns opening media, track discovery/selection, playback, seeking, timing, audio/video synchronization, and audio extraction.

Expose a stable abstraction through `go-media`. Keep libmpv handles, render API contexts, and thread-affinity details inside the libmpv adapter (`go-media-mpv`).

Never route decoded video frames through the webview or high-frequency Tauri IPC. Use a native rendering surface or an appropriate platform texture/surface integration.

### Subtitle subsystem

Owns a canonical subtitle document and operations such as parsing, validation, timing adjustment, merging, editing, and export.

Support formats through adapters, initially considering SRT, WebVTT, and ASS/SSA. Do not make a file format the canonical internal representation.

Preserve source documents when creating translated documents. Keep timestamps stable unless the user explicitly requests synchronization or timing edits.

### AI subsystem

Owns provider selection, inference orchestration, progress, cancellation, resource checks, and normalized results.

Keep speech recognition and translation as separate capabilities. Whisper's speech-to-English translation behavior must not be mistaken for arbitrary multilingual subtitle translation.

Local and cloud implementations must satisfy the same domain-level contracts. Cloud processing must be explicit and disclose what data leaves the device.

### Model manager

Owns catalog metadata, download, resume, checksum/signature verification, installation, compatibility checks, versioning, and deletion.

Model files are optional data assets, not application binaries. Never execute downloaded model files as code. Validate metadata and formats, use trusted sources, and account for licenses.

### Storage subsystem

Owns SQLite persistence and migrations. Store large media and model files outside SQLite; store metadata and references in the database. Use platform-appropriate application data directories.

### Platform adapters

Own OS-specific file access, native video surfaces, hardware decoding, secure credentials, background execution, permissions, and lifecycle integration. Do not pretend all platforms have identical capabilities.

## Media engineering guidance

When implementing or debugging playback:

- Identify the container, video codec, audio codec, subtitle format, bit depth, resolution, frame rate, and relevant stream metadata.
- Distinguish demuxing, decoding, rendering, and audio output failures.
- Check actual selected decoder and hardware path; do not infer acceleration from device capability alone.
- Preserve A/V synchronization, seeking accuracy, track selection, and playback responsiveness.
- Handle missing plugins, malformed files, unsupported profiles, and resource exhaustion gracefully.
- Test on each target OS and maintain a compatibility matrix.
- Keep long-running discovery and conversion off real-time playback threads.
- Use FFmpeg only where it provides a justified capability; avoid duplicating full media stacks without a clear reason.

For libmpv work, respect the mpv handle's threading model (most calls are thread-safe but render/update callbacks must be marshalled correctly), the render API's GPU context requirements (OpenGL/Vulkan/Metal/D3D11), and platform-specific surface integration constraints. Do not move render contexts across threads casually.

## AI and subtitle workflow guidance

A typical local transcription flow:

1. Validate the media source and selected audio track.
2. Extract or decode audio without disrupting playback.
3. Resample to the inference model's required format (often 16 kHz mono PCM for Whisper workflows; verify the selected model/runtime).
4. Run inference in a background job with cancellation and progress.
5. Normalize segments into the canonical subtitle domain.
6. Validate timestamps and text.
7. Persist the result and attach it as a subtitle track.
8. Offer editing and export.

Do not assume every speech model produces accurate word-level timestamps. Record the granularity actually available.

For translation:

1. Preserve the original subtitle document.
2. Translate text while retaining segment identity and timing by default.
3. Handle language direction, unsupported languages, provider limits, and partial failures.
4. Avoid splitting or merging segments unless explicitly required by the translation algorithm and documented.
5. Save translation as a distinct document/track with source-language and target-language metadata.

For generated subtitles, consider confidence, punctuation, speaker labels, language detection, and custom vocabulary only when supported by the chosen model. Do not fabricate confidence values or speaker information.

## Performance requirements

- Playback responsiveness takes priority over background AI throughput.
- Do not block UI, media, or audio threads with inference, disk I/O, database queries, or network requests.
- Bound concurrency and memory use.
- Support cancellation and cleanup of temporary audio.
- Use model quantization and hardware acceleration only after verifying quality and compatibility.
- Avoid copying large audio buffers unnecessarily.
- Throttle frontend progress events to a useful cadence.
- Benchmark cold start, model load, real-time factor, memory, CPU/GPU usage, battery and thermal behavior.
- Provide safe CPU fallback where feasible.

## Frontend and interaction guidance

The interface should be elegant, modern, and focused on the video. It should not resemble a generic admin dashboard.

Prioritize:
- Clear playback controls and track selection.
- Responsive desktop and touch layouts.
- Keyboard shortcuts on desktop and gestures on mobile.
- Accessible focus, contrast, labels, and hit targets.
- Clear states for model missing, downloading, ready, generating, cancelling, completed, and failed.
- Honest offline/online indicators.
- Subtitle customization and a usable editing workspace.
- Minimal visual clutter during playback.

Use typed frontend contracts. Validate data received from native commands and events. Clean up event listeners and avoid IPC polling loops.

## Security and privacy checklist

Before shipping a feature, ask:

- Does it read or write files outside the user's selected scope?
- Can a path or archive entry cause traversal?
- Does it invoke a shell or native command with user-controlled input?
- Does media or subtitle data leave the device?
- Are API credentials stored securely?
- Are model downloads verified and obtained from trusted sources?
- Are model formats parsed safely?
- Are Tauri capabilities and plugin permissions least-privilege?
- Are logs free of private media paths, subtitle contents, and secrets?
- Are licenses and attribution recorded?

Never add cloud uploads, analytics, telemetry, or crash reporting without explicit product approval.

## Testing strategy

Use layered tests:

- Unit tests for subtitle parsing, timestamp arithmetic, domain validation, language mapping, and job state transitions.
- Contract tests for media and AI provider interfaces.
- Integration tests using small redistributable media fixtures with known metadata and expected outputs.
- Regression tests for playback state, seeking, track switching, and subtitle synchronization.
- AI evaluation sets for representative languages and noisy/clean audio.
- Platform tests for file permissions, native surfaces, model storage, background behavior, and hardware paths.
- Resource tests for low-memory devices, cancellation, corrupted downloads, and disk exhaustion.

Do not commit copyrighted movies, commercial audio, or large model weights as test fixtures. Use generated, public-domain, or properly licensed samples.

## Decision-making rules

When requirements are ambiguous:

- Do not invent product decisions.
- State the assumption and choose the least irreversible option.
- Prefer a technical spike for high-risk unknowns, especially native rendering, libmpv mobile packaging, hardware acceleration, and model performance.
- Record significant decisions in an ADR with context, options, decision, consequences, and status.
- Avoid adding a dependency until its platform support, license, maintenance status, and binary-size impact have been checked.

When proposing a library or model, report:
- What it solves.
- Supported platforms and architectures.
- License and distribution implications.
- Binary/model size implications.
- Performance and hardware requirements.
- Known limitations.
- How it fits the existing abstraction.

## Expected response format for engineering tasks

For substantial tasks, provide:

1. **Understanding** — what needs to change.
2. **Design** — boundaries and key trade-offs.
3. **Implementation** — files and behavior changed.
4. **Verification** — exact tests/builds run and their results.
5. **Limitations** — unverified platforms, missing tests, or known issues.

For small, straightforward changes, be concise while still reporting verification.

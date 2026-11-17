# AGENTS.md — Go Player

This file defines the engineering principles, project boundaries, and working conventions for AI coding agents contributing to Go Player.

## 1. Product vision

Go Player is a modern, lightweight, cross-platform media player for desktop and mobile.

Its defining capability is an integrated subtitle intelligence system:

- Discover and load embedded and external subtitles.
- Generate subtitles from speech using on-device AI.
- Translate subtitles using local or optional cloud services.
- Support offline workflows after models have been downloaded.
- Edit, synchronize, and export subtitle tracks.

Go Player must be an excellent media player even when AI features are unavailable, disabled, or not installed.

**Product principle:** playback comes first; AI subtitles are a first-class capability, not a replacement for a reliable player.

## 2. Current architectural direction

Treat these as the current intended direction, not proof that every integration has already been validated:

- **Application shell:** Tauri 2 is the preferred cross-platform shell candidate.
- **Frontend:** Solid + TypeScript (pnpm workspace).
- **Styling:** Tailwind CSS v4 with a reusable, custom design system.
- **Shared application/core logic:** Rust.
- **Media playback:** libmpv via Rust bindings (decided in `docs/decisions/0001-playback-engine.md`); GStreamer evaluated and rejected for v1, retained only as a fallback.
- **Media utilities:** FFmpeg may be used selectively for inspection, extraction, conversion, or compatibility gaps.
- **Speech recognition:** whisper.cpp through a suitable Rust integration.
- **Translation:** pluggable local and cloud backends; do not lock the architecture to a model before benchmarking.
- **Persistence:** SQLite, accessed through SQLx or another explicitly approved Rust data layer.
- **Async work:** Tokio for application/background tasks. Do not use an async runtime as a substitute for a real-time media clock.
- **Serialization/contracts:** Serde and explicit typed command/event contracts.

MōBrowser was considered for desktop but is not the selected universal shell. Do not introduce MōBrowser alongside Tauri without an explicit architecture decision.

## 3. Platform scope

The product target is desktop and mobile:

- Windows
- Linux
- macOS
- Android
- iOS

The precise release order is not yet locked. Do not silently assume a platform can be deferred or that all platforms have identical capabilities.

Prefer shared Rust domain logic and shared Solid UI where appropriate. Use native platform adapters when required for:

- Video surfaces and rendering
- Hardware decoding
- File access and permissions
- Background execution
- Model storage and downloads
- Audio routing and system integration

A shared API does not imply identical implementation or feature parity. Document platform limitations explicitly.

## 4. Non-negotiable architecture principles

1. **Separate playback, subtitles, AI, storage, and UI.**
2. **Keep the media engine behind a stable Rust abstraction.** Playback-engine-specific types (libmpv handles, render contexts) must not leak throughout the application.
3. **Keep AI providers behind interfaces.** Local Whisper and cloud providers must produce the same normalized subtitle-domain output.
4. **Keep the subtitle domain independent of file formats.** SRT, WebVTT, ASS/SSA, and embedded tracks are import/export or adapter concerns.
5. **Do not move decoded video frames through JavaScript or high-frequency IPC.** Render through an appropriate native surface/texture path.
6. **Do not block the UI thread or media playback thread with transcription, translation, downloads, database work, or expensive parsing.**
7. **Treat long-running work as cancellable jobs** with progress, failure states, and clear lifecycle ownership.
8. **Keep AI models optional.** Model files must not be mandatory installer payloads.
9. **Offline-first capability must be real.** Once a compatible model is installed, local transcription and supported local translation must work without a network connection.
10. **Cloud processing must be explicit.** Never upload user media or subtitle content to a remote provider without a clear user action and disclosure.
11. **Prefer small, focused crates and modules.** Avoid a monolithic Rust backend or frontend feature modules with unrelated responsibilities.
12. **Do not invent support claims.** Codec, hardware acceleration, model-language, and platform support must be tested and documented.

## 5. Suggested repository organization

Adapt this structure to the actual repository rather than creating every directory immediately:

```text
go-player/
├── apps/
│   └── app/
│       ├── src/                  # Solid application
│       └── src-tauri/            # Tauri host and command registration
├── crates/
│   ├── go-core/                  # Application orchestration
│   ├── go-types/                 # Shared domain types
│   ├── go-media/                 # Playback abstractions
│   ├── go-media-mpv/             # libmpv implementation
│   ├── go-subtitles/             # Subtitle domain, parsing and export
│   ├── go-ai/                    # AI orchestration and provider contracts
│   ├── go-asr-whisper/           # Whisper implementation
│   ├── go-translation/           # Translation contracts/orchestration
│   ├── go-cloud-ai/              # Cloud provider adapters
│   ├── go-model-manager/         # Model catalog, download and lifecycle
│   └── go-storage/               # Persistence
├── native/
│   ├── android/
│   ├── ios/
│   └── desktop/
├── packages/
│   ├── ui/
│   ├── contracts/
│   └── design-system/
├── assets/
├── scripts/
├── tests/
│   ├── fixtures/
│   ├── media/
│   └── integration/
├── docs/
│   ├── architecture/
│   ├── decisions/
│   ├── licensing.md
│   ├── media-support.md
│   └── model-management.md
├── Cargo.toml
├── pnpm-workspace.yaml
└── AGENTS.md
```

Do not create empty crates just to match this diagram. Introduce a crate when there is a real boundary, implementation, or test need.

## 6. Engineering workflow for agents

Before changing code:

1. Inspect the existing repository, package manifests, workspace configuration, and relevant documentation.
2. Identify the current implementation and platform scope. Do not assume the proposed architecture has already been implemented.
3. Find the narrowest module responsible for the requested behavior.
4. Check for existing abstractions and conventions before adding new dependencies or patterns.
5. For significant architectural changes, write or update a short ADR in `docs/decisions/`.

When implementing:

- Make the smallest coherent change that solves the task.
- Preserve existing public APIs unless a change is necessary and documented.
- Use typed errors; avoid swallowing errors or returning success-shaped defaults.
- Validate all external input, file paths, model metadata, and provider responses.
- Add tests for new behavior and regression tests for bugs.
- Keep user-facing strings accessible for future localization.
- Keep secrets out of source control, logs, crash reports, and frontend bundles.
- Do not add telemetry, analytics, crash upload, or cloud processing without explicit product approval.
- Avoid unnecessary dependencies. Explain why a new dependency is needed.
- Do not run destructive commands or overwrite user work without checking the repository state.

After implementing:

1. Run formatting and linting relevant to the changed code.
2. Run focused unit/integration tests.
3. Run the relevant platform build or clearly state why it could not be run.
4. Report what changed, what was tested, what failed, and any remaining limitations.
5. Never claim a build or test passed unless it actually ran successfully.

## 7. Rust conventions

- Prefer explicit types and clear ownership boundaries.
- Use `Result` and domain-specific error types for fallible operations.
- Avoid `.unwrap()` and `.expect()` in production paths. They may be acceptable in tightly controlled tests or invariant-checked initialization with justification.
- Do not hold synchronous mutex guards across `.await`.
- Keep blocking inference, media discovery, filesystem work, and database operations off latency-sensitive threads.
- Define cancellation behavior for long-running tasks.
- Avoid unnecessary `Arc<Mutex<T>>`; choose channels, actors, or ownership where more appropriate.
- Keep playback-engine objects (mpv handles, GL/Vulkan/D3D render contexts) and their thread-affinity requirements inside the media adapter.
- Do not expose playback-engine or Whisper implementation types in public domain interfaces.
- Document safety invariants for every `unsafe` block.

## 8. Frontend conventions

- Use TypeScript strictly. Avoid `any`; use `unknown` and validate at boundaries.
- Keep UI components focused and reusable.
- Separate view components from application services and native command/event adapters.
- Model async UI states explicitly: idle, loading, running, cancelling, succeeded, failed.
- Unsubscribe from native events when components or services are disposed.
- Avoid excessive IPC polling. Use events for state changes and throttle playback-position updates.
- Do not put media decoding or AI inference in the WebView.
- Design for keyboard, pointer, touch, accessibility, and different window/screen sizes.

## 9. Media engineering rules

- Distinguish containers from codecs. An MKV or MP4 extension does not guarantee a particular codec.
- Maintain a tested media compatibility matrix by OS, container, video codec, audio codec, subtitle format, and acceleration path.
- Handle missing codecs/plugins and unsupported streams with actionable errors.
- Preserve audio/video synchronization and correct seeking semantics.
- Account for variable frame rates, multiple audio tracks, embedded subtitles, chapters, and malformed metadata.
- Do not claim hardware acceleration merely because a decoder exists; verify the actual selected decoding and rendering path.
- Keep media playback responsive while background tasks run.
- Test files with different resolutions, frame rates, bit depths, HDR characteristics, and audio layouts.

## 10. AI and model management rules

- Keep model metadata explicit: identifier, version, source, license, checksum, expected size, supported languages, runtime requirements, and format.
- Verify downloaded model artifacts with a trusted checksum or signature before installation.
- Support resumable downloads where practical, and clean up partial files safely.
- Never execute downloaded model artifacts as native code.
- Treat model files and model metadata as untrusted input.
- Make model selection and storage usage understandable to users.
- Detect resource constraints and provide a safe fallback rather than exhausting memory.
- Support cancellation and progress reporting.
- Keep local and cloud provider outputs normalized.
- Preserve source text and source subtitle documents when translating.
- Translation must not silently alter source timestamps.
- Clearly distinguish transcription, translation, subtitle correction, and synchronization.
- Do not promise language quality without evaluation data.
- Benchmark representative languages and hardware before selecting defaults.

## 11. Privacy and security

- Local processing is the default for installed local models.
- Cloud processing requires an explicit user choice.
- Explain what data is sent, to which provider, and for what purpose.
- Store provider credentials using an OS-appropriate secure secret store; never in SQLite plaintext or frontend local storage.
- Use least-privilege Tauri capabilities and narrowly scoped filesystem access.
- Validate file paths and prevent path traversal.
- Avoid shelling out with interpolated user-controlled strings.
- Keep dependencies and native plugins updated.
- Maintain a licensing and attribution register for code, plugins, codecs, and AI models.

## 12. Design principles

Go Player should feel modern, elegant, fast, and focused.

- Dark-first visual direction is preferred, but support system appearance where practical.
- The video remains the visual focus.
- Controls should be discoverable without permanently cluttering playback.
- Use restrained color, consistent spacing, typography, and motion.
- Avoid generic dashboard layouts inside the player.
- Make subtitle generation and translation understandable without exposing unnecessary ML terminology.
- Ensure the UI remains useful when offline, when no model is installed, or when a cloud provider is unavailable.

## 13. Decisions that are not yet final

Do not silently make these decisions on behalf of the project owner:

- Open-source vs source-available vs proprietary licensing.
- Exact release order for desktop and mobile.
- Whether Android is first or Android/iOS are developed together.
- Final UI component foundation.
- Final playback engine per platform (libmpv chosen for desktop; mobile path unvalidated).
- Translation model/runtime and default languages.
- Cloud AI providers and pricing model.
- Minimum supported OS versions and hardware requirements.
- Whether FFmpeg is bundled, used as a development utility, or omitted from some builds.

When one of these decisions blocks implementation, explain the trade-off and propose a small technical spike or ADR.

## 14. Definition of done

A change is not complete merely because it compiles.

A feature is done when:

- It satisfies the requested behavior.
- Its errors and edge cases are handled.
- Relevant tests are added and pass.
- It does not regress playback responsiveness or platform behavior.
- Privacy, licensing, and resource implications are considered.
- Documentation is updated where necessary.
- The agent reports actual verification and known limitations.

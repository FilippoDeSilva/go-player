import { createSignal, onCleanup, onMount, Show } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Play,
  Pause,
  FolderOpen,
  ChevronDown,
  Rewind,
  FastForward,
  Square,
  Minus,
  Maximize2,
  Minimize2,
  X,
  Volume2,
  VolumeX,
  Shuffle,
  Repeat,
  Maximize,
  MoreHorizontal,
  Music,
  Subtitles,
} from "lucide-solid";
import type {
  MediaBackendInfo,
  MediaInfoDto,
  MediaMetadata,
  PlaybackState,
  SubtitleSummary,
} from "@go-player/contracts";
import "./App.css";

const MEDIA_EXTS = [
  // video
  "mp4",
  "mkv",
  "webm",
  "avi",
  "mov",
  "m4v",
  "ts",
  "m2ts",
  "wmv",
  "flv",
  // audio
  "mp3",
  "flac",
  "wav",
  "ogg",
  "oga",
  "m4a",
  "aac",
  "opus",
  "wma",
  "aiff",
];
const SUBTITLE_EXTS = ["srt", "vtt", "ass", "ssa"];
const AUDIO_EXTS = MEDIA_EXTS.slice(10);

document.documentElement.dataset.theme = "midnight";

function fmtTime(ms: number | null | undefined): string {
  if (ms == null) return "--:--";
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const h = Math.floor(m / 60);
  return h > 0
    ? `${h}:${String(m % 60).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`
    : `${m}:${String(s % 60).padStart(2, "0")}`;
}

const fileName = (p: string) => p.split(/[\\/]/).pop() ?? p;
const extOf = (p: string) => p.split(".").pop()?.toLowerCase() ?? "";
const isAudio = (p: string) => AUDIO_EXTS.includes(extOf(p));

/* ---------------- app ---------------- */
function App() {
  const [backend, setBackend] = createSignal<MediaBackendInfo | null>(null);
  const [media, setMedia] = createSignal<MediaInfoDto | null>(null);
  const [mediaKind, setMediaKind] = createSignal<"video" | "audio">("video");
  const [pb, setPb] = createSignal<PlaybackState | null>(null);
  const [subtitle, setSubtitle] = createSignal<SubtitleSummary | null>(null);
  const [subtitlePath, setSubtitlePath] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [controlsVisible, setControlsVisible] = createSignal(true);
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [volume, setVolume] = createSignal(100);
  const [muted, setMuted] = createSignal(false);

  let hideTimer: ReturnType<typeof setTimeout> | undefined;
  const pokeControls = () => {
    setControlsVisible(true);
    clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (pb()?.status === "playing") setControlsVisible(false);
    }, 3000);
  };

  onMount(() => {
    void invoke<MediaBackendInfo>("media_backend").then(setBackend);
    const t = setInterval(() => {
      if (!media()) return;
      void invoke<PlaybackState>("media_state")
        .then(setPb)
        .catch(() => {});
    }, 250);
    onCleanup(() => clearInterval(t));
  });

  const pickFiles = async (kind: "media" | "subs") => {
    setMenuOpen(false);
    setError(null);
    const selected = await open({
      multiple: false,
      filters:
        kind === "media"
          ? [
              { name: "Media", extensions: MEDIA_EXTS },
              { name: "All files", extensions: ["*"] },
            ]
          : [
              { name: "Subtitles", extensions: SUBTITLE_EXTS },
              { name: "All files", extensions: ["*"] },
            ],
    });
    if (!selected || Array.isArray(selected)) return;
    const path = selected as string;
    const ext = extOf(path);
    if (kind === "subs" || SUBTITLE_EXTS.includes(ext)) {
      setSubtitlePath(path);
      try {
        setSubtitle(await invoke<SubtitleSummary>("load_subtitles", { path }));
      } catch (e) {
        setError(String(e));
      }
      return;
    }
    try {
      setMedia(await invoke<MediaInfoDto>("open_media", { path }));
      setMediaKind(isAudio(path) ? "audio" : "video");
      setSubtitle(null);
      setSubtitlePath(null);
      setPb(await invoke<PlaybackState>("media_state"));
      pokeControls();
    } catch (e) {
      setError(String(e));
    }
  };

  const playing = () => pb()?.status === "playing";
  const togglePlay = () =>
    void invoke(playing() ? "media_pause" : "media_play").catch((e) => setError(String(e)));
  const stop = () =>
    void invoke("media_stop")
      .then(() => {
        setMedia(null);
        setPb(null);
      })
      .catch((e) => setError(String(e)));
  const seekTo = (ms: number) =>
    void invoke("media_seek", { positionMs: ms }).catch((e) => setError(String(e)));
  const seekBy = (delta: number) => seekTo(Math.max(0, (pb()?.position_ms ?? 0) + delta));
  const toggleMute = () => setMuted((v) => !v);
  const setVolumeLevel = (v: number) => {
    setVolume(v);
    setMuted(false);
  };

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      switch (e.key) {
        case " ":
        case "k":
        case "K":
          if (media()) {
            e.preventDefault();
            togglePlay();
          }
          break;
        case "ArrowLeft":
          if (media()) seekBy(e.shiftKey ? -30000 : -5000);
          break;
        case "ArrowRight":
          if (media()) seekBy(e.shiftKey ? 30000 : 5000);
          break;
        case "ArrowUp":
        case "ArrowDown":
          if (media()) seekBy(e.key === "ArrowUp" ? -30000 : 30000);
          break;
        case "Home":
          if (media()) seekTo(0);
          break;
        case "End":
          if (media()) seekTo(pb()?.duration_ms ?? 0);
          break;
        case "Escape":
          setMenuOpen(false);
          break;
      }
      pokeControls();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const dur = () => pb()?.duration_ms ?? 0;
  const pos = () => pb()?.position_ms ?? 0;

  return (
    <main
      class="relative flex h-full flex-col overflow-hidden select-none"
      data-media-kind={mediaKind()}
      style={{ background: "var(--bg)", color: "var(--text)" }}
      onMouseMove={pokeControls}
    >
      {/* Frameless titlebar */}
      <header
        class="flex h-10 shrink-0 items-stretch border-b transition-opacity duration-300"
        style={{
          "border-color": "var(--border)",
          background: "var(--panel)",
          opacity: controlsVisible() ? 1 : 0,
        }}
      >
        {/* Brand — drag region */}
        <div class="flex items-center gap-2.5 pl-3 pr-4" data-tauri-drag-region>
          <span
            class="flex h-6 w-6 items-center justify-center rounded-lg"
            style={{
              background: "linear-gradient(135deg, var(--accent), #0ea5e9)",
              color: "#fff",
            }}
          >
            <Play class="ml-px h-3 w-3" />
          </span>
          <h1 class="text-[13px] font-semibold tracking-wide" data-tauri-drag-region>
            Go Player
          </h1>
        </div>

        {/* Open split-button + dropdown */}
        <div class="flex items-center gap-2 py-1.5">
          <div class="relative flex items-center">
            <button
              onClick={() => void pickFiles("media")}
              class="flex h-7 items-center gap-1.5 rounded-l-lg pl-2.5 pr-2 text-xs font-medium transition hover:brightness-110 active:scale-[0.98] cursor-pointer"
              style={{ background: "var(--hover)", color: "var(--text)" }}
            >
              <FolderOpen class="h-3.5 w-3.5" />
              Open
            </button>
            <button
              onClick={() => setMenuOpen((v) => !v)}
              class="flex h-7 w-6 items-center justify-center rounded-r-lg border-l transition hover:brightness-110 active:scale-[0.98] cursor-pointer"
              style={{
                background: "var(--hover)",
                color: "var(--text-dim)",
                "border-color": "var(--border)",
              }}
              aria-haspopup="menu"
              aria-expanded={menuOpen()}
              aria-label="More open options"
            >
              <ChevronDown
                class={`h-3 w-3 transition-transform duration-200 ${menuOpen() ? "rotate-180" : ""}`}
              />
            </button>
          </div>
          <Show when={menuOpen()}>
            <div
              class="anim-pop-in absolute left-3 top-10 z-50 w-52 overflow-hidden rounded-xl border p-1 shadow-2xl"
              style={{
                "border-color": "var(--border)",
                background: "var(--panel-solid)",
                "box-shadow": "0 12px 40px rgba(0,0,0,0.5)",
              }}
              role="menu"
            >
              <MenuItem
                icon={<FolderOpen class="h-4 w-4 opacity-70" />}
                label="Open Media…"
                hint="Ctrl+O"
                onClick={() => void pickFiles("media")}
              />
              <MenuItem
                icon={<Subtitles class="h-4 w-4 opacity-70" />}
                label="Open Subtitles…"
                onClick={() => void pickFiles("subs")}
              />
            </div>
          </Show>
        </div>

        {/* Drag spacer */}
        <div class="flex-1" data-tauri-drag-region />

        <span
          class="self-center pr-2 text-[10px] tracking-wide uppercase"
          style={{ color: "var(--text-faint)" }}
        >
          {backend()?.name ?? ""}
        </span>

        <WindowControls />
      </header>

      {/* Stage */}
      <section class="relative flex flex-1 items-center justify-center overflow-hidden">
        <Show
          when={media()}
          fallback={
            <div class="anim-fade-up text-center">
              <div
                class="mx-auto mb-5 flex h-20 w-20 items-center justify-center rounded-3xl border"
                style={{
                  "border-color": "var(--border)",
                  background: "var(--hover)",
                  color: "var(--text-dim)",
                }}
              >
                {mediaKind() === "audio" ? (
                  <Music class="h-9 w-9" />
                ) : (
                  <FolderOpen class="h-9 w-9" />
                )}
              </div>
              <p class="text-lg font-medium" style={{ color: "var(--text-dim)" }}>
                No media loaded
              </p>
              <p class="mt-1 text-sm" style={{ color: "var(--text-faint)" }}>
                Open video or music files
              </p>
              <p class="mt-4 text-[11px]" style={{ color: "var(--text-faint)" }}>
                Space play/pause · ←/→ seek 5s · Shift+←/→ seek 30s · ↑/↓ volume soon
              </p>
            </div>
          }
        >
          {/* Audio-only modern player */}
          <Show when={mediaKind() === "audio"}>
            <div class="anim-fade-up flex h-full w-full max-w-4xl flex-col items-center justify-center gap-6 px-8">
              {/* Album art / poster */}
              <div
                class="relative flex h-64 w-64 shrink-0 items-center justify-center rounded-3xl border shadow-2xl"
                style={{
                  "border-color": "var(--border)",
                  background: "var(--panel-solid)",
                  color: "var(--accent)",
                  "box-shadow": "0 32px 80px rgba(0,0,0,0.5)",
                }}
              >
                <Music class="h-24 w-24" />
              </div>

              {/* Song info */}
              <div class="text-center">
                <h2 class="max-w-lg truncate text-2xl font-semibold">
                  {media()?.metadata.title || fileName(media()!.path)}
                </h2>
                <p class="mt-1 text-lg" style={{ color: "var(--text-dim)" }}>
                  {media()?.metadata.artist || "Unknown Artist"}
                </p>
                <Show when={media()?.metadata.album}>
                  <p class="mt-0.5 text-sm" style={{ color: "var(--text-faint)" }}>
                    {media()!.metadata.album}
                    <Show when={media()!.metadata.year}>{" · " + media()!.metadata.year}</Show>
                  </p>
                </Show>
              </div>

              {/* Progress bar */}
              <div class="flex w-full max-w-lg flex-col gap-2">
                <input
                  type="range"
                  class="seek flex-1"
                  min="0"
                  max="1000"
                  value={dur() ? Math.round((pos() / dur()) * 1000) : 0}
                  onInput={(e) =>
                    dur() && seekTo(Math.round((Number(e.currentTarget.value) / 1000) * dur()))
                  }
                  disabled={!media()}
                  aria-label="Seek"
                />
                <div
                  class="flex justify-between text-xs tabular-nums"
                  style={{ color: "var(--text-faint)" }}
                >
                  <span>{fmtTime(pos())}</span>
                  <span>{fmtTime(dur())}</span>
                </div>
              </div>

              {/* Playback controls */}
              <div class="flex items-center gap-4">
                <button
                  class="flex h-10 w-10 items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label="Shuffle"
                  title="Shuffle"
                >
                  <Shuffle class="h-5 w-5" />
                </button>

                <button
                  onClick={() => seekBy(-10000)}
                  disabled={!media()}
                  class="flex h-12 w-12 items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 disabled:opacity-30 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label="Previous track"
                  title="Previous"
                >
                  <Rewind class="h-6 w-6" />
                </button>

                <button
                  onClick={togglePlay}
                  disabled={!media()}
                  class="flex h-16 w-16 items-center justify-center rounded-full transition hover:scale-105 active:scale-95 disabled:opacity-30 cursor-pointer"
                  style={{
                    background: "linear-gradient(135deg, var(--accent), #a855f7)",
                    color: "#fff",
                    "box-shadow": "0 8px 24px rgba(168, 85, 247, 0.4)",
                  }}
                  aria-label={playing() ? "Pause" : "Play"}
                >
                  <Show when={playing()} fallback={<Play class="ml-0.5 h-7 w-7" />}>
                    <Pause class="h-7 w-7" />
                  </Show>
                </button>

                <button
                  onClick={() => seekBy(10000)}
                  disabled={!media()}
                  class="flex h-12 w-12 items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 disabled:opacity-30 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label="Next track"
                  title="Next"
                >
                  <FastForward class="h-6 w-6" />
                </button>

                <button
                  class="flex h-10 w-10 items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label="Repeat"
                  title="Repeat"
                >
                  <Repeat class="h-5 w-5" />
                </button>
              </div>

              {/* Volume and extra controls */}
              <div class="flex items-center gap-4">
                <button
                  onClick={toggleMute}
                  class="flex h-8 w-8 items-center justify-center rounded-lg transition hover:brightness-125 active:scale-95 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label={muted() ? "Unmute" : "Mute"}
                  title={muted() ? "Unmute" : "Mute"}
                >
                  <Show when={muted()} fallback={<Volume2 class="h-4 w-4" />}>
                    <VolumeX class="h-4 w-4" />
                  </Show>
                </button>

                <input
                  type="range"
                  class="w-24 cursor-pointer"
                  min="0"
                  max="100"
                  value={muted() ? 0 : volume()}
                  onInput={(e) => setVolumeLevel(Number(e.currentTarget.value))}
                  disabled={!media()}
                  aria-label="Volume"
                />

                <button
                  class="flex h-8 w-8 items-center justify-center rounded-lg transition hover:brightness-125 active:scale-95 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label="Fullscreen"
                  title="Fullscreen"
                >
                  <Maximize class="h-4 w-4" />
                </button>

                <button
                  class="flex h-8 w-8 items-center justify-center rounded-lg transition hover:brightness-125 active:scale-95 cursor-pointer"
                  style={{ color: "var(--text-dim)" }}
                  aria-label="More options"
                  title="More"
                >
                  <MoreHorizontal class="h-4 w-4" />
                </button>
              </div>
            </div>
          </Show>

          {/* Center play overlay */}
          <Show when={!playing()}>
            <button
              onClick={togglePlay}
              class="absolute inset-0 z-10 flex items-center justify-center transition"
              style={{ background: mediaKind() === "video" ? "var(--scrim)" : "transparent" }}
              aria-label="Play"
            >
              <Show
                when={mediaKind() === "video"}
                fallback={
                  <span
                    class="flex h-20 w-20 items-center justify-center rounded-full backdrop-blur-md transition hover:scale-105 cursor-pointer"
                    style={{ background: "var(--hover)", color: "var(--text)" }}
                  >
                    <Play class="ml-1 h-9 w-9" />
                  </span>
                }
              >
                <span
                  class="flex h-20 w-20 items-center justify-center rounded-full backdrop-blur-md transition hover:scale-105 cursor-pointer"
                  style={{ background: "var(--hover)", color: "var(--text)" }}
                >
                  <Play class="ml-1 h-9 w-9" />
                </span>
              </Show>
            </button>
          </Show>
        </Show>

        {/* Subtitle overlay */}
        <div class="pointer-events-none absolute inset-x-0 bottom-24 flex justify-center px-10">
          <p
            class="text-2xl font-medium"
            style={{
              color: "#fff",
              "text-shadow": "0 2px 6px rgba(0,0,0,0.9), 0 0 2px rgba(0,0,0,0.9)",
            }}
          >
            Subtitle preview goes here
          </p>
        </div>
      </section>

      {/* Subtitle info strip */}
      <Show when={subtitle()}>
        {(s) => (
          <div
            class="shrink-0 border-t px-4 py-1.5 text-[11px] backdrop-blur"
            style={{
              "border-color": "var(--border)",
              background: "var(--panel)",
              color: "var(--text-dim)",
            }}
          >
            <span style={{ color: "var(--text)" }}>{subtitlePath()}</span>
            {" · "}
            {s().cue_count} cues{s().language ? ` · ${s().language}` : ""}
          </div>
        )}
      </Show>

      <Show when={error()}>
        {(e) => (
          <div
            class="shrink-0 border-t px-4 py-1.5 text-[11px] backdrop-blur"
            style={{
              "border-color": "var(--border)",
              "border-top-color": "#ef444466",
              background: "#7f1d1d55",
              color: "#fca5a5",
            }}
          >
            {e()}
          </div>
        )}
      </Show>

      {/* Control bar (video only) */}
      <Show when={mediaKind() === "video"}>
        <footer
          class="flex h-14 shrink-0 items-center gap-2 border-t px-4 backdrop-blur transition-opacity duration-300"
          style={{
            "border-color": "var(--border)",
            background: "var(--panel)",
            opacity: controlsVisible() || !playing() ? 1 : 0,
          }}
        >
          <button
            onClick={() => seekBy(-10000)}
            disabled={!media()}
            class="flex h-9 w-9 items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 disabled:opacity-30 cursor-pointer"
            style={{ color: "var(--text-dim)" }}
            aria-label="Back 10 seconds"
            title="Back 10s"
          >
            <Rewind />
          </button>

          <button
            onClick={togglePlay}
            disabled={!media()}
            class="flex h-10 w-10 items-center justify-center rounded-full transition hover:scale-105 active:scale-95 disabled:opacity-30 cursor-pointer"
            style={{ background: "var(--text)", color: "var(--bg)" }}
            aria-label={playing() ? "Pause" : "Play"}
          >
            <Show when={playing()} fallback={<Play class="h-4.5 w-4.5" />}>
              <Pause class="h-4.5 w-4.5" />
            </Show>
          </button>

          <button
            onClick={() => seekBy(10000)}
            disabled={!media()}
            class="flex h-9 w-9 items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 disabled:opacity-30 cursor-pointer"
            style={{ color: "var(--text-dim)" }}
            aria-label="Forward 10 seconds"
            title="Forward 10s"
          >
            <FastForward />
          </button>

          <span
            class="w-10 text-right text-[11px] tabular-nums"
            style={{ color: "var(--text-dim)" }}
          >
            {fmtTime(pos())}
          </span>

          <input
            type="range"
            class="seek flex-1"
            min="0"
            max="1000"
            value={dur() ? Math.round((pos() / dur()) * 1000) : 0}
            onInput={(e) =>
              dur() && seekTo(Math.round((Number(e.currentTarget.value) / 1000) * dur()))
            }
            disabled={!media()}
            aria-label="Seek"
          />

          <span class="w-10 text-[11px] tabular-nums" style={{ color: "var(--text-faint)" }}>
            {fmtTime(dur())}
          </span>

          <button
            onClick={stop}
            disabled={!media()}
            class="flex h-8 w-8 items-center justify-center rounded-lg transition hover:brightness-125 active:scale-95 disabled:opacity-30 cursor-pointer"
            style={{ color: "var(--text-dim)" }}
            aria-label="Stop"
            title="Stop"
          >
            <Square class="h-3.5 w-3.5" />
          </button>
        </footer>
      </Show>
    </main>
  );
}

function WindowControls() {
  const win = getCurrentWindow();
  const [maximized, setMaximized] = createSignal(false);

  onMount(() => {
    void win.isMaximized().then(setMaximized);
    let unlisten: (() => void) | undefined;
    void win.onResized(() => void win.isMaximized().then(setMaximized)).then((u) => (unlisten = u));
    onCleanup(() => unlisten?.());
  });

  const btn =
    "flex h-8 w-11 items-center justify-center transition-colors hover:bg-white/10 active:bg-white/15 cursor-pointer";

  return (
    <div class="flex items-stretch">
      <button
        class={btn}
        onClick={() => void win.minimize()}
        aria-label="Minimize"
        title="Minimize"
      >
        <Minimize2 class="h-3.5 w-3.5" />
      </button>
      <button
        class={btn}
        onClick={() => void win.toggleMaximize()}
        aria-label={maximized() ? "Restore" : "Maximize"}
        title={maximized() ? "Restore" : "Maximize"}
      >
        <Show when={maximized()} fallback={<Maximize2 class="h-3 w-3" />}>
          <Minimize2 class="h-3 w-3" />
        </Show>
      </button>
      <button
        class={`${btn} hover:!bg-red-500/90 hover:!text-white`}
        onClick={() => void win.close()}
        aria-label="Close"
        title="Close"
      >
        <X class="h-3.5 w-3.5" />
      </button>
    </div>
  );
}

function MenuItem(props: { icon: any; label: string; hint?: string; onClick: () => void }) {
  return (
    <button
      role="menuitem"
      onClick={props.onClick}
      class="flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-xs transition hover:brightness-110 cursor-pointer"
      style={{ color: "var(--text)" }}
      onMouseEnter={(e) => ((e.currentTarget as HTMLElement).style.background = "var(--hover)")}
      onMouseLeave={(e) => ((e.currentTarget as HTMLElement).style.background = "transparent")}
    >
      {props.icon}
      <span class="flex-1">{props.label}</span>
      {props.hint && (
        <kbd class="text-[10px]" style={{ color: "var(--text-faint)" }}>
          {props.hint}
        </kbd>
      )}
    </button>
  );
}

export default App;

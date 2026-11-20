import { createEffect, createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  Play,
  Pause,
  FolderOpen,
  ChevronDown,
  Rewind,
  FastForward,
  Minus,
  Maximize2,
  Minimize2,
  X,
  Volume2,
  VolumeX,
  Shuffle,
  Repeat,
  Music,
  Subtitles,
  List,
  Cast,
  History,
  Clapperboard,
} from "lucide-solid";
import type {
  MediaBackendInfo,
  MediaInfoDto,
  PlaybackState,
  SubtitleSummary,
} from "@go-player/contracts";
import "./App.css";

const MEDIA_EXTS = [
  "mp4",
  "mkv",
  "webm",
  "mov",
  "avi",
  "m4v",
  "mpg",
  "mpeg",
  "wmv",
  "flv",
  "mp3",
  "flac",
  "wav",
  "ogg",
  "m4a",
  "aac",
  "wma",
  "opus",
  "aiff",
];
const SUBTITLE_EXTS = ["srt", "vtt", "ass", "ssa"];
const UI_HIDE_MS = 3000;

function extOf(path: string): string {
  const i = path.lastIndexOf(".");
  return i === -1 ? "" : path.slice(i + 1).toLowerCase();
}

function isAudio(path: string): boolean {
  return !["mp4", "mkv", "webm", "mov", "avi", "m4v", "mpg", "mpeg", "wmv", "flv"].includes(
    extOf(path),
  );
}

function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

function fmtTime(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${m}:${r.toString().padStart(2, "0")}`;
}

export default function App() {
  const [media, setMedia] = createSignal<MediaInfoDto | null>(null);
  const [mediaKind, setMediaKind] = createSignal<"video" | "audio">("video");
  const [pb, setPb] = createSignal<PlaybackState | null>(null);
  const [backend, setBackend] = createSignal<MediaBackendInfo | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [subtitle, setSubtitle] = createSignal<SubtitleSummary | null>(null);
  const [subtitlePath, setSubtitlePath] = createSignal<string | null>(null);
  const [menuOpen, setMenuOpen] = createSignal(false);
  const [volume, setVolume] = createSignal(100);
  const [muted, setMuted] = createSignal(false);
  const [loopMode, setLoopMode] = createSignal<"off" | "one" | "all">("off");
  const [shuffled, setShuffled] = createSignal(false);
  const [showQueue, setShowQueue] = createSignal(false);
  const [showCast, setShowCast] = createSignal(false);
  const [showRecent, setShowRecent] = createSignal(false);
  const [showVolume, setShowVolume] = createSignal(false);
  const [queue] = createSignal<string[]>([]);
  const [uiVisible, setUiVisible] = createSignal(true);
  // True during the brief EOF→rewind window: mpv's surface is hidden and
  // the stage paints black so mpv's cleared (grey) frame never shows.
  const [eofHold, setEofHold] = createSignal(false);
  // While the user drags the seek thumb, playback-state pushes must not
  // fight the drag; the input shows the dragged value until release.
  const [scrubbing, setScrubbing] = createSignal(false);
  const [dragValue, setDragValue] = createSignal(0);
  let lastSeekAt = 0;

  // Timestamp of the last user activity; a 500ms interval hides the
  // bars once it is older than UI_HIDE_MS. setTimeout proved unreliable
  // in this WebView2 setup, while setInterval demonstrably fires.
  let lastActivityAt = 0;
  let eofAt = 0;

  const playing = () => pb()?.status === "playing";
  const pos = () => pb()?.position_ms ?? 0;
  const dur = () => pb()?.duration_ms ?? 0;
  const coverUrl = () => media()?.cover_art ?? null;

  // Bars overlay the video; they auto-hide after idle and always stay
  // visible when a menu/modal is open or no media is loaded (so window
  // controls remain reachable).
  const barsShown = () =>
    uiVisible() ||
    !media() ||
    menuOpen() ||
    showVolume() ||
    showQueue() ||
    showCast() ||
    showRecent();

  const pokeUi = () => {
    lastActivityAt = Date.now();
    setUiVisible(true);
  };

  // Ignore cursor jitter: only count the mouse as active once it has
  // moved a few px from where it last woke the UI, otherwise sub-pixel
  // sensor noise keeps resetting the auto-hide timer forever.
  let lastPokeX = 0;
  let lastPokeY = 0;
  let hasPokePos = false;
  const onMouseMove = (e: MouseEvent) => {
    if (!hasPokePos || Math.abs(e.clientX - lastPokeX) > 8 || Math.abs(e.clientY - lastPokeY) > 8) {
      lastPokeX = e.clientX;
      lastPokeY = e.clientY;
      hasPokePos = true;
      pokeUi();
    }
  };

  // Any playback-state change (play/pause, new media, end of track)
  // re-shows the bars and restarts the idle countdown. Mouse movement
  // and key presses do the same via pokeUi(). Manual previous-key guard:
  // pb() returns a new object every poll, so we compare the derived
  // string ourselves — `on()` could not be trusted to dedupe here.
  let prevStateKey = "";
  createEffect(() => {
    const key = `${pb()?.status ?? ""}|${media()?.path ?? ""}`;
    if (key === prevStateKey) return;
    prevStateKey = key;
    lastActivityAt = Date.now();
    setUiVisible(true);
  });

  // At EOF, rewind to the start and pause: mpv keeps its window open
  // (keep-open) so the first frame stays on screen, and Play/Space then
  // resumes from the beginning like a normal replay.
  let prevEnded = false;
  createEffect(() => {
    const ended = pb()?.status === "ended";
    if (ended && !prevEnded && media()) {
      void invoke("media_seek", { positionMs: 0 })
        .then(() => invoke("media_pause"))
        .catch((e) => setError(String(e)));
    }
    prevEnded = ended;
  });

  // Once the engine's EOF rewind lands, playback reads paused/playing at
  // position 0 — restore mpv's surface (Rust already hid it on the eof
  // event, so the grey frame never renders).
  createEffect(() => {
    if (!eofHold()) return;
    const st = pb()?.status;
    if (st === "paused" || st === "playing" || st === "stopped") {
      setEofHold(false);
      void invoke("media_surface_show").catch(() => {});
    }
  });

  onMount(() => {
    void invoke<MediaBackendInfo>("media_backend")
      .then(setBackend)
      .catch((e) => setError(String(e)));

    // Playback state arrives as push events from the engine's mpv
    // property observer — no polling. media_state remains for the one
    // initial fetch after open_media.
    let unlistenState: (() => void) | undefined;
    void listen<PlaybackState>("media://state", (e) => setPb(e.payload)).then(
      (u) => (unlistenState = u),
    );

    // Auto-hide check on a UI timer: hides the bars once the user has
    // been inactive for UI_HIDE_MS while media is loaded.
    const uiInterval = window.setInterval(() => {
      if (media() && uiVisible() && Date.now() - lastActivityAt > UI_HIDE_MS) {
        setUiVisible(false);
      }
      // Safety: never leave the surface hidden if the paused state is
      // somehow missed (e.g. status stays "ended" indefinitely).
      if (eofHold() && Date.now() - eofAt > 1500) {
        setEofHold(false);
        void invoke("media_surface_show").catch(() => {});
      }
    }, 500);
    onCleanup(() => window.clearInterval(uiInterval));

    let unlistenEof: (() => void) | undefined;
    void listen("media://eof", () => {
      if (mediaKind() !== "video") return;
      eofAt = Date.now();
      setEofHold(true);
      void invoke("media_surface_hide").catch(() => {});
    }).then((u) => (unlistenEof = u));

    onCleanup(() => {
      unlistenState?.();
      unlistenEof?.();
    });

    const onKey = (e: KeyboardEvent) => {
      const tag = (e.target as HTMLElement | null)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      pokeUi();
      switch (e.key) {
        case " ":
          e.preventDefault();
          togglePlay();
          break;
        case "ArrowLeft":
          e.preventDefault();
          seekBy(e.shiftKey ? -30000 : -5000);
          break;
        case "ArrowRight":
          e.preventDefault();
          seekBy(e.shiftKey ? 30000 : 5000);
          break;
        case "ArrowUp":
          e.preventDefault();
          setVolumeLevel(volume() + 5);
          break;
        case "ArrowDown":
          e.preventDefault();
          setVolumeLevel(volume() - 5);
          break;
        case "m":
        case "M":
          toggleMute();
          break;
        case "f":
        case "F":
          toggleFullscreen();
          break;
        case "o":
        case "O":
          if (e.ctrlKey || e.metaKey) {
            e.preventDefault();
            void pickFiles("media");
          }
          break;
      }
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  const togglePlay = () => {
    if (pb()?.status === "ended") {
      seekTo(0);
      void invoke("media_play").catch((e) => setError(String(e)));
    } else {
      void invoke(playing() ? "media_pause" : "media_play").catch((e) => setError(String(e)));
    }
  };

  const seekTo = (ms: number) =>
    void invoke("media_seek", { positionMs: ms }).catch((e) => setError(String(e)));

  const seekBy = (delta: number) => seekTo(Math.max(0, pos() + delta));

  const applyMute = (m: boolean) => {
    setMuted(m);
    if (media()) {
      void invoke("media_set_muted", { muted: m }).catch((e) => setError(String(e)));
    }
  };

  const toggleMute = () => applyMute(!muted());

  const setVolumeLevel = (v: number) => {
    const clamped = Math.min(100, Math.max(0, Math.round(v)));
    setVolume(clamped);
    if (clamped > 0 && muted()) applyMute(false);
    if (media()) {
      void invoke("media_set_volume", { volume: clamped }).catch((e) => setError(String(e)));
    }
  };

  const toggleFullscreen = () => {
    const win = getCurrentWindow();
    void win.isFullscreen().then((fs) => void win.setFullscreen(!fs));
  };

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
    // Stop any previous playback before opening new media
    if (media()) {
      await invoke("media_stop").catch(() => {});
    }
    try {
      setMedia(await invoke<MediaInfoDto>("open_media", { path, hasVideo: !isAudio(path) }));
      setMediaKind(isAudio(path) ? "audio" : "video");
      setSubtitle(null);
      setSubtitlePath(null);
      setEofHold(false);
      setPb(await invoke<PlaybackState>("media_state"));
    } catch (e) {
      setError(String(e));
    }
  };

  const pickFolder = async () => {
    setMenuOpen(false);
    setShowQueue(true);
  };

  const seekPct = () => (dur() ? (pos() / dur()) * 100 : 0);

  const topBarClass = () =>
    `absolute inset-x-0 top-0 z-30 transition-opacity duration-300 ease-out ${
      barsShown() ? "translate-y-0 opacity-100" : "pointer-events-none -translate-y-full opacity-0"
    } ${mediaKind() === "audio" ? "bg-[var(--bg)]" : ""}`;

  const bottomBarClass = () =>
    `absolute inset-x-0 bottom-0 z-30 transition-opacity duration-300 ease-out ${
      barsShown() ? "translate-y-0 opacity-100" : "pointer-events-none translate-y-8 opacity-0"
    } ${mediaKind() === "audio" ? "bg-[var(--bg)]" : ""}`;

  return (
    <main
      class="relative h-screen overflow-hidden"
      style={{ color: "var(--text)" }}
      data-media-kind={mediaKind()}
      onMouseMove={onMouseMove}
      onMouseDown={() => pokeUi()}
    >
      {/* Stage — the native video surface sits behind this transparent
          webview and fills the whole window; the bars overlay it. */}
      <section class="absolute inset-0" onDblClick={toggleFullscreen}>
        <Show
          when={media()}
          fallback={
            <div
              class="anim-fade-up flex h-full flex-col items-center justify-center text-center"
              style={{ background: "var(--bg)" }}
            >
              <div
                class="mx-auto mb-5 flex h-20 w-20 items-center justify-center rounded-3xl border"
                style={{
                  "border-color": "var(--border)",
                  background: "var(--hover)",
                  color: "var(--text-dim)",
                }}
              >
                <FolderOpen class="h-9 w-9" />
              </div>
              <p class="text-lg font-medium" style={{ color: "var(--text-dim)" }}>
                No media loaded
              </p>
              <p class="mt-1 text-sm" style={{ color: "var(--text-faint)" }}>
                Open video or music files
              </p>
              <p class="mt-4 text-[11px]" style={{ color: "var(--text-faint)" }}>
                Space play/pause · ←/→ seek 5s · Shift+←/→ seek 30s · ↑/↓ volume · M mute · F
                fullscreen
              </p>
            </div>
          }
        >
          <Show
            when={mediaKind() === "audio"}
            fallback={
              <div class="h-full" style={{ background: eofHold() ? "#000" : "transparent" }} />
            }
          >
            <div
              class="anim-fade-up flex h-full flex-col items-center justify-center gap-5 px-8 text-center"
              style={{ background: "var(--bg)" }}
            >
              <div
                class="flex h-52 w-52 items-center justify-center overflow-hidden rounded-[28px] border shadow-2xl"
                style={{
                  "border-color": "var(--border)",
                  background: "var(--panel-solid)",
                  color: "var(--accent)",
                  "box-shadow": "0 24px 64px rgba(0,0,0,0.5)",
                }}
              >
                <Show when={coverUrl()} fallback={<Music class="h-24 w-24" />}>
                  {(url) => (
                    <img
                      src={url()}
                      alt="Cover art"
                      class="h-full w-full object-cover"
                      draggable={false}
                    />
                  )}
                </Show>
              </div>
              <div class="max-w-xl">
                <h2 class="truncate text-3xl font-semibold">
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
            </div>
          </Show>
        </Show>
      </section>

      {/* Top bar — overlays the video, auto-hides while playing */}
      <header class={topBarClass()}>
        <div
          class="flex h-full items-center gap-2 border-b px-3"
          style={{
            "border-color": "var(--border)",
            background:
              mediaKind() === "audio"
                ? "var(--bg)"
                : "color-mix(in srgb, var(--bg) 38%, transparent)",
            "-webkit-app-region": "drag",
          }}
          data-tauri-drag-region
        >
          <div class="relative">
            <button
              onClick={() => setMenuOpen(!menuOpen())}
              class="flex cursor-pointer items-center gap-1.5 rounded-lg px-2.5 py-1 text-sm font-medium transition hover:bg-white/10"
              style={{ color: "var(--text)" }}
              data-tauri-drag-region="false"
            >
              <FolderOpen class="h-4 w-4" />
              <span>Open</span>
              <ChevronDown class="h-3 w-3 opacity-60" />
            </button>

            <Show when={menuOpen()}>
              <div
                class="anim-pop-in absolute left-3 top-10 z-9999 w-56 overflow-hidden rounded-xl border p-1 shadow-2xl"
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
                  icon={<FolderOpen class="h-4 w-4 opacity-70" />}
                  label="Open Folder…"
                  onClick={() => void pickFolder()}
                />
                <MenuItem
                  icon={<Subtitles class="h-4 w-4 opacity-70" />}
                  label="Open Subtitles…"
                  onClick={() => void pickFiles("subs")}
                />
                <div class="my-1 border-t" style={{ "border-color": "var(--border)" }} />
                <MenuItem
                  icon={<List class="h-4 w-4 opacity-70" />}
                  label="Queue"
                  onClick={() => setShowQueue(true)}
                />
                <MenuItem
                  icon={<Cast class="h-4 w-4 opacity-70" />}
                  label="Cast"
                  onClick={() => setShowCast(true)}
                />
                <MenuItem
                  icon={<History class="h-4 w-4 opacity-70" />}
                  label="Recent Plays"
                  onClick={() => setShowRecent(true)}
                />
              </div>
            </Show>
          </div>

          <div class="flex-1" data-tauri-drag-region />

          <span
            class="self-center pr-2 text-[10px] tracking-wide uppercase"
            style={{ color: "var(--text-faint)" }}
          >
            {backend()?.name ?? ""}
          </span>

          <WindowControls />
        </div>
      </header>

      {/* Bottom overlay: info strips + player bar */}
      <div class={bottomBarClass()}>
        {/* Subtitle info strip */}
        <Show when={subtitle()}>
          {(s) => (
            <div
              class="border-t px-4 py-1.5 text-[11px]"
              style={{
                "border-color": "var(--border)",
                background:
                  mediaKind() === "audio"
                    ? "var(--bg)"
                    : "color-mix(in srgb, var(--bg) 38%, transparent)",
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
              class="border-t px-4 py-1.5 text-[11px]"
              style={{
                "border-color": "var(--border)",
                "border-top-color": "#ef444466",
                background:
                  mediaKind() === "audio"
                    ? "var(--bg)"
                    : "color-mix(in srgb, #7f1d1d 45%, transparent)",
                color: "#fca5a5",
              }}
            >
              {e()}
            </div>
          )}
        </Show>

        {/* Player bar — shared by video and audio */}
        <footer
          class="flex h-25 shrink-0 flex-col border-t"
          style={{
            "border-color": "var(--border)",
            background:
              mediaKind() === "audio"
                ? "var(--bg)"
                : "color-mix(in srgb, var(--bg) 38%, transparent)",
          }}
        >
          {/* Seek row */}
          <div class="flex h-9 items-center gap-3 px-6">
            <span
              class="w-10 shrink-0 text-right text-[11px] tabular-nums"
              style={{ color: "var(--text-faint)" }}
            >
              {fmtTime(pos())}
            </span>
            <div class="min-w-0 flex-1">
              <input
                type="range"
                class="seek w-full"
                min="0"
                max="1000"
                value={scrubbing() ? dragValue() : dur() ? Math.round((pos() / dur()) * 1000) : 0}
                onPointerDown={() => setScrubbing(true)}
                onInput={(e) => {
                  const v = Number(e.currentTarget.value);
                  setDragValue(v);
                  // Throttle seek IPC during drag; the release handler
                  // issues the final accurate seek.
                  const now = Date.now();
                  if (now - lastSeekAt > 150) {
                    lastSeekAt = now;
                    dur() && seekTo(Math.round((v / 1000) * dur()));
                  }
                }}
                onPointerUp={() => {
                  setScrubbing(false);
                  dur() && seekTo(Math.round((dragValue() / 1000) * dur()));
                }}
                onChange={() => {
                  setScrubbing(false);
                  dur() && seekTo(Math.round((dragValue() / 1000) * dur()));
                }}
                disabled={!media()}
                aria-label="Seek"
                style={`--progress:${seekPct()}%;`}
              />
            </div>
            <span
              class="w-10 shrink-0 text-[11px] tabular-nums"
              style={{ color: "var(--text-faint)" }}
            >
              {fmtTime(dur())}
            </span>
          </div>

          <div class="flex flex-1 items-center gap-4 px-6 pb-3">
            {/* Now playing */}
            <div class="flex w-60 min-w-0 items-center gap-3">
              <div
                class="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg border"
                style={{
                  "border-color": "var(--border)",
                  background: "var(--panel-solid)",
                  color: "var(--accent)",
                }}
              >
                <Show
                  when={mediaKind() === "audio" && coverUrl()}
                  fallback={
                    <Show when={mediaKind() === "video"} fallback={<Music class="h-5 w-5" />}>
                      <Clapperboard class="h-5 w-5" />
                    </Show>
                  }
                >
                  <img
                    src={coverUrl()!}
                    alt=""
                    class="h-full w-full object-cover"
                    draggable={false}
                  />
                </Show>
              </div>
              <div class="min-w-0">
                <p class="truncate text-sm font-medium">
                  {media() ? media()!.metadata.title || fileName(media()!.path) : "No media"}
                </p>
                <p class="truncate text-xs" style={{ color: "var(--text-dim)" }}>
                  {media()?.metadata.artist || " "}
                </p>
                <Show when={media()?.metadata.album}>
                  <p class="truncate text-[11px]" style={{ color: "var(--text-faint)" }}>
                    {media()!.metadata.album}
                    <Show when={media()!.metadata.year}>{" · " + media()!.metadata.year}</Show>
                  </p>
                </Show>
              </div>
            </div>

            {/* Transport controls */}
            <div class="flex min-w-0 flex-1 items-center justify-center gap-1">
              <CtrlBtn onClick={() => setShuffled(!shuffled())} label="Shuffle" active={shuffled()}>
                <Shuffle class="h-4 w-4" />
              </CtrlBtn>
              <CtrlBtn onClick={() => seekBy(-10000)} disabled={!media()} label="Back 10s">
                <Rewind class="h-4.5 w-4.5" />
              </CtrlBtn>
              <button
                onClick={togglePlay}
                disabled={!media()}
                class="mx-1 flex h-11 w-11 shrink-0 cursor-pointer items-center justify-center rounded-full transition hover:scale-105 active:scale-95 disabled:opacity-30"
                style={{
                  background: "linear-gradient(135deg, #f97316, #a855f7)",
                  color: "#fff",
                  "box-shadow": "0 8px 24px rgba(168, 85, 247, 0.4)",
                }}
                aria-label={playing() ? "Pause" : "Play"}
              >
                <Show when={playing()} fallback={<Play class="ml-0.5 h-5 w-5" />}>
                  <Pause class="h-5 w-5" />
                </Show>
              </button>
              <CtrlBtn onClick={() => seekBy(10000)} disabled={!media()} label="Forward 10s">
                <FastForward class="h-4.5 w-4.5" />
              </CtrlBtn>
              <CtrlBtn
                onClick={() =>
                  setLoopMode(loopMode() === "off" ? "one" : loopMode() === "one" ? "all" : "off")
                }
                label={`Loop: ${loopMode()}`}
                active={loopMode() !== "off"}
              >
                <Repeat class={`h-4 w-4 ${loopMode() === "all" ? "fill-current" : ""}`} />
              </CtrlBtn>
            </div>

            {/* Audio button — opens the volume modal */}
            <div class="flex w-60 shrink-0 items-center justify-end gap-2">
              <CtrlBtn onClick={() => setShowVolume(true)} label="Audio" active={muted()}>
                <Show when={muted()} fallback={<Volume2 class="h-4 w-4" />}>
                  <VolumeX class="h-4 w-4" />
                </Show>
              </CtrlBtn>
            </div>
          </div>
        </footer>
      </div>

      {/* Modals */}
      <QueuePanel open={showQueue()} onClose={() => setShowQueue(false)} queue={queue()} />
      <CastPanel open={showCast()} onClose={() => setShowCast(false)} />
      <RecentPanel open={showRecent()} onClose={() => setShowRecent(false)} />
      <VolumePanel
        open={showVolume()}
        onClose={() => setShowVolume(false)}
        volume={volume()}
        muted={muted()}
        onVolume={setVolumeLevel}
        onMute={toggleMute}
      />
    </main>
  );
}

function CtrlBtn(props: {
  onClick?: () => void;
  disabled?: boolean;
  label: string;
  active?: boolean;
  children: JSX.Element;
}) {
  return (
    <button
      onClick={props.onClick}
      disabled={props.disabled}
      class="flex h-9 w-9 cursor-pointer items-center justify-center rounded-full transition hover:brightness-125 active:scale-95 disabled:opacity-30"
      style={{ color: props.active ? "var(--accent)" : "var(--text-dim)" }}
      aria-label={props.label}
      title={props.label}
    >
      {props.children}
    </button>
  );
}

function MenuItem(props: { icon: JSX.Element; label: string; hint?: string; onClick: () => void }) {
  return (
    <button
      onClick={props.onClick}
      class="flex w-full cursor-pointer items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition hover:bg-white/10"
      style={{ color: "var(--text)" }}
      role="menuitem"
    >
      <span class="opacity-70">{props.icon}</span>
      <span class="flex-1">{props.label}</span>
      {props.hint && (
        <span class="text-[10px]" style={{ color: "var(--text-faint)" }}>
          {props.hint}
        </span>
      )}
    </button>
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
    "flex h-8 w-11 cursor-pointer items-center justify-center transition-colors hover:bg-white/10 active:bg-white/15";

  return (
    <div class="flex items-stretch">
      <button
        class={btn}
        onClick={() => void win.minimize()}
        aria-label="Minimize"
        title="Minimize"
      >
        <Minus class="h-3.5 w-3.5" />
      </button>
      <button
        class={btn}
        onClick={() => void win.toggleMaximize()}
        aria-label={maximized() ? "Restore" : "Maximize"}
        title={maximized() ? "Restore" : "Maximize"}
      >
        <Show when={maximized()} fallback={<Maximize2 class="h-3.5 w-3.5" />}>
          <Minimize2 class="h-3.5 w-3.5" />
        </Show>
      </button>
      <button
        class={`${btn} hover:bg-red-500/80 hover:text-white`}
        onClick={() => void win.close()}
        aria-label="Close"
        title="Close"
      >
        <X class="h-4 w-4" />
      </button>
    </div>
  );
}

function QueuePanel(props: { open: boolean; onClose: () => void; queue: string[] }) {
  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
        onClick={props.onClose}
      >
        <div
          class="max-h-[80vh] w-96 rounded-2xl border p-4 shadow-2xl"
          style={{
            "border-color": "var(--border)",
            background: "var(--panel-solid)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div class="mb-4 flex items-center justify-between">
            <h3 class="text-lg font-semibold">Queue</h3>
            <button
              onClick={props.onClose}
              class="cursor-pointer"
              style={{ color: "var(--text-dim)" }}
            >
              <X class="h-5 w-5" />
            </button>
          </div>
          <div class="max-h-[60vh] space-y-2 overflow-y-auto">
            {props.queue.length === 0 ? (
              <p class="text-sm" style={{ color: "var(--text-faint)" }}>
                Queue is empty
              </p>
            ) : (
              props.queue.map((path, i) => (
                <div class="flex cursor-pointer items-center gap-3 rounded-lg p-2 hover:bg-white/5">
                  <span class="w-6 text-xs" style={{ color: "var(--text-faint)" }}>
                    {i + 1}
                  </span>
                  <span class="flex-1 truncate text-sm">{fileName(path)}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </Show>
  );
}

function CastPanel(props: { open: boolean; onClose: () => void }) {
  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
        onClick={props.onClose}
      >
        <div
          class="w-96 rounded-2xl border p-4 shadow-2xl"
          style={{
            "border-color": "var(--border)",
            background: "var(--panel-solid)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div class="mb-4 flex items-center justify-between">
            <h3 class="text-lg font-semibold">Cast</h3>
            <button
              onClick={props.onClose}
              class="cursor-pointer"
              style={{ color: "var(--text-dim)" }}
            >
              <X class="h-5 w-5" />
            </button>
          </div>
          <p class="text-sm" style={{ color: "var(--text-faint)" }}>
            Cast feature coming soon
          </p>
        </div>
      </div>
    </Show>
  );
}

function RecentPanel(props: { open: boolean; onClose: () => void }) {
  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
        onClick={props.onClose}
      >
        <div
          class="max-h-[80vh] w-96 rounded-2xl border p-4 shadow-2xl"
          style={{
            "border-color": "var(--border)",
            background: "var(--panel-solid)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div class="mb-4 flex items-center justify-between">
            <h3 class="text-lg font-semibold">Recent Plays</h3>
            <button
              onClick={props.onClose}
              class="cursor-pointer"
              style={{ color: "var(--text-dim)" }}
            >
              <X class="h-5 w-5" />
            </button>
          </div>
          <p class="text-sm" style={{ color: "var(--text-faint)" }}>
            Recent plays feature coming soon
          </p>
        </div>
      </div>
    </Show>
  );
}

function VolumePanel(props: {
  open: boolean;
  onClose: () => void;
  volume: number;
  muted: boolean;
  onVolume: (v: number) => void;
  onMute: () => void;
}) {
  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
        onClick={props.onClose}
      >
        <div
          class="w-80 rounded-2xl border p-5 shadow-2xl"
          style={{
            "border-color": "var(--border)",
            background: "var(--panel-solid)",
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div class="mb-5 flex items-center justify-between">
            <h3 class="text-lg font-semibold">Volume</h3>
            <button
              onClick={props.onClose}
              class="cursor-pointer"
              style={{ color: "var(--text-dim)" }}
            >
              <X class="h-5 w-5" />
            </button>
          </div>
          <div class="flex items-center gap-4">
            <CtrlBtn
              onClick={props.onMute}
              label={props.muted ? "Unmute" : "Mute"}
              active={props.muted}
            >
              <Show when={props.muted} fallback={<Volume2 class="h-4 w-4" />}>
                <VolumeX class="h-4 w-4" />
              </Show>
            </CtrlBtn>
            <input
              type="range"
              class="seek flex-1 cursor-pointer"
              min="0"
              max="100"
              value={props.muted ? 0 : props.volume}
              onInput={(e) => props.onVolume(Number(e.currentTarget.value))}
              aria-label="Volume"
              style={`--progress:${props.muted ? 0 : props.volume}%;`}
            />
            <span class="w-10 text-right text-sm tabular-nums" style={{ color: "var(--text-dim)" }}>
              {props.muted ? 0 : props.volume}%
            </span>
          </div>
          <p class="mt-4 text-[11px]" style={{ color: "var(--text-faint)" }}>
            M mute · ↑/↓ volume
          </p>
        </div>
      </div>
    </Show>
  );
}

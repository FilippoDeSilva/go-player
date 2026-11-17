/**
 * Typed contracts mirroring the Rust domain types in `crates/go-media`.
 * These are the shapes crossing the Tauri IPC boundary — keep them in
 * sync with the serde-serialized Rust types (snake_case enums).
 */

export type TrackKind = "video" | "audio" | "subtitle";

export interface TrackInfo {
  id: number;
  kind: TrackKind;
  title: string;
  language: string | null;
  is_default: boolean;
  is_selected: boolean;
}

export interface MediaInfo {
  path: string;
  /** Duration in milliseconds, or null if unknown. */
  duration: number | null;
  tracks: TrackInfo[];
}

export type PlaybackStatus =
  | "stopped"
  | "playing"
  | "paused"
  | "ended"
  | "error";

export interface PlaybackState {
  status: PlaybackStatus;
  /** Position in milliseconds. */
  position: number;
  /** Duration in milliseconds, or null if unknown. */
  duration: number | null;
}

export type MediaBackend = "mpv" | "other";

export interface MediaBackendInfo {
  backend: MediaBackend;
  name: string;
}

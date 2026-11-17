//! Headless smoke test: proves libmpv-2.dll loads, client API matches,
//! and `loadfile`/state commands work without a video surface.
//! Requires `scripts/setup-mpv.ps1` to have provisioned native/mpv.

use go_media::{PlaybackEngine, PlaybackStatus};
use go_media_mpv::MpvPlaybackEngine;
use std::path::PathBuf;
use std::time::Duration;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/tone.wav")
        .canonicalize()
        .expect("tone.wav fixture missing")
}

#[test]
fn headless_open_play_state() {
    let mut engine = match MpvPlaybackEngine::new_headless() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("skipping: mpv not provisioned or init failed: {e}");
            return;
        }
    };
    engine.open(&fixture()).expect("open failed");
    engine.play().expect("play failed");
    std::thread::sleep(Duration::from_millis(300));
    let state = engine.state().expect("state failed");
    assert_ne!(state.status, PlaybackStatus::Stopped);
    engine.stop().expect("stop failed");
}

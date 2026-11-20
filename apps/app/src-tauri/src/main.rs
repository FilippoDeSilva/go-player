//! Prevents additional console window on Windows, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    go_player_lib::run()
}

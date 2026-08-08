// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(feature = "desktop")]
fn main() {
    novavei_agent_lib::run();
}

#[cfg(not(feature = "desktop"))]
fn main() {}

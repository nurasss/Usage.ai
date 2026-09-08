//! Usage.ai platform shell.
//!
//! This crate is composition only: Tauri command wiring, tray/window
//! events, macOS lifecycle and host implementations. Provider business
//! logic lives in `usage-runtime`; parsing in `usage-providers`;
//! storage in `usage-storage`; OS access behind `usage-host` traits.

pub mod appstate;
pub mod commands;
pub mod composition;
pub mod dto;
pub mod platform;
pub mod refresh_flow;

pub use composition::run;

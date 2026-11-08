//! Tandem core: all business logic for the AI body-doubling companion.
//!
//! Rules that live here (see CLAUDE.md): timers and session state are owned by this crate;
//! image requests to cloud providers are blocked in `router::privacy`; secrets only go to
//! the OS keychain; images never touch disk.

pub mod app;
pub mod audio;
pub mod character;
pub mod checkin;
#[cfg(feature = "tauri")]
pub mod commands;
pub mod conversation;
pub mod error;
pub mod events;
pub mod providers;
pub mod router;
pub mod secrets;
pub mod session;
pub mod settings;
pub mod store;
#[cfg(feature = "tauri")]
pub mod windows;

pub use app::Core;
pub use error::{CoreError, Result};

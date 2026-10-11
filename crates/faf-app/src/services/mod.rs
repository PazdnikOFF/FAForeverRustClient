//! Business logic. One module per feature.
//!
//! A service consumes a command, performs side effects through [`crate::ports`],
//! and emits events via the [`EventSink`](crate::EventSink). A service **never**
//! touches `AppState` directly (ARCHITECTURE.md §3.4).

/// The wall clock as Unix seconds.
///
/// Shared because three services need it for the same reason: deciding
/// whether something has already happened: and three copies is three chances
/// to get the saturation wrong. Saturates at zero rather than wrapping: a clock
/// set before 1970 would otherwise become a colossal future timestamp, which
/// reads as "every tournament has finished" and "every match is old enough to
/// spectate".
pub(crate) fn now_seconds() -> u32 {
    u32::try_from(chrono::Utc::now().timestamp()).unwrap_or(0)
}

pub mod auth;
pub mod changelog;
pub mod chat;
pub mod clan;
pub mod client_update;
pub mod coop;
pub mod discord;
pub mod events;
pub mod galactic_war;
pub mod guides;
pub mod launcher;
pub mod leaderboard;
pub mod lobby;
pub mod mac_setup;
pub mod map_generator;
pub mod maps;
pub mod mods;
pub mod nav;
pub mod notifications;
pub mod player_card;
pub mod reconnect;
pub mod replays;
pub mod reporting;
pub mod reviews;
pub mod session;
pub mod settings;
pub mod social;
pub mod streams;
pub mod tourney;
pub mod training;
pub mod tutorials;
pub mod uploads;

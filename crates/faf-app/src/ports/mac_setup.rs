//! Preparing a Mac to run Forged Alliance: Rosetta, Wine, a prefix, the
//! connectivity runtime and the game files.
//!
//! One port for the whole chain because every step writes into the same
//! client-owned folder and the later ones depend on the earlier ones being
//! there: the prefix needs Wine, the DirectX libraries need the prefix.
//! Splitting it by step would only move that ordering into the service.

use async_trait::async_trait;
use faf_domain::state::{MacSetupProbe, SetupStep, SteamGuardKind};
use tokio::sync::mpsc;

/// Progress of one step. The stream always ends in [`StepUpdate::Finished`]: a
/// consumer that sees the channel close without one knows the producer died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepUpdate {
    /// `permille` is `None` while the total is unknown.
    Progress {
        permille: Option<u32>,
        done_mb: u32,
        total_mb: u32,
    },
    Finished(Result<(), String>),
}

/// What the Steam download asks of the player, or how far it has got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SteamUpdate {
    /// A Steam Guard code is needed. `retry` when the last one was refused.
    NeedsCode {
        kind: SteamGuardKind,
        retry: bool,
    },
    /// Waiting for the player to approve the sign-in in the Steam Mobile app.
    AwaitingApproval,
    /// Signing in again, this time to ask for a code: see
    /// [`MacSetupPort::steam_use_code`].
    Restarting,
    /// Signed in and downloading.
    Downloading {
        permille: Option<u32>,
        done_mb: u32,
        total_mb: u32,
    },
    Finished(Result<(), String>),
}

/// What a cancelled Steam download finishes with, so the service can tell a
/// cancel from a failure.
pub const STEAM_CANCELLED: &str = "cancelled";

#[async_trait]
pub trait MacSetupPort: Send + Sync {
    /// What is on disk now. Cheap enough to run at startup and after every
    /// step.
    async fn probe(&self) -> MacSetupProbe;

    /// Install one component. Never [`SetupStep::Game`], which has its own
    /// two ways in below.
    async fn install(&self, step: SetupStep) -> mpsc::Receiver<StepUpdate>;

    /// Sign in to Steam and download Forged Alliance into the client's folder.
    ///
    /// The password goes to the downloader's standard input, never to its
    /// command line, where any process on the machine could read it.
    async fn steam_download(
        &self,
        username: String,
        password: String,
    ) -> mpsc::Receiver<SteamUpdate>;

    /// Answer the question the running download asked. False when nothing is
    /// waiting for an answer.
    async fn steam_code(&self, code: String) -> bool;

    /// Stop waiting for the Steam Mobile app and ask for a code instead. False
    /// when nothing is running.
    async fn steam_use_code(&self) -> bool;

    /// Stop the running download, if any.
    async fn cancel_steam(&self);

    /// Adopt a Forged Alliance folder the player already has. Returns the
    /// folder in use, which is the game root even when the player picked the
    /// `bin` folder or the executable inside it.
    async fn use_game_folder(&self, path: String) -> Result<String, String>;

    /// Delete the Wine prefix so the next install creates it afresh.
    async fn reset_prefix(&self) -> Result<(), String>;

    /// Delete the whole client-owned folder.
    async fn remove_everything(&self) -> Result<(), String>;

    /// Write the graphics settings that measured fastest on Apple silicon into
    /// the game's `game.prefs`. The game must not be running: it rewrites the
    /// file when it exits. Returns how many settings changed.
    async fn optimize_graphics(&self) -> Result<u32, String>;

    /// Apply those settings by themselves the first time there is a
    /// `game.prefs` to apply them to, and never again after that.
    async fn optimize_graphics_once(&self);
}

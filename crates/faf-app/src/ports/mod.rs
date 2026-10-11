//! Port traits: the external boundaries of the application.
//!
//! Each external system becomes a trait here, implemented in [`crate::infra`]
//! and mocked in tests. Services depend on these traits, never on concrete IO.
//! See ARCHITECTURE.md §5 for the full Port table.

pub mod auth;
pub mod changelog;
pub mod chat;
pub mod clan;
pub mod client_update;
pub mod coop;
pub mod discord;
pub mod error;
pub mod events;
pub mod galactic_war;
pub mod game_cache;
pub mod guides;
pub mod ice;
pub mod leaderboard;
pub mod lobby;
pub mod mac_setup;
pub mod map_generator;
pub mod maps;
pub mod mods;
pub mod notification_sounds;
pub mod paths;
pub mod player_card;
pub mod process;
pub mod replay;
pub mod reporting;
pub mod reviews;
pub mod settings;
pub mod streams;
pub mod tourney;
pub mod training;
pub mod tutorials;
pub mod updater;
pub mod uploads;

pub use auth::{AuthError, AuthPort, AuthResult};
pub use changelog::ChangelogPort;
pub use chat::{ChatPort, ChatUpdate};
pub use clan::ClanPort;
pub use client_update::{ClientUpdatePort, DownloadProgress};
pub use coop::CoopPort;
pub use discord::{DiscordPort, DiscordRequest};
pub use error::RequestError;
pub use events::EventsPort;
pub use galactic_war::{GalacticWarPort, InstallProgress};
pub use game_cache::GameCachePort;
pub use guides::{DeviceCode, GuidesPort};
pub use ice::{ConnectivitySession, IceDebugWindows, IceParams, IcePort, RelayMsg};
pub use leaderboard::LeaderboardPort;
pub use lobby::{LobbyPort, LobbyUpdate, ServerNoticeStyle};
pub use mac_setup::{MacSetupPort, SteamUpdate, StepUpdate};
pub use map_generator::{GeneratorUpdate, MapGeneratorPort};
pub use maps::{MapSearchPage, MapsPort};
pub use mods::{ModPrepFailure, ModSearchPage, ModsPort};
pub use notification_sounds::NotificationSoundsPort;
pub use paths::PathsPort;
pub use player_card::PlayerCardPort;
pub use process::{
    DiscoveredInstallPaths, GameLaunchParams, InstallPresence, ProcessPort, ReplayMetadata,
};
pub use replay::{
    PreparationSink, ReplayDetailsPort, ReplayLibraryPort, ReplayPlaybackPort, ReplayVaultPort,
    VaultSearchResult, DEFAULT_LOCAL_REPLAY_LIMIT,
};
pub use reporting::{GameParticipation, ReportPlayerRequest, ReportingPort};
pub use reviews::{ReviewPage, ReviewsPort};
pub use settings::SettingsPort;
pub use streams::StreamsPort;
pub use tourney::{
    TourneyChatPort, TourneyEntryPort, TourneyMapsPort, TourneyMatchPort, TourneyOrganiserPort,
    TourneyReadPort, TourneySitePort,
};
pub use training::TrainingPort;
pub use tutorials::TutorialsPort;
pub use updater::{
    GamePreparation, GameUpdaterPort, InstalledBuild, PreparationPhase, PreparationStep,
    UpdateProgress,
};
pub use uploads::UploadsPort;

use std::sync::Arc;

/// The bundle of ports injected into every service via [`crate::ServiceCtx`].
///
/// Cheap to clone (everything behind `Arc`). Grows one field per external system.
#[derive(Clone)]
pub struct Ports {
    pub auth: Arc<dyn AuthPort>,
    pub chat: Arc<dyn ChatPort>,
    pub coop: Arc<dyn CoopPort>,
    pub discord: Arc<dyn DiscordPort>,
    pub lobby: Arc<dyn LobbyPort>,
    pub settings: Arc<dyn SettingsPort>,
    /// Connectivity + launch ports. Authenticated sessions use real adapters;
    /// the explicit offline/test port set supplies inert fakes. The GPGNet
    /// relay server is an internal detail of the Go adapter backend, so it is
    /// no longer a top-level port.
    pub ice: Arc<dyn IcePort>,
    pub process: Arc<dyn ProcessPort>,
    /// Brings the live install up to date before a launch. Paired with
    /// `process`: it patches the very install `process` is about to run.
    pub updater: Arc<dyn GameUpdaterPort>,
    /// The replay ports, one per capability (see [`replay`]), so a test fakes
    /// the one it exercises and takes the inert default for the rest.
    pub replay_vault: Arc<dyn ReplayVaultPort>,
    pub replay_library: Arc<dyn ReplayLibraryPort>,
    pub replay_details: Arc<dyn ReplayDetailsPort>,
    /// Launches a replay on the `process` port above, after preparing the
    /// replay install it targets.
    pub replay_playback: Arc<dyn ReplayPlaybackPort>,
    pub maps: Arc<dyn MapsPort>,
    pub map_generator: Arc<dyn MapGeneratorPort>,
    pub mods: Arc<dyn ModsPort>,
    /// Where file lookups resolve to. See [`paths::PathsPort`] for why this is
    /// a port at all.
    pub paths: Arc<dyn PathsPort>,
    pub leaderboard: Arc<dyn LeaderboardPort>,
    pub player_card: Arc<dyn PlayerCardPort>,
    /// Clan management: the half of a clan that used to be the website's alone.
    /// Reads and edits are Elide; founding, inviting and joining go through
    /// `ClansController`, which exists because they cannot be resource writes.
    pub clan: Arc<dyn ClanPort>,
    pub reporting: Arc<dyn ReportingPort>,
    pub reviews: Arc<dyn ReviewsPort>,
    /// The tournament service's ports, one per capability (see [`tourney`]).
    /// The real client and the offline fake fill every one from a single
    /// instance, so a write through one is seen by a read through another; a
    /// test replaces the one it exercises and keeps the fake for the rest.
    pub tourney_read: Arc<dyn TourneyReadPort>,
    pub tourney_entry: Arc<dyn TourneyEntryPort>,
    pub tourney_match: Arc<dyn TourneyMatchPort>,
    pub tourney_maps: Arc<dyn TourneyMapsPort>,
    pub tourney_chat: Arc<dyn TourneyChatPort>,
    pub tourney_organiser: Arc<dyn TourneyOrganiserPort>,
    pub tourney_site: Arc<dyn TourneySitePort>,
    /// Whether FAF's own Twitch channel is broadcasting. The only port whose
    /// correct answer on most builds is "nothing, forever": Twitch requires an
    /// application's own credentials, which a public repository cannot hold, so
    /// [`StreamsPort::can_check`] is how a build says it was never given any.
    pub streams: Arc<dyn StreamsPort>,
    /// The community calendar's catalogue. A read of one document, and the last
    /// of the three sources the Events tab draws: the other two, tournaments
    /// and the changelog index, already have ports of their own.
    pub events: Arc<dyn EventsPort>,
    /// The training hub's catalogue of community material. A read of one
    /// document; the hub's two write-shaped paths compose a forum post the
    /// player sends themselves rather than crossing a port.
    pub training: Arc<dyn TrainingPort>,
    /// The catalogue's Git repository, which is the one port that writes with
    /// an identity that is not the FAF account: committing to it is a GitHub
    /// operation, authorised by GitHub.
    pub guides: Arc<dyn GuidesPort>,
    pub tutorials: Arc<dyn TutorialsPort>,
    /// FAForever/fa's published patch notes. Public documents, never gated on
    /// login: the changelog is worth reading before signing in.
    pub changelog: Arc<dyn ChangelogPort>,
    pub uploads: Arc<dyn UploadsPort>,
    /// Replaces the *client*, not the game. Distinct from `updater` above,
    /// which patches the Forged Alliance install.
    pub client_update: Arc<dyn ClientUpdatePort>,
    /// Wine, the prefix, the connectivity runtime and the game files a Mac
    /// needs. Inert everywhere else.
    pub mac_setup: Arc<dyn MacSetupPort>,
    /// Installs and starts the separate Galactic War application. Unrelated to
    /// `process` and `updater`: Galactic War is not Forged Alliance and does
    /// not go through the lobby.
    pub galactic_war: Arc<dyn GalacticWarPort>,
    /// The game-files cache the settings tab measures, expires and clears.
    /// A port so a test that loads settings cannot reach the real one.
    pub game_cache: Arc<dyn GameCachePort>,
    /// The sounds a player added, for removing one. See
    /// [`notification_sounds::NotificationSoundsPort`].
    pub notification_sounds: Arc<dyn NotificationSoundsPort>,
    /// True when `auth` is the offline stub rather than real FAF OAuth.
    ///
    /// A property of the bundle rather than of any one port: it is how the
    /// shell tells the UI that credential-free affordances are meaningful in
    /// this build. Against real ports the test login fabricates a player the
    /// server has never heard of, so the UI must not offer it.
    pub offline_auth: bool,
    /// The user's language as the OS reports it, read once at startup, empty
    /// when the platform does not say.
    ///
    /// Also a bundle property rather than a port: it is a single immutable
    /// string, and threading it here keeps `services` free of environment
    /// reads, so the auto-join tests do not depend on the locale of whichever
    /// machine runs them. Selects FAF's language channel; see
    /// [`faf_domain::state::language_channel`].
    pub os_language: String,
    /// Roles handed to the credential-free test login, from `FAF_FAKE_ROLES`.
    ///
    /// A bundle property for the same reason as `os_language`: it is read from
    /// the environment once at startup, and `services` must not read the
    /// environment itself. Empty in the normal case.
    ///
    /// This reveals role-gated UI; it authorises nothing. A test-login session
    /// holds no token at all, so every privileged call fails regardless of what
    /// is listed here.
    pub test_login_roles: Vec<String>,
}

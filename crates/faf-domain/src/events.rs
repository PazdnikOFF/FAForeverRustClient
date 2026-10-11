//! [`AppEvent`]: the single delta type.
//!
//! This is a namespaced enum-of-enums (one variant per slice), never a flat enum.
//! The exact same value is reduced into [`crate::AppState`] **and** serialized to
//! the frontend, which guarantees backend and UI never disagree (ARCHITECTURE.md §3.2).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::state::{
    AuthEvent, ChangelogEvent, ChatEvent, ClanEvent, ClientUpdateEvent, CoopEvent, EventsEvent,
    GalacticWarEvent, GuidesEvent, InstallEvent, LeaderboardEvent, LobbyEvent, MacSetupEvent,
    MapGeneratorEvent, MapsEvent, ModsEvent, NavEvent, NotificationEvent, PlayerCardEvent,
    ReplayEvent, ReportingEvent, ReviewsEvent, SessionEvent, SettingsEvent, SocialEvent,
    StreamsEvent, TourneyEvent, TrainingEvent, TutorialsEvent, UploadsEvent,
};

// No `Eq`: `ReplayEvent` carries an `f32` (vault replay review score).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "event")]
pub enum AppEvent {
    Session(SessionEvent),
    Auth(AuthEvent),
    Nav(NavEvent),
    Notifications(NotificationEvent),
    Chat(ChatEvent),
    Clan(ClanEvent),
    Coop(CoopEvent),
    Lobby(LobbyEvent),
    Replays(ReplayEvent),
    Maps(MapsEvent),
    MapGenerator(MapGeneratorEvent),
    Mods(ModsEvent),
    Leaderboard(LeaderboardEvent),
    PlayerCard(PlayerCardEvent),
    Reporting(ReportingEvent),
    Reviews(ReviewsEvent),
    Social(SocialEvent),
    Streams(StreamsEvent),
    Tourney(TourneyEvent),
    Training(TrainingEvent),
    Tutorials(TutorialsEvent),
    Changelog(ChangelogEvent),
    Events(EventsEvent),
    Uploads(UploadsEvent),
    GalacticWar(GalacticWarEvent),
    Guides(GuidesEvent),
    ClientUpdate(ClientUpdateEvent),
    MacSetup(MacSetupEvent),
    Install(InstallEvent),
    Settings(SettingsEvent),
}

impl From<SessionEvent> for AppEvent {
    fn from(e: SessionEvent) -> Self {
        AppEvent::Session(e)
    }
}

impl From<AuthEvent> for AppEvent {
    fn from(e: AuthEvent) -> Self {
        AppEvent::Auth(e)
    }
}

impl From<NavEvent> for AppEvent {
    fn from(e: NavEvent) -> Self {
        AppEvent::Nav(e)
    }
}

impl From<NotificationEvent> for AppEvent {
    fn from(e: NotificationEvent) -> Self {
        AppEvent::Notifications(e)
    }
}

impl From<ChatEvent> for AppEvent {
    fn from(e: ChatEvent) -> Self {
        AppEvent::Chat(e)
    }
}

impl From<LobbyEvent> for AppEvent {
    fn from(e: LobbyEvent) -> Self {
        AppEvent::Lobby(e)
    }
}

impl From<SettingsEvent> for AppEvent {
    fn from(e: SettingsEvent) -> Self {
        AppEvent::Settings(e)
    }
}

impl From<ReplayEvent> for AppEvent {
    fn from(e: ReplayEvent) -> Self {
        AppEvent::Replays(e)
    }
}

impl From<MapsEvent> for AppEvent {
    fn from(e: MapsEvent) -> Self {
        AppEvent::Maps(e)
    }
}

impl From<LeaderboardEvent> for AppEvent {
    fn from(e: LeaderboardEvent) -> Self {
        AppEvent::Leaderboard(e)
    }
}

impl From<PlayerCardEvent> for AppEvent {
    fn from(e: PlayerCardEvent) -> Self {
        AppEvent::PlayerCard(e)
    }
}

impl From<ReportingEvent> for AppEvent {
    fn from(e: ReportingEvent) -> Self {
        AppEvent::Reporting(e)
    }
}

impl From<ModsEvent> for AppEvent {
    fn from(e: ModsEvent) -> Self {
        AppEvent::Mods(e)
    }
}

impl From<SocialEvent> for AppEvent {
    fn from(e: SocialEvent) -> Self {
        AppEvent::Social(e)
    }
}

impl From<TourneyEvent> for AppEvent {
    fn from(e: TourneyEvent) -> Self {
        AppEvent::Tourney(e)
    }
}

impl From<InstallEvent> for AppEvent {
    fn from(e: InstallEvent) -> Self {
        AppEvent::Install(e)
    }
}

impl From<MapGeneratorEvent> for AppEvent {
    fn from(e: MapGeneratorEvent) -> Self {
        AppEvent::MapGenerator(e)
    }
}

impl From<CoopEvent> for AppEvent {
    fn from(e: CoopEvent) -> Self {
        AppEvent::Coop(e)
    }
}

impl From<GuidesEvent> for AppEvent {
    fn from(e: GuidesEvent) -> Self {
        AppEvent::Guides(e)
    }
}

impl From<TrainingEvent> for AppEvent {
    fn from(e: TrainingEvent) -> Self {
        AppEvent::Training(e)
    }
}

impl From<TutorialsEvent> for AppEvent {
    fn from(e: TutorialsEvent) -> Self {
        AppEvent::Tutorials(e)
    }
}

impl From<EventsEvent> for AppEvent {
    fn from(e: EventsEvent) -> Self {
        AppEvent::Events(e)
    }
}

impl From<ChangelogEvent> for AppEvent {
    fn from(e: ChangelogEvent) -> Self {
        AppEvent::Changelog(e)
    }
}

impl From<ClanEvent> for AppEvent {
    fn from(e: ClanEvent) -> Self {
        AppEvent::Clan(e)
    }
}

impl From<ReviewsEvent> for AppEvent {
    fn from(e: ReviewsEvent) -> Self {
        AppEvent::Reviews(e)
    }
}

impl From<UploadsEvent> for AppEvent {
    fn from(e: UploadsEvent) -> Self {
        AppEvent::Uploads(e)
    }
}

impl From<GalacticWarEvent> for AppEvent {
    fn from(e: GalacticWarEvent) -> Self {
        AppEvent::GalacticWar(e)
    }
}

impl From<ClientUpdateEvent> for AppEvent {
    fn from(e: ClientUpdateEvent) -> Self {
        AppEvent::ClientUpdate(e)
    }
}

impl From<MacSetupEvent> for AppEvent {
    fn from(e: MacSetupEvent) -> Self {
        AppEvent::MacSetup(e)
    }
}

impl From<StreamsEvent> for AppEvent {
    fn from(e: StreamsEvent) -> Self {
        AppEvent::Streams(e)
    }
}

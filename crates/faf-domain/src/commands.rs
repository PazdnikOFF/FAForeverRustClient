//! [`AppCommand`]: intents flowing from the UI to the backend.
//!
//! Namespaced enum-of-enums, mirroring [`crate::AppEvent`]. A command is a
//! *request*; it never mutates state directly. A service handles it and emits
//! events, which are the only thing that changes state (ARCHITECTURE.md §3.4).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::state::{
    AuthCommand, ChangelogCommand, ChatCommand, ClanCommand, ClientUpdateCommand, CoopCommand,
    EventsCommand, GalacticWarCommand, GuidesCommand, LeaderboardCommand, LobbyCommand,
    MacSetupCommand, MapGeneratorCommand, MapsCommand, ModsCommand, NavCommand,
    NotificationCommand, PlayerCardCommand, ReplayCommand, ReportingCommand, ReviewsCommand,
    SessionCommand, SettingsCommand, SocialCommand, StreamsCommand, TourneyCommand, TourneyRead,
    TourneyWrite, TrainingCommand, TutorialsCommand, UploadsCommand,
};

// No `Eq`: `ReplayCommand` carries a `ReplayQuery`, which has an `f32`
// (minimum review score): the same reason `AppEvent` has no `Eq`.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", content = "command")]
pub enum AppCommand {
    Session(SessionCommand),
    Auth(AuthCommand),
    Nav(NavCommand),
    Notifications(NotificationCommand),
    Chat(ChatCommand),
    Clan(ClanCommand),
    Coop(CoopCommand),
    Lobby(LobbyCommand),
    Replays(ReplayCommand),
    Maps(MapsCommand),
    MapGenerator(MapGeneratorCommand),
    Mods(ModsCommand),
    Leaderboard(LeaderboardCommand),
    PlayerCard(PlayerCardCommand),
    Reporting(ReportingCommand),
    Reviews(ReviewsCommand),
    Social(SocialCommand),
    Streams(StreamsCommand),
    Tourney(TourneyCommand),
    Training(TrainingCommand),
    Tutorials(TutorialsCommand),
    Changelog(ChangelogCommand),
    Events(EventsCommand),
    Uploads(UploadsCommand),
    GalacticWar(GalacticWarCommand),
    Guides(GuidesCommand),
    ClientUpdate(ClientUpdateCommand),
    MacSetup(MacSetupCommand),
    Settings(SettingsCommand),
}

impl From<SessionCommand> for AppCommand {
    fn from(c: SessionCommand) -> Self {
        AppCommand::Session(c)
    }
}

impl From<AuthCommand> for AppCommand {
    fn from(c: AuthCommand) -> Self {
        AppCommand::Auth(c)
    }
}

impl From<NavCommand> for AppCommand {
    fn from(c: NavCommand) -> Self {
        AppCommand::Nav(c)
    }
}

impl From<NotificationCommand> for AppCommand {
    fn from(c: NotificationCommand) -> Self {
        AppCommand::Notifications(c)
    }
}

impl From<ChatCommand> for AppCommand {
    fn from(c: ChatCommand) -> Self {
        AppCommand::Chat(c)
    }
}

impl From<LobbyCommand> for AppCommand {
    fn from(c: LobbyCommand) -> Self {
        AppCommand::Lobby(c)
    }
}

impl From<SettingsCommand> for AppCommand {
    fn from(c: SettingsCommand) -> Self {
        AppCommand::Settings(c)
    }
}

impl From<ReplayCommand> for AppCommand {
    fn from(c: ReplayCommand) -> Self {
        AppCommand::Replays(c)
    }
}

impl From<MapsCommand> for AppCommand {
    fn from(c: MapsCommand) -> Self {
        AppCommand::Maps(c)
    }
}

impl From<LeaderboardCommand> for AppCommand {
    fn from(c: LeaderboardCommand) -> Self {
        AppCommand::Leaderboard(c)
    }
}

impl From<PlayerCardCommand> for AppCommand {
    fn from(c: PlayerCardCommand) -> Self {
        AppCommand::PlayerCard(c)
    }
}

impl From<ReportingCommand> for AppCommand {
    fn from(c: ReportingCommand) -> Self {
        AppCommand::Reporting(c)
    }
}

impl From<ModsCommand> for AppCommand {
    fn from(c: ModsCommand) -> Self {
        AppCommand::Mods(c)
    }
}

impl From<SocialCommand> for AppCommand {
    fn from(c: SocialCommand) -> Self {
        AppCommand::Social(c)
    }
}

impl From<TourneyCommand> for AppCommand {
    fn from(c: TourneyCommand) -> Self {
        AppCommand::Tourney(c)
    }
}

impl From<TourneyRead> for AppCommand {
    fn from(c: TourneyRead) -> Self {
        AppCommand::Tourney(c.into())
    }
}

impl From<TourneyWrite> for AppCommand {
    fn from(c: TourneyWrite) -> Self {
        AppCommand::Tourney(c.into())
    }
}

impl From<MapGeneratorCommand> for AppCommand {
    fn from(c: MapGeneratorCommand) -> Self {
        AppCommand::MapGenerator(c)
    }
}

impl From<CoopCommand> for AppCommand {
    fn from(c: CoopCommand) -> Self {
        AppCommand::Coop(c)
    }
}

impl From<GuidesCommand> for AppCommand {
    fn from(c: GuidesCommand) -> Self {
        AppCommand::Guides(c)
    }
}

impl From<TrainingCommand> for AppCommand {
    fn from(c: TrainingCommand) -> Self {
        AppCommand::Training(c)
    }
}

impl From<TutorialsCommand> for AppCommand {
    fn from(c: TutorialsCommand) -> Self {
        AppCommand::Tutorials(c)
    }
}

impl From<EventsCommand> for AppCommand {
    fn from(c: EventsCommand) -> Self {
        AppCommand::Events(c)
    }
}

impl From<ChangelogCommand> for AppCommand {
    fn from(c: ChangelogCommand) -> Self {
        AppCommand::Changelog(c)
    }
}

impl From<ClanCommand> for AppCommand {
    fn from(c: ClanCommand) -> Self {
        AppCommand::Clan(c)
    }
}

impl From<ReviewsCommand> for AppCommand {
    fn from(c: ReviewsCommand) -> Self {
        AppCommand::Reviews(c)
    }
}

impl From<UploadsCommand> for AppCommand {
    fn from(c: UploadsCommand) -> Self {
        AppCommand::Uploads(c)
    }
}

impl From<GalacticWarCommand> for AppCommand {
    fn from(c: GalacticWarCommand) -> Self {
        AppCommand::GalacticWar(c)
    }
}

impl From<ClientUpdateCommand> for AppCommand {
    fn from(c: ClientUpdateCommand) -> Self {
        AppCommand::ClientUpdate(c)
    }
}

impl From<MacSetupCommand> for AppCommand {
    fn from(c: MacSetupCommand) -> Self {
        AppCommand::MacSetup(c)
    }
}

impl From<StreamsCommand> for AppCommand {
    fn from(c: StreamsCommand) -> Self {
        AppCommand::Streams(c)
    }
}

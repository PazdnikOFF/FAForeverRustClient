//! Application state: the single source of truth.
//!
//! [`AppState`] is pure aggregation of independent slices. It has no behaviour
//! beyond holding slices; mutation happens only through [`crate::reduce`].
//! Add a feature by adding a slice module here (see ARCHITECTURE.md §8).

pub mod auth;
pub mod changelog;
pub mod chat;
pub mod clan;
pub mod client_update;
pub mod coop;
pub mod events;
pub mod failure;
pub mod galactic_war;
pub mod guides;
pub mod install;
pub mod leaderboard;
pub mod lobby;
pub mod mac_setup;
pub mod map_generator;
pub mod maps;
pub mod mods;
pub mod nav;
pub mod notifications;
pub mod player_card;
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

pub use auth::{AuthCommand, AuthEvent, AuthMode, AuthState, AuthStatus, Player};
pub use changelog::{
    ChangelogCommand, ChangelogEntryStatus, ChangelogEvent, ChangelogState, ChangelogStatus,
};
pub use chat::{
    auto_join_channels, language_channel, mentions, normalize_channels, player_total_games,
    read_marker_key, ChatChannel, ChatCommand, ChatEvent, ChatMessage, ChatMessageKind, ChatState,
    ChatStatus, ChatUser, MessageReactions, Reaction, TypingNotice, DEFAULT_CHANNEL,
    DEFAULT_NEWBIE_THRESHOLD, NEWBIE_CHANNEL, TYPING_TIMEOUT_SECONDS,
};
pub use clan::{
    ClanAction, ClanActionStatus, ClanCommand, ClanDraft, ClanDraftProblem, ClanEvent,
    ClanIdentity, ClanInvitation, ClanState, ClanStatus, MAX_CLAN_TAG,
};
pub use client_update::{
    compare_versions, is_release_version, should_update, strip_version_prefix, ClientRelease,
    ClientUpdateCommand, ClientUpdateEvent, ClientUpdateState, ClientUpdateStatus, ReleaseChannel,
};
pub use coop::{
    missions_of, rank_results, CoopCategory, CoopCommand, CoopEvent, CoopFaction, CoopMission,
    CoopResult, CoopScenario, CoopState, CoopStatus, ANY_PLAYER_COUNT, PLAYER_COUNT_OPTIONS,
};
pub use events::{
    CalendarEvent, CalendarView, EventCatalogue, EventCategory, EventLink, EventOrigin,
    EventsCommand, EventsEvent, EventsQuery, EventsSource, EventsState, EventsStatus, Recurrence,
};
pub use failure::RequestFailureKind;
pub use galactic_war::{
    ClientVersions, GalacticWarAlltime, GalacticWarCommand, GalacticWarEvent, GalacticWarFaction,
    GalacticWarSeason, GalacticWarState, GalacticWarStatistics, GalacticWarStatus,
    StatisticsStatus,
};
pub use guides::{
    accept_commit_message, catalogue_with, compose_submission, entry_from_body, entry_from_draft,
    guide_file_path, guide_from_body, guide_raw_url, new_issue_url, prose_from_body,
    read_draft_images, rejection_comment, slug, submission_body, submission_title, DeviceLogin,
    DraftImage, GuideImage, GuideSubmission, GuidesAuthStatus, GuidesCommand, GuidesEvent,
    GuidesIdentity, GuidesState, GuidesStatus, GuidesWrite, RejectReason, SubmissionPull,
    SubmitStatus, CATALOGUE_PATH, GUIDES_REPO, SUBMISSION_LABEL,
};
pub use install::{InstallEvent, InstallState, ResolvedPaths};
pub use leaderboard::{
    BoardRating, LeaderboardCommand, LeaderboardEntry, LeaderboardEvent, LeaderboardMode,
    LeaderboardState, LeaderboardStatus, LeaderboardTier, League, LeagueSeason, PlayerRatings,
    RatingLeaderboard, RatingPage, RatingQuery, SeasonLeaderboard,
};
pub use lobby::{
    rating_for_game, rating_gate_blocks, reconcile_avatar, remember_avatar, title_marks_go_adapter,
    with_go_adapter_tag, AvailableAvatar, AvatarListStatus, AvatarReconciliation, Game, GameLaunch,
    HostGameConfig, JoinState, LobbyCommand, LobbyEvent, LobbyState, LobbyStatus, MatchmakerQueue,
    MatchmakingState, PartyMember, PartyState, PlayMode, PlayerVeto, PreparationPhase, RatingRange,
    GLOBAL_LEADERBOARD, GO_ADAPTER_TITLE_TAG,
};
pub use mac_setup::{
    MacSetupCommand, MacSetupEvent, MacSetupProbe, MacSetupState, Secret, SetupStep,
    SteamGuardKind, SteamPhase, StepProgress, StepStatus,
};
pub use map_generator::{
    is_valid_preset_name, preset_file_name, DecodedMapName, DecodedStyle, GenerationType,
    GeneratorOptionLists, GeneratorOptionQuery, GeneratorOptions, GeneratorPreset, GeneratorStatus,
    GeneratorVersion, MapGeneratorCommand, MapGeneratorEvent, MapGeneratorState, StyleConstraints,
    ValidationIssue, MAX_PRESET_NAME,
};
pub use maps::{
    InstalledMap, LocalMapPreview, MapInstallStatus, MapListStatus, MapVisibilityStatus,
    MapsCommand, MapsEvent, MapsState, MatchmakerMapPool, MatchmakerPoolMap, VaultMap,
};
pub use mods::{
    InstalledMod, ModDownloadSize, ModDownloadTarget, ModInstallStatus, ModListStatus,
    ModToggleStatus, ModType, ModVersionConflict, ModsCommand, ModsEvent, ModsState, VaultMod,
};
pub use nav::{
    MapsSection, ModsSection, NavCommand, NavEvent, NavState, ReplaysSection, SettingsSection, Tab,
};
pub use notifications::{
    ClientNotification, NotificationAction, NotificationCommand, NotificationEvent,
    NotificationKind, NotificationState, NotificationText,
};
pub use player_card::{
    aggregate_map_stats, is_retired_leaderboard, leaderboard_display_rank, sort_league_placements,
    sort_rating_summaries, AccountLookup, AccountLookupMatch, ClanMember, MatchmakerPlayerProfile,
    PlayedGame, PlayerAchievement, PlayerAchievementState, PlayerAvatar, PlayerCardCommand,
    PlayerCardEvent, PlayerCardProfile, PlayerCardState, PlayerCardStatus, PlayerClan,
    PlayerEventCount, PlayerGameResult, PlayerLeaguePlacement, PlayerMapStat, PlayerMapStats,
    PlayerNameRecord, PlayerRatingSummary, PlayerSummary, RatingHistoryPage, RatingHistoryPeriod,
    RatingHistoryPoint, RatingHistoryQuery,
};
pub use replays::{
    live_replay_delay_remaining, sort_vault_replays, LiveReplayTarget, LiveReplayTracking,
    LiveReplayTrackingAction, LocalReplay, LocalReplayPlayer, LocalReplayStatus, LocalReplayTeam,
    ReplayActivity, ReplayAnalysis, ReplayArmy, ReplayChatMessage, ReplayCommand,
    ReplayCommandStats, ReplayDetails, ReplayEvent, ReplayGameOption, ReplayNotice, ReplayOrder,
    ReplayPlayer, ReplayPlayerStats, ReplayPoint, ReplayPreparation, ReplayQuery, ReplayReadError,
    ReplayResourceStat, ReplayScenario, ReplaySortField, ReplayState, ReplayStatus, ReplayTeam,
    ReplayTotals, ReplayUnitStat, ResolvedReplayMap, VaultReplay, VaultStatus,
    LIVE_REPLAY_DELAY_SECONDS,
};
pub use reporting::{
    ModerationReportSummary, ReportHistoryStatus, ReportStatus, ReportingCommand, ReportingEvent,
    ReportingState,
};
pub use reviews::{
    clamp_score, own_review, summarize, Review, ReviewKind, ReviewSubmitStatus, ReviewSummary,
    ReviewTarget, ReviewsCommand, ReviewsEvent, ReviewsState, ReviewsStatus, MAX_SCORE, MIN_SCORE,
};
pub use session::{ConnectionStatus, SessionCommand, SessionEvent, SessionState};
pub use settings::{
    AppearancePreferences, BrowsingPreferences, CachedGameVersion, ChatNameColors, ChatPreferences,
    ConnectivityPreferences, CustomGameBrowserPreferences, CustomGameFilterConstraint,
    CustomGameFilterField, CustomGameFilterRule, CustomGameSort, CustomGameView, DebugPreferences,
    DiscordPreferences, EventReminder, EventsPreferences, FilterMemory, GameCacheInfo,
    GamePreferences, GeneralPreferences, HostGamePreferences, IceAdapter, LiveReplayFilters,
    NotificationPreferences, NotificationSound, NotificationSoundChoices, PathPreferences,
    PlayerNote, ReplayChatTransfers, ReplayNote, SettingsCommand, SettingsEvent, SettingsState,
    SocialPreferences, Theme, ToastPosition, UiDensity, UpdatePreferences, WeekStart,
    MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, SIDEBAR_RAIL_BELOW,
};
pub use social::{
    LoginLookup, PlayerLobbyRating, PlayerProfile, Relation, SocialCommand, SocialEvent,
    SocialState, MAX_LOGIN_LOOKUPS,
};
pub use streams::{
    LiveStream, StreamPlatform, StreamsCommand, StreamsEvent, StreamsState, StreamsStatus,
};
pub use tourney::{
    map_key, match_vault_map, Article, AuditEntry, BracketKind, BracketSide, ChatMute, ChatPost,
    ChatRoom, Competition, DraftRejection, Formation, HostingStatus, InviteStatus, MapDraft,
    MapPool, MatchLink, MatchReport, MatchStatus, NewsPost, Organiser, PendingReport, PoolAction,
    PoolAssignment, PoolDraft, PoolRejection, PoolSide, PoolStep, RatingGate, RatingKind,
    SeedOrder, Seeding, SignupMode, Standing, StandingOutcome, StandingsKind, TeamExit,
    TeamRequest, Tourney, TourneyAction, TourneyActionFailure, TourneyCategory, TourneyCommand,
    TourneyDraft, TourneyEvent, TourneyInvite, TourneyLoadStatus, TourneyMap, TourneyMatch,
    TourneyPhase, TourneyPlayer, TourneyRead, TourneyState, TourneyStatus, TourneyTeam,
    TourneyViewer, TourneyWrite,
};
pub use tourney::{
    AccessKind, AccessRequest, AccessStatus, AdminArticle, ArchivedTourney, ConsoleRole,
    HallOfFame, HallPlayer, HallTeam, ListedAccount, PendingItem, PendingSummary, SiteAdminData,
    SiteDocument, SiteLogEntry, SiteRead, SiteWrite, TourneyAccount, TourneySite,
};
pub use tourney::{BanScope, EntrantBoardRating, EntrantRatings, OwnBan, RatingCheck};
pub use tourney::{
    BracketConfig, Caster, Currency, FeedsInto, FormatDraft, MatchPlan, Prize, Qualifier,
    QualifierKind, QualifierRejection, QualifierRule, RoomBadge, RoundKey, RoundPlan, SeriesColour,
    SeriesDetail, SeriesDraft, SeriesEdition, Stream, TourneySeries, BEST_OF_CHOICES,
};
pub use tourney::{CaptainMode, Replacement, TeamLineup, TeamSeed};
pub use tourney::{
    ChatQuote, FactionChoices, FactionResult, FactionStep, FactionVetoConfig, FactionVetoGame,
    MapSpec, MatchFactionVeto, RoundMaps, SwissCuts, SwissTiebreak, TourneyAdmin, TourneyFaction,
};
pub use tourney::{CopySource, CopySourceMaps, MapPick, PickMode, PlanList};
pub use tourney::{
    Draft, DraftPick, FfaConfig, FfaMode, FfaReport, MatchVeto, TeamPoints, VetoChoice, VetoConfig,
    VetoDecider, VetoMode, VetoTeamA, VetoTurn,
};
pub use tourney::{EarlyFinish, Rename, RenameCheck, Survivors, TourneyBan};
pub use tourney::{ImportedGroup, ImportedPlacing, ImportedRow};
pub use tourney::{PendingImage, PickSettings, PlanLists, SwissExtras, TourneyPreset};
pub use tourney::{PickLogEntry, PickMade, PickPhase, Playoffs, StageTwoPlan, TeamRecord};
pub use training::{
    compose_contribution, compose_review_request, compose_url, contribution_problem,
    filter_resources, game_mode_of, hosted_guide, hosted_recording, kind_label, leaderboard_word,
    level_label, normalise_map, official_map_name, own_row, percent_encode, profile_from,
    profile_from_state, recommend, related_resources, review_problem, score, topic_counts,
    topic_label, video_still, within_band, ContributionDraft, ContributionProblem, ForumPost,
    HostedGuide, ReviewProblem, ReviewRequestDraft, Trainer, TrainingCatalogue, TrainingCommand,
    TrainingDocument, TrainingEvent, TrainingKind, TrainingLevel, TrainingLinks, TrainingProfile,
    TrainingQuery, TrainingResource, TrainingSource, TrainingState, TrainingStatus, TrainingTopic,
    FORUM_BASE, OFFICIAL_MAPS, PROFILE_REPLAY_WINDOW, RECOMMENDED_LIMIT,
};
pub use tutorials::{
    tutorials_of, Tutorial, TutorialCategory, TutorialLaunchStatus, TutorialsCommand,
    TutorialsEvent, TutorialsState, TutorialsStatus, TUTORIALS_FEATURED_MOD,
};
pub use uploads::{
    is_safe_folder_name, UploadKind, UploadRequest, UploadStatus, UploadsCommand, UploadsEvent,
    UploadsState,
};

use serde::{Deserialize, Serialize};
use specta::Type;

/// The complete client state. One field per domain slice.
// No `Eq`: `ReplayState` carries an `f32` (vault replay review score).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    pub session: SessionState,
    pub install: InstallState,
    pub auth: AuthState,
    pub nav: NavState,
    pub notifications: NotificationState,
    pub chat: ChatState,
    pub clan: ClanState,
    pub coop: CoopState,
    pub events: EventsState,
    pub lobby: LobbyState,
    pub replays: ReplayState,
    pub maps: MapsState,
    pub map_generator: MapGeneratorState,
    pub mods: ModsState,
    pub leaderboard: LeaderboardState,
    pub player_card: PlayerCardState,
    pub reporting: ReportingState,
    pub reviews: ReviewsState,
    pub social: SocialState,
    pub streams: StreamsState,
    pub tourney: TourneyState,
    pub training: TrainingState,
    pub tutorials: TutorialsState,
    pub uploads: UploadsState,
    pub galactic_war: GalacticWarState,
    pub guides: GuidesState,
    pub client_update: ClientUpdateState,
    pub mac_setup: MacSetupState,
    pub settings: SettingsState,
    pub changelog: ChangelogState,
}

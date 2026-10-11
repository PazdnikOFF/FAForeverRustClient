//! Mac setup slice: everything Forged Alliance needs before it can start on a
//! Mac, prepared by the client itself.
//!
//! Forged Alliance is a 32-bit Windows game and FAF ships no native build of
//! it, so on a Mac it runs under Wine, which itself runs under Rosetta 2 on
//! Apple silicon. The Linux answer to that is a settings field naming a `wine`
//! the player installed themselves. On a Mac the client owns the whole chain
//! instead, in its own data folder, and walks the player through it once:
//!
//! 1. Rosetta 2, the one piece macOS itself has to install.
//! 2. Wine: the CrossOver build that is known to keep a multiplayer game in
//!    sync, pinned by checksum.
//! 3. A Wine prefix for the game, with the DirectX libraries it needs.
//! 4. The connectivity runtime: a private Java runtime for FAF's ICE adapter,
//!    which nobody else on the machine sees.
//! 5. The game itself, downloaded from Steam with the player's own account,
//!    or adopted from a folder the player already has.
//!
//! Each of those is a [`SetupStep`] with a [`StepStatus`]. The statuses are a
//! *derived* fact, re-read from disk by a probe, for the same reason as the
//! `install` slice: "it was there last time" goes stale the moment somebody
//! deletes a folder.
//!
//! The Steam password never enters this slice. It travels in a command as a
//! [`Secret`] straight to the port that hands it to the downloader, and the
//! state only ever learns which question Steam is asking next.

use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

/// One of the things the game needs, in the order the assistant shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum SetupStep {
    Rosetta,
    Wine,
    Prefix,
    Connectivity,
    Game,
}

impl SetupStep {
    /// The order the steps are run in by [`MacSetupCommand::Prepare`]. The game
    /// is not in it: downloading it needs the player's Steam account, so it is
    /// a step of its own that the player starts.
    pub const COMPONENTS: [SetupStep; 4] = [
        SetupStep::Rosetta,
        SetupStep::Wine,
        SetupStep::Prefix,
        SetupStep::Connectivity,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum StepStatus {
    /// Not on disk yet.
    #[default]
    Missing,
    Running,
    Ready,
    Failed {
        reason: String,
    },
}

impl StepStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// How far a long step has got.
///
/// Megabytes rather than bytes because specta refuses 64-bit integers and the
/// game is larger than four gigabytes. `permille` is `None` while the size is
/// not known yet, which the assistant draws as an indeterminate bar.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StepProgress {
    pub step: Option<SetupStep>,
    pub permille: Option<u32>,
    pub done_mb: u32,
    pub total_mb: u32,
}

/// Which kind of Steam Guard code Steam asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum SteamGuardKind {
    /// A code from the Steam Mobile app's authenticator.
    Authenticator,
    /// A code Steam sent by email; `hint` is the masked address Steam names.
    Email { hint: String },
}

/// Where the Steam sign-in and download currently stands.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum SteamPhase {
    /// Nothing running: the sign-in form is what the assistant shows.
    #[default]
    Idle,
    SigningIn,
    /// Steam wants a Steam Guard code. `retry` is true when the previous code
    /// was refused, so the field can say so instead of silently clearing.
    NeedsCode {
        kind: SteamGuardKind,
        retry: bool,
    },
    /// Steam sent a confirmation to the Steam Mobile app and is waiting for it.
    AwaitingApproval,
    /// Signed in; the depots are downloading.
    Downloading,
}

/// What the probe found on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MacSetupProbe {
    /// Whether this client manages a Wine setup at all: true on a Mac only.
    pub supported: bool,
    pub rosetta: bool,
    pub wine: bool,
    pub prefix: bool,
    pub connectivity: bool,
    pub game: bool,
    /// The folder everything lives in, for "Show in Finder".
    pub root_dir: String,
    /// The retail game folder in use, empty while there is none.
    pub game_dir: String,
    /// The Steam account last used to download the game. Not a secret: it is
    /// what fills the sign-in form the next time.
    pub steam_user: String,
    /// Free space where the game would be downloaded, in megabytes. Zero when
    /// it could not be read.
    pub free_mb: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MacSetupState {
    /// See [`MacSetupProbe::supported`]. False everywhere until the first
    /// probe, so no other platform ever draws the assistant.
    pub supported: bool,
    /// Whether a probe has completed, so nothing flashes at startup.
    pub checked: bool,
    pub rosetta: StepStatus,
    pub wine: StepStatus,
    pub prefix: StepStatus,
    pub connectivity: StepStatus,
    pub game: StepStatus,
    /// The step currently running, if any.
    pub active: Option<SetupStep>,
    pub progress: StepProgress,
    pub steam: SteamPhase,
    pub steam_user: String,
    pub root_dir: String,
    pub game_dir: String,
    pub free_mb: u32,
    /// Whether the assistant window is open. In the state rather than in a
    /// component so the settings page, the banner and the first launch all
    /// open the same assistant.
    pub assistant_open: bool,
}

impl MacSetupState {
    pub fn status(&self, step: SetupStep) -> &StepStatus {
        match step {
            SetupStep::Rosetta => &self.rosetta,
            SetupStep::Wine => &self.wine,
            SetupStep::Prefix => &self.prefix,
            SetupStep::Connectivity => &self.connectivity,
            SetupStep::Game => &self.game,
        }
    }

    fn status_mut(&mut self, step: SetupStep) -> &mut StepStatus {
        match step {
            SetupStep::Rosetta => &mut self.rosetta,
            SetupStep::Wine => &mut self.wine,
            SetupStep::Prefix => &mut self.prefix,
            SetupStep::Connectivity => &mut self.connectivity,
            SetupStep::Game => &mut self.game,
        }
    }

    /// Every component is in place, the game included.
    pub fn complete(&self) -> bool {
        self.components_ready() && self.game.is_ready()
    }

    /// Everything but the game is in place.
    pub fn components_ready(&self) -> bool {
        SetupStep::COMPONENTS
            .iter()
            .all(|step| self.status(*step).is_ready())
    }

    /// Something is running: a component install or the Steam download.
    pub fn busy(&self) -> bool {
        self.active.is_some() || self.steam != SteamPhase::Idle
    }
}

/// A value that must never be logged: a password or a one-time code.
///
/// Crosses the boundary as a plain string, and that is the only place it is
/// visible. `Debug` prints a placeholder, so a command logged with `{:?}`
/// anywhere in the runtime cannot leak it.
#[derive(Clone, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(transparent)]
pub struct Secret(pub String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MacSetupEvent {
    Probed {
        probe: MacSetupProbe,
    },
    StepStarted {
        step: SetupStep,
    },
    Progressed {
        progress: StepProgress,
    },
    StepFinished {
        step: SetupStep,
    },
    StepFailed {
        step: SetupStep,
        reason: String,
    },
    /// A step was stopped on purpose: back to missing, nothing to report.
    StepCancelled {
        step: SetupStep,
    },
    SteamPhaseChanged {
        phase: SteamPhase,
    },
    AssistantShown,
    AssistantHidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MacSetupCommand {
    /// Look at what is on disk. Runs at startup and opens the assistant once
    /// when anything is missing.
    Probe,
    /// Install every missing component, in order, stopping at the first that
    /// fails. Rosetta asks for the administrator password through macOS's own
    /// dialog; nothing else needs one.
    Prepare,
    /// Sign in to Steam and download Forged Alliance into the client's folder.
    SteamDownload {
        username: String,
        password: Secret,
    },
    /// Answer a Steam Guard question.
    SteamCode {
        code: Secret,
    },
    /// Stop a Steam download. What was downloaded stays, and the next download
    /// resumes from it.
    CancelSteam,
    /// Stop waiting for the Steam Mobile app and type a Steam Guard code
    /// instead. The sign-in starts over; the password is not asked again.
    SteamUseCode,
    /// Use a Forged Alliance folder the player already has instead.
    UseGameFolder {
        path: String,
    },
    /// Remove the Wine prefix and create a fresh one, for a prefix that was
    /// damaged. The game and Wine itself stay.
    ResetPrefix,
    /// Delete everything the setup put on this Mac: Wine, the prefix, the
    /// runtime and the downloaded game. A folder the player adopted is theirs
    /// and is left alone.
    RemoveEverything,
    /// Write the graphics settings that measured fastest on Apple silicon into
    /// the game's settings. Refused while the game is running, since it
    /// rewrites them on exit. The outcome arrives as a notification.
    OptimizeGraphics,
    ShowAssistant,
    HideAssistant,
}

pub fn reduce(state: &mut MacSetupState, event: &MacSetupEvent) {
    match event {
        MacSetupEvent::Probed { probe } => {
            state.supported = probe.supported;
            state.checked = true;
            state.root_dir = probe.root_dir.clone();
            state.game_dir = probe.game_dir.clone();
            state.free_mb = probe.free_mb;
            if !probe.steam_user.is_empty() {
                state.steam_user = probe.steam_user.clone();
            }
            for (step, present) in [
                (SetupStep::Rosetta, probe.rosetta),
                (SetupStep::Wine, probe.wine),
                (SetupStep::Prefix, probe.prefix),
                (SetupStep::Connectivity, probe.connectivity),
                (SetupStep::Game, probe.game),
            ] {
                // A probe reports what is on disk. It does not overwrite a step
                // that is running, which is not on disk *yet*, nor a failure
                // the player has not seen, which the disk cannot know about.
                let status = state.status_mut(step);
                match (status.clone(), present) {
                    (StepStatus::Running, _) => {}
                    (_, true) => *status = StepStatus::Ready,
                    (StepStatus::Failed { .. }, false) => {}
                    (_, false) => *status = StepStatus::Missing,
                }
            }
        }
        MacSetupEvent::StepStarted { step } => {
            *state.status_mut(*step) = StepStatus::Running;
            state.active = Some(*step);
            state.progress = StepProgress {
                step: Some(*step),
                ..StepProgress::default()
            };
        }
        MacSetupEvent::Progressed { progress } => {
            state.progress = progress.clone();
        }
        MacSetupEvent::StepFinished { step } => {
            *state.status_mut(*step) = StepStatus::Ready;
            if state.active == Some(*step) {
                state.active = None;
                state.progress = StepProgress::default();
            }
        }
        MacSetupEvent::StepFailed { step, reason } => {
            *state.status_mut(*step) = StepStatus::Failed {
                reason: reason.clone(),
            };
            if state.active == Some(*step) {
                state.active = None;
                state.progress = StepProgress::default();
            }
        }
        MacSetupEvent::StepCancelled { step } => {
            *state.status_mut(*step) = StepStatus::Missing;
            if state.active == Some(*step) {
                state.active = None;
                state.progress = StepProgress::default();
            }
        }
        MacSetupEvent::SteamPhaseChanged { phase } => {
            state.steam = phase.clone();
        }
        MacSetupEvent::AssistantShown => state.assistant_open = true,
        MacSetupEvent::AssistantHidden => state.assistant_open = false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe_all(present: bool) -> MacSetupProbe {
        MacSetupProbe {
            supported: true,
            rosetta: present,
            wine: present,
            prefix: present,
            connectivity: present,
            game: present,
            root_dir: "/data".into(),
            game_dir: if present {
                "/data/game".into()
            } else {
                String::new()
            },
            steam_user: String::new(),
            free_mb: 20_000,
        }
    }

    #[test]
    fn unsupported_until_probed_so_other_platforms_never_see_it() {
        let state = MacSetupState::default();
        assert!(!state.supported);
        assert!(!state.checked);
        assert!(!state.complete());
    }

    #[test]
    fn a_probe_marks_what_is_on_disk() {
        let mut state = MacSetupState::default();
        reduce(
            &mut state,
            &MacSetupEvent::Probed {
                probe: probe_all(true),
            },
        );
        assert!(state.checked);
        assert!(state.complete());
        assert_eq!(state.game_dir, "/data/game");
    }

    #[test]
    fn a_probe_leaves_a_running_step_and_an_unseen_failure_alone() {
        let mut state = MacSetupState::default();
        reduce(
            &mut state,
            &MacSetupEvent::StepStarted {
                step: SetupStep::Wine,
            },
        );
        reduce(
            &mut state,
            &MacSetupEvent::StepFailed {
                step: SetupStep::Rosetta,
                reason: "declined".into(),
            },
        );
        reduce(
            &mut state,
            &MacSetupEvent::Probed {
                probe: probe_all(false),
            },
        );
        assert_eq!(state.wine, StepStatus::Running);
        assert!(matches!(state.rosetta, StepStatus::Failed { .. }));
        assert_eq!(state.prefix, StepStatus::Missing);
    }

    #[test]
    fn a_step_runs_reports_progress_and_finishes() {
        let mut state = MacSetupState::default();
        reduce(
            &mut state,
            &MacSetupEvent::StepStarted {
                step: SetupStep::Game,
            },
        );
        assert!(state.busy());
        reduce(
            &mut state,
            &MacSetupEvent::Progressed {
                progress: StepProgress {
                    step: Some(SetupStep::Game),
                    permille: Some(500),
                    done_mb: 2000,
                    total_mb: 4000,
                },
            },
        );
        assert_eq!(state.progress.permille, Some(500));
        reduce(
            &mut state,
            &MacSetupEvent::StepFinished {
                step: SetupStep::Game,
            },
        );
        assert_eq!(state.game, StepStatus::Ready);
        assert_eq!(state.active, None);
        assert_eq!(state.progress, StepProgress::default());
    }

    #[test]
    fn a_secret_never_reaches_a_debug_log() {
        let command = MacSetupCommand::SteamDownload {
            username: "player".into(),
            password: Secret("hunter2".into()),
        };
        let printed = format!("{command:?}");
        assert!(!printed.contains("hunter2"));
        assert!(printed.contains("player"));
    }
}

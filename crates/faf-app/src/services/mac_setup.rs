//! Mac setup orchestration: the order the components go in, and what the
//! assistant shows while they do.
//!
//! The port does the work; this decides that Rosetta comes before Wine, that
//! a failure stops the run rather than building on it, and that the install
//! banner and the game launcher hear about a finished setup.

use std::sync::Arc;

use faf_domain::state::{
    MacSetupCommand, MacSetupEvent, SettingsCommand, SetupStep, SteamPhase, StepProgress,
    StepStatus,
};

use crate::ports::mac_setup::{SteamUpdate, StepUpdate, STEAM_CANCELLED};
use crate::runtime::{EventSink, ServiceCtx};

pub async fn handle(cmd: MacSetupCommand, ctx: &ServiceCtx, out: &EventSink) {
    match cmd {
        MacSetupCommand::Probe => probe(ctx, out).await,
        MacSetupCommand::Prepare => prepare(ctx, out).await,
        MacSetupCommand::SteamDownload { username, password } => {
            steam_download(ctx, out, username, password.expose().to_string()).await
        }
        MacSetupCommand::SteamCode { code } => {
            if ctx
                .ports
                .mac_setup
                .steam_code(code.expose().to_string())
                .await
            {
                out.emit(MacSetupEvent::SteamPhaseChanged {
                    phase: SteamPhase::SigningIn,
                });
            }
        }
        MacSetupCommand::CancelSteam => ctx.ports.mac_setup.cancel_steam().await,
        MacSetupCommand::SteamUseCode => {
            ctx.ports.mac_setup.steam_use_code().await;
        }
        MacSetupCommand::UseGameFolder { path } => use_game_folder(ctx, out, path).await,
        MacSetupCommand::ResetPrefix => reset_prefix(ctx, out).await,
        MacSetupCommand::RemoveEverything => remove_everything(ctx, out).await,
        MacSetupCommand::OptimizeGraphics => optimize_graphics(ctx, out).await,
        MacSetupCommand::ShowAssistant => out.emit(MacSetupEvent::AssistantShown),
        MacSetupCommand::HideAssistant => out.emit(MacSetupEvent::AssistantHidden),
    }
}

/// At startup: look, and when anything is missing open the assistant and
/// start installing what can be installed without asking anything.
///
/// The game is the one thing left for the player, because it needs their
/// Steam account or a folder only they know about.
pub fn spawn(ctx: Arc<ServiceCtx>, sink: EventSink) {
    tokio::spawn(async move {
        // Silent where there is nothing to set up: the slice's defaults
        // already say "unsupported", and an event nobody draws would only be
        // noise in every other platform's event stream.
        let found = ctx.ports.mac_setup.probe().await;
        if !found.supported {
            return;
        }
        sink.emit(MacSetupEvent::Probed { probe: found });
        ctx.ports.mac_setup.optimize_graphics_once().await;
        let (complete, components_ready) = sink.with_state(|state| {
            (
                state.mac_setup.complete(),
                state.mac_setup.components_ready(),
            )
        });
        if complete {
            return;
        }
        sink.emit(MacSetupEvent::AssistantShown);
        if !components_ready {
            crate::runtime::run_command(MacSetupCommand::Prepare.into(), &ctx, &sink).await;
        }
    });
}

async fn probe(ctx: &ServiceCtx, out: &EventSink) {
    let probe = ctx.ports.mac_setup.probe().await;
    out.emit(MacSetupEvent::Probed { probe });
}

/// After anything that changes what the launcher can find: look again, and
/// let the install slice re-check, which is what the install banner and the
/// play buttons read.
async fn settle(ctx: &ServiceCtx, out: &EventSink) {
    probe(ctx, out).await;
    crate::runtime::run_command(SettingsCommand::CheckInstalls.into(), ctx, out).await;
}

async fn prepare(ctx: &ServiceCtx, out: &EventSink) {
    crate::runtime::expect_admitted(crate::runtime::Key::MacSetup);
    probe(ctx, out).await;
    if !out.with_state(|state| state.mac_setup.supported) {
        return;
    }
    for step in SetupStep::COMPONENTS {
        let ready = out.with_state(|state| state.mac_setup.status(step).is_ready());
        if ready {
            continue;
        }
        if !run_step(ctx, out, step).await {
            break;
        }
    }
    settle(ctx, out).await;
}

/// Run one component, mirroring its progress. False when it failed.
async fn run_step(ctx: &ServiceCtx, out: &EventSink, step: SetupStep) -> bool {
    out.emit(MacSetupEvent::StepStarted { step });
    let mut updates = ctx.ports.mac_setup.install(step).await;
    while let Some(update) = updates.recv().await {
        match update {
            StepUpdate::Progress {
                permille,
                done_mb,
                total_mb,
            } => out.emit(MacSetupEvent::Progressed {
                progress: StepProgress {
                    step: Some(step),
                    permille,
                    done_mb,
                    total_mb,
                },
            }),
            StepUpdate::Finished(Ok(())) => {
                out.emit(MacSetupEvent::StepFinished { step });
                return true;
            }
            StepUpdate::Finished(Err(reason)) => {
                out.emit(MacSetupEvent::StepFailed { step, reason });
                return false;
            }
        }
    }
    // The port always ends with `Finished`; a stream that closes without one
    // means its task died, and the bar must not wait for it forever.
    out.emit(MacSetupEvent::StepFailed {
        step,
        reason: "the installation stopped without finishing".into(),
    });
    false
}

async fn steam_download(ctx: &ServiceCtx, out: &EventSink, username: String, password: String) {
    crate::runtime::expect_admitted(crate::runtime::Key::MacSetupSteam);
    if out.with_state(|state| state.mac_setup.steam != SteamPhase::Idle) {
        return;
    }
    out.emit(MacSetupEvent::StepStarted {
        step: SetupStep::Game,
    });
    out.emit(MacSetupEvent::SteamPhaseChanged {
        phase: SteamPhase::SigningIn,
    });
    let mut updates = ctx.ports.mac_setup.steam_download(username, password).await;
    let mut finished = false;
    while let Some(update) = updates.recv().await {
        match update {
            SteamUpdate::NeedsCode { kind, retry } => out.emit(MacSetupEvent::SteamPhaseChanged {
                phase: SteamPhase::NeedsCode { kind, retry },
            }),
            SteamUpdate::AwaitingApproval => out.emit(MacSetupEvent::SteamPhaseChanged {
                phase: SteamPhase::AwaitingApproval,
            }),
            SteamUpdate::Restarting => out.emit(MacSetupEvent::SteamPhaseChanged {
                phase: SteamPhase::SigningIn,
            }),
            SteamUpdate::Downloading {
                permille,
                done_mb,
                total_mb,
            } => {
                let phase_changed =
                    out.with_state(|state| state.mac_setup.steam != SteamPhase::Downloading);
                if phase_changed {
                    out.emit(MacSetupEvent::SteamPhaseChanged {
                        phase: SteamPhase::Downloading,
                    });
                }
                out.emit(MacSetupEvent::Progressed {
                    progress: StepProgress {
                        step: Some(SetupStep::Game),
                        permille,
                        done_mb,
                        total_mb,
                    },
                });
            }
            SteamUpdate::Finished(outcome) => {
                finished = true;
                match outcome {
                    Ok(()) => out.emit(MacSetupEvent::StepFinished {
                        step: SetupStep::Game,
                    }),
                    Err(reason) if reason == STEAM_CANCELLED => {
                        out.emit(MacSetupEvent::StepCancelled {
                            step: SetupStep::Game,
                        })
                    }
                    Err(reason) => out.emit(MacSetupEvent::StepFailed {
                        step: SetupStep::Game,
                        reason,
                    }),
                }
                break;
            }
        }
    }
    if !finished {
        out.emit(MacSetupEvent::StepFailed {
            step: SetupStep::Game,
            reason: "the download stopped without finishing".into(),
        });
    }
    out.emit(MacSetupEvent::SteamPhaseChanged {
        phase: SteamPhase::Idle,
    });
    settle(ctx, out).await;
}

async fn use_game_folder(ctx: &ServiceCtx, out: &EventSink, path: String) {
    crate::runtime::expect_admitted(crate::runtime::Key::MacSetupSteam);
    if out.with_state(|state| state.mac_setup.steam != SteamPhase::Idle) {
        return;
    }
    match ctx.ports.mac_setup.use_game_folder(path).await {
        Ok(_) => out.emit(MacSetupEvent::StepFinished {
            step: SetupStep::Game,
        }),
        Err(reason) => out.emit(MacSetupEvent::StepFailed {
            step: SetupStep::Game,
            reason,
        }),
    }
    settle(ctx, out).await;
}

async fn reset_prefix(ctx: &ServiceCtx, out: &EventSink) {
    crate::runtime::expect_admitted(crate::runtime::Key::MacSetup);
    if out.with_state(|state| state.mac_setup.status(SetupStep::Prefix) == &StepStatus::Running) {
        return;
    }
    if let Err(reason) = ctx.ports.mac_setup.reset_prefix().await {
        out.emit(MacSetupEvent::StepFailed {
            step: SetupStep::Prefix,
            reason,
        });
        return;
    }
    probe(ctx, out).await;
    if run_step(ctx, out, SetupStep::Prefix).await {
        settle(ctx, out).await;
    }
}

async fn remove_everything(ctx: &ServiceCtx, out: &EventSink) {
    crate::runtime::expect_admitted(crate::runtime::Key::MacSetup);
    if out.with_state(|state| state.mac_setup.active.is_some()) {
        return;
    }
    // Each step reported as missing again, rather than a probe alone, which
    // would keep a failure from before on screen.
    for step in SetupStep::COMPONENTS.into_iter().chain([SetupStep::Game]) {
        out.emit(MacSetupEvent::StepCancelled { step });
    }
    if let Err(reason) = ctx.ports.mac_setup.remove_everything().await {
        tracing::warn!(%reason, "could not remove the Mac setup");
    }
    settle(ctx, out).await;
}

async fn optimize_graphics(ctx: &ServiceCtx, out: &EventSink) {
    use crate::services::notifications::{add_required_text, Text};
    use faf_domain::state::NotificationKind;

    let outcome = if ctx.ports.process.game_running() {
        Err("Forged Alliance is running, and it saves its own settings over these when it exits: close the game, then try again".to_string())
    } else {
        ctx.ports.mac_setup.optimize_graphics().await
    };
    match outcome {
        Ok(changed) => add_required_text(
            out,
            NotificationKind::GameInstall,
            Text::new("notifications.msg.macGraphics").with("count", changed.to_string()),
            "Graphics optimised for this Mac",
            format!(
                "{changed} settings changed: shadows, vertical sync, antialiasing and bloom are off. \
                 They apply the next time the game starts."
            ),
            None,
        ),
        Err(reason) => add_required_text(
            out,
            NotificationKind::GameInstall,
            Text::new("notifications.msg.macGraphicsFailed").with("reason", &reason),
            "The graphics settings were not changed",
            reason,
            None,
        ),
    }
}

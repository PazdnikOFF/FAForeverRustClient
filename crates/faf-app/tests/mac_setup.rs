//! Mac setup service tests, through a scripted [`MacSetupPort`].
//!
//! What the service owns is the order: components one after another, a
//! failure stopping the run instead of building on it, a cancelled download
//! reading as "not there" rather than as an error. None of that needs Wine.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use faf_app::infra::fake_ports;
use faf_app::ports::mac_setup::{SteamUpdate, StepUpdate, STEAM_CANCELLED};
use faf_app::ports::MacSetupPort;
use faf_app::{App, Ports};
use faf_domain::state::{
    MacSetupCommand, MacSetupProbe, Secret, SetupStep, SteamGuardKind, SteamPhase, StepStatus,
};
use tokio::sync::mpsc;

/// Installs succeed except for the step named in `fail`; the Steam download
/// asks for an email code and then reports what `steam_outcome` says.
struct Scripted {
    installed: Mutex<Vec<SetupStep>>,
    fail: Option<SetupStep>,
    steam_outcome: Result<(), String>,
    codes: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(fail: Option<SetupStep>, steam_outcome: Result<(), String>) -> Arc<Self> {
        Arc::new(Self {
            installed: Mutex::new(Vec::new()),
            fail,
            steam_outcome,
            codes: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl MacSetupPort for Scripted {
    async fn probe(&self) -> MacSetupProbe {
        let installed = self.installed.lock().unwrap();
        MacSetupProbe {
            supported: true,
            rosetta: true,
            wine: installed.contains(&SetupStep::Wine),
            prefix: installed.contains(&SetupStep::Prefix),
            connectivity: installed.contains(&SetupStep::Connectivity),
            game: false,
            root_dir: "/data/macOS".into(),
            ..MacSetupProbe::default()
        }
    }

    async fn install(&self, step: SetupStep) -> mpsc::Receiver<StepUpdate> {
        let (tx, rx) = mpsc::channel(4);
        let outcome = if self.fail == Some(step) {
            Err(format!("{step:?} broke"))
        } else {
            self.installed.lock().unwrap().push(step);
            Ok(())
        };
        tx.send(StepUpdate::Progress {
            permille: Some(500),
            done_mb: 1,
            total_mb: 2,
        })
        .await
        .unwrap();
        tx.send(StepUpdate::Finished(outcome)).await.unwrap();
        rx
    }

    async fn steam_download(
        &self,
        username: String,
        password: String,
    ) -> mpsc::Receiver<SteamUpdate> {
        assert_eq!(username, "player");
        assert_eq!(password, "hunter2");
        let (tx, rx) = mpsc::channel(4);
        tx.send(SteamUpdate::NeedsCode {
            kind: SteamGuardKind::Email {
                hint: "p***@x".into(),
            },
            retry: false,
        })
        .await
        .unwrap();
        tx.send(SteamUpdate::Finished(self.steam_outcome.clone()))
            .await
            .unwrap();
        rx
    }

    async fn steam_code(&self, code: String) -> bool {
        self.codes.lock().unwrap().push(code);
        true
    }

    async fn steam_use_code(&self) -> bool {
        true
    }

    async fn cancel_steam(&self) {}

    async fn use_game_folder(&self, path: String) -> Result<String, String> {
        Ok(path)
    }

    async fn reset_prefix(&self) -> Result<(), String> {
        Ok(())
    }

    async fn remove_everything(&self) -> Result<(), String> {
        Ok(())
    }

    async fn optimize_graphics(&self) -> Result<u32, String> {
        Ok(0)
    }

    async fn optimize_graphics_once(&self) {}
}

fn app_with(port: Arc<Scripted>) -> App {
    let ports = Ports {
        mac_setup: port,
        ..fake_ports()
    };
    let (app, app_loop) = App::new("test", ports);
    tokio::spawn(app_loop.run());
    app
}

#[tokio::test]
async fn components_install_in_order_skipping_what_is_there() {
    let port = Scripted::new(None, Ok(()));
    let app = app_with(port.clone());
    app.dispatch_and_wait(MacSetupCommand::Prepare.into())
        .await
        .unwrap();

    assert_eq!(
        *port.installed.lock().unwrap(),
        vec![SetupStep::Wine, SetupStep::Prefix, SetupStep::Connectivity],
        "Rosetta was already there, so it is skipped; the rest go in order"
    );
    let setup = app.snapshot().mac_setup;
    assert!(setup.components_ready());
    assert_eq!(setup.active, None);
    assert!(
        !setup.complete(),
        "the game is still the player's to choose"
    );
}

#[tokio::test]
async fn a_failed_component_stops_the_run_and_says_why() {
    let port = Scripted::new(Some(SetupStep::Prefix), Ok(()));
    let app = app_with(port.clone());
    app.dispatch_and_wait(MacSetupCommand::Prepare.into())
        .await
        .unwrap();

    assert_eq!(*port.installed.lock().unwrap(), vec![SetupStep::Wine]);
    let setup = app.snapshot().mac_setup;
    assert_eq!(
        setup.prefix,
        StepStatus::Failed {
            reason: "Prefix broke".into()
        }
    );
    assert_eq!(setup.connectivity, StepStatus::Missing, "never attempted");
}

#[tokio::test]
async fn a_cancelled_steam_download_reads_as_missing_not_failed() {
    let port = Scripted::new(None, Err(STEAM_CANCELLED.into()));
    let app = app_with(port);
    app.dispatch_and_wait(
        MacSetupCommand::SteamDownload {
            username: "player".into(),
            password: Secret("hunter2".into()),
        }
        .into(),
    )
    .await
    .unwrap();

    let setup = app.snapshot().mac_setup;
    assert_eq!(setup.game, StepStatus::Missing);
    assert_eq!(setup.steam, SteamPhase::Idle);
}

#[tokio::test]
async fn a_failed_steam_download_keeps_the_reason_and_frees_the_form() {
    let port = Scripted::new(
        None,
        Err("Steam did not accept that account name and password".into()),
    );
    let app = app_with(port);
    app.dispatch_and_wait(
        MacSetupCommand::SteamDownload {
            username: "player".into(),
            password: Secret("hunter2".into()),
        }
        .into(),
    )
    .await
    .unwrap();

    let setup = app.snapshot().mac_setup;
    assert_eq!(
        setup.game,
        StepStatus::Failed {
            reason: "Steam did not accept that account name and password".into()
        }
    );
    assert_eq!(setup.steam, SteamPhase::Idle);
}

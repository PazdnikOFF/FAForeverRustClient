//! The Mac setup: everything Forged Alliance needs, in one client-owned folder.
//!
//! ```text
//! <data dir>/macOS/
//!   wine/Wine Crossover.app   Wine, run under Rosetta 2
//!   prefix/                   the Wine prefix the game runs in
//!   runtime/jre/              a private Java runtime for the ICE adapter
//!   runtime/ice/              faf-ice-adapter.jar and the JavaFX it needs
//!   game/                     Forged Alliance as downloaded from Steam
//!   tools/                    the Steam downloader
//!   downloads/                archives on their way in, removed once used
//! ```
//!
//! Nothing is written outside this folder, nothing goes into `/Applications`
//! and nothing asks for Homebrew. The one exception is Rosetta 2, which is
//! part of macOS and can only be installed by macOS, through its own
//! administrator prompt.
//!
//! Inside the application bundle would have been the other obvious place, and
//! it is the wrong one: writing into a signed bundle invalidates its
//! signature, a bundle in `/Applications` is not writable by a standard user,
//! and a client update replaces the bundle wholesale, which would take a four
//! gigabyte game with it.
//!
//! Each component carries a marker file naming the pinned version it was made
//! from, so a component left half-written by a crash or a full disk reads as
//! missing and is made again, and a future pin bump replaces it.

mod connectivity;
mod download;
mod focus;
mod graphics;
mod steam;
mod wine;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use faf_domain::state::{MacSetupProbe, SetupStep};
use tokio::process::Command;
use tokio::sync::{mpsc, Mutex};

use crate::ports::mac_setup::{MacSetupPort, SteamUpdate, StepUpdate};

/// The marker a finished component writes beside itself.
const MARKER: &str = ".faf-component";

/// Where everything lives. Resolved once; see the module docs for the layout.
#[derive(Debug, Clone)]
pub(crate) struct Layout {
    root: PathBuf,
}

impl Layout {
    pub(crate) fn resolve() -> Option<Self> {
        crate::infra::data_dir().ok().map(|data| Self {
            root: data.join("macOS"),
        })
    }

    #[cfg(test)]
    fn at(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    fn wine_dir(&self) -> PathBuf {
        self.root.join("wine")
    }
    fn wine_app(&self) -> PathBuf {
        self.wine_dir().join("Wine Crossover.app")
    }
    fn wine_bin(&self) -> PathBuf {
        self.wine_app().join("Contents/Resources/wine/bin")
    }
    pub(crate) fn wine64(&self) -> PathBuf {
        self.wine_bin().join("wine64")
    }
    fn wineserver(&self) -> PathBuf {
        self.wine_bin().join("wineserver")
    }
    pub(crate) fn prefix(&self) -> PathBuf {
        self.root.join("prefix")
    }
    fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }
    pub(crate) fn java(&self) -> PathBuf {
        self.runtime_dir().join("jre/bin/java")
    }
    pub(crate) fn ice_jar(&self) -> PathBuf {
        self.runtime_dir().join("ice/faf-ice-adapter.jar")
    }
    fn default_game_dir(&self) -> PathBuf {
        self.root.join("game")
    }
    fn game_dir_record(&self) -> PathBuf {
        self.root.join("game-folder.txt")
    }
    fn steam_user_record(&self) -> PathBuf {
        self.root.join("steam-user.txt")
    }
    fn tools_dir(&self) -> PathBuf {
        self.root.join("tools")
    }
    fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }

    /// The retail game folder in use: one the player pointed at, else the
    /// client's own download.
    pub(crate) fn game_dir(&self) -> PathBuf {
        std::fs::read_to_string(self.game_dir_record())
            .ok()
            .map(|text| PathBuf::from(text.trim()))
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| self.default_game_dir())
    }

    fn steam_user(&self) -> String {
        std::fs::read_to_string(self.steam_user_record())
            .map(|text| text.trim().to_string())
            .unwrap_or_default()
    }
}

/// Write `version` as the marker of a finished component.
fn mark(dir: &Path, version: &str) -> Result<(), String> {
    std::fs::write(dir.join(MARKER), version)
        .map_err(|error| format!("could not record the finished install: {error}"))
}

/// Whether `dir` holds a finished component of exactly `version`.
fn marked(dir: &Path, version: &str) -> bool {
    std::fs::read_to_string(dir.join(MARKER)).is_ok_and(|text| text.trim() == version)
}

/// Whether `dir` is a Forged Alliance install: the same probe file the game
/// updater validates a retail folder with, plus the executable.
pub(crate) fn is_game_dir(dir: &Path) -> bool {
    dir.join("gamedata").join("lua.scd").is_file()
        && dir.join("bin").join("SupremeCommander.exe").is_file()
}

/// The FA root for a path the player picked: the folder itself, its parent
/// when they picked `bin`, or the grandparent when they picked an executable.
fn game_root_of(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .take(3)
        .find(|candidate| is_game_dir(candidate))
        .map(Path::to_path_buf)
}

/// The managed pieces other adapters fall back to. All `None` off a Mac, and
/// each `None` until its component is installed, so a half-made setup is
/// never handed to the launcher.
pub(crate) fn managed_wine() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let layout = Layout::resolve()?;
    (marked(&layout.wine_dir(), wine::WINE_VERSION) && layout.wine64().is_file())
        .then(|| layout.wine64())
}

pub(crate) fn managed_prefix() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let layout = Layout::resolve()?;
    marked(&layout.prefix(), wine::PREFIX_VERSION).then(|| layout.prefix())
}

pub(crate) fn managed_java() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let layout = Layout::resolve()?;
    (marked(&layout.runtime_dir(), connectivity::RUNTIME_VERSION) && layout.java().is_file())
        .then(|| layout.java())
}

pub(crate) fn managed_ice_jar() -> Option<PathBuf> {
    managed_java()?;
    let layout = Layout::resolve()?;
    layout.ice_jar().is_file().then(|| layout.ice_jar())
}

pub(crate) fn managed_game_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let dir = Layout::resolve()?.game_dir();
    is_game_dir(&dir).then_some(dir)
}

/// Give the game the Mac graphics defaults, once.
///
/// Runs at startup and whenever a game exits, and does something only the
/// first time a `game.prefs` exists to change: from then on the settings are
/// the player's, and a choice they made in the game's own options is not
/// undone behind their back. The marker lives in the client's folder, so
/// removing everything starts over. Never while the game is running, which
/// is why the launcher calls it only after the process has gone.
pub(crate) fn optimize_graphics_once() {
    if !cfg!(target_os = "macos") || managed_wine().is_none() {
        return;
    }
    let Some(layout) = Layout::resolve() else {
        return;
    };
    let marker = layout.root().join(".graphics-optimized");
    // The marker holds how many releases of defaults are already applied; a
    // marker from before it held a number held "1", which reads the same.
    let applied: u32 = std::fs::read_to_string(&marker)
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0);
    if applied >= graphics::VERSION {
        return;
    }
    let prefs = crate::infra::mods::game_prefs_path();
    if !prefs.is_file() {
        return;
    }
    match graphics::optimize_since(&prefs, applied) {
        Ok(changed) => {
            tracing::info!(changed, "applied the Mac graphics defaults");
            let _ = std::fs::write(marker, graphics::VERSION.to_string());
        }
        Err(reason) => tracing::warn!(%reason, "could not apply the Mac graphics defaults"),
    }
}

/// Where `fa-window.exe` is: the build's own helper, beside the client or in
/// its resources. `FAF_FA_WINDOW_HELPER` overrides, and is what the shell sets
/// for a packaged build.
fn window_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("FAF_FA_WINDOW_HELPER").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }
    crate::infra::helper_search_roots()
        .into_iter()
        .flat_map(|root| {
            [
                root.join("natives").join("fa-window.exe"),
                root.join("resources").join("natives").join("fa-window.exe"),
            ]
        })
        .find(|candidate| candidate.is_file())
}

/// Start `fa-window.exe` beside a game that runs through the client's Wine.
///
/// It makes a windowed game borderless across its monitor and keeps the
/// cursor in the game while the game has the focus; see
/// `natives-src/fa-window/main.rs`. It finds the game itself and exits with
/// it, so nothing here waits for it. The environment matches the game's:
/// two processes in one prefix must agree on MSync, or they disagree about
/// every lock they share.
pub(crate) fn start_window_helper(wine: &Path, prefix: &str, msync: bool) {
    let Some(helper) = window_helper() else {
        tracing::info!("no fa-window helper in this build; the game keeps its own window");
        return;
    };
    let mut command = Command::new(wine);
    command
        .arg(&helper)
        .env("WINEDEBUG", "-all")
        .env("WINEMSYNC", if msync { "1" } else { "0" })
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if !prefix.is_empty() {
        command.env("WINEPREFIX", prefix);
    }
    match command.spawn() {
        Ok(_) => tracing::info!("started the borderless window helper"),
        Err(error) => tracing::warn!(%error, "could not start the borderless window helper"),
    }
    // The helper restores the window inside Wine; only the client, as the
    // active application, can make macOS bring it to the front.
    focus::hand_focus_to_game();
}

/// Whether Rosetta 2 can run an Intel binary right now. On an Intel Mac there
/// is nothing to translate, and the same check passes natively.
async fn rosetta_ready() -> bool {
    Command::new("/usr/bin/arch")
        .args(["-x86_64", "/usr/bin/true"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// Free space on the volume `path` is on, in megabytes, from `df`. Zero when
/// it cannot be read; the assistant then simply does not warn.
async fn free_megabytes(path: &Path) -> u32 {
    let Some(existing) = path.ancestors().find(|candidate| candidate.exists()) else {
        return 0;
    };
    let Ok(output) = Command::new("/bin/df")
        .arg("-Pm")
        .arg(existing)
        .output()
        .await
    else {
        return 0;
    };
    parse_df_available(&String::from_utf8_lossy(&output.stdout)).unwrap_or(0)
}

fn parse_df_available(text: &str) -> Option<u32> {
    let line = text.lines().nth(1)?;
    line.split_whitespace().nth(3)?.parse().ok()
}

/// The real adapter. Holds the running Steam download so a Steam Guard code
/// and a cancel can reach it from other commands.
pub struct MacSetup {
    layout: Option<Layout>,
    http: reqwest::Client,
    steam: Arc<Mutex<Option<steam::Running>>>,
}

impl MacSetup {
    pub fn faf() -> Self {
        Self {
            layout: Layout::resolve(),
            http: super::http::shared_http_client(),
            steam: Arc::new(Mutex::new(None)),
        }
    }

    fn layout(&self) -> Result<Layout, String> {
        self.layout
            .clone()
            .ok_or_else(|| "could not resolve the client's data folder".to_string())
    }
}

#[async_trait]
impl MacSetupPort for MacSetup {
    async fn probe(&self) -> MacSetupProbe {
        let Some(layout) = self.layout.clone().filter(|_| cfg!(target_os = "macos")) else {
            return MacSetupProbe::default();
        };
        let game_dir = layout.game_dir();
        let game = is_game_dir(&game_dir);
        MacSetupProbe {
            supported: true,
            rosetta: rosetta_ready().await,
            wine: marked(&layout.wine_dir(), wine::WINE_VERSION) && layout.wine64().is_file(),
            prefix: marked(&layout.prefix(), wine::PREFIX_VERSION) && wine::tuned(&layout.prefix()),
            connectivity: marked(&layout.runtime_dir(), connectivity::RUNTIME_VERSION)
                && layout.java().is_file()
                && layout.ice_jar().is_file(),
            game,
            root_dir: layout.root().display().to_string(),
            game_dir: if game {
                game_dir.display().to_string()
            } else {
                String::new()
            },
            steam_user: layout.steam_user(),
            free_mb: free_megabytes(layout.root()).await,
        }
    }

    async fn install(&self, step: SetupStep) -> mpsc::Receiver<StepUpdate> {
        let (tx, rx) = mpsc::channel(32);
        let layout = self.layout();
        let http = self.http.clone();
        // In a task so the receiver reaches the caller before the first
        // progress report: see `galactic_war::install` for why that matters.
        tokio::spawn(async move {
            let outcome = match layout {
                Err(reason) => Err(reason),
                Ok(layout) => match step {
                    SetupStep::Rosetta => wine::install_rosetta().await,
                    SetupStep::Wine => wine::install_wine(&layout, &http, &tx).await,
                    SetupStep::Prefix => wine::install_prefix(&layout, &http, &tx).await,
                    SetupStep::Connectivity => connectivity::install(&layout, &http, &tx).await,
                    SetupStep::Game => {
                        Err("the game is downloaded from Steam, not installed here".into())
                    }
                },
            };
            if let Err(reason) = &outcome {
                tracing::warn!(?step, %reason, "a Mac setup step failed");
            } else {
                tracing::info!(?step, "a Mac setup step finished");
            }
            let _ = tx.send(StepUpdate::Finished(outcome)).await;
        });
        rx
    }

    async fn steam_download(
        &self,
        username: String,
        password: String,
    ) -> mpsc::Receiver<SteamUpdate> {
        let (tx, rx) = mpsc::channel(32);
        match self.layout() {
            Err(reason) => {
                let _ = tx.send(SteamUpdate::Finished(Err(reason))).await;
            }
            Ok(layout) => {
                steam::start(
                    layout,
                    self.http.clone(),
                    self.steam.clone(),
                    username,
                    password,
                    tx,
                );
            }
        }
        rx
    }

    async fn steam_code(&self, code: String) -> bool {
        match self.steam.lock().await.as_mut() {
            Some(running) => running.answer(&code).await,
            None => false,
        }
    }

    async fn steam_use_code(&self) -> bool {
        match self.steam.lock().await.as_mut() {
            Some(running) => running.use_code(),
            None => false,
        }
    }

    async fn cancel_steam(&self) {
        if let Some(running) = self.steam.lock().await.as_mut() {
            running.cancel();
        }
    }

    async fn use_game_folder(&self, path: String) -> Result<String, String> {
        let layout = self.layout()?;
        let root = game_root_of(Path::new(path.trim())).ok_or_else(|| {
            "that folder is not a Supreme Commander: Forged Alliance install: it needs gamedata and bin/SupremeCommander.exe".to_string()
        })?;
        std::fs::create_dir_all(layout.root())
            .map_err(|error| format!("could not create {}: {error}", layout.root().display()))?;
        std::fs::write(layout.game_dir_record(), root.display().to_string())
            .map_err(|error| format!("could not remember the game folder: {error}"))?;
        Ok(root.display().to_string())
    }

    async fn reset_prefix(&self) -> Result<(), String> {
        let layout = self.layout()?;
        wine::remove_prefix(&layout).await
    }

    async fn optimize_graphics(&self) -> Result<u32, String> {
        let changed = graphics::optimize(&crate::infra::mods::game_prefs_path())?;
        if let Ok(layout) = self.layout() {
            let _ = std::fs::write(
                layout.root().join(".graphics-optimized"),
                graphics::VERSION.to_string(),
            );
        }
        Ok(changed)
    }

    async fn optimize_graphics_once(&self) {
        // Called at startup, before this client can have launched anything.
        optimize_graphics_once();
    }

    async fn remove_everything(&self) -> Result<(), String> {
        let layout = self.layout()?;
        if let Some(running) = self.steam.lock().await.as_mut() {
            running.cancel();
        }
        wine::remove_prefix(&layout).await?;
        match tokio::fs::remove_dir_all(layout.root()).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "could not remove {}: {error}",
                layout.root().display()
            )),
        }
    }
}

/// Off a Mac, and in tests: nothing is supported, nothing is installed, and
/// every action says so.
pub struct NoMacSetup;

#[async_trait]
impl MacSetupPort for NoMacSetup {
    async fn probe(&self) -> MacSetupProbe {
        MacSetupProbe::default()
    }

    async fn install(&self, _step: SetupStep) -> mpsc::Receiver<StepUpdate> {
        let (tx, rx) = mpsc::channel(1);
        let _ = tx
            .send(StepUpdate::Finished(Err(
                "the Mac setup is only available on macOS".into(),
            )))
            .await;
        rx
    }

    async fn steam_download(
        &self,
        _username: String,
        _password: String,
    ) -> mpsc::Receiver<SteamUpdate> {
        let (tx, rx) = mpsc::channel(1);
        let _ = tx
            .send(SteamUpdate::Finished(Err(
                "the Mac setup is only available on macOS".into(),
            )))
            .await;
        rx
    }

    async fn steam_code(&self, _code: String) -> bool {
        false
    }

    async fn steam_use_code(&self) -> bool {
        false
    }

    async fn cancel_steam(&self) {}

    async fn use_game_folder(&self, _path: String) -> Result<String, String> {
        Err("the Mac setup is only available on macOS".into())
    }

    async fn reset_prefix(&self) -> Result<(), String> {
        Err("the Mac setup is only available on macOS".into())
    }

    async fn remove_everything(&self) -> Result<(), String> {
        Err("the Mac setup is only available on macOS".into())
    }

    async fn optimize_graphics(&self) -> Result<u32, String> {
        Err("the Mac setup is only available on macOS".into())
    }

    async fn optimize_graphics_once(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_game(dir: &Path) {
        std::fs::create_dir_all(dir.join("gamedata")).unwrap();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("gamedata/lua.scd"), b"x").unwrap();
        std::fs::write(dir.join("bin/SupremeCommander.exe"), b"x").unwrap();
    }

    #[test]
    fn a_picked_bin_folder_or_executable_resolves_to_the_game_root() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("Forged Alliance");
        fake_game(&game);
        assert_eq!(game_root_of(&game), Some(game.clone()));
        assert_eq!(game_root_of(&game.join("bin")), Some(game.clone()));
        assert_eq!(
            game_root_of(&game.join("bin/SupremeCommander.exe")),
            Some(game.clone())
        );
        assert_eq!(game_root_of(temp.path()), None);
    }

    #[test]
    fn a_component_counts_only_with_the_marker_of_its_pinned_version() {
        let temp = tempfile::tempdir().unwrap();
        assert!(!marked(temp.path(), "1"));
        mark(temp.path(), "1").unwrap();
        assert!(marked(temp.path(), "1"));
        assert!(!marked(temp.path(), "2"), "a pin bump replaces it");
    }

    #[test]
    fn the_game_folder_defaults_to_the_clients_own_download() {
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::at(temp.path());
        assert_eq!(layout.game_dir(), temp.path().join("game"));
        std::fs::write(layout.game_dir_record(), "/Volumes/Games/FA\n").unwrap();
        assert_eq!(layout.game_dir(), PathBuf::from("/Volumes/Games/FA"));
    }

    #[test]
    fn df_output_yields_the_available_column() {
        let text = "Filesystem 1048576-blocks Used Available Capacity Mounted on\n\
                    /dev/disk3s5 948000 500000 448000 53% /System/Volumes/Data\n";
        assert_eq!(parse_df_available(text), Some(448_000));
        assert_eq!(parse_df_available(""), None);
    }
}

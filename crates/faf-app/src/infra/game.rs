//! Real game launcher: runs `ForgedAlliance.exe`.
//!
//! Builds the FA command line the way the Python client's `fa/play.py`
//! `build_argument_list` does: the server-provided launch args, then
//! `/init init_<mod>.lua`, `/nobugreport`, and `/gpgnet 127.0.0.1:<port>`. The
//! working directory is the executable's folder (where `init_<mod>.lua` lives).
//!
//! Two separate installs, two separate paths: mirrors the Python client's
//! `GameProcess` (live join) vs `ReplayProcess` (replay playback, a distinct
//! `REPLAYDATA_DIR/bin` install so replays can run a different FA
//! build/version than live games). [`GameConfig::game_path`] (`FAF_GAME_PATH`)
//! is used for [`ProcessPort::launch_game`]; [`GameConfig::replay_game_path`]
//! (`FAF_REPLAY_GAME_PATH`) for [`ProcessPort::launch_replay`]: they are
//! never interchanged, even if only one is set.
//!
//! Authenticated sessions use this launcher by default. Explicit offline/test
//! sessions inject [`FakeGame`] so the rest of the app remains install-free.
//!
//! Known gaps (later phases, see the plan): no replay server (`/savereplay`
//! omitted), no client-derived rating args, and no game-bin/init staging: the
//! FAF `init_<mod>.lua` must already be present beside the executable.

use std::collections::HashSet;
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use std::time::Duration;

use async_trait::async_trait;
use tokio::process::{Child, Command};
use tokio::sync::Notify;

use crate::ports::{DiscoveredInstallPaths, GameLaunchParams, InstallPresence, ProcessPort};

const MANAGED_EXE: &str = "ForgedAlliance.exe";
const MAX_REFERENCE_CONFIG_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Default)]
pub struct GameConfig {
    /// Path to `ForgedAlliance.exe` for live games (`FAF_GAME_PATH`).
    pub game_path: String,
    /// Path to `ForgedAlliance.exe` for replay playback (`FAF_REPLAY_GAME_PATH`)
    ///: a separate install, never falls back to [`Self::game_path`].
    pub replay_game_path: String,
    /// Literal arguments supplied by the user in Settings. They are prepended,
    /// leaving protocol-critical arguments later in the command line.
    pub additional_arguments: Vec<String>,
    /// A command to run the executable *with*, as typed in Settings
    /// (`FAF_LAUNCH_WRAPPER`). Empty runs the executable directly.
    ///
    /// This is how the client starts a game on Linux, where the configured
    /// executable is a Windows binary: `wine`, or whatever else a working
    /// prefix is reached through. Split into arguments by [`split_command`],
    /// never handed to a shell.
    pub launch_wrapper: String,
    /// The Wine prefix to run in (`WINEPREFIX`), from Settings, Paths. Empty
    /// leaves the child's environment as this process found it.
    pub wine_prefix: String,
}

impl GameConfig {
    /// The command the game is run through, split into arguments.
    ///
    /// The setting when there is one. Otherwise, on a Mac, the Wine the
    /// client installed itself (`infra::mac_setup`), so a player who never
    /// opened Settings still gets a working launch. Elsewhere, nothing.
    fn wrapper(&self) -> Vec<String> {
        let configured = split_command(&self.launch_wrapper);
        if !configured.is_empty() {
            return configured;
        }
        crate::infra::mac_setup::managed_wine()
            .map(|wine| vec![wine.to_string_lossy().into_owned()])
            .unwrap_or_default()
    }

    /// The prefix the game runs in: the setting, else the client's own prefix
    /// when the game runs through the client's own Wine. Empty leaves the
    /// child's environment alone.
    fn prefix(&self) -> String {
        if !self.wine_prefix.is_empty() {
            return self.wine_prefix.clone();
        }
        if !self.launch_wrapper.trim().is_empty() {
            return String::new();
        }
        crate::infra::mac_setup::managed_prefix()
            .map(|prefix| prefix.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    pub fn faf() -> Self {
        Self {
            game_path: std::env::var("FAF_GAME_PATH").unwrap_or_default(),
            replay_game_path: std::env::var("FAF_REPLAY_GAME_PATH").unwrap_or_default(),
            additional_arguments: Vec::new(),
            launch_wrapper: std::env::var("FAF_LAUNCH_WRAPPER").unwrap_or_default(),
            wine_prefix: String::new(),
        }
    }
}

/// A running Forged Alliance, and which executable it is running.
///
/// The path is kept because two instances of the *same* install fight over
/// that install's shader cache and lock files: see [`GameProcess::spawn`]. It
/// is kept in [`install_key`] form, because "same install" is a question about
/// the file, not about how the setting spells its path.
struct Running {
    child: Child,
    exe: PathBuf,
}

/// A path in a form two spellings of the same file agree on.
///
/// A live game and a replay are two different executables in a normal FAF
/// install: `FAForever\bin\ForgedAlliance.exe` and
/// `FAForever\replaydata\bin\ForgedAlliance.exe`, which share a filename and
/// nothing else. So the comparison has to be on the full path, and on Windows
/// a full path typed into a settings field differs from the same path picked
/// from a dialog in case and in separators. `canonicalize` resolves both, plus
/// any junction or symlink between them.
///
/// Falls back to the path as given when it cannot be resolved, which is the
/// safe direction: the caller has already established the file exists, so this
/// only fires on something exotic, and an unresolved path compares equal to
/// itself.
fn install_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Which of the two installs a launch belongs to.
///
/// Separate slots, because a live game and a replay are separate installs by
/// design ([`GameConfig::replay_game_path`] never falls back to
/// [`GameConfig::game_path`]) and there is no reason one should end the other.
/// They shared a slot until now, which is exactly the reported bug: starting a
/// replay while in a lobby closed the game first.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Slot {
    Game,
    Replay,
}

pub struct GameProcess {
    /// Behind a lock because Settings can repoint the installs at runtime
    /// (`ProcessPort::set_paths`): the paths are no longer startup-only.
    config: Mutex<GameConfig>,
    game_child: Arc<Mutex<Option<Running>>>,
    replay_child: Arc<Mutex<Option<Running>>>,
    /// Woken when the *game* exits; see [`GameProcess::watch_for_exit`].
    ///
    /// Only the game slot notifies. This is what releases the launch, tells
    /// the server the game ended and stops the ICE adapter, and a replay
    /// window closing is none of those things.
    exited: Arc<Notify>,
    /// Kept alive for the duration of a live game so FA has somewhere to stream
    /// its replay. Replaced on the next launch, which drops and stops the
    /// previous one.
    replay_recorder: Mutex<Option<crate::infra::replay_recorder::ReplayRecorder>>,
    /// What the recorder needs to forward the stream on to FAF, so the game
    /// lands in the vault rather than only on this disk.
    ///
    /// `None` is a launcher that records locally and uploads nothing: the
    /// offline/dev wiring, which has no session to authenticate with. It is
    /// deliberately not fatal, here or at launch time, because a replay that
    /// only exists locally is still a replay.
    replay_relay: Option<crate::infra::replay_relay::ReplayRelayConfig>,
    /// Show each game on Steam while it runs; see `infra::steam_presence`.
    /// Off until Settings says otherwise, which is also its default there.
    steam_presence: AtomicBool,
    /// Make the game's window borderless; see `infra::borderless`.
    borderless_window: AtomicBool,
}

impl GameProcess {
    pub fn new(config: GameConfig) -> Self {
        Self {
            config: Mutex::new(config),
            game_child: Arc::new(Mutex::new(None)),
            replay_child: Arc::new(Mutex::new(None)),
            exited: Arc::new(Notify::new()),
            replay_recorder: Mutex::new(None),
            replay_relay: None,
            steam_presence: AtomicBool::new(false),
            borderless_window: AtomicBool::new(false),
        }
    }

    pub fn faf() -> Self {
        Self::new(GameConfig::faf())
    }

    /// Upload the replays of the games this launcher starts, as `tokens`'
    /// account.
    ///
    /// Separate from [`Self::faf`] because the offline wiring uses the same
    /// real launcher (game paths are a local capability, not a network one) and
    /// has no session to upload with.
    pub fn uploading_replays(mut self, tokens: crate::infra::session::TokenStore) -> Self {
        self.replay_relay = Some(crate::infra::replay_relay::ReplayRelayConfig::faf(tokens));
        self
    }
}

impl GameProcess {
    /// Resolve `game_path`, spawn it with `args` in its own parent directory
    /// (the FA working directory), and track the child for [`ProcessPort::kill`].
    /// Shared by [`ProcessPort::launch_game`] and [`ProcessPort::launch_replay`]
    ///: they differ in which path/args they pass in, never in this logic.
    /// `what` names the install in the error when `game_path` is unset, so a
    /// misconfigured live-game vs. replay path is easy to tell apart. The
    /// message points at Settings rather than at the env var, because that is
    /// now the primary way to configure it.
    fn slot(&self, slot: Slot) -> &Arc<Mutex<Option<Running>>> {
        match slot {
            Slot::Game => &self.game_child,
            Slot::Replay => &self.replay_child,
        }
    }

    /// Is the other slot running this same executable right now?
    ///
    /// `try_wait` rather than "is the slot occupied": the watcher clears a slot
    /// within half a second of the process ending, and half a second is exactly
    /// the window in which somebody closes a replay and immediately joins.
    fn other_slot_holds(&self, slot: Slot, exe: &Path) -> bool {
        let other = match slot {
            Slot::Game => &self.replay_child,
            Slot::Replay => &self.game_child,
        };
        let mut guard = other.lock().unwrap();
        let Some(running) = guard.as_mut() else {
            return false;
        };
        match running.child.try_wait() {
            Ok(None) => running.exe == install_key(exe),
            _ => {
                *guard = None;
                false
            }
        }
    }

    /// End whatever this slot is still running, and wait until it is gone.
    ///
    /// Both halves matter, and the waiting half is the reported bug. Starting a
    /// replay while the previous one is still open used to spawn the new
    /// process first and terminate the old one afterwards, so for as long as
    /// the old process took to die there were two copies of one install
    /// running: the state the whole of [`Self::spawn`] exists to prevent,
    /// because they fight over that install's shader cache and lock files and
    /// one of them freezes on a blank post-shader-compile screen with no crash
    /// and no error anywhere. Which one freezes is a race, which is why the
    /// report is "50% of the time".
    ///
    /// The wait is bounded. A process that has been sent a termination signal
    /// and has not gone in five seconds is not going to, and a launch is worth
    /// attempting anyway at that point: refusing would leave the user with
    /// nothing, where the odds are simply back to what they were.
    ///
    /// Only ever called for [`Slot::Replay`], and that is not incidental.
    /// Taking a child out of its slot is what tells [`Self::watch_for_exit`] it
    /// is done, and the game slot's watcher announces an exit as it goes: a
    /// live game relaunched through here would report the previous game as
    /// having ended, over the top of the one that just started. The replay
    /// slot's watcher announces nothing, because a replay window closing is not
    /// the end of a game session.
    async fn end_previous(&self, slot: Slot) {
        let Some(mut previous) = self.slot(slot).lock().unwrap().take() else {
            return;
        };
        if matches!(previous.child.try_wait(), Ok(Some(_))) {
            return;
        }
        let _ = previous.child.start_kill();
        for _ in 0..100 {
            match previous.child.try_wait() {
                Ok(None) => tokio::time::sleep(Duration::from_millis(50)).await,
                _ => return,
            }
        }
        tracing::warn!("the previous Forged Alliance process has not exited; launching anyway");
    }

    fn spawn(
        &self,
        slot: Slot,
        game_path: &str,
        args: &[String],
        what: &str,
    ) -> Result<(), String> {
        if game_path.is_empty() {
            return Err(format!(
                "no {what} install configured: set it in Settings → Paths"
            ));
        }
        let exe = PathBuf::from(game_path);
        if !exe.is_file() {
            return Err(format!(
                "the configured {what} install no longer exists: {game_path}"
            ));
        }
        if is_original_game_executable(&exe) {
            return Err(format!(
                "the configured {what} path is the original Steam/retail game, not a FAF-managed executable: select the ForgedAlliance.exe under FAForever\\bin"
            ));
        }
        // Two instances of one install do not coexist: they fight over that
        // install's shader cache and lock files, and the older one freezes on a
        // blank post-shader-compile screen with no crash and no error. Refusing
        // is the change here. The previous code killed whatever was running,
        // which is how starting a replay ended the game somebody was in the
        // middle of joining, and that is what was reported. A second install
        // (Settings, Paths, replay install) makes both run side by side.
        if self.other_slot_holds(slot, &exe) {
            return Err(match slot {
                Slot::Game => "a replay is already running from this install, and two copies of one install cannot run at once: close the replay window, or point Settings → Paths at a separate replay install".to_string(),
                Slot::Replay => "the game is already running from this install, and two copies of one install cannot run at once: set a separate replay install in Settings → Paths to watch a replay while playing".to_string(),
            });
        }

        let work_dir = exe
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| format!("game path has no parent dir: {game_path}"))?;

        let config = self.config.lock().unwrap();
        let wrapper = config.wrapper();
        tracing::info!(
            argument_count = args.len(),
            wrapper = wrapper.first().map(String::as_str).unwrap_or(""),
            "launching Forged Alliance"
        );

        // With a wrapper the executable stops being the program and becomes
        // the wrapper's first argument, which is what `wine <exe> <args>` is.
        // Everything after it keeps its order, so the arguments FA receives are
        // the same either way.
        let mut command = match wrapper.split_first() {
            Some((program, rest)) => {
                let mut command = Command::new(program);
                command.args(rest).arg(&exe);
                command
            }
            None => Command::new(&exe),
        };
        command
            .args(&config.additional_arguments)
            .args(args)
            .current_dir(&work_dir);
        // The prefix decides where FA's `%LOCALAPPDATA%` is, and therefore
        // which `game.prefs` a launched game reads. Passing it to the child
        // rather than asking the user to export it keeps the client and the
        // game looking at one file: `infra::mods` resolves the same setting.
        let prefix = config.prefix();
        if !prefix.is_empty() {
            command.env("WINEPREFIX", &prefix);
        }
        // Wine's own trace output is a firehose nobody reads, and on a Mac it
        // measurably slows the simulation thread under Rosetta.
        if !wrapper.is_empty() && std::env::var_os("WINEDEBUG").is_none() {
            command.env("WINEDEBUG", "-all");
        }
        // The client's own Wine is CrossOver's, which can synchronise threads
        // through Mach semaphores instead of round trips to the wine server.
        // The game's render and simulation threads hand work to each other
        // constantly, so that is where the server would cost the most.
        // `FAF_WINE_MSYNC=0` turns it off for a comparison.
        let managed = crate::infra::mac_setup::managed_wine().filter(|wine| {
            wrapper
                .first()
                .is_some_and(|program| Path::new(program) == wine)
        });
        let msync = std::env::var("FAF_WINE_MSYNC").map_or(true, |value| value != "0");
        if managed.is_some() {
            command.env("WINEMSYNC", if msync { "1" } else { "0" });
        }
        let launched = wrapper.first().cloned();
        drop(config);

        let child = command.spawn().map_err(|e| match &launched {
            // Naming the wrapper matters more than naming the executable here:
            // "no such file" for `wine` on a machine without Wine installed
            // otherwise reads as the game being missing, which it is not.
            Some(program) => format!("could not start '{program}' to run the game: {e}"),
            None => format!("could not start '{}': {e}", exe.display()),
        })?;

        // The replay slot is normally empty by now: a replay launch ends the
        // previous occupant through `end_previous` first, and waits for it,
        // because two copies of one install must never be alive at the same
        // time. This is the backstop for that, and for the game slot, which
        // does not go through it. It is deliberately not a `drop(prev)` (the
        // bug this replaces): dropping the handle sends no signal, so the old
        // FA process kept running as an orphan. Confirmed live: relaunching a
        // replay left two
        // `ForgedAlliance.exe` processes alive simultaneously, fighting over
        // the same install's shader cache/lock files, and the previous process
        // froze on a blank post-shader-compile screen with zero further disk
        // activity, no crash, no error. `start_kill()` mirrors
        // `ProcessPort::kill`'s own termination call.
        // The window helper rides along with every game and replay the
        // client's own Wine runs: see `mac_setup::start_window_helper`.
        if let Some(wine) = &managed {
            crate::infra::mac_setup::start_window_helper(wine, &prefix, msync);
        }
        let game_pid = child.id();
        let running = Running {
            child,
            exe: install_key(&exe),
        };
        if let Some(mut prev) = self.slot(slot).lock().unwrap().replace(running) {
            let _ = prev.child.start_kill();
        }
        self.watch_for_exit(slot);

        // The game slot only: a live game or a tutorial is Forged Alliance
        // being played, and a replay window is somebody watching one. Nothing
        // to stop later, because the helper waits on this very process and
        // ends when it does.
        if slot == Slot::Game && self.steam_presence.load(Ordering::Relaxed) {
            if let Some(game_pid) = game_pid {
                crate::infra::steam_presence::start(game_pid);
            }
        }
        // Both slots: a replay is watched in the same window a game is
        // played in, and the player who wants one borderless wants the other
        // so too.
        if self.borderless_window.load(Ordering::Relaxed) {
            if let Some(game_pid) = game_pid {
                crate::infra::borderless::start(game_pid);
            }
        }
        Ok(())
    }

    /// Poll the tracked child until it exits, then wake [`Self::wait_for_exit`].
    ///
    /// Polling rather than `Child::wait()` because `wait` needs `&mut Child`,
    /// and the handle has to stay in the shared slot for [`ProcessPort::kill`]
    /// to reach it. A game session lasts minutes, so a second of latency on
    /// noticing the exit costs nothing.
    fn watch_for_exit(&self, slot: Slot) {
        let child = self.slot(slot).clone();
        // Only the game slot wakes the launcher. A replay window closing is
        // not the end of a game session, and telling the server "GameState
        // Ended" because somebody finished watching a replay is how a live
        // game would lose its launch out from under it.
        let exited = (slot == Slot::Game).then(|| self.exited.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let finished = {
                    let mut guard = child.lock().unwrap();
                    match guard.as_mut() {
                        // Taken by a newer launch or by `kill`: that launch owns
                        // its own watcher, so this one is done.
                        None => true,
                        Some(process) => match process.child.try_wait() {
                            Ok(Some(_status)) => {
                                *guard = None;
                                true
                            }
                            Ok(None) => false,
                            // The handle is unusable; treat it as gone rather
                            // than spinning on it forever.
                            Err(_) => {
                                *guard = None;
                                true
                            }
                        },
                    }
                };
                if finished {
                    // The game writes its settings file as it exits, and the
                    // first time it does is the first chance to give a Mac
                    // the faster defaults: see `mac_setup::optimize_graphics_once`.
                    if exited.is_some() {
                        crate::infra::mac_setup::optimize_graphics_once();
                    }
                    if let Some(exited) = &exited {
                        exited.notify_waiters();
                    }
                    return;
                }
            }
        });
    }
}

#[async_trait]
impl ProcessPort for GameProcess {
    fn supports_live_launch(&self) -> bool {
        true
    }

    fn replay_running(&self) -> bool {
        self.replay_child.lock().unwrap().is_some()
    }

    fn game_running(&self) -> bool {
        self.game_child.lock().unwrap().is_some()
    }

    async fn launch_game(&self, params: GameLaunchParams) -> Result<(), String> {
        let path = self.config.lock().unwrap().game_path.clone();
        let log_path = crate::infra::game_logs::next_path("game", Some(params.game_id))?;

        // Started before the game, so the port in `/savereplay` is already
        // listening when FA connects to it. A recorder that cannot bind is not
        // a reason to refuse the launch: the game is playable, it just leaves no
        // replay behind, which is what happened on every launch before this.
        let recorder = match crate::infra::replay_recorder::ReplayRecorder::start(
            crate::infra::replay::local_replays_dir(),
            params.replay.clone(),
            self.replay_relay.clone(),
        )
        .await
        {
            Ok(recorder) => Some(recorder),
            Err(error) => {
                tracing::warn!(%error, "starting without replay recording");
                None
            }
        };
        let savereplay = recorder
            .as_ref()
            .map(|recorder| recorder.savereplay_url(params.game_id, &params.player_login));

        let result = self.spawn(
            Slot::Game,
            &path,
            &build_arguments(
                &params,
                &self.game_argument_path(&log_path),
                savereplay.as_deref(),
            ),
            "game",
        );

        // Held for the game's lifetime: dropping the recorder aborts its
        // listener, and FA connects seconds after launch.
        if let Some(recorder) = recorder {
            *self.replay_recorder.lock().unwrap() = Some(recorder);
        }
        result
    }

    async fn launch_offline(&self, featured_mod: String, map: String) -> Result<(), String> {
        let path = self.config.lock().unwrap().game_path.clone();
        let log_path = crate::infra::game_logs::next_path("offline", None)?;
        self.spawn(
            Slot::Game,
            &path,
            &offline_arguments(&featured_mod, &map, &self.game_argument_path(&log_path)),
            "game",
        )
    }

    async fn launch_replay(&self, args: Vec<String>) -> Result<(), String> {
        self.end_previous(Slot::Replay).await;
        let path = self.config.lock().unwrap().replay_game_path.clone();
        self.spawn(Slot::Replay, &path, &args, "replay")
    }

    /// End the *game*, not whatever Forged Alliance happens to be running.
    ///
    /// This is the lobby service tearing a game session down, and a replay the
    /// user is watching in a second window is no part of that session.
    fn kill(&self) {
        if let Some(mut running) = self.game_child.lock().unwrap().take() {
            let _ = running.child.start_kill();
        }
    }

    async fn wait_for_exit(&self) {
        self.exited.notified().await
    }

    fn set_paths(&self, game_path: String, replay_game_path: String) {
        let mut config = self.config.lock().unwrap();
        config.game_path = game_path;
        config.replay_game_path = replay_game_path;
    }

    fn set_additional_arguments(&self, arguments: Vec<String>) {
        self.config.lock().unwrap().additional_arguments = arguments;
    }

    fn set_launch_wrapper(&self, wrapper: String, wine_prefix: String) {
        let mut config = self.config.lock().unwrap();
        config.launch_wrapper = wrapper;
        config.wine_prefix = wine_prefix;
    }

    fn set_steam_presence(&self, enabled: bool) {
        self.steam_presence.store(enabled, Ordering::Relaxed);
    }

    fn set_borderless_window(&self, enabled: bool) {
        self.borderless_window.store(enabled, Ordering::Relaxed);
    }

    fn game_argument_path(&self, path: &Path) -> String {
        let wrapped = !self.config.lock().unwrap().wrapper().is_empty();
        game_visible_path(path, wrapped)
    }

    fn game_install_dir(&self) -> Option<PathBuf> {
        managed_install_dir_of(&self.config.lock().unwrap().game_path)
    }

    fn replay_install_dir(&self) -> Option<PathBuf> {
        managed_install_dir_of(&self.config.lock().unwrap().replay_game_path)
    }

    fn installs_present(&self) -> InstallPresence {
        let config = self.config.lock().unwrap();
        let present = managed_executable_is_present;
        InstallPresence {
            game: present(&config.game_path),
            replay: present(&config.replay_game_path),
            game_pending: managed_executable_is_pending(&config.game_path),
            replay_pending: managed_executable_is_pending(&config.replay_game_path),
        }
    }

    fn install_path_is_present(&self, path: &str) -> bool {
        managed_executable_is_present(path)
    }

    fn discover_install_paths(&self) -> DiscoveredInstallPaths {
        discover_reference_install_paths()
    }

    fn default_install_paths(&self) -> DiscoveredInstallPaths {
        default_managed_install_paths()
    }

    fn is_original_game_install(&self, path: &str) -> bool {
        is_original_game_executable(Path::new(path))
    }

    fn replay_path_beside_game(&self, game_path: &str) -> Option<String> {
        replay_path_beside(game_path)
    }

    fn is_live_install(&self, path: &str) -> bool {
        same_install(path, &self.config.lock().unwrap().game_path)
    }

    async fn stop_replay(&self) {
        self.end_previous(Slot::Replay).await;
    }
}

/// Split a command line into a program and its arguments.
///
/// Whitespace separates arguments and quotes group them, and that is the whole
/// grammar: this is not a shell, and nothing else here is interpreted. A `$`,
/// a `~`, a `;` or a `|` in this field is a character in an argument, so
/// setting the wrapper cannot run a second command as a side effect of running
/// the first.
///
/// Both quote characters are honoured because the field holds paths, and a
/// Linux path with a space in it is quoted with whichever one is at hand. A
/// quote that is never closed ends the argument at the end of the line, which
/// is the reading that loses the least of what somebody typed.
fn split_command(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    for character in input.chars() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => current.push(character),
            None if character == '"' || character == '\'' => {
                // An empty pair of quotes is still an argument: it is the only
                // way to write one.
                started = true;
                quote = Some(character);
            }
            None if character.is_whitespace() => {
                if started {
                    parts.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                started = true;
                current.push(character);
            }
        }
    }
    if started {
        parts.push(current);
    }
    parts
}

/// How a path has to be spelled for the launched game to be able to open it.
///
/// Under a wrapper the thing being launched is a Windows program, and an
/// absolute Linux path is not a path it can open: FA hands `/home/a/game.log`
/// to `CreateFile` and gets nothing back. Every prefix Wine creates maps the
/// root of the filesystem to the `Z:` drive, so the same file is
/// `Z:\home\game.log` seen from inside, and that is the spelling to pass.
///
/// Only paths the client generates go through this: the game's own switches
/// (`/init`, `/log`, `/gpgnet`) also start with a slash and are not paths, so
/// this cannot be applied to an argument list as a whole.
///
/// Without a wrapper, or for a path that is already a Windows one, the path is
/// returned unchanged.
pub(crate) fn game_visible_path(path: &Path, wrapped: bool) -> String {
    let text = path.display().to_string();
    if !wrapped || !text.starts_with('/') {
        return text;
    }
    format!("Z:{}", text.replace('/', "\\"))
}

/// Locate the data roots used by the Java and Python clients, then select an
/// existing managed executable from them. Both reference clients store the
/// original Steam/retail directory separately; importing that directory here
/// would be unsafe because our updater derives its write target from the
/// configured executable.
fn discover_reference_install_paths() -> DiscoveredInstallPaths {
    if cfg!(windows) {
        let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
        let program_data = std::env::var_os("PROGRAMDATA")
            .or_else(|| std::env::var_os("ALLUSERSPROFILE"))
            .map(PathBuf::from);

        let java_prefs = app_data
            .as_ref()
            .map(|root| root.join("Forged Alliance Forever").join("client.prefs"));
        let python_ini = app_data
            .as_ref()
            .map(|root| root.join("ForgedAllianceForever").join("FA Lobby.ini"));

        return discover_from_reference_configs(
            java_prefs.as_deref(),
            python_ini.as_deref(),
            program_data.as_deref(),
            &[],
        );
    }

    let (java_prefs, python_ini, roots) = linux_reference_config_paths(
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf()),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
    );
    discover_from_reference_configs(java_prefs.as_deref(), python_ini.as_deref(), None, &roots)
}

/// Where the two reference clients keep their configuration on Linux, and
/// where a FAF install is if neither of them has one recorded.
///
/// `APPDATA` and `PROGRAMDATA` are Windows variables and are unset here, so
/// the Windows version of this finds nothing at all and every path has to be
/// typed by hand. The Linux locations are not guesses: the Java client is a
/// Java application and follows the XDG base directories, which is
/// `~/.local/share/Forged Alliance Forever` (its data directory is
/// `~/.faforever` when it has been told to use the legacy layout, so both are
/// offered), and the Python client writes `~/.config/ForgedAllianceForever`.
///
/// There is no per-machine equivalent of `%PROGRAMDATA%\FAForever`, because
/// nothing on Linux installs a game for every user. `~/.faforever` takes its
/// place as the root to look under when no reference client is installed at
/// all, which is where the Java client puts an install it downloaded itself.
///
/// Split out from the discovery above so it can be tested on the machine this
/// is written on, which is not the machine it describes.
fn linux_reference_config_paths(
    home: Option<PathBuf>,
    xdg_data_home: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
) -> (Option<PathBuf>, Option<PathBuf>, Vec<PathBuf>) {
    let data_home = xdg_data_home.or_else(|| home.as_ref().map(|home| home.join(".local/share")));
    let config_home = xdg_config_home.or_else(|| home.as_ref().map(|home| home.join(".config")));

    let java_prefs = data_home.map(|root| root.join("Forged Alliance Forever/client.prefs"));
    let python_ini = config_home.map(|root| root.join("ForgedAllianceForever/FA Lobby.ini"));
    let roots = home
        .map(|home| vec![home.join(".faforever")])
        .unwrap_or_default();
    (java_prefs, python_ini, roots)
}

fn discover_from_reference_configs(
    java_prefs: Option<&Path>,
    python_ini: Option<&Path>,
    default_program_data: Option<&Path>,
    default_roots: &[PathBuf],
) -> DiscoveredInstallPaths {
    let mut roots = Vec::new();
    if let Some(path) = java_prefs.and_then(java_data_root) {
        roots.push(path);
    }
    if let Some(path) = python_ini.and_then(python_data_root) {
        roots.push(path);
    }
    if let Some(path) = default_program_data {
        roots.push(path.join("FAForever"));
    }
    roots.extend(default_roots.iter().cloned());

    let mut seen = HashSet::new();
    roots.retain(|path| seen.insert(path.clone()));

    DiscoveredInstallPaths {
        game: newest_existing_executable(&roots, &["bin", MANAGED_EXE]),
        replay: newest_existing_executable(&roots, &["replaydata", "bin", MANAGED_EXE]),
    }
}

/// The retail Supreme Commander: Forged Alliance root, as the reference
/// clients have it configured. Distinct from the FAF data roots above: this
/// is the untouched Steam/THQ install holding `gamedata/*.scd`, `movies`,
/// `sounds` and `fonts`, which the game updater has to name as `fa_path` in
/// `fa_path.lua`.
///
/// Both reference clients persist it, under keys unrelated to the data roots:
/// the Java client as `forgedAlliance.installationPath` in `client.prefs`,
/// the Python client as `app\path` in `FA Lobby.ini`'s `[ForgedAlliance]`
/// section. Java first, matching the order [`discover_from_reference_configs`]
/// uses. Nothing here is validated: the caller decides what counts as a usable
/// install, and importing the path is safe precisely because it is only ever
/// read from, never an update target.
pub fn reference_retail_install_paths() -> Vec<PathBuf> {
    let Some(app_data) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    reference_retail_install_paths_from(
        &app_data
            .join("Forged Alliance Forever")
            .join("client.prefs"),
        &app_data.join("ForgedAllianceForever").join("FA Lobby.ini"),
    )
}

fn reference_retail_install_paths_from(java_prefs: &Path, python_ini: &Path) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = [
        java_prefs_path(java_prefs, "/forgedAlliance/installationPath"),
        python_ini_path(python_ini, "ForgedAlliance", r"app\path"),
    ]
    .into_iter()
    .flatten()
    .collect();

    let mut seen = HashSet::new();
    roots.retain(|path| seen.insert(path.clone()));
    roots
}

fn java_data_root(path: &Path) -> Option<PathBuf> {
    java_prefs_path(path, "/data/baseDataDirectory")
}

fn java_prefs_path(path: &Path, pointer: &str) -> Option<PathBuf> {
    let text = read_small_text_file(path)?;
    let document: serde_json::Value = serde_json::from_str(&text).ok()?;
    document
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
}

fn python_data_root(path: &Path) -> Option<PathBuf> {
    python_ini_path(path, "client", "data_path")
}

/// Read one QSettings-style ini value. Qt writes nested keys as
/// `app\path=…` inside a `[Section]`, but also accepts the flattened
/// `Section\app\path=…` form, so both are matched, with `/` and `\` treated
/// as the same separator.
fn python_ini_path(path: &Path, section: &str, key: &str) -> Option<PathBuf> {
    let text = read_small_text_file(path)?;
    let flattened = format!("{section}/{key}");
    let mut current = "";

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
        {
            current = name.trim();
            continue;
        }
        let Some((raw_key, value)) = line.split_once('=') else {
            continue;
        };
        let raw_key = raw_key.trim();
        let matches = (current.eq_ignore_ascii_case(section) && ini_key_eq(raw_key, key))
            || ini_key_eq(raw_key, &flattened);
        if !matches {
            continue;
        }

        let value = value.trim().trim_matches('"').replace(r"\\", r"\");
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    None
}

fn ini_key_eq(left: &str, right: &str) -> bool {
    left.replace('\\', "/")
        .eq_ignore_ascii_case(&right.replace('\\', "/"))
}

fn read_small_text_file(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_REFERENCE_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_REFERENCE_CONFIG_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn newest_existing_executable(roots: &[PathBuf], suffix: &[&str]) -> Option<String> {
    roots
        .iter()
        .map(|root| {
            suffix
                .iter()
                .fold(root.clone(), |path, part| path.join(part))
        })
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        })
        .map(|path| path.to_string_lossy().into_owned())
}

/// `…/FAForever/bin/ForgedAlliance.exe` → `…/FAForever`: two `parent()` calls
/// up from the executable, the same derivation `infra::replay` uses for the
/// replay install. The file itself need not exist: the updater's whole job is
/// to put it there: but the path must be shaped like an install, so a bare
/// `ForgedAlliance.exe` with no directories above it yields `None` rather than
/// an empty path that would patch into the working directory.
fn install_dir_of(exe_path: &str) -> Option<PathBuf> {
    if exe_path.is_empty() {
        return None;
    }
    PathBuf::from(exe_path)
        .parent()?
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(PathBuf::from)
}

fn managed_install_dir_of(exe_path: &str) -> Option<PathBuf> {
    let exe = Path::new(exe_path);
    let install_dir = install_dir_of(exe_path)?;
    (!is_original_game_executable(exe)).then_some(install_dir)
}

fn managed_executable_is_present(path: &str) -> bool {
    let exe = Path::new(path);
    exe.is_file() && managed_install_dir_of(path).is_some()
}

/// Whether the updater would create this executable rather than fail on it.
///
/// The one state a fresh machine is in, and the reason it needs a name of its
/// own: the file is absent, which looks exactly like a broken setting, but the
/// path is a managed install root and the updater's whole job is to put the
/// engine there. Saying "install not found" about it sends the user looking for
/// something that has never existed.
fn managed_executable_is_pending(path: &str) -> bool {
    let exe = Path::new(path);
    !exe.is_file() && managed_install_dir_of(path).is_some()
}

/// Where this client puts its own copy of the game when nothing else has one.
///
/// The same roots the reference clients use, which is what makes an install
/// made here visible to them and theirs visible to us:
/// `%PROGRAMDATA%\FAForever` on Windows, `~/.faforever` on Linux. Nothing is
/// created here; these are the paths the updater is pointed at, and it creates
/// what it needs when the first game or replay is prepared.
fn default_managed_install_paths() -> DiscoveredInstallPaths {
    let root = if cfg!(windows) {
        std::env::var_os("PROGRAMDATA")
            .or_else(|| std::env::var_os("ALLUSERSPROFILE"))
            .map(|base| PathBuf::from(base).join("FAForever"))
    } else if cfg!(target_os = "macos") {
        // Beside Wine, the prefix and the retail game, so everything the Mac
        // setup made lives in one folder and leaves with it.
        crate::infra::mac_setup::Layout::resolve().map(|layout| layout.root().join("faforever"))
    } else {
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".faforever"))
    };
    let Some(root) = root else {
        return DiscoveredInstallPaths::default();
    };
    let spell = |path: PathBuf| Some(path.to_string_lossy().into_owned());
    DiscoveredInstallPaths {
        game: spell(root.join("bin").join(MANAGED_EXE)),
        replay: spell(root.join("replaydata").join("bin").join(MANAGED_EXE)),
    }
}

/// The replay install that belongs to a given live install: the `replaydata`
/// directory beside its `bin`.
///
/// This is the shape both reference clients use, and the reason it is derived
/// from the configured game path rather than from the default root: somebody
/// whose game lives in a Java client's data directory on another drive has no
/// use for a replay install under `%PROGRAMDATA%`. The two belong to one FAF
/// install and are two directories inside it.
///
/// `None` for an empty path, for a path that is not shaped like an install,
/// and for the retail game, which is never an update target and therefore has
/// no replay install to sit beside.
fn replay_path_beside(game_path: &str) -> Option<String> {
    let root = managed_install_dir_of(game_path)?;
    Some(
        root.join("replaydata")
            .join("bin")
            .join(MANAGED_EXE)
            .to_string_lossy()
            .into_owned(),
    )
}

/// Whether two configured executables name the same install.
///
/// The question the replay field has to ask about a pick, and it is a question
/// about the directory rather than the file: `bin\ForgedAlliance.exe` and
/// `replaydata\bin\ForgedAlliance.exe` differ only in the directories above
/// them, and the same install reached two ways differs only in spelling.
/// [`install_key`] settles the spelling, and the directory is used rather than
/// the executable because the executable of an install FAF has not downloaded
/// yet does not exist to canonicalise.
fn same_install(left: &str, right: &str) -> bool {
    match (install_dir_of(left), install_dir_of(right)) {
        (Some(left), Some(right)) => install_key(&left) == install_key(&right),
        _ => false,
    }
}

/// The reference clients validate an original FA root through its base archive.
/// Its executable has the same filename and `bin/` shape as the managed copy,
/// so checking only `ForgedAlliance.exe` cannot keep the updater out of Steam.
fn is_original_game_executable(exe: &Path) -> bool {
    install_dir_of(&exe.to_string_lossy())
        .is_some_and(|root| root.join("gamedata").join("lua.scd").is_file())
}

/// Build the FA argument list: server args first, then init/bugreport/gpgnet.
///
/// Mirrors `fa/play.py:build_argument_list`. `savereplay` is the address of the
/// local [`crate::infra::replay_recorder::ReplayRecorder`]; without it FA writes
/// no replay for a networked game at all, which is why played games used to
/// leave nothing in the local library.
fn build_arguments(
    params: &GameLaunchParams,
    log_path: &str,
    savereplay: Option<&str>,
) -> Vec<String> {
    let mut args = params.args.clone();
    args.push("/init".into());
    args.push(format!("init_{}.lua", params.featured_mod));
    args.push("/nobugreport".into());
    if let Some(url) = savereplay {
        args.push("/savereplay".into());
        args.push(url.to_string());
    }
    args.push("/gpgnet".into());
    args.push(format!("127.0.0.1:{}", params.game_port));
    args.push("/log".into());
    args.push(log_path.to_string());
    args
}

/// The command line for a single-player launch.
///
/// Mirrors the Java client's `LaunchCommandBuilder` for `launchOfflineGame`:
/// the init script for the featured mod, no bug reporter, and the scenario to
/// load. Deliberately no `/gpgnet`: there is no adapter and no lobby.
///
/// An empty `map` leaves `/map` out, which opens the game's own menu rather
/// than a scenario: the "Play offline" on the login screen (#397).
fn offline_arguments(featured_mod: &str, map: &str, log_path: &str) -> Vec<String> {
    let mut arguments = vec![
        "/init".into(),
        format!("init_{featured_mod}.lua"),
        "/nobugreport".into(),
    ];
    if !map.is_empty() {
        arguments.extend(["/map".into(), map.into()]);
    }
    arguments.extend(["/log".into(), log_path.to_string()]);
    arguments
}

/// Inert game launcher: used offline and in tests. Launching is a no-op success.
#[derive(Debug, Clone, Default)]
pub struct FakeGame;

#[async_trait]
impl ProcessPort for FakeGame {
    fn supports_live_launch(&self) -> bool {
        false
    }

    async fn launch_game(&self, _params: GameLaunchParams) -> Result<(), String> {
        Ok(())
    }
    async fn launch_offline(&self, _featured_mod: String, _map: String) -> Result<(), String> {
        Ok(())
    }
    async fn launch_replay(&self, _args: Vec<String>) -> Result<(), String> {
        Ok(())
    }
    fn kill(&self) {}
    fn set_paths(&self, _game_path: String, _replay_game_path: String) {}

    fn set_additional_arguments(&self, _arguments: Vec<String>) {}

    fn set_launch_wrapper(&self, _wrapper: String, _wine_prefix: String) {}

    fn game_install_dir(&self) -> Option<PathBuf> {
        None
    }

    fn replay_install_dir(&self) -> Option<PathBuf> {
        None
    }

    /// Reports nothing installed: there is no real install behind this fake, and
    /// claiming otherwise would suppress the missing-install banner in exactly
    /// the offline mode where it is most accurate.
    fn installs_present(&self) -> InstallPresence {
        InstallPresence::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrapper_is_split_into_a_program_and_its_arguments() {
        assert_eq!(split_command("wine"), vec!["wine"]);
        assert_eq!(
            split_command("  flatpak run   org.winehq.Wine  "),
            vec!["flatpak", "run", "org.winehq.Wine"]
        );
        assert!(split_command("").is_empty());
        assert!(split_command("   ").is_empty());
    }

    #[test]
    fn quotes_hold_a_path_with_a_space_in_it_together() {
        assert_eq!(
            split_command("\"/opt/my wine/bin/wine\" --keep"),
            vec!["/opt/my wine/bin/wine", "--keep"]
        );
        assert_eq!(split_command("'/opt/a b/wine'"), vec!["/opt/a b/wine"]);
        // An unclosed quote takes the rest of the line rather than dropping
        // it: the user typed those characters and meant them.
        assert_eq!(split_command("\"/opt/a b/wine"), vec!["/opt/a b/wine"]);
    }

    #[test]
    fn nothing_but_quoting_is_interpreted() {
        // Not a shell: no expansion, no operators, no second command. Each of
        // these is one literal argument.
        assert_eq!(
            split_command("wine $HOME/x; rm -rf /"),
            vec!["wine", "$HOME/x;", "rm", "-rf", "/"]
        );
    }

    #[test]
    fn a_wrapped_launch_names_a_path_the_way_the_game_will_see_it() {
        let log = Path::new("/home/player/.faf/logs/game-99.log");
        // Wine maps the root of the filesystem to `Z:` in every prefix it
        // makes, so this is the same file named from inside one.
        assert_eq!(
            game_visible_path(log, true),
            r"Z:\home\player\.faf\logs\game-99.log"
        );
        // Unwrapped, and on Windows, the path is already in the right
        // alphabet and must not be touched.
        assert_eq!(
            game_visible_path(log, false),
            "/home/player/.faf/logs/game-99.log"
        );
        let windows = Path::new(r"C:\Users\me\game.log");
        assert_eq!(
            game_visible_path(windows, true),
            game_visible_path(windows, false)
        );
    }

    #[test]
    fn linux_discovery_looks_where_each_reference_client_actually_writes() {
        let (java, python, roots) =
            linux_reference_config_paths(Some(PathBuf::from("/home/player")), None, None);
        assert_eq!(
            java.unwrap(),
            PathBuf::from("/home/player/.local/share/Forged Alliance Forever/client.prefs")
        );
        assert_eq!(
            python.unwrap(),
            PathBuf::from("/home/player/.config/ForgedAllianceForever/FA Lobby.ini")
        );
        // No `%PROGRAMDATA%` equivalent: nothing here installs a game for
        // every user, so the fallback root is the Java client's own.
        assert_eq!(roots, vec![PathBuf::from("/home/player/.faforever")]);
    }

    #[test]
    fn the_xdg_variables_win_over_the_home_directory_defaults() {
        let (java, python, _) = linux_reference_config_paths(
            Some(PathBuf::from("/home/player")),
            Some(PathBuf::from("/data")),
            Some(PathBuf::from("/conf")),
        );
        assert_eq!(
            java.unwrap(),
            PathBuf::from("/data/Forged Alliance Forever/client.prefs")
        );
        assert_eq!(
            python.unwrap(),
            PathBuf::from("/conf/ForgedAllianceForever/FA Lobby.ini")
        );
    }

    fn params() -> GameLaunchParams {
        GameLaunchParams {
            game_id: 99,
            game_port: 7237,
            init_mode: 0,
            featured_mod: "faf".into(),
            player_id: 1,
            player_login: "me".into(),
            args: vec!["/numgames".into(), "5".into()],
            replay: Default::default(),
        }
    }

    #[test]
    fn an_offline_launch_without_a_map_opens_the_game_menu() {
        let menu = offline_arguments("faf", "", "game.log");
        assert!(!menu.iter().any(|argument| argument == "/map"));
        assert_eq!(&menu[..2], ["/init", "init_faf.lua"]);

        let tutorial = offline_arguments("tutorials", "SCMP_001", "game.log");
        let at = tutorial
            .iter()
            .position(|argument| argument == "/map")
            .unwrap();
        assert_eq!(tutorial[at + 1], "SCMP_001");
    }

    #[test]
    fn builds_fa_command_line_in_order() {
        let args = build_arguments(&params(), "diagnostics/game-99.log", None);
        assert_eq!(
            &args[..7],
            vec![
                "/numgames",
                "5",
                "/init",
                "init_faf.lua",
                "/nobugreport",
                "/gpgnet",
                "127.0.0.1:7237",
            ]
        );
        assert_eq!(args[7], "/log");
        assert!(args[8].ends_with("game-99.log"));
    }

    #[test]
    fn savereplay_precedes_gpgnet_when_a_recorder_is_listening() {
        // Ordering matches `fa/play.py`, and the flag is what makes FA emit a
        // replay for a networked game at all: without it a played game leaves
        // nothing on disk, which is the bug this fixes.
        let url = "gpgnet://127.0.0.1:5000/99/me.SCFAreplay";
        let args = build_arguments(&params(), "diagnostics/game-99.log", Some(url));
        let save = args.iter().position(|a| a == "/savereplay").unwrap();
        assert_eq!(args[save + 1], url);
        assert!(save < args.iter().position(|a| a == "/gpgnet").unwrap());
    }

    #[test]
    fn no_recorder_means_no_savereplay_flag() {
        // A recorder that could not bind must not leave FA streaming at a dead
        // port: the game is still perfectly playable without a replay.
        let args = build_arguments(&params(), "diagnostics/game-99.log", None);
        assert!(!args.iter().any(|a| a == "/savereplay"));
    }

    #[test]
    fn an_unset_path_is_not_present_and_a_real_file_is() {
        let process = GameProcess::new(GameConfig::default());
        assert_eq!(process.installs_present(), InstallPresence::default());

        // The test binary itself is a file that certainly exists, which is all
        // `installs_present` checks. (`file!()` would be relative to the
        // workspace root, not the crate dir the test runs in.)
        let existing = std::env::current_exe().unwrap().display().to_string();
        process.set_paths(existing, String::new());
        assert_eq!(
            process.installs_present(),
            InstallPresence {
                game: true,
                replay: false,
                game_pending: false,
                replay_pending: false,
            }
        );
    }

    #[test]
    fn a_configured_but_missing_path_is_not_present() {
        // The case a persisted setting hits after the user moves or uninstalls
        // the game: a non-empty path that no longer resolves.
        let process = GameProcess::new(GameConfig {
            game_path: "definitely/not/here/ForgedAlliance.exe".into(),
            replay_game_path: String::new(),
            ..GameConfig::default()
        });
        let present = process.installs_present();
        assert!(!present.game);
        // Absent, but the updater would create it: the state a fresh machine
        // starts in, which must not be reported as a missing install.
        assert!(present.game_pending);
    }

    #[test]
    fn the_original_retail_game_is_neither_present_nor_pending() {
        // Steam's install has the same `bin/` shape as a managed one, and a
        // fresh machine has nothing else to point at. It is refused either way:
        // the updater derives its write target from this path, and FAF never
        // writes into the original game. See `redirect_retail_pick`.
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Supreme Commander Forged Alliance");
        std::fs::create_dir_all(root.join("gamedata")).unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("gamedata").join("lua.scd"), b"base game").unwrap();
        let exe = root.join("bin").join(MANAGED_EXE);
        std::fs::write(&exe, b"engine").unwrap();

        let process = GameProcess::new(GameConfig {
            game_path: exe.display().to_string(),
            ..GameConfig::default()
        });
        let present = process.installs_present();
        assert!(!present.game, "the retail game is never the update target");
        assert!(
            !present.game_pending,
            "and it is not somewhere the updater may create one either"
        );
        assert!(process.is_original_game_install(&exe.display().to_string()));
    }

    #[test]
    fn a_replay_and_a_game_are_tracked_separately() {
        // The reported bug: joining a lobby and then starting a replay closed
        // the game first, because both launches shared one child slot. Nothing
        // is spawned here; what is asserted is that the two slots exist and are
        // not the same cell.
        let process = GameProcess::faf();
        assert!(process.game_child.lock().unwrap().is_none());
        assert!(process.replay_child.lock().unwrap().is_none());
        assert!(!Arc::ptr_eq(&process.game_child, &process.replay_child));
    }

    #[test]
    fn the_game_and_the_replay_install_are_different_files() {
        // A normal FAF install, as the maintainer has it: the two executables
        // share a filename and differ only in the directory, so anything
        // comparing filenames would call them the same install and refuse the
        // second launch. `install_key` compares the whole path.
        // Forward slashes, so `file_name` means the same thing on the Linux
        // box that runs CI as it does on the Windows box this describes.
        let game = PathBuf::from("C:/ProgramData/FAForever/bin/ForgedAlliance.exe");
        let replay = PathBuf::from("C:/ProgramData/FAForever/replaydata/bin/ForgedAlliance.exe");
        assert_eq!(game.file_name(), replay.file_name());
        assert_ne!(super::install_key(&game), super::install_key(&replay));
    }

    #[tokio::test]
    async fn ending_a_replay_that_is_not_running_is_not_a_wait() {
        // Every replay launch calls this, and almost every one of them has
        // nothing to end. It must not be the place a first replay of the
        // session spends five seconds.
        let process = GameProcess::faf();
        let started = std::time::Instant::now();
        process.end_previous(Slot::Replay).await;
        assert!(process.replay_child.lock().unwrap().is_none());
        assert!(started.elapsed() < Duration::from_millis(500));
    }

    #[tokio::test]
    async fn a_replay_launch_frees_the_install_before_the_next_one_takes_it() {
        // The reported bug, as far as it can be reproduced without an FA
        // install: watching a second replay while the first is still open used
        // to start the second process and *then* kill the first, so two copies
        // of one install overlapped and one of them froze on a loading screen.
        // Afterwards the slot is empty and the process is really gone, which is
        // what makes the next launch the only copy running.
        let process = GameProcess::faf();
        let child = a_process_that_keeps_running();
        let exe = PathBuf::from("C:/games/FAForever/replaydata/bin/ForgedAlliance.exe");
        *process.replay_child.lock().unwrap() = Some(Running {
            child,
            exe: install_key(&exe),
        });

        process.end_previous(Slot::Replay).await;

        assert!(
            process.replay_child.lock().unwrap().is_none(),
            "the slot has to be free before anything is spawned into it"
        );
        assert!(
            !process.other_slot_holds(Slot::Game, &exe),
            "and nothing is left holding that install"
        );
    }

    /// A child that stays alive until it is killed, on both the platform this
    /// is developed on and the one CI runs.
    fn a_process_that_keeps_running() -> Child {
        let mut command = if cfg!(windows) {
            let mut command = Command::new("cmd");
            command.args(["/c", "ping", "-n", "60", "127.0.0.1"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("a long-running child")
    }

    #[test]
    fn an_empty_slot_never_blocks_a_launch() {
        // The guard has to answer "no" for a slot nothing has run in, or the
        // first replay of a session would be refused on the strength of a
        // process that does not exist.
        let process = GameProcess::faf();
        let exe = PathBuf::from("C:/games/FAForever/bin/ForgedAlliance.exe");
        assert!(!process.other_slot_holds(Slot::Game, &exe));
        assert!(!process.other_slot_holds(Slot::Replay, &exe));
    }

    #[test]
    fn the_install_dir_is_two_levels_up_from_the_executable() {
        // `…/FAForever/bin/ForgedAlliance.exe` is where FAF puts the patched
        // engine; the file set the updater manages is rooted one level above
        // `bin/`.
        let process = GameProcess::new(GameConfig {
            game_path: "C:/games/FAForever/bin/ForgedAlliance.exe".into(),
            ..GameConfig::default()
        });
        assert_eq!(
            process.game_install_dir(),
            Some(PathBuf::from("C:/games/FAForever"))
        );
    }

    #[test]
    fn the_replay_install_sits_beside_the_live_one() {
        // Issue #267: the replay field used to want a path of its own, and on a
        // machine where FAF has only ever installed the live game there is no
        // such file to browse to. It is the same install's `replaydata` half,
        // and derived from wherever the live install actually is: a game in a
        // Java client's data directory on another drive has no use for a replay
        // install under `%PROGRAMDATA%`.
        let beside = super::replay_path_beside("D:/faf-data/bin/ForgedAlliance.exe").unwrap();
        assert_eq!(
            PathBuf::from(beside),
            PathBuf::from("D:/faf-data")
                .join("replaydata")
                .join("bin")
                .join(MANAGED_EXE)
        );
    }

    #[test]
    fn nothing_is_derived_from_a_path_that_is_not_an_install() {
        // Same three cases `install_dir_of` refuses, for the same reason: a
        // derived path built on the empty root would land in whatever directory
        // the client is running from.
        for path in ["", "ForgedAlliance.exe", "bin/ForgedAlliance.exe"] {
            assert_eq!(super::replay_path_beside(path), None, "for {path:?}");
        }
    }

    #[test]
    fn the_replay_half_is_not_the_live_install() {
        // What the settings service asks before accepting a replay pick. The
        // two executables share a filename, so the comparison is on the
        // directory above `bin`, and the same install spelled two ways is still
        // one install.
        let live = "C:/ProgramData/FAForever/bin/ForgedAlliance.exe";
        let replay = "C:/ProgramData/FAForever/replaydata/bin/ForgedAlliance.exe";
        assert!(super::same_install(live, live));
        assert!(!super::same_install(live, replay));
        assert!(!super::same_install(
            live,
            "D:/other/FAForever/bin/ForgedAlliance.exe"
        ));
        // Nothing configured is not "the same install as nothing".
        assert!(!super::same_install("", ""));
    }

    #[test]
    fn an_unconfigured_or_rootless_path_has_no_install_dir() {
        // Nothing configured yet, and a bare filename: the latter would
        // otherwise resolve to the empty path and patch into whatever
        // directory the client happens to be running from.
        for path in ["", "ForgedAlliance.exe", "bin/ForgedAlliance.exe"] {
            let process = GameProcess::new(GameConfig {
                game_path: path.into(),
                ..GameConfig::default()
            });
            assert_eq!(process.game_install_dir(), None, "for {path:?}");
        }
    }

    #[test]
    fn original_steam_install_is_not_a_managed_update_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("Supreme Commander Forged Alliance");
        let exe = root.join("bin").join(MANAGED_EXE);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join("gamedata")).unwrap();
        std::fs::write(&exe, b"vanilla").unwrap();
        std::fs::write(root.join("gamedata").join("lua.scd"), b"base game").unwrap();

        let process = GameProcess::new(GameConfig {
            game_path: exe.to_string_lossy().into_owned(),
            ..GameConfig::default()
        });
        assert!(!process.installs_present().game);
        assert_eq!(process.game_install_dir(), None);
    }

    #[test]
    fn repointing_the_install_in_settings_moves_the_updates_target() {
        let process = GameProcess::new(GameConfig {
            game_path: "C:/old/bin/ForgedAlliance.exe".into(),
            ..GameConfig::default()
        });
        process.set_paths("C:/new/bin/ForgedAlliance.exe".into(), String::new());
        assert_eq!(
            process.game_install_dir(),
            Some(PathBuf::from("C:/new")),
            "the updater must follow the path the launcher will actually use"
        );
    }

    #[test]
    fn set_paths_replaces_both_paths() {
        let process = GameProcess::new(GameConfig {
            game_path: "old-game".into(),
            replay_game_path: "old-replay".into(),
            ..GameConfig::default()
        });
        process.set_paths("new-game".into(), "new-replay".into());
        let config = process.config.lock().unwrap();
        assert_eq!(config.game_path, "new-game");
        assert_eq!(config.replay_game_path, "new-replay");
    }

    #[test]
    fn changing_paths_preserves_additional_arguments() {
        let process = GameProcess::new(GameConfig::default());
        process.set_additional_arguments(vec!["/windowed".into()]);
        process.set_paths("new-game".into(), "new-replay".into());
        assert_eq!(
            process.config.lock().unwrap().additional_arguments,
            vec!["/windowed"]
        );
    }

    #[tokio::test]
    async fn launching_without_a_configured_install_points_at_settings() {
        let process = GameProcess::new(GameConfig::default());
        let error = process.launch_replay(vec![]).await.unwrap_err();
        assert!(error.contains("replay"), "names which install: {error}");
        assert!(
            error.contains("Settings"),
            "points somewhere useful: {error}"
        );
    }

    #[tokio::test]
    async fn launching_a_vanished_install_says_so() {
        let process = GameProcess::new(GameConfig {
            game_path: String::new(),
            replay_game_path: "definitely/not/here/ForgedAlliance.exe".into(),
            ..GameConfig::default()
        });
        let error = process.launch_replay(vec![]).await.unwrap_err();
        assert!(error.contains("no longer exists"), "{error}");
    }

    #[test]
    fn discovers_custom_java_managed_live_and_replay_installs() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("custom-faf-data");
        let live = data.join("bin").join(MANAGED_EXE);
        let replay = data.join("replaydata").join("bin").join(MANAGED_EXE);
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        std::fs::create_dir_all(replay.parent().unwrap()).unwrap();
        std::fs::write(&live, b"live").unwrap();
        std::fs::write(&replay, b"replay").unwrap();

        let prefs = temp.path().join("client.prefs");
        std::fs::write(
            &prefs,
            serde_json::json!({ "data": { "baseDataDirectory": data } }).to_string(),
        )
        .unwrap();

        let found = discover_from_reference_configs(Some(&prefs), None, None, &[]);
        assert_eq!(found.game.as_deref(), live.to_str());
        assert_eq!(found.replay.as_deref(), replay.to_str());
    }

    #[test]
    fn discovers_python_custom_data_path_but_not_original_game_path() {
        let temp = tempfile::tempdir().unwrap();
        let data = temp.path().join("python-data");
        let live = data.join("bin").join(MANAGED_EXE);
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        std::fs::write(&live, b"live").unwrap();

        let ini = temp.path().join("FA Lobby.ini");
        std::fs::write(
            &ini,
            format!(
                "[ForgedAlliance] debt=ignored\napp/path=C:/original-game\n[client]\ndata_path={}\n",
                data.display()
            ),
        )
        .unwrap();

        let found = discover_from_reference_configs(None, Some(&ini), None, &[]);
        assert_eq!(found.game.as_deref(), live.to_str());
        assert_eq!(found.replay, None);
    }

    #[test]
    fn reads_the_retail_install_root_both_reference_clients_configured() {
        // The keys and escaping are copied from real config files. A retail
        // root outside `%ProgramFiles%` is unguessable, so reading it here is
        // the only thing that keeps `fa_path` off the FAF patch dir.
        let temp = tempfile::tempdir().unwrap();

        let prefs = temp.path().join("client.prefs");
        std::fs::write(
            &prefs,
            serde_json::json!({
                "data": { "baseDataDirectory": "C:/ProgramData/FAForever" },
                "forgedAlliance": { "installationPath": "C:/Games/THQ/FA" },
            })
            .to_string(),
        )
        .unwrap();

        let ini = temp.path().join("FA Lobby.ini");
        std::fs::write(
            &ini,
            "[client]\ndata_path=C:\\\\ProgramData\\\\FAForever\n\
             [ForgedAlliance]\napp\\path=D:\\\\Steam\\\\common\\\\SCFA\n",
        )
        .unwrap();

        assert_eq!(
            reference_retail_install_paths_from(&prefs, &ini),
            vec![
                PathBuf::from("C:/Games/THQ/FA"),
                PathBuf::from(r"D:\Steam\common\SCFA"),
            ],
            "the Java client's path comes first, and neither data root leaks in"
        );
    }

    #[test]
    fn a_missing_or_unconfigured_reference_config_yields_no_retail_root() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("client.prefs");
        // Configured, but only with the data root: the shape a user who never
        // pointed the Java client at their install has.
        std::fs::write(
            &prefs,
            serde_json::json!({ "data": { "baseDataDirectory": "C:/ProgramData/FAForever" } })
                .to_string(),
        )
        .unwrap();

        assert!(
            reference_retail_install_paths_from(&prefs, &temp.path().join("absent.ini")).is_empty()
        );
    }
}

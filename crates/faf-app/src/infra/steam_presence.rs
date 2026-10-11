//! Steam's "playing Supreme Commander: Forged Alliance", for exactly as long as
//! a game runs (issue 364).
//!
//! Steam counts a process as playing app N once that process has called
//! `SteamAPI_Init` as app N through Valve's Steamworks API library, and stops
//! when it calls `SteamAPI_Shutdown` or exits. The Java client makes that call
//! from the client itself, once, at startup (`SteamService`), which is why its
//! status and the hours Steam records cover the client being open rather than a
//! game being played. That is the complaint the issue was opened with.
//!
//! Here the call is made by a helper: this client's own executable started again
//! with [`HELPER_FLAG`], which loads the library, signs in as app
//! [`FORGED_ALLIANCE_APP_ID`], and watches the game's process. When the game
//! exits, the helper shuts the API down and exits too, and the status goes with
//! it. A separate process rather than this one, for three reasons:
//!
//! * `SteamAPI_Init` loads Steam's own client library into whichever process
//!   calls it. That should not be the process holding the player's session and
//!   the webview.
//! * Valve does not support initialising again after a shutdown in the same
//!   process, and one client runs many games.
//! * A helper that dies for any reason takes the status with it. A flag in this
//!   process could outlive the game it was about.
//!
//! Windows and Linux, the two platforms this client is released for. On Linux
//! the game runs through Wine or Proton but the library is the native one, and
//! it talks to the native Steam client, which does not mind that app 9420 is a
//! Windows game: ownership is what it checks. The process watched there is the
//! one the launch wrapper started, the same one the launcher already waits on
//! to notice the game ending.
//!
//! Off unless the player turns it on (`GamePreferences::steam_presence`), and
//! quiet when it cannot work: no library in this build, Steam not running, or
//! Forged Alliance not in the signed-in account's library. Each of those is a
//! log line, never a message to the player, because none of them is something
//! going wrong with the game.
//!
//! The library is Valve's and is not covered by this client's MIT licence: it is
//! redistributed under the Steamworks SDK Access Agreement, see
//! `vendor/steamworks/README.md`. Nothing here talks to Steam except through it,
//! which is what the agreement's section 2.4 asks.

/// Supreme Commander: Forged Alliance on Steam. The same id the Java client
/// signs in as, from its `steam_appid.txt`.
pub const FORGED_ALLIANCE_APP_ID: u32 = 9420;

/// The first argument that makes this executable the helper instead of the
/// client. See [`run_helper_if_asked`].
pub const HELPER_FLAG: &str = "--steam-presence";

/// Valve's library, as the SDK's `redistributable_bin` names it for this
/// platform, and the folder of `redistributable_bin` it comes from.
#[cfg(windows)]
pub const LIBRARY: Option<(&str, &str)> = Some(("steam_api64.dll", "win64"));
#[cfg(target_os = "linux")]
pub const LIBRARY: Option<(&str, &str)> = Some(("libsteam_api.so", "linux64"));
#[cfg(not(any(windows, target_os = "linux")))]
pub const LIBRARY: Option<(&str, &str)> = None;

/// Where Valve's library is, if this build carries it.
///
/// `FAF_STEAM_API_PATH` first, which the shell sets to the copy bundled under
/// the resource directory, then the same few roots every other bundled helper
/// is searched in: `natives/steam/` is where the installer puts it, and
/// `vendor/steamworks/<platform>/` is where it is kept in the repository, which
/// is what a development build finds.
pub(crate) fn library_path() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let (name, folder) = LIBRARY?;
    if let Some(path) = std::env::var_os("FAF_STEAM_API_PATH").filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    super::helper_search_roots()
        .into_iter()
        .flat_map(|root| {
            [
                root.join("natives").join("steam").join(name),
                root.join("vendor")
                    .join("steamworks")
                    .join(folder)
                    .join(name),
            ]
        })
        .find(|candidate| candidate.is_file())
}

/// Show the game running as `game_pid` on Steam until it exits.
///
/// Returns at once. The helper ends by itself when the game does, so nothing
/// has to stop it, and what it reported is logged when it has gone.
pub(crate) fn start(game_pid: u32) {
    use std::process::Stdio;

    let Some(library) = library_path() else {
        tracing::info!("Steam status is on, but this build carries no Steamworks library");
        return;
    };
    let executable = match std::env::current_exe() {
        Ok(executable) => executable,
        Err(error) => {
            tracing::warn!(%error, "cannot start the Steam status helper");
            return;
        }
    };
    let mut command = tokio::process::Command::new(executable);
    command
        .arg(HELPER_FLAG)
        .arg(&library)
        .arg(game_pid.to_string())
        // How the library learns which app it is. Steam sets both for a game
        // it launches itself; `steam_appid.txt` is the other way, and would
        // have to be written into a working directory.
        .env("SteamAppId", FORGED_ALLIANCE_APP_ID.to_string())
        .env("SteamGameId", FORGED_ALLIANCE_APP_ID.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    super::hide_console(&mut command);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, "cannot start the Steam status helper");
            return;
        }
    };
    tracing::info!(game_pid, "Steam status helper started");
    tokio::spawn(async move {
        match child.wait_with_output().await {
            Ok(output) => {
                let reason = String::from_utf8_lossy(&output.stderr);
                match output.status.code() {
                    Some(0) => tracing::info!("Steam status cleared: the game ended"),
                    code => tracing::info!(
                        ?code,
                        reason = %reason.trim(),
                        "Steam status helper ended without showing a status"
                    ),
                }
            }
            Err(error) => tracing::warn!(%error, "lost track of the Steam status helper"),
        }
    });
}

/// Run as the helper if this process was started as one.
///
/// `arguments` is the process's own, program name first. `None` means this is
/// the client, which carries on starting; `Some` is the helper's exit code,
/// once the game it was watching has gone. The shell asks before anything else
/// it does, because the helper must neither open a window nor hand itself to a
/// running client as a second instance.
pub fn run_helper_if_asked(arguments: &[String]) -> Option<i32> {
    if arguments.get(1).map(String::as_str) != Some(HELPER_FLAG) {
        return None;
    }
    Some(helper::run(arguments.get(2), arguments.get(3)))
}

/// What the helper's exit code says. Only [`Exit::GameEnded`] ever showed a
/// status; the rest are the reasons it could not.
// Every reason but `Unsupported` is only reached by the Steamworks helper,
// which is not built where Valve's library is not shipped: macOS among them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
#[cfg_attr(not(any(windows, target_os = "linux")), allow(dead_code))]
enum Exit {
    GameEnded = 0,
    Usage = 2,
    GameGone = 3,
    NoLibrary = 4,
    NoEntryPoint = 5,
    SteamRefused = 6,
    /// A platform the client is not released for.
    #[cfg_attr(any(windows, target_os = "linux"), allow(dead_code))]
    Unsupported = 7,
}

fn report(message: &str) {
    use std::io::Write as _;
    // The client reads this from the pipe and logs it. There is no console to
    // write to otherwise, so a failed write has nowhere better to go either.
    let _ = writeln!(std::io::stderr(), "{message}");
}

#[cfg(any(windows, target_os = "linux"))]
mod helper {
    use std::ffi::{c_char, c_void, CStr};

    use super::{report, system, Exit};

    /// How often the helper looks up from watching the game to let the library
    /// run its callbacks. The status needs none, but Valve asks every process
    /// that initialises the API to pump them, and once a second costs nothing.
    const POLL_MILLISECONDS: u32 = 1_000;
    /// `SteamErrMsg` in the SDK's `steam_api_common.h`.
    const ERROR_MESSAGE_LENGTH: usize = 1024;

    /// `SteamAPI_InitFlat`, the entry point the SDK documents for a library
    /// loaded at runtime: zero is success, anything else fills the message
    /// buffer with Steam's reason in English.
    type InitFlat = unsafe extern "C" fn(message: *mut c_char) -> i32;
    /// `SteamAPI_Init` as libraries from before `InitFlat` export it.
    type InitLegacy = unsafe extern "C" fn() -> bool;
    /// `SteamAPI_RunCallbacks` and `SteamAPI_Shutdown`.
    type Procedure = unsafe extern "C" fn();

    pub(super) fn run(library: Option<&String>, game_pid: Option<&String>) -> i32 {
        let (Some(path), Some(game_pid)) =
            (library, game_pid.and_then(|pid| pid.parse::<u32>().ok()))
        else {
            report("usage: --steam-presence <Steamworks library> <game process id>");
            return Exit::Usage as i32;
        };

        // The game first. If it has already gone there is nothing to show.
        let Some(game) = system::Game::open(game_pid) else {
            report("the game is not running");
            return Exit::GameGone as i32;
        };
        let library = match system::Library::load(path) {
            Ok(library) => library,
            Err(reason) => {
                report(&format!("cannot load {path}: {reason}"));
                return Exit::NoLibrary as i32;
            }
        };

        if let Some(init) = library.symbol(c"SteamAPI_InitFlat") {
            // Sound: the export has this signature in every SDK that has it
            // (`steam_api.h`, SDK 1.65).
            let init = unsafe { std::mem::transmute::<*mut c_void, InitFlat>(init) };
            let mut message: [c_char; ERROR_MESSAGE_LENGTH] = [0; ERROR_MESSAGE_LENGTH];
            // Sound: the buffer is the size the SDK declares for it, and the
            // library writes a nul-terminated message of at most that length.
            let result = unsafe { init(message.as_mut_ptr()) };
            if result != 0 {
                // Sound: zero-initialised and written nul-terminated, so a
                // terminator is always inside the buffer.
                let reason = unsafe { CStr::from_ptr(message.as_ptr()) };
                report(&format!(
                    "Steam refused (result {result}): {}",
                    reason.to_string_lossy()
                ));
                return Exit::SteamRefused as i32;
            }
        } else if let Some(init) = library.symbol(c"SteamAPI_Init") {
            // Sound: the export has this signature in every SDK that has it.
            let init = unsafe { std::mem::transmute::<*mut c_void, InitLegacy>(init) };
            // Sound: no arguments; the library reports failure as `false`.
            if !unsafe { init() } {
                report("Steam refused: it is not running, or Forged Alliance is not in this account's library");
                return Exit::SteamRefused as i32;
            }
        } else {
            report(&format!("{path} exports no Steam API entry point"));
            return Exit::NoEntryPoint as i32;
        }

        // Sound, for both: the exports take no arguments and have this
        // signature in every SDK; they are only called after a successful init.
        let run_callbacks = library
            .symbol(c"SteamAPI_RunCallbacks")
            .map(|address| unsafe { std::mem::transmute::<*mut c_void, Procedure>(address) });
        let shutdown = library
            .symbol(c"SteamAPI_Shutdown")
            .map(|address| unsafe { std::mem::transmute::<*mut c_void, Procedure>(address) });

        while game.still_running_after(POLL_MILLISECONDS) {
            if let Some(run_callbacks) = run_callbacks {
                // Sound: see above.
                unsafe { run_callbacks() };
            }
        }
        if let Some(shutdown) = shutdown {
            // Sound: see above; called once, after the last callback.
            unsafe { shutdown() };
        }
        Exit::GameEnded as i32
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod helper {
    use super::{report, Exit};

    pub(super) fn run(_library: Option<&String>, _game_pid: Option<&String>) -> i32 {
        report("Steam status is only available on Windows and Linux");
        Exit::Unsupported as i32
    }
}

/// The two things the helper needs from the operating system: a library loaded
/// at runtime, and a process it did not start to watch.
#[cfg(windows)]
mod system {
    use std::ffi::{c_char, c_void, CStr};
    use std::os::windows::ffi::OsStrExt as _;
    use std::path::Path;

    type Handle = isize;

    /// From `winnt.h`: the one right needed to wait on a process.
    const SYNCHRONIZE: u32 = 0x0010_0000;
    /// From `winbase.h`.
    const WAIT_TIMEOUT: u32 = 0x0000_0102;

    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, process_id: u32) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn CloseHandle(handle: Handle) -> i32;
        fn LoadLibraryW(file_name: *const u16) -> Handle;
        fn GetProcAddress(module: Handle, name: *const c_char) -> *mut c_void;
    }

    /// The game, by handle. Holding the handle from the start means a process
    /// id reused by something else later cannot keep the status alive.
    pub(super) struct Game(Handle);

    impl Game {
        pub(super) fn open(pid: u32) -> Option<Self> {
            // Sound: plain integer arguments; a failure returns a null handle.
            let handle = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
            (handle != 0).then_some(Self(handle))
        }

        /// Wait up to `milliseconds` for the game to end. Anything but a
        /// timeout counts as ended: the game exited, or the handle stopped
        /// being waitable, and either way there is no game left to show.
        pub(super) fn still_running_after(&self, milliseconds: u32) -> bool {
            // Sound: an open process handle with `SYNCHRONIZE` access.
            unsafe { WaitForSingleObject(self.0, milliseconds) == WAIT_TIMEOUT }
        }
    }

    impl Drop for Game {
        fn drop(&mut self) {
            // Sound: the handle came from a successful `OpenProcess` and is
            // closed exactly once, here.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// A loaded library. Never unloaded: the process exits soon after.
    pub(super) struct Library(Handle);

    impl Library {
        pub(super) fn load(path: &str) -> Result<Self, String> {
            let wide: Vec<u16> = Path::new(path)
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            // Sound: `wide` is a nul-terminated UTF-16 path that outlives the
            // call.
            let module = unsafe { LoadLibraryW(wide.as_ptr()) };
            if module == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            Ok(Self(module))
        }

        pub(super) fn symbol(&self, name: &CStr) -> Option<*mut c_void> {
            // Sound: a loaded library that is never unloaded, and a
            // nul-terminated name that outlives the call.
            let address = unsafe { GetProcAddress(self.0, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        }
    }
}

#[cfg(target_os = "linux")]
mod system {
    use std::ffi::{c_char, c_int, c_void, CStr, CString};
    use std::time::Duration;

    /// From `dlfcn.h`: resolve every symbol at load time, so a library missing
    /// one fails here rather than in the middle of a call.
    const RTLD_NOW: c_int = 2;
    /// From `errno.h`: the process exists but belongs to somebody else.
    const EPERM: i32 = 1;

    // In glibc itself since 2.34, and in `libdl`, which the standard library
    // links on this target, before that.
    extern "C" {
        fn dlopen(file_name: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }

    /// The game, by process id. Linux has no handle to hold on to without a
    /// newer kernel than every player has, so this asks once a second whether
    /// the process still exists. A reused id in the second after the game
    /// ends is the only way to be wrong, and the client reaps its child within
    /// half a second of it exiting, before any reuse is likely.
    pub(super) struct Game(c_int);

    impl Game {
        pub(super) fn open(pid: u32) -> Option<Self> {
            let game = Self(c_int::try_from(pid).ok()?);
            game.exists().then_some(game)
        }

        fn exists(&self) -> bool {
            // Sound: signal 0 sends nothing; it only checks the process exists
            // and that we may signal it.
            if unsafe { kill(self.0, 0) } == 0 {
                return true;
            }
            std::io::Error::last_os_error().raw_os_error() == Some(EPERM)
        }

        pub(super) fn still_running_after(&self, milliseconds: u32) -> bool {
            std::thread::sleep(Duration::from_millis(u64::from(milliseconds)));
            self.exists()
        }
    }

    /// A loaded library. Never unloaded: the process exits soon after.
    pub(super) struct Library(*mut c_void);

    impl Library {
        pub(super) fn load(path: &str) -> Result<Self, String> {
            let path = CString::new(path).map_err(|error| error.to_string())?;
            // Sound: a nul-terminated path that outlives the call.
            let handle = unsafe { dlopen(path.as_ptr(), RTLD_NOW) };
            if handle.is_null() {
                // Sound: `dlerror` returns the message for the failure just
                // above, or null, and the message stays valid until the next
                // `dl*` call on this thread.
                let message = unsafe { dlerror() };
                return Err(if message.is_null() {
                    "unknown error".to_string()
                } else {
                    unsafe { CStr::from_ptr(message) }
                        .to_string_lossy()
                        .into_owned()
                });
            }
            Ok(Self(handle))
        }

        pub(super) fn symbol(&self, name: &CStr) -> Option<*mut c_void> {
            // Sound: a loaded library that is never unloaded, and a
            // nul-terminated name that outlives the call.
            let address = unsafe { dlsym(self.0, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(list: &[&str]) -> Vec<String> {
        list.iter().map(|argument| argument.to_string()).collect()
    }

    #[test]
    fn the_client_is_not_the_helper() {
        assert_eq!(run_helper_if_asked(&arguments(&["faf.exe"])), None);
        assert_eq!(
            run_helper_if_asked(&arguments(&["faf.exe", "replay.fafreplay"])),
            None
        );
        // Only as the first argument: a replay path that happens to follow
        // the flag's spelling somewhere later is still the client.
        assert_eq!(
            run_helper_if_asked(&arguments(&["faf.exe", "x", HELPER_FLAG])),
            None
        );
    }

    #[test]
    fn a_helper_without_a_game_exits_without_showing_anything() {
        let code = run_helper_if_asked(&arguments(&["faf.exe", HELPER_FLAG]));
        assert!(matches!(code, Some(code) if code != Exit::GameEnded as i32));
    }

    #[test]
    fn a_helper_for_a_game_that_is_gone_says_so() {
        // Not a process id any system hands out: the largest a pid can be on
        // Linux is 2^22, and Windows ids are multiples of four.
        let code = run_helper_if_asked(&arguments(&[
            "faf.exe",
            HELPER_FLAG,
            "steam_api64.dll",
            "4194311",
        ]));
        if LIBRARY.is_some() {
            assert_eq!(code, Some(Exit::GameGone as i32));
        }
    }
}

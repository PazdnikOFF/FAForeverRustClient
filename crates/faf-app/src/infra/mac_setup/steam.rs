//! Downloading Forged Alliance from Steam with the player's own account.
//!
//! Steam for macOS will not install a game that has no Mac build, so the
//! Windows depots are fetched directly with DepotDownloader, SteamRE's open
//! source depot client (GPL-2.0), run as a separate program. It signs in with
//! the player's account, so only a copy the account owns can be downloaded.
//!
//! The password is written to the downloader's standard input when it asks
//! for it, never put on its command line, where every process on the machine
//! could read it with `ps`. It is not stored: `-remember-password` makes the
//! downloader keep Steam's own refresh token instead, in its own settings, so
//! a later repair or update does not ask again.
//!
//! The downloader is driven by reading what it prints. [`Parser`] is that
//! reading, kept free of IO so the prompts and failure sentences it relies on
//! are pinned by tests against the strings DepotDownloader 3.4.0 prints.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use faf_domain::state::SteamGuardKind;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

use super::download::{self, Pinned, Span};
use super::{is_game_dir, mark, marked, Layout};
use crate::ports::mac_setup::{SteamUpdate, StepUpdate};

/// Supreme Commander: Forged Alliance.
const APP_ID: &str = "9420";

const TOOL_VERSION: &str = "DepotDownloader-3.4.0";

const TOOL: Pinned = Pinned {
    url: "https://github.com/SteamRE/DepotDownloader/releases/download/DepotDownloader_3.4.0/DepotDownloader-macos-arm64.zip",
    sha256: "60e80c7c496f3f9a079cd3c62036b35d088c27bc0149baf38f009eb57a52f6a5",
    label: "the Steam downloader",
};

use crate::ports::mac_setup::STEAM_CANCELLED as CANCELLED;

/// The handle other commands reach a running download through.
pub(super) struct Running {
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    waiting_for_code: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    /// Set when the player would rather type a code than approve the sign-in
    /// in the Steam Mobile app: the downloader is stopped and started again
    /// with `-no-mobile`.
    use_code: Arc<AtomicBool>,
    kill: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Running {
    pub(super) async fn answer(&mut self, code: &str) -> bool {
        if !self.waiting_for_code.swap(false, Ordering::SeqCst) {
            return false;
        }
        write_line(&self.stdin, code.trim()).await
    }

    pub(super) fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Some(kill) = self.kill.take() {
            let _ = kill.send(());
        }
    }

    /// Stop waiting for the mobile approval and ask for a code instead.
    pub(super) fn use_code(&mut self) -> bool {
        let Some(kill) = self.kill.take() else {
            return false;
        };
        self.use_code.store(true, Ordering::SeqCst);
        kill.send(()).is_ok()
    }
}

async fn write_line(stdin: &Mutex<Option<ChildStdin>>, text: &str) -> bool {
    let mut guard = stdin.lock().await;
    let Some(stdin) = guard.as_mut() else {
        return false;
    };
    let line = format!("{text}\n");
    stdin.write_all(line.as_bytes()).await.is_ok() && stdin.flush().await.is_ok()
}

/// What a line or a prompt from the downloader means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Action {
    SendPassword,
    Report(SteamUpdate),
}

/// Reads the downloader's output as it arrives.
#[derive(Debug, Default)]
pub(super) struct Parser {
    partial: String,
    previous_code_refused: bool,
    downloading: bool,
    /// The most specific failure seen so far, as a sentence for the player.
    pub failure: Option<String>,
}

impl Parser {
    /// Feed a chunk of output. Prompts end without a newline, so the line in
    /// progress is checked for one after every chunk.
    pub(super) fn feed(&mut self, chunk: &str) -> Vec<Action> {
        let mut actions = Vec::new();
        self.partial.push_str(chunk);
        while let Some(end) = self.partial.find(['\n', '\r']) {
            let line: String = self.partial.drain(..=end).collect();
            actions.extend(self.line(line.trim()));
        }
        if let Some(action) = self.prompt() {
            self.partial.clear();
            actions.push(action);
        }
        actions
    }

    fn prompt(&mut self) -> Option<Action> {
        let text = self.partial.trim();
        if text.starts_with("Enter account password for") {
            return Some(Action::SendPassword);
        }
        let authenticator = text.contains("2-factor auth code from your authenticator app")
            || text.contains("2 factor auth code from your authenticator app");
        if authenticator && text.ends_with(':') {
            return Some(self.needs_code(SteamGuardKind::Authenticator));
        }
        if let Some(rest) = text.split("auth code sent to the email at ").nth(1) {
            if text.ends_with(':') {
                let hint = rest.trim_end_matches(':').trim().to_string();
                return Some(self.needs_code(SteamGuardKind::Email { hint }));
            }
        }
        if text.starts_with("Please enter the authentication code sent to your email address")
            && text.ends_with(':')
        {
            return Some(self.needs_code(SteamGuardKind::Email {
                hint: String::new(),
            }));
        }
        None
    }

    fn needs_code(&mut self, kind: SteamGuardKind) -> Action {
        let retry = std::mem::take(&mut self.previous_code_refused);
        Action::Report(SteamUpdate::NeedsCode { kind, retry })
    }

    fn line(&mut self, line: &str) -> Vec<Action> {
        if line.is_empty() {
            return Vec::new();
        }
        if line.contains("previous 2-factor auth code you have provided is incorrect") {
            self.previous_code_refused = true;
            return Vec::new();
        }
        if line.contains("Use the Steam Mobile App to confirm your sign in") {
            return vec![Action::Report(SteamUpdate::AwaitingApproval)];
        }
        if let Some(percent) = percent_of(line) {
            self.downloading = true;
            return vec![Action::Report(SteamUpdate::Downloading {
                permille: Some((percent * 10.0).clamp(0.0, 1000.0) as u32),
                done_mb: 0,
                total_mb: 0,
            })];
        }
        if !self.downloading
            && (line.starts_with("Downloading depot") || line.starts_with("Processing depot"))
        {
            self.downloading = true;
            return vec![Action::Report(SteamUpdate::Downloading {
                permille: None,
                done_mb: 0,
                total_mb: 0,
            })];
        }
        // The first reason is the specific one; the downloader follows it
        // with a generic "InitializeSteam failed" that says less.
        if self.failure.is_none() {
            self.failure = failure_of(line);
        }
        Vec::new()
    }
}

/// The percentage at the start of a file-progress line: ` 12.34% path`.
fn percent_of(line: &str) -> Option<f64> {
    let (number, rest) = line.split_once('%')?;
    if !rest.starts_with(' ') {
        return None;
    }
    number.trim().parse::<f64>().ok()
}

/// A failure the downloader printed, as a sentence the player can act on.
fn failure_of(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    if lower.contains("invalidpassword") {
        return Some("Steam did not accept that account name and password".into());
    }
    if lower.contains("ratelimitexceeded") || lower.contains("accountlogindeniedthrottle") {
        return Some(
            "Steam is refusing sign-ins for a while after too many attempts: try again later"
                .into(),
        );
    }
    if lower.contains("twofactorcodemismatch") || lower.contains("invalidloginauthcode") {
        return Some("Steam did not accept that Steam Guard code".into());
    }
    if lower.contains("is not available from this account")
        || lower.contains("insufficient privileges")
    {
        return Some("This Steam account does not own Supreme Commander: Forged Alliance".into());
    }
    if lower.contains("could not connect to steam") || lower.contains("unable to login to steam3") {
        return Some(format!("Steam could not be reached: {line}"));
    }
    if lower.contains("asyncjobfailedexception") || lower.contains("timeout") {
        return Some(
            "Steam did not finish the sign-in in time: try again, and approve it or enter the code promptly".into(),
        );
    }
    if lower.contains("initializesteam failed") {
        return Some(
            "Steam did not complete the sign-in: check the password and the Steam Guard code, then try again".into(),
        );
    }
    if lower.starts_with("failed to authenticate with steam") || lower.starts_with("error") {
        return Some(line.to_string());
    }
    None
}

/// Make sure the downloader is in the tools folder.
async fn ensure_tool(
    layout: &Layout,
    http: &reqwest::Client,
) -> Result<std::path::PathBuf, String> {
    let dir = layout.tools_dir().join("depotdownloader");
    let program = dir.join("DepotDownloader");
    if marked(&dir, TOOL_VERSION) && program.is_file() {
        return Ok(program);
    }
    let archive = layout.downloads().join("DepotDownloader-macos-arm64.zip");
    // Its progress is not worth a bar of its own: thirty megabytes, once.
    let (sink, mut drain) = mpsc::channel::<StepUpdate>(8);
    tokio::spawn(async move { while drain.recv().await.is_some() {} });
    download::fetch(http, TOOL, &archive, Span::WHOLE, &sink).await?;

    let _ = tokio::fs::remove_dir_all(&dir).await;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    let status = Command::new("/usr/bin/ditto")
        .args(["-x", "-k"])
        .arg(&archive)
        .arg(&dir)
        .status()
        .await
        .map_err(|error| format!("could not unpack the Steam downloader: {error}"))?;
    if !status.success() || !program.is_file() {
        return Err("could not unpack the Steam downloader".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("could not make the Steam downloader executable: {error}"))?;
    }
    mark(&dir, TOOL_VERSION)?;
    let _ = tokio::fs::remove_file(&archive).await;
    Ok(program)
}

/// Start a download. Returns at once; everything is reported on `updates`.
pub(super) fn start(
    layout: Layout,
    http: reqwest::Client,
    slot: Arc<Mutex<Option<Running>>>,
    username: String,
    password: String,
    updates: mpsc::Sender<SteamUpdate>,
) {
    tokio::spawn(async move {
        let outcome = run(&layout, &http, &slot, &username, password, &updates).await;
        *slot.lock().await = None;
        if outcome.is_ok() {
            let _ = std::fs::write(layout.steam_user_record(), &username);
            // The client's own download is the folder in use from now on,
            // even if the player had pointed at another one before.
            let _ = std::fs::remove_file(layout.game_dir_record());
        }
        match &outcome {
            Ok(()) => tracing::info!("Forged Alliance was downloaded from Steam"),
            Err(reason) if reason == CANCELLED => {
                tracing::info!("the Steam download was cancelled")
            }
            Err(reason) => tracing::warn!(%reason, "the Steam download failed"),
        }
        let _ = updates.send(SteamUpdate::Finished(outcome)).await;
    });
}

async fn run(
    layout: &Layout,
    http: &reqwest::Client,
    slot: &Arc<Mutex<Option<Running>>>,
    username: &str,
    password: String,
    updates: &mpsc::Sender<SteamUpdate>,
) -> Result<(), String> {
    let username = username.trim();
    if username.is_empty() {
        return Err("enter your Steam account name".into());
    }
    let program = ensure_tool(layout, http).await?;
    let target = layout.default_game_dir();
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|error| format!("could not create {}: {error}", target.display()))?;

    // The password stays in memory until the sign-in is over, because
    // switching to a code means signing in again from the start.
    let mut no_mobile = false;
    loop {
        match attempt(
            &program, &target, slot, username, &password, no_mobile, updates,
        )
        .await?
        {
            Attempt::Finished => return Ok(()),
            Attempt::UseCode => {
                no_mobile = true;
                let _ = updates.send(SteamUpdate::Restarting).await;
            }
        }
    }
}

/// How one run of the downloader ended, when it did not fail.
enum Attempt {
    Finished,
    /// Stopped so it can be started again asking for a code.
    UseCode,
}

async fn attempt(
    program: &Path,
    target: &Path,
    slot: &Arc<Mutex<Option<Running>>>,
    username: &str,
    password: &str,
    no_mobile: bool,
    updates: &mpsc::Sender<SteamUpdate>,
) -> Result<Attempt, String> {
    let mut command = Command::new(program);
    if no_mobile {
        command.arg("-no-mobile");
    }
    let mut child = command
        .args([
            "-app",
            APP_ID,
            "-os",
            "windows",
            "-all-archs",
            "-max-downloads",
            "8",
        ])
        .arg("-dir")
        .arg(target)
        .args(["-username", username, "-remember-password"])
        .current_dir(program.parent().unwrap_or(Path::new("/")))
        // The downloader's own settings (the remembered sign-in) follow HOME;
        // it is the user's, so that is where it belongs.
        .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("could not start the Steam downloader: {error}"))?;

    let stdin = Arc::new(Mutex::new(child.stdin.take()));
    let waiting_for_code = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));
    let use_code = Arc::new(AtomicBool::new(false));
    let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel();
    *slot.lock().await = Some(Running {
        stdin: stdin.clone(),
        waiting_for_code: waiting_for_code.clone(),
        cancelled: cancelled.clone(),
        use_code: use_code.clone(),
        kill: Some(kill_tx),
    });

    // Both streams into one channel: the prompts are on stderr, the progress
    // on stdout, and their relative order matters less than seeing both.
    let (text_tx, mut text_rx) = mpsc::channel::<String>(64);
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let text_tx = text_tx.clone();
        tokio::spawn(async move {
            let mut stream = stream;
            let mut buffer = [0u8; 4096];
            while let Ok(read) = stream.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                if text_tx.send(text).await.is_err() {
                    break;
                }
            }
        });
    }
    drop(text_tx);

    let mut parser = Parser::default();
    let mut password = Some(password.to_string());
    let mut killed = false;
    loop {
        tokio::select! {
            chunk = text_rx.recv() => {
                let Some(chunk) = chunk else { break };
                for action in parser.feed(&chunk) {
                    match action {
                        Action::SendPassword => match password.take().filter(|p| !p.is_empty()) {
                            Some(secret) => {
                                write_line(&stdin, &secret).await;
                            }
                            None => {
                                parser.failure = Some(if parser.failure.is_some() {
                                    "Steam did not accept that account name and password".into()
                                } else {
                                    "enter your Steam password".into()
                                });
                                let _ = child.start_kill();
                                killed = true;
                            }
                        },
                        Action::Report(update) => {
                            if matches!(update, SteamUpdate::NeedsCode { .. }) {
                                waiting_for_code.store(true, Ordering::SeqCst);
                            }
                            let _ = updates.send(update).await;
                        }
                    }
                }
            }
            _ = &mut kill_rx, if !killed => {
                let _ = child.start_kill();
                killed = true;
            }
        }
    }
    let status = child.wait().await;
    *stdin.lock().await = None;

    if cancelled.load(Ordering::SeqCst) {
        return Err(CANCELLED.into());
    }
    if use_code.load(Ordering::SeqCst) {
        return Ok(Attempt::UseCode);
    }
    if is_game_dir(target) && status.as_ref().is_ok_and(|status| status.success()) {
        return Ok(Attempt::Finished);
    }
    Err(parser.failure.unwrap_or_else(|| {
        "the download did not finish: check your connection and try again".into()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_password_prompt_asks_for_the_password() {
        let mut parser = Parser::default();
        assert_eq!(
            parser.feed("Logging 'player' into Steam3...\nEnter account password for \"player\": "),
            vec![Action::SendPassword]
        );
    }

    #[test]
    fn a_prompt_split_across_chunks_is_still_seen_once() {
        let mut parser = Parser::default();
        assert!(parser
            .feed("STEAM GUARD! Please enter your 2-factor ")
            .is_empty());
        assert_eq!(
            parser.feed("auth code from your authenticator app: "),
            vec![Action::Report(SteamUpdate::NeedsCode {
                kind: SteamGuardKind::Authenticator,
                retry: false
            })]
        );
        assert!(
            parser.feed("").is_empty(),
            "answered prompts are not repeated"
        );
    }

    #[test]
    fn a_refused_code_marks_the_next_question_as_a_retry() {
        let mut parser = Parser::default();
        let actions = parser.feed(
            "The previous 2-factor auth code you have provided is incorrect.\n\
             STEAM GUARD! Please enter the auth code sent to the email at pl***@example.com: ",
        );
        assert_eq!(
            actions,
            vec![Action::Report(SteamUpdate::NeedsCode {
                kind: SteamGuardKind::Email {
                    hint: "pl***@example.com".into()
                },
                retry: true
            })]
        );
    }

    #[test]
    fn mobile_approval_and_progress_are_reported() {
        let mut parser = Parser::default();
        assert_eq!(
            parser.feed("STEAM GUARD! Use the Steam Mobile App to confirm your sign in...\n"),
            vec![Action::Report(SteamUpdate::AwaitingApproval)]
        );
        assert_eq!(
            parser.feed(" 42.50% /game/gamedata/units.scd\n"),
            vec![Action::Report(SteamUpdate::Downloading {
                permille: Some(425),
                done_mb: 0,
                total_mb: 0
            })]
        );
    }

    #[test]
    fn a_sign_in_that_never_completes_is_explained() {
        let mut parser = Parser::default();
        parser.feed("Failed to authenticate with Steam: Exception of type 'SteamKit2.AsyncJobFailedException' was thrown.\n");
        assert!(parser
            .failure
            .as_deref()
            .unwrap()
            .starts_with("Steam did not finish the sign-in"));
        // A later generic line must not replace the specific reason with a
        // vaguer one: the first explanation is the useful one.
        let mut parser = Parser::default();
        parser.feed("Error: InitializeSteam failed\n");
        assert!(parser
            .failure
            .as_deref()
            .unwrap()
            .starts_with("Steam did not complete the sign-in"));
    }

    #[test]
    fn failures_become_sentences_the_player_can_act_on() {
        let mut parser = Parser::default();
        parser.feed("Failed to authenticate with Steam: InvalidPassword\n");
        assert_eq!(
            parser.failure.as_deref(),
            Some("Steam did not accept that account name and password")
        );
        let mut parser = Parser::default();
        parser.feed("Depot 9421 is not available from this account.\n");
        assert_eq!(
            parser.failure.as_deref(),
            Some("This Steam account does not own Supreme Commander: Forged Alliance")
        );
    }
}

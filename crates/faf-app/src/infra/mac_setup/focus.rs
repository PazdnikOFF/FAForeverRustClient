//! Handing the focus from the client to the game when it starts.
//!
//! macOS (14 and later) activates applications cooperatively: one that is in
//! the background cannot bring itself forward, and the game, started while the
//! client is the active application, is exactly that. Its window came up
//! minimised or behind, and behind anything in full screen on its display.
//! What macOS does allow is the active application handing activation on, so
//! the client does it, once the game's process is there.
//!
//! Only while the client is still the active application: if the player has
//! gone somewhere else in the meantime, that is where they want to be, and the
//! game is not pulled in front of it. Activation also moves to the Space the
//! game's window is on, which is what lifts it over another full-screen
//! window.

use std::time::{Duration, Instant};

/// How long to keep trying after the launch: a cold start compiles shaders
/// before it shows a window, and activating an application with no window
/// yet does not keep it active.
const WINDOW: Duration = Duration::from_secs(90);
const POLL: Duration = Duration::from_millis(500);

/// The process running `ForgedAlliance.exe` itself, not the `start.exe` that
/// launched it, which names the same executable on its command line.
fn game_pid() -> Option<i32> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-axo", "pid=,command="])
        .output()
        .ok()?;
    parse_game_pid(&String::from_utf8_lossy(&output.stdout))
}

fn parse_game_pid(listing: &str) -> Option<i32> {
    listing.lines().find_map(|line| {
        let line = line.trim_start();
        let (pid, command) = line.split_once(' ')?;
        let lower = command.to_ascii_lowercase();
        (lower.contains("forgedalliance.exe") && !lower.contains("start.exe"))
            .then(|| pid.parse().ok())
            .flatten()
    })
}

/// Start handing the focus over, in the background. Returns at once.
pub(crate) fn hand_focus_to_game() {
    if !cfg!(target_os = "macos") {
        return;
    }
    std::thread::spawn(|| {
        let started = Instant::now();
        while started.elapsed() < WINDOW {
            std::thread::sleep(POLL);
            if !client_is_active() {
                return;
            }
            if let Some(pid) = game_pid() {
                if activate(pid) {
                    tracing::info!("handed the focus to the game");
                    return;
                }
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn client_is_active() -> bool {
    use objc2_app_kit::NSRunningApplication;
    NSRunningApplication::currentApplication().isActive()
}

/// Activate `pid`. True once it is the active application.
#[cfg(target_os = "macos")]
fn activate(pid: i32) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    let Some(game) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
        return false;
    };
    if game.isActive() {
        return true;
    }
    game.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
    std::thread::sleep(Duration::from_millis(200));
    game.isActive()
}

#[cfg(not(target_os = "macos"))]
fn client_is_active() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
fn activate(_pid: i32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_is_found_and_its_launcher_is_not() {
        let listing =
            "  101 /wine/bin/wine64-preloader start.exe /exec /a/ForgedAlliance.exe /init x\n  \
                       202 /wine/bin/wine64-preloader /a/ForgedAlliance.exe /init x\n  \
                       303 /usr/bin/java -jar ice.jar\n";
        assert_eq!(parse_game_pid(listing), Some(202));
        assert_eq!(parse_game_pid("  1 /sbin/launchd\n"), None);
    }
}

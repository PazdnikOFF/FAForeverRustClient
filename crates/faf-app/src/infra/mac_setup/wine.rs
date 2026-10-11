//! Rosetta 2, Wine, and the prefix the game runs in.
//!
//! The Wine build is CrossOver's FOSS Wine, wine-crossover 23.7.1-1. It is the
//! one build known to hold a multiplayer game of Forged Alliance in sync on
//! Apple silicon: the official FAF client's macOS scripts were soak-tested on
//! it over fifty multiplayer games, and they record why the two obvious
//! alternatives are unusable (Apple's Game Porting Toolkit asserts in
//! `alloc_pages_vprot` at startup; WineHQ 11 crashes mid-match). The original
//! Gcenx release was withdrawn in 2026, so it is fetched from the mirror the
//! FAF client documents, and pinned by checksum so the mirror cannot change it.
//!
//! The prefix gets the DirectX runtime pieces Forged Alliance loads: the
//! `d3dx9` helper libraries and the XACT audio engine. They come from
//! Microsoft's own June 2010 redistributable, extracted with the `tar` macOS
//! ships (libarchive reads cabinet files, including the self-extracting one),
//! and installed the way winetricks' `d3dx9` and `xact` verbs install them.
//! That avoids winetricks and the `cabextract` it needs, neither of which a
//! Mac has.

use std::path::{Path, PathBuf};

use tokio::process::Command;
use tokio::sync::mpsc;

use super::download::{self, Pinned, Span};
use super::{mark, Layout};
use crate::ports::mac_setup::StepUpdate;

/// The marker contents for a finished Wine. Bump with the pin.
pub(super) const WINE_VERSION: &str = "wine-crossover-23.7.1-1";

const WINE: Pinned = Pinned {
    url: "https://github.com/jfuruness/wine-crossover-mac/releases/download/23.7.1-1/wine-crossover-23.7.1-1-osx64.tar.xz",
    sha256: "e24ba084737c8823e8439f7cb75d436a917fd92fc34b832bcaa0c0037eb33d03",
    label: "Wine",
};

/// The marker contents for a finished prefix: the Wine it was made with and
/// the DirectX pieces in it. Bump when either changes.
pub(super) const PREFIX_VERSION: &str = "wine-crossover-23.7.1-1+directx-jun2010.1";

/// The performance settings applied on top of a finished prefix, recorded in
/// their own marker so a change to them is applied to an existing prefix in
/// place: rebuilding it would throw away the player's `game.prefs`.
pub(super) const TUNING_VERSION: &str = "tuning.2";
const TUNING_MARKER: &str = ".faf-tuning";

/// Wine settings that matter for Forged Alliance on a Mac.
///
/// `VideoMemorySize`: wined3d does not recognise Apple GPUs and can fall back
/// to a small default, which makes the game stream textures constantly. Four
/// gigabytes is more than a 32-bit DirectX 9 game can address and well within
/// what any Apple-silicon Mac shares with its GPU.
///
/// `RetinaMode` off: the game renders at the resolution it asks for, scaled
/// by macOS, instead of at the display's full pixel count, which on a 5K
/// display quadruples the fill work for no gain in a 2007 engine.
///
/// `UseConfinementCursorClipping`: when the game confines the cursor to its
/// window (`ClipCursor`), do it with the window's own confinement rectangle,
/// which needs no Accessibility permission, rather than an event tap, which
/// does and silently fails without it. That is what keeps the cursor from
/// sliding onto a second display instead of scrolling the map.
const TUNING_REG: &str = "REGEDIT4\n\n\
[HKEY_CURRENT_USER\\Software\\Wine\\Direct3D]\n\
\"VideoMemorySize\"=\"4096\"\n\n\
[HKEY_CURRENT_USER\\Software\\Wine\\Mac Driver]\n\
\"RetinaMode\"=\"n\"\n\
\"UseConfinementCursorClipping\"=\"y\"\n\n";

/// Whether the prefix carries the current tuning.
pub(super) fn tuned(prefix: &Path) -> bool {
    std::fs::read_to_string(prefix.join(TUNING_MARKER))
        .is_ok_and(|text| text.trim() == TUNING_VERSION)
}

async fn apply_tuning(layout: &Layout) -> Result<(), String> {
    let reg = layout.downloads().join("faf-tuning.reg");
    if let Some(parent) = reg.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(&reg, TUNING_REG)
        .await
        .map_err(|error| format!("could not write the Wine settings: {error}"))?;
    let reg_windows = format!("Z:{}", reg.display().to_string().replace('/', "\\"));
    run_wine(
        layout,
        &["regedit", "/S", &reg_windows],
        "apply the Wine settings",
    )
    .await?;
    wait_for_wineserver(layout).await;
    let _ = tokio::fs::remove_file(&reg).await;
    std::fs::write(layout.prefix().join(TUNING_MARKER), TUNING_VERSION)
        .map_err(|error| format!("could not record the Wine settings: {error}"))
}

/// The checksum winetricks pins for the same file.
const DIRECTX: Pinned = Pinned {
    url: "https://download.microsoft.com/download/8/4/A/84A35BF1-DAFE-4AE8-82AF-AD2AE20B6B14/directx_Jun2010_redist.exe",
    sha256: "053f76dcbb28802e23341b6a787e3b0791c0fa5c8d4d011b1044172dbf89c73b",
    label: "the DirectX runtime",
};

/// Which cabinets inside the redistributable to open, by lower-case name.
/// 32-bit only: Forged Alliance is a 32-bit game.
fn wanted_cabinet(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with("_x86.cab")
        && ["d3dx9_", "_xact_", "_xaudio_", "_x3daudio_"]
            .iter()
            .any(|part| name.contains(part))
}

/// Which libraries inside those cabinets to install, by lower-case name.
fn wanted_library(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.ends_with(".dll")
        && ["d3dx9_", "xactengine", "xaudio2_", "x3daudio"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

/// Whether a library is a COM server that has to be registered, as
/// winetricks' `xact` verb does with `regsvr32`.
fn needs_registration(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("xactengine") || name.starts_with("xaudio2_")
}

/// The registry file that makes Wine prefer the installed libraries over its
/// own. `native,builtin` rather than winetricks' bare `native` so a library
/// that somehow went missing falls back instead of failing to load.
fn overrides_reg(libraries: &[String]) -> String {
    let mut text = String::from("REGEDIT4\n\n[HKEY_CURRENT_USER\\Software\\Wine\\DllOverrides]\n");
    let mut names: Vec<String> = libraries
        .iter()
        .map(|name| {
            name.to_ascii_lowercase()
                .trim_end_matches(".dll")
                .to_string()
        })
        .collect();
    names.sort();
    names.dedup();
    for name in names {
        text.push_str(&format!("\"{name}\"=\"native,builtin\"\n"));
    }
    text.push('\n');
    text
}

/// Install Rosetta 2 through macOS's own administrator prompt.
///
/// `softwareupdate` needs root, and the only honest way for an application to
/// get it is to ask macOS to ask the user: `osascript … with administrator
/// privileges` raises the standard authentication dialog, which names the
/// reason given here. The client never sees the password.
pub(super) async fn install_rosetta() -> Result<(), String> {
    if super::rosetta_ready().await {
        return Ok(());
    }
    let script = "do shell script \"/usr/sbin/softwareupdate --install-rosetta --agree-to-license\" \
                  with prompt \"FAForever Client needs Rosetta to run Supreme Commander: Forged Alliance.\" \
                  with administrator privileges";
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .output()
        .await
        .map_err(|error| format!("could not ask macOS to install Rosetta: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        // -128 is AppleScript's "User canceled".
        return Err(if detail.contains("-128") {
            "Rosetta was not installed because the password prompt was cancelled".into()
        } else {
            format!("macOS could not install Rosetta: {}", detail.trim())
        });
    }
    if super::rosetta_ready().await {
        Ok(())
    } else {
        Err("Rosetta reports as installed but cannot run Intel programs yet: restart the Mac and try again".into())
    }
}

/// Run `/usr/bin/tar` and turn a failure into a sentence.
async fn tar(args: &[&std::ffi::OsStr], what: &str) -> Result<String, String> {
    let output = Command::new("/usr/bin/tar")
        .args(args)
        .output()
        .await
        .map_err(|error| format!("could not unpack {what}: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not unpack {what}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Unpack an archive into a fresh `staging` directory.
pub(super) async fn unpack(archive: &Path, staging: &Path, what: &str) -> Result<(), String> {
    let _ = tokio::fs::remove_dir_all(staging).await;
    tokio::fs::create_dir_all(staging)
        .await
        .map_err(|error| format!("could not create {}: {error}", staging.display()))?;
    tar(
        &[
            "-xf".as_ref(),
            archive.as_os_str(),
            "-C".as_ref(),
            staging.as_os_str(),
        ],
        what,
    )
    .await
    .map(|_| ())
}

fn indeterminate() -> StepUpdate {
    StepUpdate::Progress {
        permille: None,
        done_mb: 0,
        total_mb: 0,
    }
}

pub(super) async fn install_wine(
    layout: &Layout,
    http: &reqwest::Client,
    progress: &mpsc::Sender<StepUpdate>,
) -> Result<(), String> {
    let archive = layout.downloads().join("wine-crossover.tar.xz");
    download::fetch(http, WINE, &archive, Span::WHOLE, progress).await?;
    let _ = progress.send(indeterminate()).await;

    let staging = layout.root().join("wine.unpacking");
    unpack(&archive, &staging, "Wine").await?;
    let unpacked = staging.join("Wine Crossover.app");
    if !unpacked
        .join("Contents/Resources/wine/bin/wine64")
        .is_file()
    {
        return Err("the Wine archive does not contain Wine Crossover.app".into());
    }

    // A Wine that is replaced must not leave a server from the old one running
    // against the prefix.
    stop_wineserver(layout).await;
    let target = layout.wine_dir();
    let _ = tokio::fs::remove_dir_all(&target).await;
    tokio::fs::create_dir_all(&target)
        .await
        .map_err(|error| format!("could not create {}: {error}", target.display()))?;
    tokio::fs::rename(&unpacked, layout.wine_app())
        .await
        .map_err(|error| format!("could not move Wine into place: {error}"))?;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    // Files the client downloads itself carry no quarantine flag, but a copy
    // that arrived some other way might; Gatekeeper would then stop every one
    // of Wine's binaries on first launch. Best effort.
    let _ = Command::new("/usr/bin/xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(layout.wine_app())
        .status()
        .await;
    mark(&target, WINE_VERSION)?;
    let _ = tokio::fs::remove_file(&archive).await;
    Ok(())
}

/// A Wine command against the managed prefix, quiet and with no installer
/// prompts: `mscoree` and `mshtml` disabled keeps `wineboot` from offering to
/// download Mono and Gecko in a window of its own, which the game needs
/// neither of.
fn wine_command(layout: &Layout, program: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env("WINEPREFIX", layout.prefix())
        .env("WINEDEBUG", "-all")
        .env("WINEDLLOVERRIDES", "mscoree,mshtml=")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    command
}

async fn run_wine(layout: &Layout, args: &[&str], what: &str) -> Result<(), String> {
    let status = wine_command(layout, &layout.wine64())
        .args(args)
        .status()
        .await
        .map_err(|error| format!("could not start Wine to {what}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Wine could not {what} ({status})"))
    }
}

/// Wait for the prefix's server to exit, so every registry write is on disk.
async fn wait_for_wineserver(layout: &Layout) {
    let _ = wine_command(layout, &layout.wineserver())
        .arg("-w")
        .status()
        .await;
}

async fn stop_wineserver(layout: &Layout) {
    if layout.wineserver().is_file() && layout.prefix().is_dir() {
        let _ = wine_command(layout, &layout.wineserver())
            .arg("-k")
            .status()
            .await;
    }
}

pub(super) async fn remove_prefix(layout: &Layout) -> Result<(), String> {
    stop_wineserver(layout).await;
    match tokio::fs::remove_dir_all(layout.prefix()).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not remove the Wine prefix: {error}")),
    }
}

pub(super) async fn install_prefix(
    layout: &Layout,
    http: &reqwest::Client,
    progress: &mpsc::Sender<StepUpdate>,
) -> Result<(), String> {
    if !layout.wine64().is_file() {
        return Err("Wine is not installed yet".into());
    }
    // A finished prefix that only lacks the current tuning gets the tuning,
    // not a rebuild: the player's game settings live in it.
    if super::marked(&layout.prefix(), PREFIX_VERSION) {
        return apply_tuning(layout).await;
    }
    // Fetch first: the download is the slow, fallible part, and doing it
    // before touching the prefix means a failed download leaves nothing
    // half-made behind.
    let redist = layout.downloads().join("directx_Jun2010_redist.exe");
    download::fetch(
        http,
        DIRECTX,
        &redist,
        Span {
            from_permille: 0,
            to_permille: 700,
        },
        progress,
    )
    .await?;

    // A prefix without its marker is a half-made one: start it over rather
    // than build on whatever a previous attempt left.
    remove_prefix(layout).await?;
    let report = |permille| StepUpdate::Progress {
        permille: Some(permille),
        done_mb: 0,
        total_mb: 0,
    };
    let _ = progress.send(report(720)).await;
    run_wine(layout, &["wineboot", "--init"], "create the prefix").await?;
    wait_for_wineserver(layout).await;
    let system = layout.prefix().join("drive_c/windows/syswow64");
    if !system.is_dir() {
        return Err("Wine made a prefix without 32-bit support, which the game needs".into());
    }

    let _ = progress.send(report(800)).await;
    let installed = install_directx(layout, &redist, &system).await?;

    let _ = progress.send(report(900)).await;
    let reg = layout.downloads().join("faf-overrides.reg");
    tokio::fs::write(&reg, overrides_reg(&installed))
        .await
        .map_err(|error| format!("could not write the library settings: {error}"))?;
    let reg_windows = format!("Z:{}", reg.display().to_string().replace('/', "\\"));
    run_wine(
        layout,
        &["regedit", "/S", &reg_windows],
        "apply the library settings",
    )
    .await?;
    for library in installed.iter().filter(|name| needs_registration(name)) {
        run_wine(
            layout,
            &[r"C:\windows\syswow64\regsvr32.exe", "/s", library],
            &format!("register {library}"),
        )
        .await?;
    }
    wait_for_wineserver(layout).await;

    mark(&layout.prefix(), PREFIX_VERSION)?;
    let _ = tokio::fs::remove_file(&reg).await;
    apply_tuning(layout).await?;
    let _ = tokio::fs::remove_file(&redist).await;
    Ok(())
}

/// Copy the wanted libraries out of the redistributable into `system`.
/// Returns their file names.
async fn install_directx(
    layout: &Layout,
    redist: &Path,
    system: &Path,
) -> Result<Vec<String>, String> {
    let listing = tar(&["-tf".as_ref(), redist.as_os_str()], "the DirectX runtime").await?;
    let cabinets: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|name| wanted_cabinet(name))
        .collect();
    if cabinets.is_empty() {
        return Err("the DirectX runtime contains none of the libraries the game needs".into());
    }

    let staging = layout.root().join("directx.unpacking");
    let _ = tokio::fs::remove_dir_all(&staging).await;
    let cabs = staging.join("cabs");
    let libs = staging.join("libs");
    for dir in [&cabs, &libs] {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|error| format!("could not create {}: {error}", dir.display()))?;
    }
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "-xf".as_ref(),
        redist.as_os_str(),
        "-C".as_ref(),
        cabs.as_os_str(),
    ];
    args.extend(cabinets.iter().map(|name| std::ffi::OsStr::new(*name)));
    tar(&args, "the DirectX runtime").await?;

    for cabinet in &cabinets {
        let path = cabs.join(cabinet);
        tar(
            &[
                "-xf".as_ref(),
                path.as_os_str(),
                "-C".as_ref(),
                libs.as_os_str(),
            ],
            cabinet,
        )
        .await?;
    }

    let mut installed = Vec::new();
    for entry in std::fs::read_dir(&libs).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !wanted_library(&name) {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let target: PathBuf = system.join(&lower);
        tokio::fs::copy(entry.path(), &target)
            .await
            .map_err(|error| format!("could not install {lower}: {error}"))?;
        installed.push(lower);
    }
    let _ = tokio::fs::remove_dir_all(&staging).await;
    if !installed.iter().any(|name| name == "d3dx9_43.dll") {
        return Err("the DirectX runtime did not unpack completely".into());
    }
    installed.sort();
    Ok(installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_32_bit_cabinets_the_game_uses_are_opened() {
        assert!(wanted_cabinet("Jun2010_d3dx9_43_x86.cab"));
        assert!(wanted_cabinet("APR2007_XACT_x86.cab"));
        assert!(wanted_cabinet("Feb2010_X3DAudio_x86.cab"));
        assert!(wanted_cabinet("Jun2010_XAudio_x86.cab"));
        assert!(!wanted_cabinet("Jun2010_d3dx9_43_x64.cab"));
        assert!(!wanted_cabinet("Apr2006_MDX1_x86.cab"));
        assert!(!wanted_cabinet("Jun2010_D3DCompiler_43_x86.cab"));
    }

    #[test]
    fn only_runtime_libraries_are_installed_and_only_com_servers_registered() {
        assert!(wanted_library("d3dx9_43.dll"));
        assert!(wanted_library("xactengine3_7.dll"));
        assert!(wanted_library("XAudio2_7.dll"));
        assert!(wanted_library("X3DAudio1_7.dll"));
        assert!(!wanted_library("d3dx9_43_x86.inf"));
        assert!(needs_registration("xactengine3_7.dll"));
        assert!(needs_registration("xaudio2_7.dll"));
        assert!(!needs_registration("d3dx9_43.dll"));
        assert!(!needs_registration("x3daudio1_7.dll"));
    }

    #[test]
    fn the_overrides_name_each_library_once_without_its_extension() {
        let reg = overrides_reg(&["d3dx9_43.dll".into(), "D3DX9_43.dll".into()]);
        assert!(reg.starts_with("REGEDIT4\n"));
        assert_eq!(reg.matches("\"d3dx9_43\"=\"native,builtin\"").count(), 1);
    }
}

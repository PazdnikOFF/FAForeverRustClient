//! Finishing an install once its files are in place: the engine version
//! stamped into the executable, the retail game's libraries brought across,
//! and the `fa_path.lua` the game's bootstrap reads.
//!
//! Separate from `update` because every way an install comes about (a fresh
//! update, a cached build, a replay's build) ends the same way, and that end
//! is here, once.

use std::io::{Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use super::content_store;

/// Byte offsets inside `ForgedAlliance.exe` where the 4-byte little-endian
/// engine version number is stored. Mirrors `FAPatcher.version_addresses` in
/// the Python client's `fa/game_updater/patcher.py`: these must match
/// exactly or FA reports/behaves as the wrong version.
const VERSION_ADDRESSES: [u64; 3] = [0xd3d40, 0x47612d, 0x476666];

/// The retail Supreme Commander: Forged Alliance install root: where the
/// base-game `movies`, `sounds`, `fonts`, and `gamedata/*.scd` live. This is
/// **not** the FAF patch dir (`target_dir`, e.g. `.../replaydata`), which
/// only holds FAF's `.nx2` gamedata overrides and the patched executable.
///
/// Mirrors the Python client's `ForgedAlliance/app/path` setting, which
/// `writeFAPathLua` writes verbatim as `fa_path`. Getting this wrong is
/// invisible-but-crippling: the FAF init script mounts `fa_path/movies`,
/// `fa_path/sounds`, `fa_path/fonts`: point `fa_path` at the FAF patch dir
/// (which has none of those) and the game still *runs* (base unit/effect
/// blueprints come from the `.nx2` files, mounted relative to the exe), but
/// with no loading-screen movie, no audio, and broken menu fonts.
/// Confirmed live as the cause of exactly that symptom.
///
/// Resolution order: explicit `FAF_GAME_INSTALL_DIR` override → the path the
/// Java or Python client already has configured
/// ([`crate::infra::game::reference_retail_install_paths`]) → auto-detect
/// among the usual retail/Steam locations → `target_dir` as a last resort
/// (preserves the old behaviour rather than writing a knowingly bogus path
/// when nothing is found). Every candidate but the explicit override is
/// validated by `gamedata/lua.scd`, the same probe file Python's
/// `validate_game_path` uses.
///
/// The reference-client configs come before the guessed locations because
/// guessing only ever covers installs under `%ProgramFiles%`: a retail install
/// at, say, `C:\Games\THQ\Gas Powered Games\Supreme Commander - Forged
/// Alliance` fell through to the fallback and produced exactly the broken
/// game described above, silently. Hence the log lines: this decision is
/// otherwise invisible until someone reads a game log.
fn retail_install_dir(target_dir: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("FAF_GAME_INSTALL_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let candidates = crate::infra::game::reference_retail_install_paths()
        .into_iter()
        .chain(typical_retail_install_paths());
    resolve_retail_install_dir(candidates, target_dir)
}

fn resolve_retail_install_dir(
    candidates: impl IntoIterator<Item = PathBuf>,
    target_dir: &Path,
) -> PathBuf {
    if let Some(dir) = candidates.into_iter().find(|p| is_retail_install(p)) {
        tracing::info!(path = %dir.display(), "resolved retail FA install for fa_path");
        return dir;
    }
    tracing::warn!(
        fallback = %target_dir.display(),
        "no retail FA install found: fa_path falls back to the FAF patch dir, so the game \
         will start without base textures, sounds, movies or unit animations. Set \
         FAF_GAME_INSTALL_DIR to the install root holding gamedata/lua.scd."
    );
    target_dir.to_path_buf()
}

/// The probe the reference clients use to tell a real FA root from any other
/// directory: `gamedata/lua.scd` ships only with the base game, never with
/// FAF's `.nx2` overlay.
fn is_retail_install(dir: &Path) -> bool {
    dir.join("gamedata").join("lua.scd").is_file()
}

/// Candidate retail install locations, mirroring the Python client's
/// `typicalForgedAlliancePaths` (THQ/GPG retail, bare retail, and the Steam
/// library: both `%ProgramFiles%` and `%ProgramFiles(x86)%`, since the
/// 32-bit game usually sits under the x86 tree).
fn typical_retail_install_paths() -> Vec<PathBuf> {
    let mut out = Vec::new();
    // On a Mac, the copy the setup assistant downloaded or adopted.
    if let Some(dir) = crate::infra::mac_setup::managed_game_dir() {
        out.push(dir);
    }
    let program_files_vars = ["ProgramFiles(x86)", "ProgramFiles"];
    let suffixes = [
        r"THQ\Gas Powered Games\Supreme Commander - Forged Alliance",
        r"Supreme Commander - Forged Alliance",
        r"Steam\steamapps\common\Supreme Commander Forged Alliance",
    ];
    for var in program_files_vars {
        if let Ok(base) = std::env::var(var) {
            if base.is_empty() {
                continue;
            }
            for suffix in suffixes {
                out.push(PathBuf::from(&base).join(suffix));
            }
        }
    }
    out
}

/// Stamp `version` (little-endian, 4 bytes) into the three fixed offsets in
/// the FA executable.
///
/// Every failure here propagates: the callers use `?`. That is deliberate.
/// A half-patched or unpatched executable reports the wrong engine version,
/// which desyncs against everyone else in the lobby, so failing the update is
/// better than launching a subtly wrong game.
///
/// **The size check is load-bearing.** `Seek` past the end of a file is legal,
/// and the following write extends it, filling the gap with zero bytes. Without
/// the check, pointing this at any file smaller than the real executable (a
/// stub, a truncated download, or the `bin/<exe>` fallback below when the file
/// list shipped no executable at all) would not fail: it would silently produce
/// a corrupt multi-megabyte `ForgedAlliance.exe` in the user's install, which
/// only shows up when the game refuses to start.
///
/// The patch goes into a private copy that then replaces the executable (see
/// [`content_store::rewrite`]), not into the file itself: an executable that
/// is a hard link to a cache entry would otherwise change that entry under
/// its checksum, and a patch that fails halfway leaves the original intact.
pub(super) fn patch_exe_version(exe_path: &Path, version: i32) -> Result<(), String> {
    let required = VERSION_ADDRESSES
        .iter()
        .copied()
        .max()
        .unwrap_or_default()
        .saturating_add(std::mem::size_of::<i32>() as u64);

    let length = std::fs::metadata(exe_path)
        .map_err(|e| format!("could not open {} for patching: {e}", exe_path.display()))?
        .len();
    if length < required {
        return Err(format!(
            "{} is {length} bytes, too small to be Forged Alliance (the version \
             offsets need at least {required}): refusing to patch it",
            exe_path.display()
        ));
    }

    let bytes = version.to_le_bytes();
    content_store::rewrite(exe_path, |file| {
        for &addr in &VERSION_ADDRESSES {
            file.seek(SeekFrom::Start(addr))
                .map_err(|e| format!("could not seek to {addr:#x}: {e}"))?;
            file.write_all(&bytes)
                .map_err(|e| format!("could not patch version at {addr:#x}: {e}"))?;
        }
        Ok(())
    })
}

/// The last step of every install pass: the retail game's own libraries into
/// the FAF `bin`, then `fa_path.lua`. One place, so the three ways an install
/// finishes (a fresh update, a cached version, a replay's version) cannot
/// disagree about either.
pub(super) fn finish_install(
    target_dir: &Path,
    featured_mod: &str,
    version: i32,
) -> Result<(), String> {
    let retail = retail_install_dir(target_dir);
    copy_retail_binaries(&retail, target_dir)?;
    write_fa_path_lua(target_dir, &retail, featured_mod, version)
}

/// The files a FAF install's `bin` needs from the game the player owns.
///
/// FAF's file list ships the patched executable and FAF's own Lua, never the
/// engine's runtime libraries: those are the retail game's, not FAF's to hand
/// out. Something has to bring them across, and on a machine where this
/// client was the first FAF client, nothing did. The game then stopped at
/// "BugSplat.dll is missing" the moment a lobby started, and installing the
/// Java client fixed it, because the Java client copies exactly these files.
/// The list and the rule (only a file the FAF install lacks) are its
/// `GameBinariesUpdateTaskImpl`.
const RETAIL_BINARIES: [&str; 13] = [
    "BsSndRpt.exe",
    "BugSplat.dll",
    "BugSplatRc.dll",
    "DbgHelp.dll",
    "GDFBinary.dll",
    "Microsoft.VC80.CRT.manifest",
    "SHSMP.DLL",
    "msvcm80.dll",
    "msvcp80.dll",
    "msvcr80.dll",
    "sx32w.dll",
    "wxmsw24u-vs80.dll",
    "zlibwapi.dll",
];

/// Copy the [`RETAIL_BINARIES`] the FAF install is missing from the retail
/// install's `bin`.
///
/// A file already in the FAF install is left alone, whatever its content: it
/// may be the one FAF shipped. Names are compared without case, both ways,
/// since a retail install copied onto a case-sensitive disk keeps whatever
/// spelling it had. Nothing to copy from is not an error here: with no retail
/// install found, `retail_dir` is the FAF install itself, and that is already
/// reported where it is resolved.
fn copy_retail_binaries(retail_dir: &Path, target_dir: &Path) -> Result<(), String> {
    if retail_dir == target_dir {
        return Ok(());
    }
    let Ok(sources) = std::fs::read_dir(retail_dir.join("bin")) else {
        return Ok(());
    };
    let target_bin = target_dir.join("bin");
    std::fs::create_dir_all(&target_bin)
        .map_err(|error| format!("could not create {}: {error}", target_bin.display()))?;
    let present: Vec<String> = std::fs::read_dir(&target_bin)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().to_str().map(str::to_ascii_lowercase))
                .collect()
        })
        .unwrap_or_default();
    for source in sources.flatten() {
        let file_name = source.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let wanted = RETAIL_BINARIES
            .iter()
            .any(|binary| binary.eq_ignore_ascii_case(name));
        if !wanted || present.contains(&name.to_ascii_lowercase()) {
            continue;
        }
        let destination = target_bin.join(name);
        std::fs::copy(source.path(), &destination)
            .map_err(|error| format!("could not copy {name} from the game install: {error}"))?;
        tracing::info!(file = name, "copied a game library into the FAF install");
        // A retail install copied off a disc keeps the read-only flag, and a
        // read-only file in the FAF install is one the next update cannot
        // replace. The Java client clears it for the same reason. Windows
        // only, where read-only is one attribute and not the Unix mode bits
        // the lint below is about: there is nothing to make world-writable.
        #[cfg(windows)]
        #[allow(clippy::permissions_set_readonly_false)]
        if let Ok(metadata) = std::fs::metadata(&destination) {
            let mut permissions = metadata.permissions();
            if permissions.readonly() {
                permissions.set_readonly(false);
                let _ = std::fs::set_permissions(&destination, permissions);
            }
        }
    }
    Ok(())
}

/// Mirrors `fa/path.py:writeFAPathLua`. Written into `target_dir` (the FAF
/// patch dir, e.g. `.../replaydata`); the FAF init script beside the exe
/// reads it to locate everything else.
///
/// `fa_path` is the **retail install root** ([`retail_install_dir`]), not
/// `target_dir`: the init script mounts `fa_path/{movies,sounds,fonts}` and
/// `fa_path/gamedata/*.scd` from it, none of which exist under the FAF patch
/// dir. This matches Python writing its `ForgedAlliance/app/path` setting
/// verbatim. (FAF's own `.nx2` gamedata overrides are mounted separately by
/// the init script, relative to the exe: `InitFileDir/../gamedata`: so
/// they keep coming from `target_dir` regardless of `fa_path`.)
///
/// `custom_vault_path` is the user's actual vault root
/// ([`documents_vault_dir`], mirroring Python's `util.VAULTS_BASE_DIR`),
/// the same root [`default_map_search_dirs`]'s first entry stages maps into,
/// so the two never diverge.
fn write_fa_path_lua(
    target_dir: &Path,
    retail_dir: &Path,
    featured_mod: &str,
    version: i32,
) -> Result<(), String> {
    let vault_path = documents_vault_dir().unwrap_or_else(|| target_dir.join("vault"));
    let content = format!(
        "fa_path = \"{}\"\ncustom_vault_path = \"{}\"\nGameType = \"{featured_mod}\"\nGameVersion = \"{version}\"\nClientVersion = \"{}\"\nForceAffinity = false\n",
        slashed(retail_dir),
        slashed(&vault_path),
        env!("CARGO_PKG_VERSION"),
    );
    std::fs::write(target_dir.join("fa_path.lua"), content)
        .map_err(|e| format!("could not write fa_path.lua: {e}"))
}

/// The user's vault root, including a Java-client configured custom vault when
/// available. Shared by
/// [`write_fa_path_lua`] (as `custom_vault_path`) and
/// [`default_map_search_dirs`] (as its first, and primary, map search dir),
/// they must never diverge, since that's exactly the bug this fixes.
pub(super) fn documents_vault_dir() -> Option<PathBuf> {
    Some(crate::infra::faf_content::vault_dir())
}

fn slashed(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// Read the 4-byte engine version stamped into `ForgedAlliance.exe` at `0xd3d40`.
pub fn read_exe_version(exe_path: &Path) -> Option<i32> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(exe_path).ok()?;
    let length = file.metadata().ok()?.len();
    let min_len = VERSION_ADDRESSES[0] + 4;
    if length < min_len {
        return None;
    }
    file.seek(SeekFrom::Start(VERSION_ADDRESSES[0])).ok()?;
    let mut buf = [0u8; 4];
    file.read_exact(&mut buf).ok()?;
    let ver = i32::from_le_bytes(buf);
    if (3000..=10000).contains(&ver) {
        Some(ver)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;

    use crate::infra::game_updater::content_store::link_into;
    use crate::infra::game_updater::test_support::put_in_store;

    #[test]
    fn patches_all_three_version_offsets_little_endian() {
        let dir = std::env::temp_dir().join(format!("forge-patch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake.exe");
        // Large enough to cover the highest offset (0x47612d) plus 4 bytes.
        std::fs::write(&path, vec![0u8; 0x476670]).unwrap();

        patch_exe_version(&path, 3828).expect("should patch");

        let mut file = std::fs::File::open(&path).unwrap();
        for &addr in &VERSION_ADDRESSES {
            let mut buf = [0u8; 4];
            file.seek(SeekFrom::Start(addr)).unwrap();
            file.read_exact(&mut buf).unwrap();
            assert_eq!(i32::from_le_bytes(buf), 3828, "offset {addr:#x}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_patch_a_file_too_small_to_be_the_executable() {
        // The regression this guards: seeking past the end of a short file and
        // writing is legal, and would leave a zero-filled 4.6 MB "executable"
        // behind rather than reporting a problem.
        let dir = std::env::temp_dir().join(format!("forge-patch-small-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stub.exe");
        std::fs::write(&path, b"not really an executable").unwrap();

        let error = patch_exe_version(&path, 3828).expect_err("a stub must not be patched");
        assert!(error.contains("too small"), "{error}");
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            24,
            "the file must be left exactly as it was"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_retail_root_outside_the_guessed_locations_still_wins_over_the_fallback() {
        // The shape of the install this fixes: the right THQ layout, but
        // under `C:\Games`, so `typical_retail_install_paths` never sees it.
        // Before the reference configs were consulted this fell straight
        // through to `target_dir` and the game launched with no base content.
        let temp = tempfile::tempdir().unwrap();
        let retail = temp
            .path()
            .join("Games/THQ/Supreme Commander - Forged Alliance");
        std::fs::create_dir_all(retail.join("gamedata")).unwrap();
        std::fs::write(retail.join("gamedata").join("lua.scd"), b"base game").unwrap();

        let patch_dir = temp.path().join("FAForever/replaydata");
        let guessed_but_absent = temp.path().join("Program Files/THQ/SCFA");

        assert_eq!(
            resolve_retail_install_dir([guessed_but_absent, retail.clone()], &patch_dir),
            retail,
            "the first candidate that actually holds gamedata/lua.scd must win"
        );
    }

    #[test]
    fn the_faf_patch_dir_is_only_a_last_resort() {
        // It has `gamedata/*.nx2` but no `.scd`, so it must never satisfy the
        // probe: reaching it means we knowingly write a degraded `fa_path`,
        // which is what the warning in `resolve_retail_install_dir` is for.
        let temp = tempfile::tempdir().unwrap();
        let patch_dir = temp.path().join("replaydata");
        std::fs::create_dir_all(patch_dir.join("gamedata")).unwrap();
        std::fs::write(patch_dir.join("gamedata").join("units.nx2"), b"overlay").unwrap();

        assert!(!is_retail_install(&patch_dir));
        assert_eq!(
            resolve_retail_install_dir([patch_dir.clone()], &patch_dir),
            patch_dir
        );
    }

    /// The reported bug: a player whose first FAF client was this one got
    /// "BugSplat.dll is missing" on joining a lobby, because nothing brought
    /// the retail game's libraries into the FAF install. Installing the Java
    /// client fixed it, since that is what the Java client does.
    #[test]
    fn the_retail_games_libraries_are_copied_into_the_faf_install() {
        let temp = tempfile::tempdir().unwrap();
        let retail = temp.path().join("retail");
        let faf = temp.path().join("faf");
        std::fs::create_dir_all(retail.join("bin")).unwrap();
        std::fs::create_dir_all(faf.join("bin")).unwrap();
        std::fs::write(retail.join("bin").join("BugSplat.dll"), b"retail bugsplat").unwrap();
        std::fs::write(retail.join("bin").join("shsmp.dll"), b"retail shsmp").unwrap();
        std::fs::write(retail.join("bin").join("msvcr80.dll"), b"retail crt").unwrap();
        std::fs::write(retail.join("bin").join("SupremeCommander.exe"), b"not ours").unwrap();
        // Already in the FAF install, possibly FAF's own: left as it is.
        std::fs::write(faf.join("bin").join("MSVCR80.dll"), b"faf crt").unwrap();

        copy_retail_binaries(&retail, &faf).unwrap();

        let read = |name: &str| std::fs::read(faf.join("bin").join(name)).ok();
        assert_eq!(
            read("BugSplat.dll").as_deref(),
            Some(&b"retail bugsplat"[..])
        );
        assert_eq!(read("shsmp.dll").as_deref(), Some(&b"retail shsmp"[..]));
        assert_eq!(read("MSVCR80.dll").as_deref(), Some(&b"faf crt"[..]));
        assert_eq!(
            read("SupremeCommander.exe"),
            None,
            "only the listed libraries"
        );
        let names = std::fs::read_dir(faf.join("bin")).unwrap().count();
        assert_eq!(names, 3, "no second spelling of a file already there");
    }

    #[test]
    fn no_retail_install_means_nothing_to_copy() {
        let temp = tempfile::tempdir().unwrap();
        let faf = temp.path().join("faf");
        std::fs::create_dir_all(faf.join("bin")).unwrap();
        std::fs::write(faf.join("bin").join("BugSplat.dll"), b"faf").unwrap();

        // The fallback when no retail install is found is the FAF install itself.
        copy_retail_binaries(&faf, &faf).unwrap();
        // And a retail path without a bin folder is not an error either.
        copy_retail_binaries(&temp.path().join("missing"), &faf).unwrap();

        assert_eq!(std::fs::read_dir(faf.join("bin")).unwrap().count(), 1);
    }

    #[test]
    fn writes_fa_path_lua_with_expected_fields() {
        let dir = std::env::temp_dir().join(format!("forge-lua-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let retail = dir.join("retail-install");
        write_fa_path_lua(&dir, &retail, "ladder1v1", 3684).expect("should write");
        let content = std::fs::read_to_string(dir.join("fa_path.lua")).unwrap();

        assert!(content.contains("GameType = \"ladder1v1\""));
        assert!(content.contains("GameVersion = \"3684\""));
        assert!(content.contains("ForceAffinity = false"));
        assert!(!content.contains('\\'), "paths must use forward slashes");
        // fa_path must be the retail install root, never the FAF patch dir,
        // the game mounts movies/sounds/fonts from it (the bug this guards).
        assert!(
            content.contains(&format!("fa_path = \"{}\"", slashed(&retail))),
            "fa_path should be the retail install dir: {content}",
        );
        assert!(
            !content.contains(&format!("fa_path = \"{}/bin\"", slashed(&dir))),
            "fa_path must not point at the FAF patch dir's bin: {content}",
        );
        // Only meaningful on a host that can resolve a Documents dir (not
        // every CI runner can): where it does, `write_fa_path_lua` must use
        // it rather than falling back to the never-populated
        // `target_dir/vault` (the bug this test guards).
        if let Some(vault_dir) = documents_vault_dir() {
            assert!(
                content.contains(&format!("custom_vault_path = \"{}\"", slashed(&vault_dir))),
                "custom_vault_path should be the user's real vault root ({}): {content}",
                slashed(&vault_dir),
            );
            assert!(
                !content.contains(&format!(
                    "custom_vault_path = \"{}",
                    slashed(&dir.join("vault"))
                )),
                "custom_vault_path must not point at an unpopulated target_dir/vault: {content}",
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn patching_a_staged_executable_leaves_the_cached_entry_alone() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let exe = vec![0u8; 0x476670];
        let md5 = put_in_store(&cache, "bin", &exe);
        let entry = cache.join("bin").join(&md5);
        // Linked rather than copied, as a staged non-executable file is: the
        // patch must still not reach through to the store.
        let staged = temp.path().join("replaydata").join("bin").join("game.exe");
        link_into(&entry, &md5, &staged).unwrap();

        patch_exe_version(&staged, 3838).unwrap();

        assert_eq!(read_exe_version(&staged), Some(3838));
        assert_eq!(std::fs::read(&entry).unwrap(), exe);
    }
}

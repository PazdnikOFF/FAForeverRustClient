//! Graphics settings for Forged Alliance on Apple silicon.
//!
//! What is changed is what measured: EP93's forged-alliance-mac lab, on an M4
//! Max under the same Wine-and-Rosetta path, found turning shadows off the one
//! setting worth having (+29% frame rate at a fixed view), with fidelity and
//! the cursor mode within noise. Vertical sync, antialiasing and bloom go too, and
//! the cursor is kept inside the game in fullscreen:
//! each is work the render thread does while the simulation, which is what
//! actually limits a Mac, waits for it. Resolution is left alone; it is the
//! player's choice and the display's.
//!
//! `game.prefs` is a Lua table the game writes. Only lines of the form
//! `key = <number>,` are touched, which is how the game writes the current
//! value of each option; the option *definitions* further down the file are
//! tables (`key = {`) and never match. The first change keeps a backup.

use std::path::Path;

/// The settings, in the release that introduced them. Each entry is applied
/// once by itself (see `mac_setup::optimize_graphics_once`), so adding one
/// later reaches existing installs without resetting what an earlier one set
/// and the player has since changed back.
const RELEASES: [&[(&str, &str)]; 3] = [
    &[
        ("shadow_quality", "0"),
        ("vsync", "0"),
        ("antialiasing", "0"),
        ("bloom_render", "0"),
    ],
    // On a second display the cursor otherwise leaves the game in fullscreen,
    // and the edge of the screen stops scrolling the map. FA confines it
    // itself with this on, through `ClipCursor`, which the Wine Mac driver
    // implements with the window's confinement rectangle.
    &[("lock_fullscreen_cursor_to_window", "1")],
    // Windowed rather than Direct3D's exclusive full screen, which minimises
    // the game whenever another application takes the focus: on a second
    // display that is every click outside the game. `fa-window.exe` then
    // takes the frame away and fills the monitor, and keeps the cursor in.
    &[("primary_adapter", "'windowed'")],
];

/// How many releases there are: the version an up-to-date install records.
pub(super) const VERSION: u32 = RELEASES.len() as u32;

/// Apply every release after `since`. `optimize` is the same from zero.
pub(super) fn optimize_since(prefs: &Path, since: u32) -> Result<u32, String> {
    let settings: Vec<(&str, &str)> = RELEASES
        .iter()
        .skip(since as usize)
        .flat_map(|release| release.iter().copied())
        .collect();
    optimize_with(prefs, &settings)
}

pub(super) fn optimize(prefs: &Path) -> Result<u32, String> {
    optimize_since(prefs, 0)
}

fn optimize_with(prefs: &Path, settings: &[(&str, &str)]) -> Result<u32, String> {
    let text = std::fs::read_to_string(prefs).map_err(|_| {
        "Forged Alliance has not saved its settings yet: start a game once, close it, then try again"
            .to_string()
    })?;
    let (updated, changed) = apply(&text, settings);
    if changed == 0 {
        return Ok(0);
    }
    let backup = prefs.with_extension("prefs.faf-backup");
    if !backup.exists() {
        std::fs::copy(prefs, &backup)
            .map_err(|error| format!("could not back up the game settings: {error}"))?;
    }
    std::fs::write(prefs, updated)
        .map_err(|error| format!("could not save the game settings: {error}"))?;
    Ok(changed)
}

/// The file with the recommended values, and how many lines changed.
fn apply(text: &str, settings: &[(&str, &str)]) -> (String, u32) {
    let mut changed = 0;
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        out.push_str(&rewrite(line, settings, &mut changed));
    }
    (out, changed)
}

fn rewrite(line: &str, settings: &[(&str, &str)], changed: &mut u32) -> String {
    let indent_len = line.len() - line.trim_start().len();
    let (indent, rest) = line.split_at(indent_len);
    for &(key, value) in settings {
        let Some(after) = rest.strip_prefix(key).and_then(|r| r.strip_prefix(" = ")) else {
            continue;
        };
        // The current value: a number, or a quoted string. Anything else
        // (`{` opening an option's definition) is not a value to replace.
        let current_len = match after.strip_prefix('\'') {
            Some(quoted) => match quoted.find('\'') {
                Some(end) => end + 2,
                None => continue,
            },
            None => after.chars().take_while(char::is_ascii_digit).count(),
        };
        if current_len == 0 {
            continue;
        }
        let (current, tail) = after.split_at(current_len);
        if current == value {
            return line.to_string();
        }
        *changed += 1;
        return format!("{indent}{key} = {value}{tail}");
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_values_change_and_option_definitions_do_not() {
        let text = "profiles = {\n        options = {\n                shadow_quality = 1,\n                vsync = 1,\n                bloom_intensity = 15,\n                antialiasing = 0,\n        },\n}\n    shadow_quality = {\n        default = 1,\n";
        let all: Vec<(&str, &str)> = RELEASES.iter().flat_map(|r| r.iter().copied()).collect();
        let (out, changed) = apply(text, &all);
        assert_eq!(changed, 2);
        assert!(out.contains("                shadow_quality = 0,\n"));
        assert!(out.contains("                vsync = 0,\n"));
        assert!(
            out.contains("bloom_intensity = 15,"),
            "a longer key is not the key"
        );
        assert!(
            out.contains("    shadow_quality = {\n"),
            "definitions are untouched"
        );
        assert!(out.ends_with("default = 1,\n"));
    }

    #[test]
    fn a_later_release_leaves_an_earlier_ones_settings_to_the_player() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("Game.prefs");
        // Shadows back on by the player after the first release applied.
        std::fs::write(
            &prefs,
            "    shadow_quality = 2,\n    lock_fullscreen_cursor_to_window = 0,\n",
        )
        .unwrap();
        assert_eq!(optimize_since(&prefs, 1).unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(&prefs).unwrap(),
            "    shadow_quality = 2,\n    lock_fullscreen_cursor_to_window = 1,\n"
        );
    }

    #[test]
    fn a_quoted_value_is_replaced_whole() {
        let mut changed = 0;
        let line = rewrite(
            "                primary_adapter = '2560,1440,165',\n",
            &[("primary_adapter", "'windowed'")],
            &mut changed,
        );
        assert_eq!(line, "                primary_adapter = 'windowed',\n");
        assert_eq!(changed, 1);
    }

    #[test]
    fn a_missing_prefs_file_says_what_to_do() {
        let temp = tempfile::tempdir().unwrap();
        let reason = optimize(&temp.path().join("Game.prefs")).unwrap_err();
        assert!(reason.contains("start a game once"));
    }

    #[test]
    fn the_first_change_keeps_a_backup() {
        let temp = tempfile::tempdir().unwrap();
        let prefs = temp.path().join("Game.prefs");
        std::fs::write(&prefs, "    vsync = 1,\n").unwrap();
        assert_eq!(optimize(&prefs).unwrap(), 1);
        assert_eq!(std::fs::read_to_string(&prefs).unwrap(), "    vsync = 0,\n");
        let backup = temp.path().join("Game.prefs.faf-backup");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "    vsync = 1,\n");
        assert_eq!(optimize(&prefs).unwrap(), 0, "already optimized");
    }
}

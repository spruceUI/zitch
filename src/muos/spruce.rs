//! spruceOS: the same handheld paths as muOS, with spruce's systems and
//! launcher. A game cannot run under zitch here, since on KMSDRM devices
//! only one process owns the screen, so zitch writes the command to
//! [`HANDOFF`] and quits; the app's launch.sh runs it and starts zitch
//! again. ROMs are also copied flat into spruce's Roms folders, so the
//! menu lists them.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::butlerd::types::Cave;

use super::*;

const SDCARD: &str = "/mnt/SDCARD";
const HELPERS: &str = "/mnt/SDCARD/spruce/scripts/helperFunctions.sh";
/// What the app's launch.sh runs after zitch exits, then starts zitch again.
pub(super) const HANDOFF: &str = "/tmp/zitch_next.sh";

/// dash ids and the spruce Emu folder that plays them.
const SYSTEM_DIRS: [(&str, &str); 24] = [
    ("nes", "FC"),
    ("snes", "SFC"),
    ("gb", "GB"),
    ("gbc", "GBC"),
    ("gba", "GBA"),
    ("nds", "NDS"),
    ("md", "MD"),
    ("32x", "THIRTYTWOX"),
    ("sms", "MS"),
    ("gg", "GG"),
    ("pce", "PCE"),
    ("lynx", "LYNX"),
    ("ngp", "NGPC"),
    ("a26", "ATARI"),
    ("c64", "COMMODORE"),
    ("amiga", "AMIGA"),
    ("n64", "N64"),
    ("psx", "PS"),
    ("psp", "PSP"),
    ("saturn", "SATURN"),
    ("segacd", "SEGACD"),
    ("dreamcast", "DC"),
    // fake-08 needs no PICO-8 licence, unlike spruce's PICO8 system.
    (PICO8, "FAKE08"),
    (TIC80, "TIC"),
];

static EXIT: AtomicBool = AtomicBool::new(false);

/// Whether this is spruce: its helper scripts are on the card.
pub fn is_spruce() -> bool {
    static SPRUCE: OnceLock<bool> = OnceLock::new();
    *SPRUCE.get_or_init(|| Path::new(HELPERS).exists())
}

/// Set once a game has been handed to launch.sh; the interface closes.
pub fn exit_requested() -> bool {
    EXIT.load(Ordering::SeqCst)
}

#[derive(Deserialize)]
struct EmuConfig {
    #[serde(default)]
    label: String,
    #[serde(default)]
    default_emulator: String,
}

/// The systems whose Emu folder is on the card.
pub(super) fn systems() -> HashMap<String, System> {
    SYSTEM_DIRS
        .iter()
        .filter_map(|(id, dir)| {
            let json = std::fs::read_to_string(emu_dir(dir).join("config.json")).ok()?;
            let config: EmuConfig = match serde_json::from_str(&json) {
                Ok(config) => config,
                Err(error) => {
                    log::warn!("Emu/{dir}/config.json: {error}");
                    return None;
                }
            };
            let label = if config.label.is_empty() {
                dir.to_string()
            } else {
                config.label
            };
            Some((
                id.to_string(),
                System {
                    id: id.to_string(),
                    assign: dir.to_string(),
                    launcher: "standard_launch.sh".to_string(),
                    core: config.default_emulator,
                    label,
                },
            ))
        })
        .collect()
}

fn emu_dir(dir: &str) -> PathBuf {
    Path::new(SDCARD).join("Emu").join(dir)
}

fn roms_dir(dir: &str) -> PathBuf {
    Path::new(SDCARD).join("Roms").join(dir)
}

/// Writes the command for launch.sh and asks the interface to close.
pub(super) fn launch(
    content: &Content,
    args: &[String],
    env: &HashMap<String, String>,
) -> Result<()> {
    let script = match content {
        Content::Rom { path, system } => {
            let rom = copy_of(path).unwrap_or_else(|| path.clone());
            // standard_launch.sh takes its system from the Emu folder in
            // its own path.
            let launcher =
                emu_dir(&system.assign).join("../../spruce/scripts/emu/standard_launch.sh");
            format!("{} {}\n", quote_path(&launcher), quote_path(&rom))
        }
        Content::Love { path } => {
            let love = love().context("no LÖVE on this device")?;
            let dir = love.binary.parent().unwrap_or(Path::new("/"));
            let mut lib_path = vec![love.libs.clone()];
            let mut vars: Vec<(String, String)> = Vec::new();
            for (name, value) in game_env() {
                let value = value.to_string_lossy().into_owned();
                if name == "LD_LIBRARY_PATH" {
                    lib_path.extend(std::env::split_paths(&value));
                } else {
                    vars.push((name, value));
                }
            }
            // launch.sh runs this with spruce's environment, not ours.
            for name in [
                "SDL_VIDEODRIVER",
                "SDL_AUDIODRIVER",
                "SDL_JOYSTICK_DISABLE_UDEV",
                "SDL_GAMECONTROLLERCONFIG",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    vars.push((name.into(), value.to_string_lossy().into_owned()));
                }
            }
            vars.push(("LOVE_GRAPHICS_USE_OPENGLES".into(), "1".into()));
            vars.extend(env.iter().map(|(k, v)| (k.clone(), v.clone())));
            let lib_path = std::env::join_paths(&lib_path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            vars.push(("LD_LIBRARY_PATH".into(), lib_path));
            let mut words = vec!["cd".to_string(), quote_path(dir), "&&".into(), "env".into()];
            words.extend(vars.iter().map(|(name, value)| quote(&format!("{name}={value}"))));
            words.push(quote_path(&love.binary));
            words.push(quote_path(path));
            words.extend(args.iter().map(|arg| quote(arg)));
            words.join(" ") + "\n"
        }
    };
    std::fs::write(HANDOFF, script).with_context(|| format!("writing {HANDOFF}"))?;
    log::info!("handed {} to launch.sh", content.path().display());
    EXIT.store(true, Ordering::SeqCst);
    Ok(())
}

fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

fn quote_path(path: &Path) -> String {
    quote(&path.to_string_lossy())
}

/// A ROM butler installed and the copy of it in the Roms folders.
#[derive(Serialize, Deserialize)]
struct Copied {
    cave: String,
    source: PathBuf,
    dest: PathBuf,
}

fn ledger_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".config/zitch/spruce-roms.json"))
}

fn read_ledger() -> Vec<Copied> {
    ledger_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn write_ledger(ledger: &[Copied]) {
    let Some(path) = ledger_path() else { return };
    let json = match serde_json::to_string_pretty(ledger) {
        Ok(json) => json,
        Err(error) => {
            log::warn!("spruce-roms.json: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::write(&path, json) {
        log::warn!("writing {}: {error}", path.display());
    }
}

/// The Roms copy of a ROM butler installed, if there is one.
fn copy_of(source: &Path) -> Option<PathBuf> {
    read_ledger()
        .into_iter()
        .find(|c| c.source == source && c.dest.is_file())
        .map(|c| c.dest)
}

/// Copies each installed ROM into its system's Roms folder, named for the
/// game, and removes the copies of games no longer installed. A copy the
/// user deleted is not made again.
pub fn sync_roms(caves: &[Cave]) {
    if !is_spruce() {
        return;
    }
    let mut ledger = read_ledger();
    let installed: HashSet<&str> = caves.iter().map(|c| c.id.as_str()).collect();
    ledger.retain(|copied| {
        if installed.contains(copied.cave.as_str()) {
            return true;
        }
        match std::fs::remove_file(&copied.dest) {
            Ok(()) => log::info!("removed {}", copied.dest.display()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("removing {}: {error}", copied.dest.display()),
        }
        false
    });
    for cave in caves {
        let (Some(info), Some(game)) = (cave.install_info.as_ref(), cave.game.as_ref()) else {
            continue;
        };
        let roms = roms_in(Path::new(&info.install_folder));
        for rom in &roms {
            if ledger.iter().any(|c| c.cave == cave.id && c.source == *rom) {
                continue;
            }
            let Some(system) = System::for_file(rom) else { continue };
            let Some(dest) = destination(&game.title, rom, roms.len() > 1, &system.assign) else {
                log::warn!("no free name in Roms/{} for {}", system.assign, rom.display());
                continue;
            };
            if let Some(parent) = dest.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                log::warn!("creating {}: {error}", parent.display());
                continue;
            }
            match std::fs::copy(rom, &dest) {
                Ok(_) => {
                    log::info!("copied {} to {}", rom.display(), dest.display());
                    ledger.push(Copied {
                        cave: cave.id.clone(),
                        source: rom.clone(),
                        dest,
                    });
                }
                Err(error) => {
                    log::warn!("copying {} to {}: {error}", rom.display(), dest.display())
                }
            }
        }
    }
    write_ledger(&ledger);
}

/// The ROM files in an install folder, by name. Disc images need their
/// companions and stay where butler put them.
fn roms_in(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut dirs = vec![(folder.to_path_buf(), 0)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 {
                    dirs.push((path, depth + 1));
                }
            } else if System::for_file(&path).is_some() {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// `Roms/<SYS>/<Title>.<ext>`, with the file's own name added when the game
/// has several ROMs, and "(itch.io)" when the name is taken.
fn destination(title: &str, rom: &Path, several: bool, dir: &str) -> Option<PathBuf> {
    let name = rom.file_name()?.to_str()?;
    // .p8.png keeps both parts.
    let ext = if name.to_ascii_lowercase().ends_with(".p8.png") {
        "p8.png".to_string()
    } else {
        rom.extension()?.to_str()?.to_string()
    };
    let stem = name[..name.len() - ext.len() - 1].to_string();
    let mut base = safe_name(title);
    if several {
        base = format!("{base} - {}", safe_name(&stem));
    }
    let folder = roms_dir(dir);
    [format!("{base}.{ext}"), format!("{base} (itch.io).{ext}")]
        .into_iter()
        .map(|file| folder.join(file))
        .find(|path| !path.exists())
}

/// A title as a FAT file name.
fn safe_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "itch.io game".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_fat_names() {
        assert_eq!(safe_name("Bob's Game: Part 2?"), "Bob's Game- Part 2-");
        assert_eq!(safe_name("  ...  "), "itch.io game");
        assert_eq!(safe_name("Ends with a dot."), "Ends with a dot");
    }

    #[test]
    fn quoting_survives_single_quotes() {
        assert_eq!(quote_path(Path::new("/a/it's.gba")), r"'/a/it'\''s.gba'");
    }
}

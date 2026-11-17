//! Where things live on disk, plus the account list and settings.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The two folders everything hangs off. Tests point these at temp dirs.
#[derive(Debug, Clone)]
pub struct Env {
    /// `%LOCALAPPDATA%\Riot Games`
    pub riot_games_dir: PathBuf,
    /// `%LOCALAPPDATA%\QuickSwitch`: our accounts, settings, snapshots and
    /// secrets. Local, not Roaming: Roaming folders can be copied to other
    /// machines by Windows domain profiles, and this data should stay put.
    pub data_dir: PathBuf,
}

impl Env {
    pub fn from_system() -> Env {
        let var = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let home = var("USERPROFILE").or_else(|| var("HOME")).unwrap_or_else(|| PathBuf::from("."));
        let local = var("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"));
        Env { riot_games_dir: local.join("Riot Games"), data_dir: local.join("QuickSwitch") }
    }

    pub fn snapshot_file(&self, id: &str) -> PathBuf {
        self.data_dir.join("snapshots").join(format!("{id}.bin"))
    }

    pub fn secret_file(&self, id: &str) -> PathBuf {
        self.data_dir.join("secrets").join(format!("{id}.bin"))
    }
}

/// What to start after signing in.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Game {
    /// League of Legends; Teamfight Tactics runs inside the same client.
    #[default]
    Lol,
    Valorant,
    /// The account plays both. The window offers each game separately;
    /// anything that cannot ask (tray, hotkey, shortcut) opens League.
    Both,
    /// Only Riot Client.
    Client,
}

impl Game {
    /// Riot's product id, as used by `--launch-product`.
    pub fn product(self) -> &'static str {
        match self {
            Game::Lol | Game::Both => "league_of_legends",
            Game::Valorant => "valorant",
            Game::Client => "riot-client",
        }
    }

    pub fn launch_args(self) -> Vec<String> {
        vec![format!("--launch-product={}", self.product()), "--launch-patchline=live".into()]
    }

    /// The process that shows the game's own client came up.
    pub fn client_process(self) -> Option<&'static str> {
        match self {
            Game::Lol | Game::Both => Some("LeagueClientUx.exe"),
            Game::Valorant => Some("VALORANT-Win64-Shipping.exe"),
            Game::Client => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Account {
    pub id: String,
    pub name: String,
    /// Riot account id, learned from the client once it is signed in.
    pub puuid: Option<String>,
    /// `Name#TAG`, for display.
    pub riot_id: Option<String>,
    pub game: Game,
    /// Global hotkey such as `Ctrl+Alt+Digit1`.
    pub hotkey: Option<String>,
    /// The saved session was rejected last time; a sign-in is due.
    pub needs_login: bool,
    /// Last known rank ("Gold II 45 LP") and level, from the League client.
    pub rank: Option<String>,
    pub level: Option<u64>,
    /// Hash of the stored snapshot; empty when there is none.
    pub hash: String,
    /// Unix seconds of the last snapshot.
    pub updated: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct State {
    pub accounts: Vec<Account>,
    /// The account whose files we last put in place. Used when the session
    /// itself does not say whose it is.
    pub current: Option<String>,
}

/// User-editable settings (`config.json`). Every field has a default, so
/// the file only needs the keys you want to change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Full path to RiotClientServices.exe. Found automatically when empty.
    pub riot_client_exe: Option<PathBuf>,
    /// Files and folders, relative to `%LOCALAPPDATA%\Riot Games`, that
    /// together make up a signed-in session. Riot Client's `Config` folder
    /// is deliberately not one of them: it holds the user's own settings
    /// (login music and video, language) and a large download cache.
    pub session_paths: Vec<String>,
    /// Client processes; they must all be gone before session files move.
    pub client_processes: Vec<String>,
    /// If any of these is running we refuse to switch (a match is on).
    pub game_processes: Vec<String>,
    /// Processes that own the Riot Client window, for typing a login.
    pub ui_processes: Vec<String>,
    /// Look for a newer version on start-up. This is the app's only
    /// connection to the internet, and only exists in builds that were
    /// made with an update signing key.
    pub check_updates: bool,
    /// Global hotkey that opens the window. Not Ctrl+Alt+letter: on many
    /// keyboards (Turkish among them) that is AltGr, which types characters.
    pub window_hotkey: String,
}

impl Default for Config {
    fn default() -> Self {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect();
        Config {
            riot_client_exe: None,
            session_paths: s(&[
                "Riot Client/Data/RiotGamesPrivateSettings.yaml",
                "League of Legends/Data/RiotGamesPrivateSettings.yaml",
                "Riot Client/Data/Sessions",
            ]),
            client_processes: s(&[
                "RiotClientServices.exe",
                "Riot Client.exe",
                "RiotClientUx.exe",
                "RiotClientUxRender.exe",
                "LeagueClient.exe",
                "LeagueClientUx.exe",
                "LeagueClientUxRender.exe",
            ]),
            game_processes: s(&["League of Legends.exe", "VALORANT-Win64-Shipping.exe"]),
            ui_processes: s(&["Riot Client.exe", "RiotClientUx.exe"]),
            check_updates: true,
            window_hotkey: "Ctrl+Shift+Space".into(),
        }
    }
}

/// Folders older versions saved with every session. They hold device
/// settings, not login state, and are removed from a stored config.
const NOT_SESSION: [&str; 1] = ["Riot Client/Config"];

impl Config {
    /// Drops entries that older versions wrote into `config.json` but that
    /// must not be part of a session.
    pub fn migrate(mut self) -> Self {
        self.session_paths.retain(|p| {
            let p = p.replace('\\', "/");
            let p = p.trim_end_matches('/');
            !NOT_SESSION.iter().any(|n| p.eq_ignore_ascii_case(n))
        });
        self
    }
}

pub fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}

/// Writes through a temp file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &bytes)
}

fn installs_record() -> Option<serde_json::Value> {
    let program_data =
        std::env::var_os("PROGRAMDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let bytes = fs::read(program_data.join("Riot Games").join("RiotClientInstalls.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Finds RiotClientServices.exe: Riot's own install record first, then the
/// default install folder.
pub fn find_riot_client() -> Option<PathBuf> {
    if let Some(v) = installs_record() {
        for key in ["rc_live", "rc_default", "rc_beta"] {
            if let Some(p) = v.get(key).and_then(|x| x.as_str()).map(PathBuf::from) {
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    let default = PathBuf::from(r"C:\Riot Games\Riot Client\RiotClientServices.exe");
    default.is_file().then_some(default)
}

/// Folders where League of Legends may be installed (its lockfile is there).
pub fn league_install_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = installs_record()
        .and_then(|v| v.get("associated_client").and_then(|a| a.as_object()).map(|o| o.keys().map(PathBuf::from).collect()))
        .unwrap_or_default();
    dirs.push(PathBuf::from(r"C:\Riot Games\League of Legends"));
    dirs
}

#[cfg(test)]
mod tests {
    use super::Game;

    #[test]
    fn both_is_stored_by_name_and_opens_league_by_default() {
        assert_eq!(serde_json::to_string(&Game::Both).unwrap(), "\"both\"");
        assert_eq!(serde_json::from_str::<Game>("\"both\"").unwrap(), Game::Both);
        assert_eq!(Game::Both.launch_args(), Game::Lol.launch_args());
    }
}

#[cfg(test)]
mod config_tests {
    use super::Config;

    #[test]
    fn old_configs_lose_the_settings_folder() {
        let mut old = Config::default();
        old.session_paths.push("Riot Client/Config".into());
        old.session_paths.push("Riot Client\\Config\\".into());
        let new = old.migrate();
        assert!(new.session_paths.iter().all(|p| !p.to_lowercase().contains("config")));
        assert_eq!(new.session_paths, Config::default().session_paths);
    }
}

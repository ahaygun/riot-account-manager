//! The switching logic itself.

use crate::api::{ClientApi, Identity, LoginState};
use crate::identity::{self, FileSession};
use crate::procs::Procs;
use crate::snapshot::{self, Snapshot};
use crate::store::{self, Account, Config, Env, Game, State};
use crate::typer::{LoginTyper, UiTarget};
use crate::vault::{self, KdfParams, Vault, VaultError};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// A match is running; switching would kick the player out.
    InGame(Vec<String>),
    /// The League client is in queue, a ready check or champion select:
    /// closing it now would leave the queue or count as a dodge.
    InMatchmaking(&'static str),
    /// The League client is running but does not say what it is doing.
    LeagueStateUnknown,
    /// Riot Client did not close.
    StillRunning(Vec<String>),
    /// Nobody is signed in.
    NoSession,
    /// Signed in, but without "stay signed in", so there is nothing to keep.
    NotPersisted,
    AlreadySaved(String),
    UnknownAccount(String),
    NameTaken(String),
    HotkeyTaken(String),
    EmptyField(&'static str),
    RiotClientNotFound,
    /// The app lock is on and has not been opened.
    Locked,
    WrongPassword,
    WeakPassword,
}

impl From<VaultError> for Error {
    fn from(e: VaultError) -> Self {
        match e {
            VaultError::Locked => Error::Locked,
            VaultError::Io(e) => Error::Io(e),
        }
    }
}

impl Error {
    /// Stable code the UI can branch on.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Io(_) => "io",
            Error::InGame(_) | Error::InMatchmaking(_) => "in_game",
            Error::LeagueStateUnknown => "league_unknown",
            Error::StillRunning(_) => "still_running",
            Error::NoSession => "no_session",
            Error::NotPersisted => "not_persisted",
            Error::AlreadySaved(_) => "already_saved",
            Error::UnknownAccount(_) => "unknown_account",
            Error::NameTaken(_) => "name_taken",
            Error::HotkeyTaken(_) => "hotkey_taken",
            Error::EmptyField(_) => "empty_field",
            Error::RiotClientNotFound => "riot_client_not_found",
            Error::Locked => "locked",
            Error::WrongPassword => "wrong_password",
            Error::WeakPassword => "weak_password",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "Dosya hatası: {e}"),
            Error::InGame(p) => write!(f, "Oyun açık ({}). Oyun kapanmadan hesap değiştirilmez.", p.join(", ")),
            Error::InMatchmaking(what) => write!(f, "{what}; şimdi hesap değiştirmek seni sıradan çıkarır ya da maçtan kaçmış sayar."),
            Error::LeagueStateUnknown => write!(
                f,
                "League istemcisi açık ama sırada ya da şampiyon seçiminde olup olmadığı okunamadı. Emin değilsen bekle; maçta olmadığından eminsen yine de geçebilirsin."
            ),
            Error::StillRunning(p) => write!(f, "Riot Client kapatılamadı: {}", p.join(", ")),
            Error::NoSession => write!(f, "Riot Client'ta açık bir oturum yok."),
            Error::NotPersisted => write!(
                f,
                "Riot Client kapandığında saklanacak bir oturum bırakmadı; \"Oturumu açık tut\" işaretli olmayabilir. Kutuyu işaretleyip yeniden giriş yaparak tekrar dene ya da hesabı şifreyle ekle."
            ),
            Error::AlreadySaved(n) => write!(f, "Bu oturum zaten \"{n}\" olarak kayıtlı; kaydı güncellendi."),
            Error::UnknownAccount(n) => write!(f, "Hesap bulunamadı: {n}"),
            Error::NameTaken(n) => write!(f, "\"{n}\" adında bir hesap zaten var."),
            Error::HotkeyTaken(n) => write!(f, "Bu kısayol \"{n}\" hesabında kullanılıyor."),
            Error::EmptyField(what) => write!(f, "{what} boş olamaz."),
            Error::RiotClientNotFound => write!(
                f,
                "RiotClientServices.exe bulunamadı. config.json içindeki riot_client_exe alanına tam yolu yaz."
            ),
            Error::Locked => write!(f, "Uygulama kilitli. Devam etmek için kilidi aç."),
            Error::WrongPassword => write!(f, "Ana şifre yanlış."),
            Error::WeakPassword => write!(f, "Ana şifre en az {} karakter olmalı.", vault::MIN_PASSWORD_LEN),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Waits used around Riot Client. Tests shrink them.
#[derive(Debug, Clone)]
pub struct Timing {
    /// How long a polite quit may take before processes are killed.
    pub quit_timeout: Duration,
    /// Pause after the last process is gone, for file handles to close.
    pub settle: Duration,
    pub poll: Duration,
    /// How long the client may take to start answering after launch.
    pub boot_wait: Duration,
    /// "Signed out" must hold this long before we believe it: a starting
    /// client reports signed out for a moment before it loads its session.
    pub signed_out_grace: Duration,
    /// Time for the login page to render before typing.
    pub ui_load: Duration,
    /// How long a typed login may take to go through.
    pub login_wait: Duration,
    /// After a typed login, how long to wait for the game before asking the
    /// client to start it.
    pub game_start_wait: Duration,
    /// How long we keep watching for the user to sign in by hand.
    pub manual_wait: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            quit_timeout: Duration::from_secs(8),
            settle: Duration::from_millis(250),
            // Asking the client is a request to 127.0.0.1; a short wait
            // means a sign-in is noticed, and acted on, without delay.
            poll: Duration::from_millis(300),
            boot_wait: Duration::from_secs(60),
            signed_out_grace: Duration::from_secs(8),
            ui_load: Duration::from_secs(3),
            login_wait: Duration::from_secs(90),
            game_start_wait: Duration::from_secs(15),
            manual_wait: Duration::from_secs(600),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    #[serde(flatten)]
    pub account: Account,
    /// A password is stored for automatic sign-in.
    pub has_password: bool,
    /// A saved session exists.
    pub has_session: bool,
    /// Whole days since Riot last accepted this account's saved session,
    /// when there is one and that is known.
    pub idle_days: Option<u64>,
    /// Unused for long enough that the session may have expired.
    pub stale: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientView {
    pub running: bool,
    /// "signed_in", "signed_out" or "unknown"
    pub login: &'static str,
    /// The "stay signed in" box, when the client says.
    pub persist: Option<bool>,
    pub riot_id: Option<String>,
}

/// What the UI shows.
#[derive(Debug, Clone, Serialize)]
pub struct View {
    pub accounts: Vec<AccountView>,
    /// Account that is signed in right now, if it is one of ours.
    pub active: Option<String>,
    pub client: ClientView,
    /// Someone is signed in, but that session is not saved yet.
    pub unsaved_session: bool,
    pub in_game: bool,
    pub security: SecurityView,
}

/// What the security screen reports. Counts only, never content.
#[derive(Debug, Clone, Serialize)]
pub struct SecurityView {
    /// The optional app lock (master password) is turned on.
    pub lock_enabled: bool,
    pub unlocked: bool,
    /// Where our files are.
    pub data_dir: String,
    pub saved_sessions: usize,
    pub saved_passwords: usize,
    /// Minutes without use before the app locks itself; zero for never.
    pub auto_lock_minutes: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switched {
    pub account: Account,
    /// The game that was started: the account's own, or the one asked for.
    pub game: Game,
    /// The client was (re)started, so the sign-in should be watched with
    /// [`Switcher::await_login`].
    pub watch_login: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LoginOutcome {
    SignedIn,
    /// The account needs a sign-in by hand; the text says why.
    NeedsLogin(String),
    /// Could not tell (client closed, cancelled, or another account).
    Unknown,
}

/// Wiped from memory as soon as it goes out of scope.
#[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
struct Credentials {
    username: String,
    password: String,
}

struct Live {
    running: bool,
    login: LoginState,
    identity: Option<Identity>,
    file: FileSession,
}

impl Live {
    fn signed_in(&self) -> bool {
        self.running && matches!(self.login, LoginState::SignedIn { .. })
    }

    /// A session exists, either in the running client or saved on disk.
    fn has_session(&self) -> bool {
        match (self.running, self.login) {
            (true, LoginState::SignedIn { .. }) => true,
            (true, LoginState::SignedOut) => false,
            // Closed, or still starting: the file is all we know.
            _ => self.file.has_tokens,
        }
    }

    fn puuid(&self) -> Option<&String> {
        if self.signed_in() {
            self.identity.as_ref().and_then(|i| i.puuid.as_ref())
        } else {
            self.file.sub.as_ref()
        }
    }
}

pub struct Switcher {
    pub env: Env,
    pub procs: Arc<dyn Procs>,
    pub api: Arc<dyn ClientApi>,
    pub typer: Arc<dyn LoginTyper>,
    pub timing: Timing,
    /// Cost of turning a new master password into a key.
    pub kdf: KdfParams,
    /// Told when the user has to click Riot Client's window for a typed
    /// login to go on (`true`), and when that is over (`false`).
    pub ask_for_focus: Arc<dyn Fn(bool) + Send + Sync>,
    /// The time in Unix seconds. Tests move it to age a session.
    pub clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    vault: Vault,
    /// Serialises read-modify-write of the account list across threads.
    state_lock: Mutex<()>,
    /// What the session files and the account list looked like when
    /// `sync_idle` last finished; while that holds it has nothing to do.
    idle_seen: Mutex<Option<IdleStamp>>,
    /// The client was signed in the last time `observe_live` looked.
    seen_signed_in: Mutex<bool>,
}

/// (session files, account list): see `Switcher::idle_stamp`.
type IdleStamp = (u64, Option<(u64, SystemTime)>);

impl Switcher {
    pub fn new(env: Env, procs: Arc<dyn Procs>, api: Arc<dyn ClientApi>, typer: Arc<dyn LoginTyper>) -> Self {
        let vault = Vault::new(env.data_dir.join("vault.json"));
        let switcher = Switcher {
            env,
            procs,
            api,
            typer,
            timing: Timing::default(),
            kdf: KdfParams::default(),
            ask_for_focus: Arc::new(|_| {}),
            clock: Arc::new(system_now),
            vault,
            state_lock: Mutex::new(()),
            idle_seen: Mutex::new(None),
            seen_signed_in: Mutex::new(false),
        };
        switcher.finish_relock();
        switcher
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    // ---- storage -----------------------------------------------------------

    fn state_file(&self) -> PathBuf {
        self.env.data_dir.join("accounts.bin")
    }

    /// The account list is encrypted for this Windows user, but not under
    /// the app lock: names stay visible on the lock screen, secrets do not.
    fn state(&self) -> Result<State> {
        match std::fs::read(self.state_file()) {
            Ok(blob) => {
                let json = self.vault.open(&blob)?;
                Ok(serde_json::from_slice(&json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(State::default()),
            Err(e) => Err(e.into()),
        }
    }

    fn save_state(&self, state: &State) -> Result<()> {
        let json = serde_json::to_vec(state).map_err(io::Error::other)?;
        Ok(store::write_atomic(&self.state_file(), &vault::seal_with(None, &json)?)?)
    }

    /// Applies `change` to the stored state under the lock and saves it.
    fn update<R>(&self, change: impl FnOnce(&mut State) -> R) -> Result<R> {
        let _guard = self.state_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.state()?;
        let out = change(&mut state);
        self.save_state(&state)?;
        Ok(out)
    }

    /// Like `update`, for changes to one account that may fail.
    fn update_account<R>(&self, id: &str, change: impl FnOnce(&mut Account, &State) -> Result<R>) -> Result<R> {
        self.update(|state| {
            let others = state.clone();
            let acc = state.accounts.iter_mut().find(|a| a.id == id).ok_or_else(|| Error::UnknownAccount(id.to_string()))?;
            change(acc, &others)
        })?
    }

    pub fn config(&self) -> Result<Config> {
        let path = self.env.data_dir.join("config.json");
        let cfg: Config = store::load_json::<Config>(&path)?.migrate();
        if !path.exists() {
            // Write the defaults once so the user has something to edit.
            let _ = store::save_json(&path, &cfg);
        }
        Ok(cfg)
    }

    pub fn set_check_updates(&self, enabled: bool) -> Result<()> {
        let mut cfg = self.config()?;
        cfg.check_updates = enabled;
        Ok(store::save_json(&self.env.data_dir.join("config.json"), &cfg)?)
    }

    pub fn set_auto_lock_minutes(&self, minutes: u32) -> Result<()> {
        let mut cfg = self.config()?;
        // A day is the most that still counts as "locks itself".
        cfg.auto_lock_minutes = minutes.min(24 * 60);
        Ok(store::save_json(&self.env.data_dir.join("config.json"), &cfg)?)
    }

    /// Locks the app if the lock is on, open, and the app has gone unused
    /// for `idle`, as long as the setting asks for it. Returns true if it
    /// locked. The caller makes sure no switch or sign-in is under way.
    pub fn auto_lock_if_idle(&self, idle: Duration) -> bool {
        if !self.vault.enabled() || !self.vault.unlocked() {
            return false;
        }
        let minutes = self.config().map(|c| c.auto_lock_minutes).unwrap_or(0);
        if minutes == 0 || idle < Duration::from_secs(minutes as u64 * 60) {
            return false;
        }
        self.vault.lock();
        self.log("lock: closed after going unused");
        true
    }

    /// `hotkey` is stored as given; the app layer decides what is valid.
    pub fn set_window_hotkey(&self, hotkey: &str) -> Result<()> {
        let hotkey = clean(hotkey, "Kısayol")?;
        if let Some(other) = self.state()?.accounts.iter().find(|a| a.hotkey.as_deref() == Some(hotkey.as_str())) {
            return Err(Error::HotkeyTaken(other.name.clone()));
        }
        let mut cfg = self.config()?;
        cfg.window_hotkey = hotkey;
        Ok(store::save_json(&self.env.data_dir.join("config.json"), &cfg)?)
    }

    fn write_snapshot(&self, id: &str, snap: &Snapshot) -> Result<()> {
        let blob = self.vault.seal(&snap.encode())?;
        Ok(store::write_atomic(&self.env.snapshot_file(id), &blob)?)
    }

    fn read_snapshot(&self, id: &str) -> Result<Option<Snapshot>> {
        match std::fs::read(self.env.snapshot_file(id)) {
            Ok(blob) => Ok(Some(Snapshot::decode(&self.vault.open(&blob)?)?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn credentials(&self, id: &str) -> Option<Credentials> {
        let blob = std::fs::read(self.env.secret_file(id)).ok()?;
        serde_json::from_slice(&self.vault.open(&blob).ok()?).ok()
    }

    // ---- what is going on right now ---------------------------------------

    fn live(&self, cfg: &Config) -> Live {
        self.live_given(!self.procs.running(&cfg.client_processes).is_empty())
    }

    /// `live`, when the caller has already looked at the process table.
    fn live_given(&self, running: bool) -> Live {
        let login = if running { self.api.login() } else { LoginState::Unknown };
        let identity = if matches!(login, LoginState::SignedIn { .. }) { self.api.identity() } else { None };
        Live { running, login, identity, file: identity::read_file_session(&self.env.riot_games_dir) }
    }

    pub fn view(&self) -> Result<View> {
        let cfg = self.config()?;
        let state = self.state()?;
        // One look at the process table answers both questions.
        let wanted: Vec<String> = cfg.client_processes.iter().chain(&cfg.game_processes).cloned().collect();
        let found = self.procs.running(&wanted);
        let any = |names: &[String]| found.iter().any(|f| names.iter().any(|n| n.eq_ignore_ascii_case(f)));
        let live = self.live_given(any(&cfg.client_processes));
        let active = owner(&state, &live).map(|a| a.id.clone());
        let (login, persist) = match live.login {
            // Login tokens already on disk prove the session is being kept,
            // whatever the client says about the box.
            LoginState::SignedIn { persist } => ("signed_in", if live.file.has_tokens { Some(true) } else { persist }),
            LoginState::SignedOut => ("signed_out", None),
            LoginState::Unknown => ("unknown", None),
        };
        let count = |dir: &str| std::fs::read_dir(self.env.data_dir.join(dir)).map(|d| d.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "bin")).count()).unwrap_or(0);
        Ok(View {
            security: SecurityView {
                lock_enabled: self.vault.enabled(),
                unlocked: self.vault.unlocked(),
                data_dir: self.env.data_dir.display().to_string(),
                saved_sessions: count("snapshots"),
                saved_passwords: count("secrets"),
                auto_lock_minutes: cfg.auto_lock_minutes,
            },
            unsaved_session: live.has_session() && active.is_none(),
            in_game: any(&cfg.game_processes),
            client: ClientView {
                running: live.running,
                login,
                persist,
                riot_id: live.identity.as_ref().and_then(|i| i.riot_id.clone()),
            },
            active: active.clone(),
            accounts: state
                .accounts
                .into_iter()
                .map(|account| {
                    let has_session = !account.hash.is_empty();
                    // The account that is signed in right now is as fresh as can be.
                    let in_use = active.as_deref() == Some(account.id.as_str()) && live.signed_in();
                    let last = account.last_ok.max(account.updated);
                    let idle_days = (has_session && !in_use && last > 0).then(|| self.now().saturating_sub(last) / 86_400);
                    AccountView {
                        has_password: self.env.secret_file(&account.id).is_file(),
                        has_session,
                        idle_days,
                        stale: cfg.stale_after_days > 0 && idle_days.is_some_and(|d| d >= cfg.stale_after_days as u64),
                        account,
                    }
                })
                .collect(),
        })
    }

    /// A match is running, or the League client is on its way into one,
    /// or it cannot be told: a queued switch waits in all three cases.
    pub fn in_game(&self) -> bool {
        self.config().map(|cfg| self.ensure_not_in_game(&cfg, false).is_err()).unwrap_or(false)
    }

    /// The stored accounts, in order. Reads only our own file: nothing is
    /// asked of the client or the process table.
    pub fn accounts(&self) -> Result<Vec<Account>> {
        Ok(self.state()?.accounts)
    }

    pub fn account_name(&self, id: &str) -> Option<String> {
        find(&self.state().ok()?, id).map(|a| a.name.clone())
    }

    // ---- closing the client and keeping its session -----------------------

    /// Refuses while a match, a queue or champion select could be
    /// interrupted. When the League client runs but its state cannot be
    /// read, it refuses too, unless `force` (the user's explicit "switch
    /// anyway"). A running game process is never overridden.
    fn ensure_not_in_game(&self, cfg: &Config, force: bool) -> Result<()> {
        let games = self.procs.running(&cfg.game_processes);
        if !games.is_empty() {
            return Err(Error::InGame(games));
        }
        // Only asked when the League client runs, which is when it matters.
        if self.procs.running(&["LeagueClientUx.exe".to_string()]).is_empty() {
            return Ok(());
        }
        match self.api.league_phase() {
            Some(phase) => match matchmaking(&phase) {
                Some(what) => Err(Error::InMatchmaking(what)),
                None => Ok(()),
            },
            None if force => Ok(()),
            None => Err(Error::LeagueStateUnknown),
        }
    }

    /// Closes Riot Client the polite way first: only a proper quit makes it
    /// write its current session to disk. Killing is the fallback.
    fn close_client(&self, cfg: &Config) -> Result<()> {
        let names = &cfg.client_processes;
        if self.procs.running(names).is_empty() {
            return Ok(());
        }
        if self.api.quit() {
            let start = Instant::now();
            while start.elapsed() < self.timing.quit_timeout && !self.procs.running(names).is_empty() {
                std::thread::sleep(self.timing.poll.min(Duration::from_millis(100)));
            }
        }
        if !self.procs.kill_and_wait(names, Duration::from_secs(3).min(self.timing.quit_timeout)) {
            return Err(Error::StillRunning(self.procs.running(names)));
        }
        std::thread::sleep(self.timing.settle);
        Ok(())
    }

    /// With the client closed, files the session on disk under the account
    /// it belongs to. `seen` is who the client said was signed in before it
    /// closed. Returns the id of the account that was updated.
    fn keep_live_session(&self, cfg: &Config, seen: Option<&Identity>) -> Result<Option<String>> {
        let file = identity::read_file_session(&self.env.riot_games_dir);
        if !file.has_tokens {
            return Ok(None);
        }
        let puuid = seen.and_then(|i| i.puuid.clone()).or(file.sub);
        let state = self.state()?;
        // Nobody said whose session this is, but a token in it may: if it
        // names an account we know, that settles it.
        let hinted = file.hint.as_ref().filter(|_| puuid.is_none()).and_then(|h| state.accounts.iter().find(|a| a.puuid.as_ref().is_some_and(|p| p.eq_ignore_ascii_case(h))));
        let Some(id) = hinted.or_else(|| owner_by(&state, puuid.as_ref())).map(|a| a.id.clone()) else {
            return Ok(None);
        };
        let snap = Snapshot::collect(&self.env.riot_games_dir, &cfg.session_paths)?;
        let hash = snap.hash();
        let known_hash = state.accounts.iter().find(|a| a.id == id).map(|a| a.hash.clone()).unwrap_or_default();
        if known_hash != hash {
            self.write_snapshot(&id, &snap)?;
        }
        let riot_id = seen.and_then(|i| i.riot_id.clone());
        let seen = seen.filter(|i| i.puuid.is_some());
        let changed = self.update(|state| {
            let Some(acc) = state.accounts.iter_mut().find(|a| a.id == id) else { return false };
            let before = acc.clone();
            if acc.hash != hash {
                acc.hash = hash;
                acc.updated = self.now();
            }
            if seen.is_some() {
                // The client itself said this account was signed in.
                acc.needs_login = false;
                acc.last_ok = self.now();
            }
            acc.puuid = acc.puuid.take().or(puuid);
            acc.riot_id = riot_id.or(acc.riot_id.take());
            *acc != before
        })?;
        Ok(changed.then_some(id))
    }

    /// Background upkeep while the client runs: notices who is signed in.
    ///
    /// A session on disk does not always say whose it is, and then it is
    /// filed under the account we last put in place. That is only right
    /// while nobody signs in to something else by hand in Riot Client, so
    /// each new sign-in is looked at: a known account becomes the current
    /// one, an unknown one means there is no current account, and its
    /// session is offered for saving instead of overwriting another's.
    /// Returns true if the current account changed.
    pub fn observe_live(&self) -> Result<bool> {
        let signed_in = matches!(self.api.login(), LoginState::SignedIn { .. });
        let was = std::mem::replace(&mut *self.seen_signed_in.lock().unwrap_or_else(|e| e.into_inner()), signed_in);
        if !signed_in || was {
            return Ok(false);
        }
        let Some(puuid) = self.api.identity().and_then(|i| i.puuid) else {
            // Not known yet: look again on the next round.
            *self.seen_signed_in.lock().unwrap_or_else(|e| e.into_inner()) = false;
            return Ok(false);
        };
        let state = self.state()?;
        let next = live_owner(&state, &puuid);
        if next == state.current {
            return Ok(false);
        }
        self.update(|state| state.current = live_owner(state, &puuid))?;
        self.log(if next.is_some() { "live: a saved account signed in by hand; it is the current one now" } else { "live: an account that is not saved signed in; no current account" });
        Ok(true)
    }

    /// Background upkeep: when the client is closed (for example the user
    /// quit it), make sure its last session is filed.
    ///
    /// This runs every few seconds for as long as the app sits in the tray,
    /// so it first checks, without reading any file, whether anything was
    /// written since it last finished. Only then are the session files read.
    pub fn sync_idle(&self) -> Result<Option<String>> {
        let cfg = self.config()?;
        if !self.vault.unlocked() || !self.procs.running(&cfg.client_processes).is_empty() {
            return Ok(None);
        }
        let before = self.idle_stamp(&cfg);
        if before.is_some() && before == *self.idle_seen() {
            return Ok(None);
        }
        let filed = self.keep_live_session(&cfg, None)?;
        // Filing rewrites the account list, so the stamp is taken again;
        // it is kept only if the session files stood still meanwhile.
        let after = self.idle_stamp(&cfg);
        *self.idle_seen() = after.filter(|a| before.is_some_and(|b| b.0 == a.0));
        Ok(filed)
    }

    fn idle_seen(&self) -> std::sync::MutexGuard<'_, Option<IdleStamp>> {
        self.idle_seen.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Everything `keep_live_session` depends on, by size and time of last
    /// write. `None` if it cannot be told, which means "do the work".
    fn idle_stamp(&self, cfg: &Config) -> Option<IdleStamp> {
        let session = snapshot::stamp(&self.env.riot_games_dir, &cfg.session_paths).ok()?;
        let list = match std::fs::metadata(self.state_file()) {
            Ok(m) => Some((m.len(), m.modified().ok()?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(_) => return None,
        };
        Some((session, list))
    }

    // ---- adding accounts ---------------------------------------------------

    /// Saves the session that is signed in right now as a new account.
    /// Riot Client is closed in the process (that is what writes the
    /// session to disk) and stays closed.
    pub fn capture_new(&self, name: &str, game: Game) -> Result<Account> {
        self.ensure_unlocked()?;
        let name = clean(name, "Hesap adı")?;
        let cfg = self.config()?;
        let live = self.live(&cfg);
        if live.running {
            if !live.signed_in() {
                return Err(Error::NoSession);
            }
            // What the client reports about the "stay signed in" box is not
            // relied on here: some versions report it wrongly. The file the
            // client writes when it quits is what decides, just below.
            self.ensure_not_in_game(&cfg, false)?;
            self.close_client(&cfg)?;
        }
        let file = identity::read_file_session(&self.env.riot_games_dir);
        if !file.has_tokens {
            return Err(if live.running { Error::NotPersisted } else { Error::NoSession });
        }
        let seen = live.identity;
        let puuid = seen.as_ref().and_then(|i| i.puuid.clone()).or(file.sub);

        let state = self.state()?;
        if let Some(existing) = puuid.as_ref().and_then(|p| state.accounts.iter().find(|a| same_account(a.puuid.as_deref(), p))) {
            let existing_name = existing.name.clone();
            self.keep_live_session(&cfg, seen.as_ref())?;
            return Err(Error::AlreadySaved(existing_name));
        }
        ensure_name_free(&state, &name, None)?;

        let snap = Snapshot::collect(&self.env.riot_games_dir, &cfg.session_paths)?;
        let account = Account {
            id: unique_id(&name, &state),
            name,
            puuid,
            riot_id: seen.and_then(|i| i.riot_id),
            game,
            hash: snap.hash(),
            updated: self.now(),
            last_ok: self.now(),
            ..Account::default()
        };
        self.write_snapshot(&account.id, &snap)?;
        self.update(|state| {
            state.current = Some(account.id.clone());
            state.accounts.push(account.clone());
        })?;
        Ok(account)
    }

    /// Adds an account that signs in by typing its password, without a
    /// saved session.
    pub fn add_with_credentials(&self, name: &str, username: &str, password: &str, game: Game) -> Result<Account> {
        self.ensure_unlocked()?;
        let name = clean(name, "Hesap adı")?;
        let state = self.state()?;
        ensure_name_free(&state, &name, None)?;
        let account = Account { id: unique_id(&name, &state), name, game, ..Account::default() };
        self.write_credentials(&account.id, username, password)?;
        self.update(|state| state.accounts.push(account.clone()))?;
        Ok(account)
    }

    /// Signs out locally and opens Riot Client on its login page, so a new
    /// account can sign in.
    pub fn new_login(&self) -> Result<()> {
        // Locked, the outgoing session could not be kept.
        self.ensure_unlocked()?;
        let cfg = self.config()?;
        let exe = self.riot_exe(&cfg)?;
        self.ensure_not_in_game(&cfg, false)?;
        let seen = self.live(&cfg).identity;
        self.close_client(&cfg)?;
        let _ = self.keep_live_session(&cfg, seen.as_ref());
        self.update(|state| state.current = None)?;
        snapshot::clear(&self.env.riot_games_dir, &cfg.session_paths)?;
        self.procs.launch(&exe, &Game::Client.launch_args())?;
        Ok(())
    }

    // ---- switching ---------------------------------------------------------

    /// Signs in as `target` (an id or a name) and starts a game: `game` if
    /// given, otherwise the account's own. Asking for another game on the
    /// account that is already signed in just starts that game.
    pub fn switch(&self, target: &str, game: Option<Game>) -> Result<Switched> {
        self.switch_with(target, game, false)
    }

    /// `switch`, where `force` overrides only an unreadable League client
    /// state (see `ensure_not_in_game`).
    pub fn switch_with(&self, target: &str, game: Option<Game>, force: bool) -> Result<Switched> {
        self.ensure_unlocked()?;
        let cfg = self.config()?;
        let state = self.state()?;
        let account = find(&state, target).cloned().ok_or_else(|| Error::UnknownAccount(target.to_string()))?;
        let game = game.unwrap_or(account.game);
        let exe = self.riot_exe(&cfg)?;
        let live = self.live(&cfg);

        // Already this account: no need to close anything.
        if owner(&state, &live).is_some_and(|a| a.id == account.id) {
            if live.signed_in() {
                if game != Game::Client && !self.api.start_product(game.product()) {
                    self.procs.launch(&exe, &game.launch_args())?;
                }
                return Ok(Switched { account, game, watch_login: false });
            }
            if !live.running {
                self.procs.launch(&exe, &game.launch_args())?;
                return Ok(Switched { account, game, watch_login: true });
            }
        }

        self.ensure_not_in_game(&cfg, force)?;
        // Read the target first: if its snapshot is unreadable we stop
        // before closing anything.
        let snap = self.read_snapshot(&account.id)?;

        let started = Instant::now();
        self.close_client(&cfg)?;
        let closed = started.elapsed();
        // Keep the outgoing account's latest session. Best effort: a
        // failure here must not block the switch.
        let _ = self.keep_live_session(&cfg, live.identity.as_ref());

        // `current` is cleared while the files are in flux, so nothing can
        // file one account's session under another.
        self.update(|state| state.current = None)?;
        match &snap {
            Some(snap) => snap.restore(&self.env.riot_games_dir, &cfg.session_paths)?,
            // No saved session (password-only account): start signed out.
            None => snapshot::clear(&self.env.riot_games_dir, &cfg.session_paths)?,
        }
        self.update(|state| state.current = Some(account.id.clone()))?;

        self.procs.launch(&exe, &game.launch_args())?;
        self.log(&format!(
            "switch: client closed in {} ms, session files swapped in {} ms ({})",
            closed.as_millis(),
            (started.elapsed() - closed).as_millis(),
            if snap.is_some() { "saved session" } else { "no saved session" }
        ));
        Ok(Switched { account, game, watch_login: true })
    }

    /// After a switch: waits for Riot Client to come up and checks that the
    /// saved session was accepted. If it was not and a password is stored,
    /// types it. `cancelled` is polled between steps.
    /// `game` is the one the switch started, when it was not the account's own.
    pub fn await_login(&self, id: &str, game: Option<Game>, cancelled: &dyn Fn() -> bool) -> Result<LoginOutcome> {
        let cfg = self.config()?;
        let t = &self.timing;

        // The grace period exists because a starting client says "signed
        // out" until it has loaded the session on disk. With no session on
        // disk (an account that signs in by password) there is nothing to
        // load, so a quarter of it is kept, only to let the client settle.
        let nothing_to_load = !identity::read_file_session(&self.env.riot_games_dir).has_tokens;
        let grace = if nothing_to_load { t.signed_out_grace / 4 } else { t.signed_out_grace };

        // 1. Wait for the client to answer at all.
        let start = Instant::now();
        while self.api.login() == LoginState::Unknown {
            if cancelled() || start.elapsed() > t.boot_wait {
                return Ok(LoginOutcome::Unknown);
            }
            std::thread::sleep(t.poll);
        }
        self.log(&format!("switch: client answered {} ms after launch", start.elapsed().as_millis()));

        // 2. Signed in, or signed out for long enough to believe it.
        let mut signed_out_since: Option<Instant> = None;
        loop {
            if cancelled() {
                return Ok(LoginOutcome::Unknown);
            }
            match self.api.login() {
                LoginState::SignedIn { .. } => {
                    self.log(&format!("switch: signed in {} ms after launch", start.elapsed().as_millis()));
                    // Never seen signed out: this sign-in came from the
                    // session we put in place, not from anyone's hands.
                    let from_saved = signed_out_since.is_none();
                    return self.confirm_signed_in(id, game, false, from_saved, cancelled);
                }
                LoginState::SignedOut => {
                    if signed_out_since.get_or_insert_with(Instant::now).elapsed() >= grace {
                        break;
                    }
                }
                LoginState::Unknown => {
                    if self.procs.running(&cfg.client_processes).is_empty() {
                        return Ok(LoginOutcome::Unknown);
                    }
                }
            }
            std::thread::sleep(t.poll);
        }

        // 3. The session was not accepted.
        let Some(creds) = self.credentials(id) else {
            self.set_needs_login(id, true)?;
            return Ok(LoginOutcome::NeedsLogin(
                "Oturumun süresi dolmuş. Riot Client'ta \"Oturumu açık tut\" işaretli giriş yap; kayıt kendiliğinden yenilenir.".into(),
            ));
        };
        // The login page needs its window: wait for the process that owns
        // it, then give the page time to render.
        let waiting = Instant::now();
        while self.procs.running(&cfg.ui_processes).is_empty() && waiting.elapsed() < t.ui_load * 5 && !cancelled() {
            std::thread::sleep(t.poll);
        }
        std::thread::sleep(t.ui_load);
        if cancelled() {
            return Ok(LoginOutcome::Unknown);
        }
        if matches!(self.api.login(), LoginState::SignedIn { .. }) {
            return self.confirm_signed_in(id, game, false, false, cancelled);
        }
        self.log(&format!("switch: typing the sign-in {} ms after launch", start.elapsed().as_millis()));
        let exe = self.riot_exe(&cfg).ok();
        let log = |line: &str| self.log(line);
        let ask = |on: bool| (self.ask_for_focus)(on);
        let target = UiTarget {
            processes: &cfg.ui_processes,
            install_dir: exe.as_deref().and_then(|e| e.parent()),
            ask_for_focus: &ask,
            cancelled,
            log: &log,
        };
        if let Err(why) = self.typer.type_login(&creds.username, &creds.password, &target) {
            self.set_needs_login(id, true)?;
            return Ok(LoginOutcome::NeedsLogin(why));
        }
        let typed_at = Instant::now();
        while typed_at.elapsed() < t.login_wait {
            if cancelled() {
                return Ok(LoginOutcome::Unknown);
            }
            if matches!(self.api.login(), LoginState::SignedIn { .. }) {
                self.log(&format!(
                    "switch: typed sign-in accepted after {} ms ({} ms after launch)",
                    typed_at.elapsed().as_millis(),
                    start.elapsed().as_millis()
                ));
                return self.confirm_signed_in(id, game, true, false, cancelled);
            }
            std::thread::sleep(t.poll);
        }
        self.set_needs_login(id, true)?;
        Ok(LoginOutcome::NeedsLogin(
            "Otomatik giriş tamamlanmadı: şifre yanlış olabilir ya da Riot doğrulama kodu istiyor.".into(),
        ))
    }

    /// After `await_login` reported `NeedsLogin`: keeps watching for the
    /// user to sign in by hand, then clears the flag.
    pub fn await_manual_login(&self, id: &str, cancelled: &dyn Fn() -> bool) -> Result<LoginOutcome> {
        let game = None;
        let cfg = self.config()?;
        let start = Instant::now();
        while start.elapsed() < self.timing.manual_wait && !cancelled() {
            match self.api.login() {
                LoginState::SignedIn { .. } => return self.confirm_signed_in(id, game, false, false, cancelled),
                LoginState::Unknown if self.procs.running(&cfg.client_processes).is_empty() => break,
                _ => {}
            }
            // Up to ten minutes of waiting for a person: no need to hurry.
            std::thread::sleep(self.timing.poll * 3);
        }
        Ok(LoginOutcome::Unknown)
    }

    /// `from_saved`: the client signed in straight from the session we
    /// restored, so whoever is signed in is whoever that snapshot holds.
    fn confirm_signed_in(&self, id: &str, started: Option<Game>, typed: bool, from_saved: bool, cancelled: &dyn Fn() -> bool) -> Result<LoginOutcome> {
        let seen = self.api.identity().unwrap_or_default();
        let game = self.update_account(id, |acc, _| {
            if let (Some(mine), Some(theirs)) = (&acc.puuid, &seen.puuid) {
                if !same_account(Some(mine), theirs) {
                    return Ok(None); // someone signed in to a different account
                }
            }
            acc.needs_login = false;
            acc.last_ok = self.now();
            acc.puuid = acc.puuid.take().or(seen.puuid.clone());
            acc.riot_id = seen.riot_id.clone().or(acc.riot_id.take());
            Ok(Some(acc.game))
        })?;
        if game.is_none() && from_saved {
            return self.discard_foreign_snapshot(id, seen.puuid.as_deref());
        }
        let Some(own_game) = game else { return Ok(LoginOutcome::Unknown) };
        let game = started.unwrap_or(own_game);

        // After a typed login the client often stays on its home page
        // instead of starting the game it was launched for; nudge it once.
        if let (true, Some(process)) = (typed, game.client_process()) {
            let names = [process.to_string()];
            let start = Instant::now();
            while self.procs.running(&names).is_empty() && !cancelled() {
                if start.elapsed() >= self.timing.game_start_wait {
                    self.api.start_product(game.product());
                    break;
                }
                std::thread::sleep(self.timing.poll);
            }
        }
        Ok(LoginOutcome::SignedIn)
    }

    /// The snapshot stored for `id` signed in as someone else, so it was
    /// never this account's. It is thrown away rather than used again, the
    /// account is marked as needing a sign-in, and the current account
    /// becomes whoever is really signed in.
    fn discard_foreign_snapshot(&self, id: &str, signed_in: Option<&str>) -> Result<LoginOutcome> {
        match std::fs::remove_file(self.env.snapshot_file(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
            _ => {}
        }
        let other = self.update(|state| {
            if let Some(acc) = state.accounts.iter_mut().find(|a| a.id == id) {
                acc.hash.clear();
                acc.needs_login = true;
            }
            let real = signed_in.and_then(|p| state.accounts.iter().find(|a| a.id != id && same_account(a.puuid.as_deref(), p)));
            let found = real.map(|a| (a.id.clone(), a.name.clone()));
            state.current = found.as_ref().map(|(id, _)| id.clone());
            found.map(|(_, name)| name)
        })?;
        self.log("switch: the saved session belonged to another account; it was discarded");
        let whose = match other {
            Some(name) => format!("\"{name}\" hesabının"),
            None => "kayıtlı olmayan başka bir hesabın".to_string(),
        };
        Ok(LoginOutcome::NeedsLogin(format!(
            "Bu hesabın kaydında {whose} oturumu vardı; yanlış kayıt silindi. Riot Client'ta doğru hesaba \"Oturumu açık tut\" işaretli giriş yap, kayıt kendiliğinden yenilenir."
        )))
    }

    fn set_needs_login(&self, id: &str, value: bool) -> Result<()> {
        self.update_account(id, |acc, _| {
            acc.needs_login = value;
            Ok(())
        })
    }

    /// Copies level and rank from the League client into the account that
    /// is signed in there. Returns true if anything changed.
    pub fn refresh_league(&self) -> Result<bool> {
        let Some(info) = self.api.league() else { return Ok(false) };
        self.update(|state| {
            let Some(id) = owner_by(state, info.puuid.as_ref()).map(|a| a.id.clone()) else { return false };
            let Some(acc) = state.accounts.iter_mut().find(|a| a.id == id) else { return false };
            let before = acc.clone();
            acc.puuid = acc.puuid.take().or(info.puuid);
            acc.riot_id = info.riot_id.or(acc.riot_id.take());
            acc.level = info.level.or(acc.level);
            acc.rank = info.rank; // unranked is information too
            acc.needs_login = false;
            *acc != before
        })
    }

    // ---- editing accounts --------------------------------------------------

    pub fn remove(&self, id: &str) -> Result<()> {
        let removed = self.update(|state| {
            let before = state.accounts.len();
            state.accounts.retain(|a| a.id != id);
            if state.current.as_deref() == Some(id) {
                state.current = None;
            }
            state.accounts.len() != before
        })?;
        if !removed {
            return Err(Error::UnknownAccount(id.to_string()));
        }
        for file in [self.env.snapshot_file(id), self.env.secret_file(id)] {
            match std::fs::remove_file(file) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<()> {
        let name = clean(name, "Hesap adı")?;
        self.update_account(id, |acc, all| {
            ensure_name_free(all, &name, Some(id))?;
            acc.name = name;
            Ok(())
        })
    }

    /// Puts the accounts in the order of `ids`. Accounts missing from
    /// `ids` keep their relative order after the listed ones; unknown ids
    /// are ignored, so a stale list from the window cannot lose accounts.
    pub fn reorder(&self, ids: &[String]) -> Result<()> {
        self.update(|state| {
            let rank = |id: &str| ids.iter().position(|x| x == id).unwrap_or(usize::MAX);
            // A stable sort keeps unlisted accounts in their current order.
            state.accounts.sort_by_key(|a| rank(&a.id));
        })
    }

    pub fn set_game(&self, id: &str, game: Game) -> Result<()> {
        self.update_account(id, |acc, _| {
            acc.game = game;
            Ok(())
        })
    }

    /// `hotkey` is stored as given; the app layer decides what is valid.
    pub fn set_hotkey(&self, id: &str, hotkey: Option<&str>) -> Result<()> {
        let hotkey = hotkey.map(str::trim).filter(|h| !h.is_empty()).map(String::from);
        self.update_account(id, |acc, all| {
            if let Some(h) = &hotkey {
                if let Some(other) = all.accounts.iter().find(|a| a.id != id && a.hotkey.as_ref() == Some(h)) {
                    return Err(Error::HotkeyTaken(other.name.clone()));
                }
            }
            acc.hotkey = hotkey;
            Ok(())
        })
    }

    /// Stores the login used when the saved session has expired.
    pub fn set_credentials(&self, id: &str, username: &str, password: &str) -> Result<()> {
        if find(&self.state()?, id).is_none() {
            return Err(Error::UnknownAccount(id.to_string()));
        }
        self.write_credentials(id, username, password)
    }

    fn write_credentials(&self, id: &str, username: &str, password: &str) -> Result<()> {
        let username = clean(username, "Kullanıcı adı")?;
        if password.is_empty() {
            return Err(Error::EmptyField("Şifre"));
        }
        let creds = Credentials { username, password: password.to_string() };
        let json = Zeroizing::new(serde_json::to_vec(&creds).map_err(io::Error::other)?);
        Ok(store::write_atomic(&self.env.secret_file(id), &self.vault.seal(&json)?)?)
    }

    pub fn clear_credentials(&self, id: &str) -> Result<()> {
        match std::fs::remove_file(self.env.secret_file(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Appends one line to `quickswitch.log` in the data folder, for
    /// finding out why something did not work. Lines never hold account
    /// names, usernames, passwords or tokens. Kept under 256 KB.
    pub fn log(&self, line: &str) {
        use std::io::Write;
        let path = self.env.data_dir.join("quickswitch.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
            let _ = std::fs::rename(&path, path.with_extension("old.log"));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "{} {line}", self.now());
        }
    }

    // ---- app lock and data ------------------------------------------------

    fn ensure_unlocked(&self) -> Result<()> {
        if self.vault.unlocked() { Ok(()) } else { Err(Error::Locked) }
    }

    /// Opens the app lock for this run. False means a wrong password.
    pub fn unlock(&self, password: &str) -> Result<bool> {
        let opened = self.vault.unlock(password)?;
        if opened {
            self.finish_relock();
        }
        Ok(opened)
    }

    /// Cleans up after a change of the app lock that was cut short (a crash
    /// or a power cut). Such a change writes every file anew beside the old
    /// one, switches the lock, then renames; whatever stopped it left
    /// `.new` files behind. One that is sealed the way the lock stands now
    /// belongs to a change that did happen and takes its place; any other
    /// belongs to a change that never did and is removed. With the lock on
    /// and closed neither can be told, so they wait for the next unlock.
    pub fn finish_relock(&self) {
        for dir in ["snapshots", "secrets"] {
            let Ok(entries) = std::fs::read_dir(self.env.data_dir.join(dir)) else { continue };
            for new in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "new")) {
                let verdict = std::fs::read(&new).ok().and_then(|blob| self.vault.sealed_for_now(&blob));
                let done = match verdict {
                    Some(true) => std::fs::rename(&new, new.with_extension("bin")).is_ok(),
                    Some(false) => std::fs::remove_file(&new).is_ok(),
                    None => continue,
                };
                self.log(match (verdict, done) {
                    (Some(true), true) => "lock: finished an interrupted change of the app lock",
                    (Some(false), true) => "lock: removed a leftover of a change of the app lock that did not happen",
                    _ => "lock: could not tidy up after an interrupted change of the app lock",
                });
            }
        }
    }

    pub fn lock_now(&self) {
        self.vault.lock();
    }

    /// Turns the app lock on (`new` given), off (`new` empty), or changes
    /// its password. `current` is required whenever a lock is already on.
    /// Every saved session and password is re-encrypted to match.
    pub fn set_lock(&self, current: Option<&str>, new: Option<&str>) -> Result<()> {
        if self.vault.enabled() {
            // Checked against the stored lock even if the app is unlocked:
            // changing the lock is exactly what someone at an unattended,
            // unlocked computer should not be able to do.
            match self.vault.check(current.unwrap_or_default())? {
                Some(_) => self.vault.unlock(current.unwrap_or_default()).map(|_| ())?,
                None => return Err(Error::WrongPassword),
            }
        }
        let next = match new {
            Some(p) if p.chars().count() < vault::MIN_PASSWORD_LEN => return Err(Error::WeakPassword),
            Some(p) => Some(vault::new_lock(p, self.kdf)?),
            None => None,
        };
        let next_key = next.as_ref().map(|(_, key)| key);

        // Re-seal next to the originals first, so a failure part-way leaves
        // the old files and the old lock intact.
        let mut staged = Vec::new();
        for dir in ["snapshots", "secrets"] {
            let Ok(entries) = std::fs::read_dir(self.env.data_dir.join(dir)) else { continue };
            for path in entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "bin")) {
                let plain = self.vault.open(&std::fs::read(&path)?)?;
                let tmp = path.with_extension("new");
                store::write_synced(&tmp, &vault::seal_with(next_key, &plain)?)?;
                staged.push((tmp, path));
            }
        }
        self.vault.install(next)?;
        for (tmp, path) in staged {
            std::fs::rename(tmp, path)?;
        }
        Ok(())
    }

    /// Deletes everything this app has stored: accounts, saved sessions,
    /// passwords, the lock and settings. Riot Client's own files are not
    /// touched, so whoever is signed in there stays signed in.
    pub fn wipe_all(&self) -> Result<()> {
        let _guard = self.state_lock.lock().unwrap_or_else(|e| e.into_inner());
        self.vault.lock();
        match std::fs::remove_dir_all(&self.env.data_dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    fn riot_exe(&self, cfg: &Config) -> Result<PathBuf> {
        cfg.riot_client_exe.clone().or_else(store::find_riot_client).ok_or(Error::RiotClientNotFound)
    }
}

/// Which stored account a session with this account id belongs to. An
/// exact id match wins. Otherwise the account whose files we last put in
/// place, as long as it has not been tied to a different id.
fn owner_by<'a>(state: &'a State, puuid: Option<&String>) -> Option<&'a Account> {
    if let Some(found) = puuid.and_then(|p| state.accounts.iter().find(|a| same_account(a.puuid.as_deref(), p))) {
        return Some(found);
    }
    let current = state.current.as_ref().and_then(|id| state.accounts.iter().find(|a| &a.id == id))?;
    (puuid.is_none() || current.puuid.is_none()).then_some(current)
}

/// Who is the current account, given that the client says `puuid` is
/// signed in: the account with that id; otherwise the present one if it
/// has no id yet (it may be signing in for the first time); otherwise none.
/// Whether a stored account id and one reported by Riot are the same
/// account. Riot account ids are UUIDs, and different Riot interfaces may
/// spell the same one in different letter case; since a mismatch can make
/// a saved session be thrown away, case never counts.
fn same_account(stored: Option<&str>, reported: &str) -> bool {
    stored.is_some_and(|s| s.eq_ignore_ascii_case(reported))
}

fn live_owner(state: &State, puuid: &str) -> Option<String> {
    if let Some(known) = state.accounts.iter().find(|a| same_account(a.puuid.as_deref(), puuid)) {
        return Some(known.id.clone());
    }
    let current = state.current.as_ref().and_then(|id| state.accounts.iter().find(|a| &a.id == id))?;
    current.puuid.is_none().then(|| current.id.clone())
}

fn owner<'a>(state: &'a State, live: &Live) -> Option<&'a Account> {
    if !live.has_session() {
        return None;
    }
    owner_by(state, live.puuid())
}

/// The League client phases in which closing it costs the player something.
fn matchmaking(phase: &str) -> Option<&'static str> {
    match phase {
        "Matchmaking" => Some("Sıradasın"),
        "ReadyCheck" => Some("Maç bulundu"),
        "ChampSelect" => Some("Şampiyon seçimindesin"),
        "GameStart" | "InProgress" | "Reconnect" => Some("Maçtasın"),
        _ => None,
    }
}

fn find<'a>(state: &'a State, target: &str) -> Option<&'a Account> {
    state
        .accounts
        .iter()
        .find(|a| a.id == target)
        .or_else(|| state.accounts.iter().find(|a| a.name.to_lowercase() == target.to_lowercase()))
}

fn clean(value: &str, what: &'static str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() { Err(Error::EmptyField(what)) } else { Ok(value.to_string()) }
}

fn ensure_name_free(state: &State, name: &str, except: Option<&str>) -> Result<()> {
    let taken = state.accounts.iter().any(|a| Some(a.id.as_str()) != except && a.name.to_lowercase() == name.to_lowercase());
    if taken { Err(Error::NameTaken(name.to_string())) } else { Ok(()) }
}

fn unique_id(name: &str, state: &State) -> String {
    let mut base: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = "hesap".to_string();
    }
    let taken = |id: &str| state.accounts.iter().any(|a| a.id == id);
    if !taken(&base) {
        return base;
    }
    (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).unwrap()
}

fn system_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

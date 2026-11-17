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
            Error::InGame(_) => "in_game",
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
            Error::InGame(p) => write!(f, "Maç devam ediyor ({}).", p.join(", ")),
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
            poll: Duration::from_secs(1),
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Switched {
    pub account: Account,
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
    vault: Vault,
    /// Serialises read-modify-write of the account list across threads.
    state_lock: Mutex<()>,
}

impl Switcher {
    pub fn new(env: Env, procs: Arc<dyn Procs>, api: Arc<dyn ClientApi>, typer: Arc<dyn LoginTyper>) -> Self {
        let vault = Vault::new(env.data_dir.join("vault.json"));
        Switcher { env, procs, api, typer, timing: Timing::default(), kdf: KdfParams::default(), vault, state_lock: Mutex::new(()) }
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
        let cfg: Config = store::load_json(&path)?;
        if !path.exists() {
            // Write the defaults once so the user has something to edit.
            let _ = store::save_json(&path, &cfg);
        }
        Ok(cfg)
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
        let running = !self.procs.running(&cfg.client_processes).is_empty();
        let login = if running { self.api.login() } else { LoginState::Unknown };
        let identity = if matches!(login, LoginState::SignedIn { .. }) { self.api.identity() } else { None };
        Live { running, login, identity, file: identity::read_file_session(&self.env.riot_games_dir) }
    }

    pub fn view(&self) -> Result<View> {
        let cfg = self.config()?;
        let state = self.state()?;
        let live = self.live(&cfg);
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
            },
            unsaved_session: live.has_session() && active.is_none(),
            in_game: !self.procs.running(&cfg.game_processes).is_empty(),
            client: ClientView {
                running: live.running,
                login,
                persist,
                riot_id: live.identity.as_ref().and_then(|i| i.riot_id.clone()),
            },
            active,
            accounts: state
                .accounts
                .into_iter()
                .map(|account| AccountView {
                    has_password: self.env.secret_file(&account.id).is_file(),
                    has_session: !account.hash.is_empty(),
                    account,
                })
                .collect(),
        })
    }

    /// A match is running right now.
    pub fn in_game(&self) -> bool {
        self.config().map(|cfg| !self.procs.running(&cfg.game_processes).is_empty()).unwrap_or(false)
    }

    pub fn account_name(&self, id: &str) -> Option<String> {
        find(&self.state().ok()?, id).map(|a| a.name.clone())
    }

    // ---- closing the client and keeping its session -----------------------

    fn ensure_not_in_game(&self, cfg: &Config) -> Result<()> {
        let games = self.procs.running(&cfg.game_processes);
        if games.is_empty() { Ok(()) } else { Err(Error::InGame(games)) }
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
                std::thread::sleep(self.timing.poll.min(Duration::from_millis(250)));
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
        let Some(id) = owner_by(&state, puuid.as_ref()).map(|a| a.id.clone()) else {
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
                acc.updated = now();
            }
            if seen.is_some() {
                // The client itself said this account was signed in.
                acc.needs_login = false;
            }
            acc.puuid = acc.puuid.take().or(puuid);
            acc.riot_id = riot_id.or(acc.riot_id.take());
            *acc != before
        })?;
        Ok(changed.then_some(id))
    }

    /// Background upkeep: when the client is closed (for example the user
    /// quit it), make sure its last session is filed.
    pub fn sync_idle(&self) -> Result<Option<String>> {
        let cfg = self.config()?;
        if !self.vault.unlocked() || !self.procs.running(&cfg.client_processes).is_empty() {
            return Ok(None);
        }
        self.keep_live_session(&cfg, None)
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
            self.ensure_not_in_game(&cfg)?;
            self.close_client(&cfg)?;
        }
        let file = identity::read_file_session(&self.env.riot_games_dir);
        if !file.has_tokens {
            return Err(if live.running { Error::NotPersisted } else { Error::NoSession });
        }
        let seen = live.identity;
        let puuid = seen.as_ref().and_then(|i| i.puuid.clone()).or(file.sub);

        let state = self.state()?;
        if let Some(existing) = puuid.as_ref().and_then(|p| state.accounts.iter().find(|a| a.puuid.as_ref() == Some(p))) {
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
            updated: now(),
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
        self.ensure_not_in_game(&cfg)?;
        let seen = self.live(&cfg).identity;
        self.close_client(&cfg)?;
        let _ = self.keep_live_session(&cfg, seen.as_ref());
        self.update(|state| state.current = None)?;
        snapshot::clear(&self.env.riot_games_dir, &cfg.session_paths)?;
        self.procs.launch(&exe, &Game::Client.launch_args())?;
        Ok(())
    }

    // ---- switching ---------------------------------------------------------

    /// Signs in as `target` (an id or a name) and starts its game.
    pub fn switch(&self, target: &str) -> Result<Switched> {
        self.ensure_unlocked()?;
        let cfg = self.config()?;
        let state = self.state()?;
        let account = find(&state, target).cloned().ok_or_else(|| Error::UnknownAccount(target.to_string()))?;
        let exe = self.riot_exe(&cfg)?;
        let live = self.live(&cfg);

        // Already this account: no need to close anything.
        if owner(&state, &live).is_some_and(|a| a.id == account.id) {
            if live.signed_in() {
                if account.game != Game::Client && !self.api.start_product(account.game.product()) {
                    self.procs.launch(&exe, &account.game.launch_args())?;
                }
                return Ok(Switched { account, watch_login: false });
            }
            if !live.running {
                self.procs.launch(&exe, &account.game.launch_args())?;
                return Ok(Switched { account, watch_login: true });
            }
        }

        self.ensure_not_in_game(&cfg)?;
        // Read the target first: if its snapshot is unreadable we stop
        // before closing anything.
        let snap = self.read_snapshot(&account.id)?;

        self.close_client(&cfg)?;
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

        self.procs.launch(&exe, &account.game.launch_args())?;
        Ok(Switched { account, watch_login: true })
    }

    /// After a switch: waits for Riot Client to come up and checks that the
    /// saved session was accepted. If it was not and a password is stored,
    /// types it. `cancelled` is polled between steps.
    pub fn await_login(&self, id: &str, cancelled: &dyn Fn() -> bool) -> Result<LoginOutcome> {
        let cfg = self.config()?;
        let t = &self.timing;

        // 1. Wait for the client to answer at all.
        let start = Instant::now();
        while self.api.login() == LoginState::Unknown {
            if cancelled() || start.elapsed() > t.boot_wait {
                return Ok(LoginOutcome::Unknown);
            }
            std::thread::sleep(t.poll);
        }

        // 2. Signed in, or signed out for long enough to believe it.
        let mut signed_out_since: Option<Instant> = None;
        loop {
            if cancelled() {
                return Ok(LoginOutcome::Unknown);
            }
            match self.api.login() {
                LoginState::SignedIn { .. } => return self.confirm_signed_in(id, false, cancelled),
                LoginState::SignedOut => {
                    if signed_out_since.get_or_insert_with(Instant::now).elapsed() >= t.signed_out_grace {
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
        std::thread::sleep(t.ui_load);
        if cancelled() {
            return Ok(LoginOutcome::Unknown);
        }
        if matches!(self.api.login(), LoginState::SignedIn { .. }) {
            return self.confirm_signed_in(id, false, cancelled);
        }
        let exe = self.riot_exe(&cfg).ok();
        let target = UiTarget { processes: &cfg.ui_processes, install_dir: exe.as_deref().and_then(|e| e.parent()) };
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
                return self.confirm_signed_in(id, true, cancelled);
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
        let cfg = self.config()?;
        let start = Instant::now();
        while start.elapsed() < self.timing.manual_wait && !cancelled() {
            match self.api.login() {
                LoginState::SignedIn { .. } => return self.confirm_signed_in(id, false, cancelled),
                LoginState::Unknown if self.procs.running(&cfg.client_processes).is_empty() => break,
                _ => {}
            }
            std::thread::sleep(self.timing.poll);
        }
        Ok(LoginOutcome::Unknown)
    }

    fn confirm_signed_in(&self, id: &str, typed: bool, cancelled: &dyn Fn() -> bool) -> Result<LoginOutcome> {
        let seen = self.api.identity().unwrap_or_default();
        let game = self.update_account(id, |acc, _| {
            if let (Some(mine), Some(theirs)) = (&acc.puuid, &seen.puuid) {
                if mine != theirs {
                    return Ok(None); // someone signed in to a different account
                }
            }
            acc.needs_login = false;
            acc.puuid = acc.puuid.take().or(seen.puuid.clone());
            acc.riot_id = seen.riot_id.clone().or(acc.riot_id.take());
            Ok(Some(acc.game))
        })?;
        let Some(game) = game else { return Ok(LoginOutcome::Unknown) };

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

    // ---- app lock and data ------------------------------------------------

    fn ensure_unlocked(&self) -> Result<()> {
        if self.vault.unlocked() { Ok(()) } else { Err(Error::Locked) }
    }

    /// Opens the app lock for this run. False means a wrong password.
    pub fn unlock(&self, password: &str) -> Result<bool> {
        Ok(self.vault.unlock(password)?)
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
                std::fs::write(&tmp, vault::seal_with(next_key, &plain)?)?;
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
    if let Some(found) = puuid.and_then(|p| state.accounts.iter().find(|a| a.puuid.as_ref() == Some(p))) {
        return Some(found);
    }
    let current = state.current.as_ref().and_then(|id| state.accounts.iter().find(|a| &a.id == id))?;
    (puuid.is_none() || current.puuid.is_none()).then_some(current)
}

fn owner<'a>(state: &'a State, live: &Live) -> Option<&'a Account> {
    if !live.has_session() {
        return None;
    }
    owner_by(state, live.puuid())
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

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

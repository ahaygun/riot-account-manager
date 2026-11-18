//! Accounts coming in from elsewhere and going out: the League Accounts
//! file format, pasted lists, and our own encrypted backup.
//!
//! Sessions never travel. They are tied to this computer's Riot Client and
//! Riot may not take them elsewhere; an account that arrives without a
//! password signs in by hand once and is saved again from there.

use super::*;
use crate::vault::PortableError;

/// One account on its way in.
#[derive(Default)]
struct Incoming {
    username: String,
    password: Zeroizing<String>,
    /// What to show it as; falls back to the Riot ID, then the username.
    name: String,
    riot_id: Option<String>,
    puuid: Option<String>,
    level: Option<u64>,
    rank: Option<String>,
    region: Option<String>,
    note: String,
    game: Game,
    hotkey: Option<String>,
}

/// One account as the League Accounts app writes and reads it. Fields it
/// has and we do not are left to their defaults on its side.
#[derive(Serialize, Deserialize, Default, Zeroize, ZeroizeOnDrop)]
struct LeagueAccountsEntry {
    #[serde(default)]
    account_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    level: String,
}

/// One account inside our backup.
#[derive(Serialize, Deserialize, Default, Zeroize, ZeroizeOnDrop)]
struct BackupAccount {
    #[serde(default)]
    name: String,
    #[zeroize(skip)]
    #[serde(default)]
    game: Game,
    #[serde(default)]
    riot_id: Option<String>,
    #[serde(default)]
    puuid: Option<String>,
    #[serde(default)]
    hotkey: Option<String>,
    #[serde(default)]
    note: String,
    #[serde(default)]
    region: Option<String>,
    #[zeroize(skip)]
    #[serde(default)]
    level: Option<u64>,
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Backup {
    accounts: Vec<BackupAccount>,
}

fn some(text: &str) -> Option<String> {
    Some(text.trim()).filter(|t| !t.is_empty()).map(str::to_string)
}

impl Switcher {
    /// Adds `items`, leaving out what is already here. With `dry_run`
    /// nothing is written and the numbers say what would happen.
    ///
    /// An account is "already here" when its username is stored (in any
    /// letter case), or its Riot account id is, or, for one that has
    /// neither, its name. With `need_password`, an item without a username
    /// and password is counted as incomplete instead of added.
    fn add_many(&self, items: Vec<Incoming>, need_password: bool, dry_run: bool) -> Result<Imported> {
        self.ensure_unlocked()?;
        let mut state = self.state()?;
        let mut usernames: Vec<String> =
            state.accounts.iter().filter_map(|a| self.credentials(&a.id)).map(|c| c.username.trim().to_lowercase()).collect();
        let mut result = Imported::default();
        for item in items {
            let username = item.username.trim().to_string();
            let has_login = !username.is_empty() && !item.password.is_empty();
            if need_password && !has_login {
                result.incomplete += 1;
                continue;
            }
            let wanted = some(&item.name).or_else(|| item.riot_id.clone()).or_else(|| some(&username));
            let Some(wanted) = wanted else {
                result.incomplete += 1;
                continue;
            };
            let here = (has_login && usernames.contains(&username.to_lowercase()))
                || item.puuid.as_deref().is_some_and(|p| state.accounts.iter().any(|a| same_account(a.puuid.as_deref(), p)))
                || (!has_login && item.puuid.is_none() && ensure_name_free(&state, &wanted, None).is_err());
            if here {
                result.already_here += 1;
                continue;
            }
            // A name that is taken gets a number: two regions can hold the same one.
            let name = (1..)
                .map(|n| if n == 1 { wanted.clone() } else { format!("{wanted} ({n})") })
                .find(|n| ensure_name_free(&state, n, None).is_ok())
                .unwrap_or(wanted);
            let account = Account {
                id: unique_id(&name, &state),
                name,
                riot_id: item.riot_id.clone().filter(|r| r.contains('#')),
                puuid: item.puuid.clone(),
                level: item.level.filter(|l| *l > 0),
                rank: item.rank.clone(),
                region: item.region.clone(),
                note: item.note.chars().take(200).collect(),
                game: item.game,
                // A shortcut another account already answers to is dropped.
                hotkey: item.hotkey.clone().filter(|h| !state.accounts.iter().any(|a| a.hotkey.as_ref() == Some(h))),
                // Nothing to sign in with yet: a sign-in by hand is due.
                needs_login: !has_login,
                ..Account::default()
            };
            if !dry_run {
                if has_login {
                    self.write_credentials(&account.id, &username, &item.password)?;
                }
                self.update(|state| state.accounts.push(account.clone()))?;
            }
            // Kept in step either way, so a dry run counts as the real one will.
            state.accounts.push(account);
            if has_login {
                usernames.push(username.to_lowercase());
            }
            result.added += 1;
        }
        Ok(result)
    }

    /// Adds the accounts of a file exported by the "League Accounts" app: a
    /// JSON list in which each entry has `account_id` (the username to sign
    /// in with), `password` in plain text and `name` (the Riot ID). Every
    /// account comes in as one that signs in by password; ranks are read
    /// from the League client once it has signed in.
    pub fn import_league_accounts(&self, json: &str) -> Result<Imported> {
        self.ensure_unlocked()?;
        let entries: Vec<LeagueAccountsEntry> = serde_json::from_str(json).map_err(|_| Error::BadImport)?;
        let items = entries
            .iter()
            .map(|e| Incoming {
                username: e.account_id.clone(),
                password: Zeroizing::new(e.password.clone()),
                name: e.name.clone(),
                riot_id: some(&e.name),
                level: e.level.trim().parse().ok(),
                region: some(&e.region).map(|r| r.to_lowercase()),
                note: e.description.clone(),
                ..Incoming::default()
            })
            .collect();
        let result = self.add_many(items, true, false)?;
        self.log(&format!("import: {} added, {} already here, {} incomplete", result.added, result.already_here, result.incomplete));
        Ok(result)
    }

    /// Adds accounts from pasted lines, one account per line:
    /// `username--password` or `username--Riot ID--password` (the form the
    /// League Accounts app uses for its bulk add). Empty lines are skipped;
    /// a line that is neither form counts as incomplete. With `dry_run`
    /// nothing is added and the numbers say what would be.
    pub fn bulk_add(&self, lines: &str, dry_run: bool) -> Result<Imported> {
        let mut bad = 0;
        let mut items = Vec::new();
        for line in lines.lines().map(str::trim).filter(|l| !l.is_empty()) {
            // The password comes last and may itself hold "--".
            let (username, rest) = line.split_once("--").unwrap_or((line, ""));
            let (riot_id, password) = match rest.split_once("--") {
                Some((middle, password)) if middle.contains('#') => (some(middle), password),
                _ => (None, rest),
            };
            if username.trim().is_empty() || password.is_empty() {
                bad += 1;
                continue;
            }
            items.push(Incoming {
                username: username.trim().to_string(),
                password: Zeroizing::new(password.to_string()),
                name: riot_id.clone().unwrap_or_default(),
                riot_id,
                ..Incoming::default()
            });
        }
        let mut result = self.add_many(items, true, dry_run)?;
        result.incomplete += bad;
        if !dry_run {
            self.log(&format!("bulk add: {} added, {} already here, {} incomplete", result.added, result.already_here, result.incomplete));
        }
        Ok(result)
    }

    /// Every account that has a stored password, as the League Accounts app
    /// reads it: **passwords in plain text**. `region` is written for
    /// accounts whose region we do not know; that app refuses an entry
    /// without one. The caller is the one who warns the user.
    pub fn export_plain(&self, master: Option<&str>, region: &str) -> Result<Zeroizing<String>> {
        self.confirm_master(master)?;
        let entries: Vec<LeagueAccountsEntry> = self
            .state()?
            .accounts
            .iter()
            .filter_map(|a| {
                let creds = self.credentials(&a.id)?;
                Some(LeagueAccountsEntry {
                    account_id: creds.username.clone(),
                    name: a.riot_id.clone().unwrap_or_else(|| a.name.clone()),
                    region: a.region.clone().unwrap_or_else(|| region.trim().to_lowercase()),
                    password: creds.password.clone(),
                    description: a.note.clone(),
                    level: a.level.map(|l| l.to_string()).unwrap_or_default(),
                })
            })
            .collect();
        if entries.is_empty() {
            return Err(Error::NothingToExport);
        }
        self.log(&format!("export: {} accounts written with passwords in plain text", entries.len()));
        Ok(Zeroizing::new(serde_json::to_string_pretty(&entries).map_err(io::Error::other)?))
    }

    /// Every account, with its password where one is stored, encrypted
    /// under `password` alone so the file opens on any computer.
    pub fn export_backup(&self, master: Option<&str>, password: &str) -> Result<String> {
        self.confirm_master(master)?;
        if password.chars().count() < vault::MIN_PASSWORD_LEN {
            return Err(Error::WeakBackupPassword);
        }
        let accounts: Vec<BackupAccount> = self
            .state()?
            .accounts
            .iter()
            .map(|a| {
                let creds = self.credentials(&a.id);
                BackupAccount {
                    name: a.name.clone(),
                    game: a.game,
                    riot_id: a.riot_id.clone(),
                    puuid: a.puuid.clone(),
                    hotkey: a.hotkey.clone(),
                    note: a.note.clone(),
                    region: a.region.clone(),
                    level: a.level,
                    rank: a.rank.clone(),
                    username: creds.as_ref().map(|c| c.username.clone()),
                    password: creds.as_ref().map(|c| c.password.clone()),
                }
            })
            .collect();
        if accounts.is_empty() {
            return Err(Error::NothingToExport);
        }
        let count = accounts.len();
        let plain = Zeroizing::new(serde_json::to_vec(&Backup { accounts }).map_err(io::Error::other)?);
        let file = vault::seal_portable(password, &plain, self.kdf)?;
        self.log(&format!("export: encrypted backup of {count} accounts"));
        Ok(file)
    }

    /// Adds the accounts of a backup made by `export_backup`. With
    /// `dry_run` the file is opened and counted, and nothing is added.
    pub fn import_backup(&self, file: &str, password: &str, dry_run: bool) -> Result<Imported> {
        self.ensure_unlocked()?;
        let plain = vault::open_portable(password, file).map_err(|e| match e {
            PortableError::NotABackup => Error::BadBackup,
            PortableError::WrongPasswordOrDamaged => Error::BackupNotOpened,
        })?;
        let backup: Backup = serde_json::from_slice(&plain).map_err(|_| Error::BadBackup)?;
        let items = backup
            .accounts
            .iter()
            .map(|a| Incoming {
                username: a.username.clone().unwrap_or_default(),
                password: Zeroizing::new(a.password.clone().unwrap_or_default()),
                name: a.name.clone(),
                riot_id: a.riot_id.clone(),
                puuid: a.puuid.clone(),
                level: a.level,
                rank: a.rank.clone(),
                region: a.region.clone(),
                note: a.note.clone(),
                game: a.game,
                hotkey: a.hotkey.clone(),
            })
            .collect();
        let result = self.add_many(items, false, dry_run)?;
        if !dry_run {
            self.log(&format!("import: backup, {} added, {} already here, {} incomplete", result.added, result.already_here, result.incomplete));
        }
        Ok(result)
    }
}

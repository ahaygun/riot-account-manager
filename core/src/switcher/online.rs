//! What the app learns from the internet: the public League profile of
//! each account, and the pictures of their profile icons. Everything here
//! goes out through `net::Gate`; this file decides only when to ask and
//! what to do with the answers.

use super::*;
use crate::net::{NetError, NetSettings, Purpose};
use crate::opgg;

/// What a round of looking up ranks came to, in numbers of accounts.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct Refreshed {
    /// Looked up, and something about the account changed.
    pub changed: usize,
    /// Looked up, and nothing was new.
    pub same: usize,
    /// No such Riot ID in that region.
    pub not_found: usize,
    /// Could not be looked up: no answer, or an answer that was not a profile.
    pub failed: usize,
    /// Not looked up: no Riot ID or no region to look under.
    pub skipped: usize,
    /// The site asked for a pause, or the user has this turned off.
    pub paused: bool,
}

/// What this module remembers between calls. Nothing here is saved.
#[derive(Default)]
pub(super) struct State {
    /// Unix seconds before which the profile site is left alone, after it
    /// refused or stopped making sense.
    quiet_until: Mutex<u64>,
    /// The game version Riot's picture server files icons under, and when
    /// it was asked.
    icon_version: Mutex<Option<(String, u64)>>,
    /// One refresh at a time.
    refreshing: Mutex<()>,
}

/// How long the profile site is left alone after it says no.
const QUIET_SECONDS: u64 = 15 * 60;
/// Accounts looked up in one automatic round, so a long list is spread out.
const PER_ROUND: usize = 4;

impl Switcher {
    pub fn net_settings(&self) -> NetSettings {
        self.config().map(|c| c.net).unwrap_or_default()
    }

    /// Stores the user's choices about the internet. Calling this at all
    /// is the answer to "may the app connect?": from here on it has been
    /// decided, one way or the other.
    pub fn set_net_settings(&self, mut settings: NetSettings) -> Result<()> {
        settings.decided = true;
        settings.refresh_minutes = settings.refresh_minutes.clamp(10, 24 * 60);
        let mut cfg = self.config()?;
        cfg.net = settings;
        store::save_json(&self.env.data_dir.join("config.json"), &cfg)?;
        self.log(&format!(
            "net: settings changed (online {}, ranks {}, icons {})",
            settings.online, settings.lol_ranks, settings.icons
        ));
        Ok(())
    }

    /// The region to look an account up under: the one it is known to play
    /// on, else what its Riot ID's tag suggests.
    fn lookup_region(account: &Account) -> Option<&'static str> {
        account.region.as_deref().and_then(opgg::region_code).or_else(|| account.riot_id.as_deref().and_then(opgg::region_from_tag))
    }

    /// Looks up the public League profile of accounts that are due:
    /// `only` names one account to look up now; otherwise `all` looks up
    /// every account, and without it a few of those not looked at for the
    /// time the user set. Accounts that only play VALORANT are left out.
    pub fn refresh_ranks(&self, only: Option<&str>, all: bool) -> Result<Refreshed> {
        let _one_at_a_time = self.online.refreshing.lock().unwrap_or_else(|e| e.into_inner());
        let settings = self.net_settings();
        let mut result = Refreshed::default();
        let now = self.now();
        if !settings.allows(Purpose::LolRanks) || now < *self.online.quiet_until.lock().unwrap_or_else(|e| e.into_inner()) {
            result.paused = true;
            return Ok(result);
        }
        let every = u64::from(settings.refresh_minutes.max(10)) * 60;
        let mut due: Vec<(String, String)> = Vec::new(); // (account id, address)
        for account in self.state()?.accounts {
            if only.is_some_and(|id| id != account.id) || account.game == Game::Valorant {
                continue;
            }
            let address = Self::lookup_region(&account).zip(account.riot_id.as_deref()).and_then(|(region, id)| opgg::profile_url(region, id));
            match address {
                None => result.skipped += 1,
                Some(_) if only.is_none() && !all && now.saturating_sub(account.looked_at) < every => {}
                Some(address) => due.push((account.id, address)),
            }
        }
        if only.is_none() && !all {
            due.truncate(PER_ROUND);
        }
        for (i, (id, address)) in due.iter().enumerate() {
            if i > 0 {
                std::thread::sleep(self.timing.net_gap);
            }
            let found = match self.net.get(&settings, Purpose::LolRanks, address) {
                Ok(page) => opgg::parse(&String::from_utf8_lossy(&page)).ok_or(None),
                Err(NetError::Status(404)) => Err(Some("not_found")),
                Err(NetError::Status(403 | 429)) | Err(NetError::Off) => {
                    // Told to stop: stop, and stay away for a while.
                    self.go_quiet("the site asked for a pause");
                    result.paused = true;
                    break;
                }
                Err(_) => Err(None),
            };
            match found {
                Ok(profile) => {
                    if self.apply_profile(id, &profile)? {
                        result.changed += 1;
                    } else {
                        result.same += 1;
                    }
                }
                Err(Some(problem)) => {
                    result.not_found += 1;
                    let at = self.now();
                    let _ = self.update_account(id, |acc, _| {
                        acc.looked_at = at;
                        acc.lookup_problem = Some(problem.to_string());
                        Ok(())
                    });
                }
                Err(None) => {
                    result.failed += 1;
                    // A page that came back but is not a profile means the
                    // site changed: asking for more would only repeat that.
                    if result.failed >= 2 && result.changed + result.same == 0 {
                        self.go_quiet("answers could not be read");
                        result.paused = true;
                        break;
                    }
                }
            }
        }
        if !due.is_empty() {
            self.log(&format!(
                "ranks: {} looked up, {} changed, {} not found, {} failed{}",
                due.len(),
                result.changed,
                result.not_found,
                result.failed,
                if result.paused { ", paused" } else { "" }
            ));
        }
        Ok(result)
    }

    fn go_quiet(&self, why: &str) {
        *self.online.quiet_until.lock().unwrap_or_else(|e| e.into_inner()) = self.now() + QUIET_SECONDS;
        self.log(&format!("ranks: leaving the profile site alone for {} minutes ({why})", QUIET_SECONDS / 60));
    }

    /// Takes over what a profile page said. The League client's own word
    /// (read while the account was signed in) stands unless the page is
    /// about a later moment. Returns whether anything changed.
    fn apply_profile(&self, id: &str, profile: &opgg::Profile) -> Result<bool> {
        let now = self.now();
        self.update_account(id, |acc, _| {
            let before = acc.clone();
            acc.looked_at = now;
            acc.lookup_problem = None;
            // Without a time of its own, a page is taken to be about now.
            let about = profile.updated.unwrap_or(now).min(now);
            let client_is_newer = acc.rank_from.as_deref() == Some("client") && acc.rank_at >= about;
            if !client_is_newer {
                acc.rank = profile.rank.clone();
                acc.level = profile.level.or(acc.level);
                acc.rank_at = about;
                acc.rank_from = Some("opgg".into());
                acc.icon = profile.icon.or(acc.icon);
            }
            // The client does not report these; the page is the only source.
            acc.wins = profile.wins;
            acc.losses = profile.losses;
            acc.icon = acc.icon.or(profile.icon);
            let changed = (&acc.rank, acc.level, acc.wins, acc.losses, acc.icon, &acc.lookup_problem)
                != (&before.rank, before.level, before.wins, before.losses, before.icon, &before.lookup_problem);
            Ok(changed)
        })
    }

    /// Sets what an account is looked up by, for accounts the League client
    /// has not told us about (never signed in here). An empty Riot ID or
    /// region clears it.
    pub fn set_lookup(&self, id: &str, riot_id: &str, region: &str) -> Result<()> {
        let riot_id = Some(riot_id.trim()).filter(|r| !r.is_empty());
        if riot_id.is_some_and(|r| !r.rsplit_once('#').is_some_and(|(name, tag)| !name.trim().is_empty() && !tag.trim().is_empty())) {
            return Err(Error::BadRiotId);
        }
        let region = match region.trim() {
            "" => None,
            given => Some(opgg::region_code(given).ok_or(Error::BadRegion)?.to_string()),
        };
        self.update_account(id, |acc, _| {
            if acc.riot_id.as_deref() != riot_id || acc.region != region {
                // Looked up afresh under the new name.
                acc.looked_at = 0;
                acc.lookup_problem = None;
            }
            acc.riot_id = riot_id.map(str::to_string);
            acc.region = region;
            Ok(())
        })
    }

    /// The picture of profile icon `icon` as PNG bytes: from the folder of
    /// pictures already fetched, else from Riot's picture server if the
    /// user allows that. Pictures never change, so each is fetched once.
    pub fn icon_png(&self, icon: u32) -> Option<Vec<u8>> {
        let file = self.env.data_dir.join("icons").join(format!("{icon}.png"));
        if let Ok(bytes) = std::fs::read(&file) {
            return Some(bytes);
        }
        let settings = self.net_settings();
        if !settings.allows(Purpose::Icons) {
            return None;
        }
        let version = self.icon_version(&settings)?;
        let address = format!("https://ddragon.leagueoflegends.com/cdn/{version}/img/profileicon/{icon}.png");
        let bytes = self.net.get(&settings, Purpose::Icons, &address).ok()?;
        // Only a picture is kept: the file must start as a PNG does.
        if !bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            self.log("icons: an answer that is not a PNG was thrown away");
            return None;
        }
        let _ = std::fs::create_dir_all(file.parent()?);
        let _ = store::write_atomic(&file, &bytes);
        Some(bytes)
    }

    /// The newest game version, which is the folder icons are served from.
    /// Asked once a day at most.
    fn icon_version(&self, settings: &NetSettings) -> Option<String> {
        let now = self.now();
        let mut known = self.online.icon_version.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((version, at)) = known.as_ref() {
            if now.saturating_sub(*at) < 86_400 {
                return Some(version.clone());
            }
        }
        let list = self.net.get(settings, Purpose::Icons, "https://ddragon.leagueoflegends.com/api/versions.json").ok()?;
        let versions: Vec<String> = serde_json::from_slice(&list).ok()?;
        // Goes into an address, so it must look like a version and nothing else.
        let version = versions.into_iter().next().filter(|v| !v.is_empty() && v.len() <= 24 && v.bytes().all(|b| b.is_ascii_digit() || b == b'.'))?;
        *known = Some((version.clone(), now));
        Some(version)
    }
}

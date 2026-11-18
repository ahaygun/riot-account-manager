//! The local HTTP interfaces Riot Client and the League client expose on
//! 127.0.0.1 while they run. Each writes a `lockfile` with the port and a
//! one-time password; the certificate is self-signed.
//!
//! Behind a trait so the switching logic can be tested without a client.

use crate::store::{self, Ban};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginState {
    /// Riot Client is not running or not answering yet.
    Unknown,
    SignedOut,
    SignedIn {
        /// The "stay signed in" box, when the client says.
        persist: Option<bool>,
    },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Identity {
    pub puuid: Option<String>,
    /// `Name#TAG`
    pub riot_id: Option<String>,
    /// A League suspension still running, if Riot reports one.
    pub ban: Option<Ban>,
}

/// Why a sign-in with a password is not going through, as far as Riot
/// Client says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProblem {
    /// A verification code (2FA) is being asked for.
    Verification,
    /// Username or password refused.
    Rejected,
    /// Too many attempts; Riot wants a pause.
    RateLimited,
    /// The captcha was not passed.
    Captcha,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LeagueInfo {
    pub puuid: Option<String>,
    pub riot_id: Option<String>,
    pub level: Option<u64>,
    /// Solo queue, e.g. "Gold II 45 LP"; `None` when unranked.
    pub rank: Option<String>,
    /// Number of the profile icon in use.
    pub icon: Option<u32>,
    /// The server the account plays on, as one of `opgg::REGIONS`.
    pub region: Option<String>,
}

pub trait ClientApi: Send + Sync {
    fn login(&self) -> LoginState;
    /// Who is signed in to Riot Client.
    fn identity(&self) -> Option<Identity>;
    /// Asks Riot Client to quit properly, which is when it writes its
    /// session to disk. Returns false if it could not be asked.
    fn quit(&self) -> bool;
    /// Starts a game the way the client's Play button does.
    fn start_product(&self, product: &str) -> bool;
    /// Summoner details from the League client, if it is running.
    fn league(&self) -> Option<LeagueInfo>;
    /// Where the League client is in a match's life ("Lobby",
    /// "ChampSelect", "InProgress", ...), if it is running.
    fn league_phase(&self) -> Option<String>;
    /// What stands in the way of a sign-in that was just sent, if the
    /// client says. `None` both for "nothing" and for "does not say".
    fn auth_problem(&self) -> Option<AuthProblem> {
        None
    }
    /// For diagnostics only: the status and JSON of a GET on one of the
    /// two local clients (`league` picks the League client). `None` when
    /// that client is not there to ask.
    fn probe(&self, _league: bool, _path: &str) -> Option<(u16, Value)> {
        None
    }
}

/// Real implementation.
pub struct LocalApi {
    riot_games_dir: PathBuf,
    /// Folder the League client was last found running from. Its lockfile
    /// is tried first, which saves a look at the process table.
    league_dir: Mutex<Option<PathBuf>>,
}

struct Access {
    port: u16,
    password: String,
    scheme: &'static str,
}

fn http() -> &'static reqwest::blocking::Client {
    static CLIENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            // Only ever used against 127.0.0.1, where the client presents a
            // self-signed certificate.
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(4))
            .build()
            .expect("local HTTP client")
    })
}

/// `name:pid:port:password:protocol`
fn parse_lockfile(text: &str) -> Option<Access> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() != 5 {
        return None;
    }
    let port: u16 = parts[2].parse().ok().filter(|p| *p >= 1024)?;
    let password = parts[3].trim();
    let scheme = match parts[4].trim() {
        "https" => "https",
        "http" => "http",
        _ => return None,
    };
    (!password.is_empty()).then(|| Access { port, password: password.to_string(), scheme })
}

fn read_lockfile(path: &Path) -> Option<Access> {
    parse_lockfile(&std::fs::read_to_string(path).ok()?)
}

/// `Ok(Some(json))` on 2xx, `Ok(None)` on any other answer, `Err` when the
/// client could not be reached at all.
fn request(access: &Access, method: reqwest::Method, path: &str) -> Result<Option<Value>, ()> {
    let url = format!("{}://127.0.0.1:{}{}", access.scheme, access.port, path);
    let resp = http().request(method, url).basic_auth("riot", Some(&access.password)).send().map_err(|_| ())?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    Ok(Some(resp.json::<Value>().unwrap_or(Value::Null)))
}

fn get(access: &Access, path: &str) -> Result<Option<Value>, ()> {
    request(access, reqwest::Method::GET, path)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

fn riot_id(name: Option<String>, tag: Option<String>) -> Option<String> {
    match (name, tag) {
        (Some(n), Some(t)) => Some(format!("{n}#{t}")),
        (Some(n), None) => Some(n),
        _ => None,
    }
}

/// Where the running League client is installed.
///
/// Two steps, because asking Windows for an executable's path means
/// opening the process: first a look at names only, which is one system
/// call for the whole table, then the path of the one process that matched.
fn running_league_dir() -> Option<PathBuf> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut sys = System::new().ok()?;
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let pids: Vec<sysinfo::Pid> = sys
        .processes()
        .iter()
        .filter(|(_, p)| p.name().to_string_lossy().eq_ignore_ascii_case("LeagueClientUx.exe"))
        .map(|(pid, _)| *pid)
        .collect();
    if pids.is_empty() {
        return None;
    }
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), false, ProcessRefreshKind::nothing().with_exe(UpdateKind::Always));
    pids.iter().find_map(|pid| sys.process(*pid)?.exe()?.parent().map(PathBuf::from))
}

/// Whether the old status route can add anything to what the session
/// route said: it can when the session route had no answer, and when it
/// reports a sign-in without saying if "stay signed in" is on.
pub(crate) fn needs_legacy_status(rso_session: Option<&Value>) -> bool {
    let Some(kind) = rso_session.and_then(|v| v.get("type")).and_then(Value::as_str) else { return true };
    kind.eq_ignore_ascii_case("authenticated") && rso_session.and_then(|v| v.get("persistLogin")).and_then(Value::as_bool).is_none()
}

/// The League suspension in the signed-in account's claims, if one is
/// still running at `now_ms`. Chat and ranked restrictions are not bans:
/// the account can still play. Of several, the one that ends last counts.
pub(crate) fn league_ban(claims: &Value, now_ms: u64) -> Option<Ban> {
    let mut found: Option<Ban> = None;
    for r in claims.get("ban")?.get("restrictions")?.as_array()? {
        if r.get("scope").and_then(Value::as_str) != Some("lol") {
            continue;
        }
        let until = match r.get("type").and_then(Value::as_str) {
            Some("PERMANENT_BAN") => None,
            Some("TIME_BAN") => match r.get("dat").and_then(|d| d.get("expirationMillis")).and_then(Value::as_u64) {
                Some(until) if until > now_ms => Some(until),
                _ => continue,
            },
            _ => continue,
        };
        found = Some(match (found, until) {
            (Some(Ban { until: None }), _) | (_, None) => Ban { until: None },
            (Some(Ban { until: Some(other) }), Some(until)) => Ban { until: Some(other.max(until)) },
            (None, until) => Ban { until },
        });
    }
    found
}

/// Reads the authentication step Riot Client is at. A request for a code
/// shows as its `type`; a refusal shows as an `error` text, sorted here by
/// the words it carries.
pub(crate) fn auth_problem_from(answer: &Value) -> Option<AuthProblem> {
    let error = answer.get("error").and_then(Value::as_str).unwrap_or_default().to_lowercase();
    if error.contains("captcha") {
        Some(AuthProblem::Captcha)
    } else if error.contains("rate") {
        Some(AuthProblem::RateLimited)
    } else if !error.is_empty() {
        Some(AuthProblem::Rejected)
    } else if answer.get("type").and_then(Value::as_str).is_some_and(|t| t.eq_ignore_ascii_case("multifactor")) {
        Some(AuthProblem::Verification)
    } else {
        None
    }
}

pub(crate) fn login_from(rso_session: Option<&Value>, legacy_status: Option<&Value>) -> LoginState {
    let legacy_persist = legacy_status.and_then(|v| v.get("persist")).and_then(Value::as_bool);
    if let Some(kind) = rso_session.and_then(|v| v.get("type")).and_then(Value::as_str) {
        if !kind.eq_ignore_ascii_case("authenticated") {
            return LoginState::SignedOut;
        }
        let persist = rso_session.and_then(|v| v.get("persistLogin")).and_then(Value::as_bool).or(legacy_persist);
        return LoginState::SignedIn { persist };
    }
    let phase = legacy_status.and_then(|v| v.get("phase")).and_then(Value::as_str).unwrap_or_default();
    if phase.eq_ignore_ascii_case("logged_in") {
        LoginState::SignedIn { persist: legacy_persist }
    } else {
        LoginState::SignedOut
    }
}

pub(crate) fn rank_from(stats: &Value) -> Option<String> {
    let solo = stats.get("queueMap")?.get("RANKED_SOLO_5x5")?;
    let tier = solo.get("tier").and_then(Value::as_str).unwrap_or_default();
    if tier.is_empty() || tier.eq_ignore_ascii_case("NONE") || tier.eq_ignore_ascii_case("UNRANKED") {
        return None;
    }
    let mut tier_name = tier.to_lowercase();
    if let Some(first) = tier_name.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    let division = solo.get("division").and_then(Value::as_str).filter(|d| !d.is_empty() && *d != "NA");
    let lp = solo.get("leaguePoints").and_then(Value::as_u64).unwrap_or(0);
    Some(match division {
        Some(d) => format!("{tier_name} {d} {lp} LP"),
        None => format!("{tier_name} {lp} LP"),
    })
}

impl LocalApi {
    pub fn new(riot_games_dir: PathBuf) -> Self {
        LocalApi { riot_games_dir, league_dir: Mutex::new(None) }
    }

    fn riot(&self) -> Option<Access> {
        read_lockfile(&self.riot_games_dir.join("Riot Client").join("Config").join("lockfile"))
    }

    fn remembered_league_dir(&self) -> Option<PathBuf> {
        self.league_dir.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn remember_league_dir(&self, dir: Option<PathBuf>) {
        *self.league_dir.lock().unwrap_or_else(|e| e.into_inner()) = dir;
    }

    /// The League client writes its lockfile next to its executable. The
    /// running process says where that is, which also covers installs in
    /// places Riot's install record does not list.
    fn find_league(&self) -> Option<Access> {
        let (dir, access) = running_league_dir()
            .into_iter()
            .chain(store::league_install_dirs())
            .find_map(|d| read_lockfile(&d.join("lockfile")).map(|a| (d, a)))?;
        self.remember_league_dir(Some(dir));
        Some(access)
    }

    /// GET on the League client. The folder it was last found in is tried
    /// first; if its lockfile is gone, or is a leftover nobody answers on,
    /// the client is looked for again.
    fn league_get(&self, path: &str) -> Option<Value> {
        if let Some(access) = self.remembered_league_dir().and_then(|d| read_lockfile(&d.join("lockfile"))) {
            if let Ok(answer) = get(&access, path) {
                return answer;
            }
        }
        self.remember_league_dir(None);
        get(&self.find_league()?, path).ok().flatten()
    }
}

impl ClientApi for LocalApi {
    fn login(&self) -> LoginState {
        let Some(access) = self.riot() else { return LoginState::Unknown };
        // The session route answers 404 while the client is still starting
        // and does not exist on old clients; the status route covers both.
        let Ok(rso) = get(&access, "/rso-auth/v1/session") else { return LoginState::Unknown };
        // The second request is skipped when the first already says it all.
        let legacy = if needs_legacy_status(rso.as_ref()) { get(&access, "/riot-login/v1/status").ok().flatten() } else { None };
        login_from(rso.as_ref(), legacy.as_ref())
    }

    fn identity(&self) -> Option<Identity> {
        let access = self.riot()?;
        let userinfo = get(&access, "/riot-client-auth/v1/userinfo").ok().flatten().unwrap_or(Value::Null);
        // Some client versions wrap the claims in a JSON string.
        let claims = match userinfo.get("userInfo").and_then(Value::as_str) {
            Some(inner) => serde_json::from_str(inner).unwrap_or(Value::Null),
            None => userinfo,
        };
        let alias = get(&access, "/player-account/aliases/v1/active").ok().flatten().unwrap_or(Value::Null);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let id = Identity {
            puuid: text(&claims, "sub"),
            riot_id: riot_id(text(&alias, "game_name"), text(&alias, "tag_line")),
            ban: league_ban(&claims, now),
        };
        (id != Identity::default()).then_some(id)
    }

    fn probe(&self, league: bool, path: &str) -> Option<(u16, Value)> {
        let access = if league {
            self.remembered_league_dir().and_then(|d| read_lockfile(&d.join("lockfile"))).or_else(|| self.find_league())?
        } else {
            self.riot()?
        };
        let url = format!("{}://127.0.0.1:{}{}", access.scheme, access.port, path);
        let resp = http().get(url).basic_auth("riot", Some(&access.password)).send().ok()?;
        let status = resp.status().as_u16();
        Some((status, resp.json::<Value>().unwrap_or(Value::Null)))
    }

    fn auth_problem(&self) -> Option<AuthProblem> {
        let answer = get(&self.riot()?, "/rso-authenticator/v1/authentication").ok().flatten()?;
        auth_problem_from(&answer)
    }

    fn quit(&self) -> bool {
        let Some(access) = self.riot() else { return false };
        request(&access, reqwest::Method::POST, "/process-control/v1/process/quit").is_ok()
    }

    fn start_product(&self, product: &str) -> bool {
        let Some(access) = self.riot() else { return false };
        let path = format!("/product-launcher/v1/products/{product}/patchlines/live");
        matches!(request(&access, reqwest::Method::POST, &path), Ok(Some(_)))
    }

    fn league(&self) -> Option<LeagueInfo> {
        let me = self.league_get("/lol-summoner/v1/current-summoner")?;
        let stats = self.league_get("/lol-ranked/v1/current-ranked-stats").unwrap_or(Value::Null);
        Some(LeagueInfo {
            puuid: text(&me, "puuid"),
            riot_id: riot_id(text(&me, "gameName"), text(&me, "tagLine")),
            level: me.get("summonerLevel").and_then(Value::as_u64),
            rank: rank_from(&stats),
            icon: me.get("profileIconId").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
            region: self
                .league_get("/riotclient/region-locale")
                .and_then(|r| text(&r, "region"))
                .and_then(|r| crate::opgg::region_code(&r))
                .map(str::to_string),
        })
    }

    fn league_phase(&self) -> Option<String> {
        let phase = self.league_get("/lol-gameflow/v1/gameflow-phase")?;
        phase.as_str().map(String::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lockfile() {
        let a = parse_lockfile("Riot Client:1234:51234:s3cret:https\n").unwrap();
        assert_eq!((a.port, a.password.as_str(), a.scheme), (51234, "s3cret", "https"));
        assert!(parse_lockfile("garbage").is_none());
        assert!(parse_lockfile("a:1:80:pw:https").is_none());
        assert!(parse_lockfile("a:1:5000:pw:ftp").is_none());
    }

    #[test]
    fn login_states() {
        let authed = json!({"type": "authenticated", "persistLogin": true});
        assert_eq!(login_from(Some(&authed), None), LoginState::SignedIn { persist: Some(true) });
        let needs = json!({"type": "needs_authentication"});
        assert_eq!(login_from(Some(&needs), None), LoginState::SignedOut);
        // Old client: no session route, status route says logged in.
        let legacy = json!({"phase": "logged_in", "persist": false});
        assert_eq!(login_from(None, Some(&legacy)), LoginState::SignedIn { persist: Some(false) });
        // Still starting: neither route has an answer.
        assert_eq!(login_from(None, None), LoginState::SignedOut);
    }

    #[test]
    fn the_old_status_route_is_asked_only_when_it_can_add_something() {
        // Signed out, or signed in with the box reported: one request is enough.
        assert!(!needs_legacy_status(Some(&json!({"type": "needs_authentication"}))));
        assert!(!needs_legacy_status(Some(&json!({"type": "authenticated", "persistLogin": false}))));
        // Signed in without a word about the box, an old client, a starting client.
        assert!(needs_legacy_status(Some(&json!({"type": "authenticated"}))));
        assert!(needs_legacy_status(Some(&json!({}))));
        assert!(needs_legacy_status(None));
    }

    #[test]
    fn league_bans_are_read_and_other_restrictions_are_not() {
        let now = 1_000_000;
        let claims = |list: Value| json!({"sub": "x", "ban": {"restrictions": list}});
        assert_eq!(league_ban(&json!({"sub": "x"}), now), None);
        assert_eq!(league_ban(&claims(json!([])), now), None);
        // Over, another game's, and a chat restriction: none of them a ban.
        let not_bans = json!([
            {"scope": "lol", "type": "TIME_BAN", "dat": {"expirationMillis": 999_999}},
            {"scope": "valorant", "type": "PERMANENT_BAN"},
            {"scope": "lol", "type": "TEXT_CHAT_MUTED", "dat": {"expirationMillis": 9_000_000}}
        ]);
        assert_eq!(league_ban(&claims(not_bans), now), None);
        // The later of two; a permanent one beats both.
        let two = json!([
            {"scope": "lol", "type": "TIME_BAN", "dat": {"expirationMillis": 2_000_000}},
            {"scope": "lol", "type": "TIME_BAN", "dat": {"expirationMillis": 5_000_000}}
        ]);
        assert_eq!(league_ban(&claims(two), now), Some(Ban { until: Some(5_000_000) }));
        let with_permanent = json!([
            {"scope": "lol", "type": "PERMANENT_BAN"},
            {"scope": "lol", "type": "TIME_BAN", "dat": {"expirationMillis": 5_000_000}}
        ]);
        assert_eq!(league_ban(&claims(with_permanent), now), Some(Ban { until: None }));
    }

    #[test]
    fn what_stands_in_the_way_of_a_sign_in() {
        assert_eq!(auth_problem_from(&json!({"type": "auth", "error": ""})), None);
        assert_eq!(auth_problem_from(&json!({})), None);
        assert_eq!(auth_problem_from(&json!({"type": "multifactor"})), Some(AuthProblem::Verification));
        assert_eq!(auth_problem_from(&json!({"type": "auth", "error": "auth_failure"})), Some(AuthProblem::Rejected));
        assert_eq!(auth_problem_from(&json!({"error": "rate_limited"})), Some(AuthProblem::RateLimited));
        assert_eq!(auth_problem_from(&json!({"error": "captcha_not_allowed"})), Some(AuthProblem::Captcha));
    }

    #[test]
    fn ranks() {
        let s = json!({"queueMap": {"RANKED_SOLO_5x5": {"tier": "GOLD", "division": "II", "leaguePoints": 45}}});
        assert_eq!(rank_from(&s).as_deref(), Some("Gold II 45 LP"));
        let m = json!({"queueMap": {"RANKED_SOLO_5x5": {"tier": "MASTER", "division": "NA", "leaguePoints": 312}}});
        assert_eq!(rank_from(&m).as_deref(), Some("Master 312 LP"));
        let u = json!({"queueMap": {"RANKED_SOLO_5x5": {"tier": "", "division": "NA", "leaguePoints": 0}}});
        assert_eq!(rank_from(&u), None);
        assert_eq!(rank_from(&json!({})), None);
    }
}

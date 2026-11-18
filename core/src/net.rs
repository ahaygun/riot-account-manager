//! The gate: the one place this app talks to the internet from.
//!
//! Everything outbound goes through `Gate::get`, and the gate is built so
//! that what the app promises can be checked by reading this file:
//!
//! - **Only named purposes.** A request says what it is for (`Purpose`);
//!   each purpose has a short list of hosts, written out below, and a
//!   request to any other host is refused before a connection is made.
//! - **Only what the user allowed.** One switch turns the internet off
//!   altogether; each purpose has its own switch besides. Nothing is asked
//!   before the user has answered the question at all.
//! - **HTTPS only, and no wandering.** No other scheme, no ports, no
//!   user:password in the address. A redirect is followed only if it stays
//!   on a host the same purpose allows.
//! - **Nothing of ours goes out.** A request is an address and nothing
//!   else: there is no body and no header the caller can set, so stored
//!   passwords, sessions and tokens have no way to travel. The address
//!   carries only what the purpose needs (a Riot ID and a region, which
//!   are public, or an icon number).
//! - **Bounded answers.** A reply larger than the purpose's limit is cut
//!   off and discarded; replies are parsed as text or stored as an image,
//!   never run.
//! - **A ledger.** Every request, allowed or refused, leaves a line: when,
//!   what for, which host, how it ended, how many bytes came back. Never
//!   the full address, so the ledger holds no account names. The app shows
//!   it to the user.
//!
//! The update check is the one connection that does not pass through
//! here (it belongs to the updater); the app layer writes its line into
//! the same ledger and obeys the same master switch.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// What a connection is for. The purpose decides where it may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Looking for a newer version of the app (made by the updater).
    Updates,
    /// An account's public League profile: rank, points, level.
    LolRanks,
    /// A profile icon picture.
    Icons,
}

impl Purpose {
    /// The only hosts this purpose may reach.
    pub fn hosts(self) -> &'static [&'static str] {
        match self {
            Purpose::Updates => &["github.com", "objects.githubusercontent.com", "release-assets.githubusercontent.com"],
            Purpose::LolRanks => &["op.gg", "www.op.gg"],
            Purpose::Icons => &["ddragon.leagueoflegends.com"],
        }
    }

    /// The most a reply may weigh.
    fn limit(self) -> usize {
        match self {
            Purpose::Updates => 0,
            Purpose::LolRanks => 6 * 1024 * 1024,
            Purpose::Icons => 512 * 1024,
        }
    }
}

/// The user's choices about the internet, kept in `config.json`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NetSettings {
    /// The user has answered the question. Until then nothing new
    /// connects; only the update check runs, as it did before there was
    /// a question to ask.
    pub decided: bool,
    /// The master switch. Off: no purpose connects, updates included.
    pub online: bool,
    /// League ranks from OP.GG.
    pub lol_ranks: bool,
    /// Profile icons from Riot's picture server.
    pub icons: bool,
    /// Minutes between automatic rank refreshes of one account.
    pub refresh_minutes: u32,
}

impl Default for NetSettings {
    fn default() -> Self {
        NetSettings { decided: false, online: false, lol_ranks: true, icons: true, refresh_minutes: 30 }
    }
}

impl NetSettings {
    pub fn allows(&self, purpose: Purpose) -> bool {
        match purpose {
            // Older than the question, and with a switch of its own in the
            // settings: only an explicit "off" here stops it.
            Purpose::Updates => !self.decided || self.online,
            Purpose::LolRanks => self.decided && self.online && self.lol_ranks,
            Purpose::Icons => self.decided && self.online && self.icons,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetError {
    /// The user has this turned off (or has not been asked yet).
    Off,
    /// The address is not one this purpose may reach.
    NotAllowed,
    /// The server answered with this status.
    Status(u16),
    /// The reply was larger than the purpose allows.
    TooLarge,
    /// No answer: no connection, a timeout, a broken transfer.
    Unreachable,
}

impl NetError {
    fn word(&self) -> String {
        match self {
            NetError::Off => "off".into(),
            NetError::NotAllowed => "refused".into(),
            NetError::Status(code) => code.to_string(),
            NetError::TooLarge => "too_large".into(),
            NetError::Unreachable => "unreachable".into(),
        }
    }
}

/// One answer from a server, as the transport saw it.
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    /// Where a 3xx points, if it says.
    pub location: Option<String>,
}

/// What actually opens connections. Kept apart so tests can stand in for
/// the internet, and so the rules above are enforced in one place whatever
/// does the fetching. A transport must not follow redirects by itself and
/// must stop reading after `max_bytes`.
pub trait Transport: Send + Sync {
    fn get(&self, url: &str, max_bytes: usize) -> io::Result<Reply>;
}

/// The transport of a gate nobody has given one to: there is no internet.
pub struct NoTransport;

impl Transport for NoTransport {
    fn get(&self, _: &str, _: usize) -> io::Result<Reply> {
        Err(io::Error::new(io::ErrorKind::NotConnected, "no transport"))
    }
}

/// The real thing. It sends the address, a plain statement of who is
/// asking, and nothing else: no cookies are kept between requests.
pub struct HttpTransport {
    client: reqwest::blocking::Client,
}

impl HttpTransport {
    pub fn new(version: &str) -> io::Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(format!("QuickSwitch/{version} (+https://github.com/ahaygun/riot-account-manager)"))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(15))
            .https_only(true)
            .build()
            .map_err(io::Error::other)?;
        Ok(HttpTransport { client })
    }
}

impl Transport for HttpTransport {
    fn get(&self, url: &str, max_bytes: usize) -> io::Result<Reply> {
        use std::io::Read;
        let response = self.client.get(url).send().map_err(io::Error::other)?;
        let status = response.status().as_u16();
        let location = response.headers().get("location").and_then(|v| v.to_str().ok()).map(str::to_string);
        // One byte past the limit is read, so "too large" can be told from "exactly full".
        let mut body = Vec::new();
        response.take(max_bytes as u64 + 1).read_to_end(&mut body)?;
        Ok(Reply { status, body, location })
    }
}

/// One line of the ledger.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Unix seconds.
    pub at: u64,
    pub purpose: Purpose,
    pub host: String,
    /// "ok", an HTTP status, "off", "refused", "too_large" or "unreachable".
    pub outcome: String,
    /// Bytes that came back.
    pub bytes: usize,
}

const LEDGER_KEPT: usize = 300;

pub struct Gate {
    transport: Box<dyn Transport>,
    ledger_file: PathBuf,
    ledger: Mutex<Option<VecDeque<Entry>>>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The host of `url` if it is a plain https address: no user, no port.
fn https_host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains(['@', ':']) {
        return None;
    }
    authority.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-').then_some(authority)
}

impl Gate {
    pub fn new(transport: Box<dyn Transport>, ledger_file: PathBuf) -> Self {
        Gate { transport, ledger_file, ledger: Mutex::new(None) }
    }

    /// Whether `url` is somewhere `purpose` may go. This is the allowlist.
    pub fn permits(purpose: Purpose, url: &str) -> bool {
        https_host(url).is_some_and(|host| purpose.hosts().iter().any(|h| h.eq_ignore_ascii_case(host)))
    }

    /// Fetches `url` for `purpose`, if the user's settings and the rules
    /// in this file allow it. Every call leaves a line in the ledger.
    pub fn get(&self, settings: &NetSettings, purpose: Purpose, url: &str) -> Result<Vec<u8>, NetError> {
        let mut url = url.to_string();
        // A few hops at most, each held to the same rules as the first.
        for _ in 0..4 {
            let host = https_host(&url).unwrap_or("?").to_string();
            if !settings.allows(purpose) {
                return Err(self.note_error(purpose, &host, NetError::Off));
            }
            if !Self::permits(purpose, &url) {
                return Err(self.note_error(purpose, &host, NetError::NotAllowed));
            }
            let limit = purpose.limit();
            let reply = match self.transport.get(&url, limit) {
                Ok(reply) => reply,
                Err(_) => return Err(self.note_error(purpose, &host, NetError::Unreachable)),
            };
            if (300..400).contains(&reply.status) {
                let Some(next) = reply.location else {
                    return Err(self.note_error(purpose, &host, NetError::Status(reply.status)));
                };
                // A path on the same host, or a full address to be checked like any other.
                url = if next.starts_with('/') { format!("https://{host}{next}") } else { next };
                continue;
            }
            if reply.status != 200 {
                return Err(self.note_error(purpose, &host, NetError::Status(reply.status)));
            }
            if reply.body.len() > limit {
                return Err(self.note_error(purpose, &host, NetError::TooLarge));
            }
            self.note(purpose, &host, "ok", reply.body.len());
            return Ok(reply.body);
        }
        Err(self.note_error(purpose, https_host(&url).unwrap_or("?"), NetError::NotAllowed))
    }

    fn note_error(&self, purpose: Purpose, host: &str, error: NetError) -> NetError {
        self.note(purpose, host, &error.word(), 0);
        error
    }

    /// Adds a line to the ledger. Public for the one connection made
    /// elsewhere (the update check), which reports itself here.
    pub fn note(&self, purpose: Purpose, host: &str, outcome: &str, bytes: usize) {
        let entry = Entry { at: now(), purpose, host: host.to_string(), outcome: outcome.to_string(), bytes };
        let mut guard = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        let ledger = guard.get_or_insert_with(|| self.load());
        ledger.push_back(entry);
        while ledger.len() > LEDGER_KEPT {
            ledger.pop_front();
        }
        let lines: Vec<String> = ledger.iter().filter_map(|e| serde_json::to_string(e).ok()).collect();
        let _ = crate::store::write_atomic(&self.ledger_file, (lines.join("\n") + "\n").as_bytes());
    }

    fn load(&self) -> VecDeque<Entry> {
        std::fs::read_to_string(&self.ledger_file)
            .map(|text| text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect())
            .unwrap_or_default()
    }

    /// The ledger, oldest first.
    pub fn ledger(&self) -> Vec<Entry> {
        let mut guard = self.ledger.lock().unwrap_or_else(|e| e.into_inner());
        guard.get_or_insert_with(|| self.load()).iter().cloned().collect()
    }

    pub fn clear_ledger(&self) {
        *self.ledger.lock().unwrap_or_else(|e| e.into_inner()) = Some(VecDeque::new());
        let _ = std::fs::remove_file(&self.ledger_file);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Answers from a script and remembers every address it was asked for.
    struct Scripted {
        asked: Arc<Mutex<Vec<String>>>,
        answer: Box<dyn Fn(&str) -> io::Result<Reply> + Send + Sync>,
    }

    impl Transport for Scripted {
        fn get(&self, url: &str, _: usize) -> io::Result<Reply> {
            self.asked.lock().unwrap().push(url.to_string());
            (self.answer)(url)
        }
    }

    fn open_gate(name: &str, answer: impl Fn(&str) -> io::Result<Reply> + Send + Sync + 'static) -> (Gate, Arc<Mutex<Vec<String>>>) {
        let file = std::env::temp_dir().join(format!("qsw-gate-{name}-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let asked = Arc::new(Mutex::new(Vec::new()));
        (Gate::new(Box::new(Scripted { asked: asked.clone(), answer: Box::new(answer) }), file), asked)
    }

    fn ok(body: &str) -> io::Result<Reply> {
        Ok(Reply { status: 200, body: body.as_bytes().to_vec(), location: None })
    }

    const ON: NetSettings = NetSettings { decided: true, online: true, lol_ranks: true, icons: true, refresh_minutes: 30 };

    #[test]
    fn the_update_check_predates_the_question_and_obeys_the_master_switch() {
        let fresh = NetSettings::default();
        assert!(fresh.allows(Purpose::Updates), "as it was before there was a question");
        assert!(!fresh.allows(Purpose::LolRanks) && !fresh.allows(Purpose::Icons), "nothing new without a yes");

        let declined = NetSettings { decided: true, online: true, lol_ranks: false, icons: false, ..fresh };
        assert!(declined.allows(Purpose::Updates));
        assert!(!declined.allows(Purpose::LolRanks) && !declined.allows(Purpose::Icons));

        let off = NetSettings { decided: true, online: false, ..fresh };
        assert!(!off.allows(Purpose::Updates) && !off.allows(Purpose::LolRanks) && !off.allows(Purpose::Icons), "off is off for everything");
    }

    #[test]
    fn only_plain_https_to_a_listed_host_gets_through() {
        for good in ["https://op.gg/lol/summoners/tr/A-TR1", "https://www.op.gg/x", "https://OP.GG/x"] {
            assert!(Gate::permits(Purpose::LolRanks, good), "{good}");
        }
        for bad in [
            "http://op.gg/x",                  // not https
            "https://op.gg.evil.example/x",    // a longer name that starts the same
            "https://evil.example/op.gg",      // the name in the path
            "https://user:pass@op.gg/x",       // credentials in the address
            "https://op.gg@evil.example/x",    // the listed host as a user name
            "https://op.gg:8443/x",            // another port
            "https://ddragon.leagueoflegends.com/x", // a host of another purpose
            "ftp://op.gg/x",
            "https:///x",
            "",
        ] {
            assert!(!Gate::permits(Purpose::LolRanks, bad), "{bad}");
        }
        assert!(Gate::permits(Purpose::Icons, "https://ddragon.leagueoflegends.com/cdn/1/img/profileicon/6.png"));
        assert!(!Gate::permits(Purpose::Icons, "https://op.gg/x"));
    }

    #[test]
    fn nothing_connects_until_the_user_has_said_yes_and_each_switch_is_obeyed() {
        let (gate, asked) = open_gate("switches", |_| ok("page"));
        let url = "https://op.gg/x";
        let undecided = NetSettings::default();
        assert_eq!(gate.get(&undecided, Purpose::LolRanks, url), Err(NetError::Off), "not asked yet");
        let offline = NetSettings { online: false, ..ON };
        assert_eq!(gate.get(&offline, Purpose::LolRanks, url), Err(NetError::Off), "the master switch");
        let no_ranks = NetSettings { lol_ranks: false, ..ON };
        assert_eq!(gate.get(&no_ranks, Purpose::LolRanks, url), Err(NetError::Off), "its own switch");
        assert!(asked.lock().unwrap().is_empty(), "refused before any connection");
        assert_eq!(gate.get(&ON, Purpose::LolRanks, url).unwrap(), b"page");
        assert_eq!(asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_host_off_the_list_is_refused_without_connecting() {
        let (gate, asked) = open_gate("refused", |_| ok("page"));
        assert_eq!(gate.get(&ON, Purpose::LolRanks, "https://evil.example/x"), Err(NetError::NotAllowed));
        assert!(asked.lock().unwrap().is_empty());
        assert_eq!(gate.ledger().last().unwrap().outcome, "refused");
    }

    #[test]
    fn a_redirect_is_followed_only_within_the_purposes_hosts() {
        let (gate, asked) = open_gate("redirect-in", |url| {
            if url.ends_with("/old") {
                Ok(Reply { status: 301, body: Vec::new(), location: Some("https://www.op.gg/new".into()) })
            } else {
                ok("moved here")
            }
        });
        assert_eq!(gate.get(&ON, Purpose::LolRanks, "https://op.gg/old").unwrap(), b"moved here");
        assert_eq!(*asked.lock().unwrap(), ["https://op.gg/old", "https://www.op.gg/new"]);

        let (gate, asked) = open_gate("redirect-out", |_| Ok(Reply { status: 302, body: Vec::new(), location: Some("https://evil.example/collect".into()) }));
        assert_eq!(gate.get(&ON, Purpose::LolRanks, "https://op.gg/x"), Err(NetError::NotAllowed));
        assert_eq!(*asked.lock().unwrap(), ["https://op.gg/x"], "the other host was never contacted");

        // One that goes in circles ends.
        let (gate, asked) = open_gate("redirect-loop", |_| Ok(Reply { status: 302, body: Vec::new(), location: Some("/again".into()) }));
        assert!(gate.get(&ON, Purpose::LolRanks, "https://op.gg/x").is_err());
        assert!(asked.lock().unwrap().len() <= 4);
    }

    #[test]
    fn failures_and_oversized_replies_come_back_as_what_they_are() {
        let (gate, _) = open_gate("status", |_| Ok(Reply { status: 404, body: b"no".to_vec(), location: None }));
        assert_eq!(gate.get(&ON, Purpose::LolRanks, "https://op.gg/x"), Err(NetError::Status(404)));
        let (gate, _) = open_gate("down", |_| Err(io::Error::other("no route")));
        assert_eq!(gate.get(&ON, Purpose::LolRanks, "https://op.gg/x"), Err(NetError::Unreachable));
        let (gate, _) = open_gate("big", |_| Ok(Reply { status: 200, body: vec![0; 512 * 1024 + 1], location: None }));
        assert_eq!(gate.get(&ON, Purpose::Icons, "https://ddragon.leagueoflegends.com/x.png"), Err(NetError::TooLarge));
    }

    #[test]
    fn every_request_leaves_a_line_and_no_line_holds_the_address() {
        let (gate, _) = open_gate("ledger", |url| if url.contains("missing") { Ok(Reply { status: 404, body: Vec::new(), location: None }) } else { ok("12345") });
        let _ = gate.get(&ON, Purpose::LolRanks, "https://op.gg/lol/summoners/tr/GizliHesap-TR1");
        let _ = gate.get(&ON, Purpose::LolRanks, "https://op.gg/lol/summoners/tr/missing-TR1");
        let _ = gate.get(&NetSettings { online: false, ..ON }, Purpose::Icons, "https://ddragon.leagueoflegends.com/x.png");
        gate.note(Purpose::Updates, "github.com", "ok", 0);
        let ledger = gate.ledger();
        let lines: Vec<(Purpose, &str, &str, usize)> = ledger.iter().map(|e| (e.purpose, e.host.as_str(), e.outcome.as_str(), e.bytes)).collect();
        assert_eq!(
            lines,
            [
                (Purpose::LolRanks, "op.gg", "ok", 5),
                (Purpose::LolRanks, "op.gg", "404", 0),
                (Purpose::Icons, "ddragon.leagueoflegends.com", "off", 0),
                (Purpose::Updates, "github.com", "ok", 0),
            ]
        );
        let on_disk = std::fs::read_to_string(&gate.ledger_file).unwrap();
        assert!(!on_disk.contains("GizliHesap") && !on_disk.contains("summoners"), "{on_disk}");

        // It survives a restart, and can be cleared.
        let again = Gate::new(Box::new(NoTransport), gate.ledger_file.clone());
        assert_eq!(again.ledger().len(), 4);
        again.clear_ledger();
        assert!(again.ledger().is_empty() && !again.ledger_file.exists());
    }
}

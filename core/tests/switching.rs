//! End-to-end tests of the switching logic against a simulated Riot Client:
//! a fake `Riot Games` folder, fake processes, a fake local API and a fake
//! keyboard.

use quickswitch_core::store::{save_json, Config};
use quickswitch_core::{AuthProblem, Ban};
use quickswitch_core::{
    ClientApi, Env, Error, Game, Identity, KdfParams, LeagueInfo, LoginOutcome, LoginState, LoginTyper, Procs, Switcher,
    Timing, UiTarget,
};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SETTINGS: &str = "Riot Client/Data/RiotGamesPrivateSettings.yaml";

#[derive(Default)]
struct Sim {
    running: Vec<String>,
    unkillable: bool,
    launches: Vec<Vec<String>>,
    /// Who the running client has signed in: (puuid, token it will write on quit).
    signed_in: Option<(String, String)>,
    persist: Option<bool>,
    quits: u32,
    started_products: Vec<String>,
    /// Password that the fake login page accepts, and the account behind it.
    accepts: Option<(String, String)>,
    typed: Vec<(String, String)>,
    typer_error: Option<String>,
    /// The login page takes the text but not the first Enter.
    first_enter_lost: bool,
    /// The typed sign-in ticks "stay signed in", as the real one tries to.
    typed_stays_signed_in: bool,
    enters_again: u32,
    league: Option<LeagueInfo>,
    /// The local API says "stay signed in" is off although it is on.
    misreport_persist: bool,
    /// League client gameflow phase, while it runs.
    phase: Option<String>,
    /// What Riot reports about the signed-in account's League ban.
    ban: Option<Ban>,
    /// What Riot says once a sign-in has been typed.
    auth_problem: Option<AuthProblem>,
    /// It was already saying so before anything was typed.
    auth_problem_is_old: bool,
    /// Windows refuses to start the client: it wants administrator rights.
    elevated: bool,
}

/// One object plays the processes, the local API and the keyboard, since
/// they are all faces of the same simulated client.
struct World {
    riot: PathBuf,
    sim: Mutex<Sim>,
}

impl World {
    fn sim(&self) -> std::sync::MutexGuard<'_, Sim> {
        self.sim.lock().unwrap()
    }

    /// The user opens Riot Client and signs in by hand.
    fn sign_in_by_hand(&self, puuid: &str, token: &str, stay_signed_in: bool) {
        let mut s = self.sim();
        s.running = vec!["RiotClientServices.exe".into(), "Riot Client.exe".into()];
        s.signed_in = Some((puuid.into(), token.into()));
        s.persist = Some(stay_signed_in);
    }

    /// Riot renews the session of whoever is signed in.
    fn renew(&self, token: &str) {
        let mut s = self.sim();
        let puuid = s.signed_in.clone().unwrap().0;
        s.signed_in = Some((puuid, token.into()));
    }

    /// What a real client does when started: it loads the session on disk.
    /// Tokens starting with "expired" are rejected.
    fn boot(&self) {
        let token = fs::read_to_string(self.riot.join(SETTINGS)).ok().and_then(|t| {
            t.lines().find_map(|l| l.trim().strip_prefix("refresh_token: ").map(|v| v.trim_matches('"').to_string()))
        });
        let mut s = self.sim();
        s.running = vec!["RiotClientServices.exe".into(), "Riot Client.exe".into()];
        s.persist = Some(true);
        s.signed_in = match token {
            Some(t) if !t.is_empty() && !t.starts_with("expired") => {
                let puuid = t.split('-').next().unwrap().to_string();
                Some((format!("puuid-{puuid}"), t))
            }
            _ => None,
        };
    }

    fn write_session(&self, token: &str) {
        let path = self.riot.join(SETTINGS);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("psl:\n    authorization:\n        riot-client:\n            refresh_token: \"{token}\"\n")).unwrap();
        let cfg = self.riot.join("Riot Client/Config");
        fs::create_dir_all(&cfg).unwrap();
        fs::write(cfg.join("lockfile"), "Riot Client:1:5000:pw:https").unwrap();
    }

    /// The client stops; a proper quit writes the session to disk first.
    fn stop(&self, flush: bool) {
        let token = {
            let mut s = self.sim();
            s.running.clear();
            s.signed_in.take().filter(|_| flush && s.persist != Some(false)).map(|(_, token)| token)
        };
        if let Some(token) = token {
            self.write_session(&token);
        }
    }

    fn live_token(&self) -> String {
        fs::read_to_string(self.riot.join(SETTINGS)).unwrap_or_default()
    }
}

impl Procs for World {
    fn running(&self, names: &[String]) -> Vec<String> {
        self.sim().running.iter().filter(|r| names.contains(r)).cloned().collect()
    }
    fn kill(&self, names: &[String]) {
        if self.sim().unkillable {
            return;
        }
        let client_dies = names.iter().any(|n| n == "RiotClientServices.exe");
        if client_dies {
            self.stop(false); // killed: nothing is written
        }
        self.sim().running.retain(|r| !names.contains(r));
    }
    fn launch(&self, _exe: &Path, args: &[String]) -> io::Result<()> {
        if self.sim().elevated {
            return Err(io::Error::from_raw_os_error(740));
        }
        self.sim().launches.push(args.to_vec());
        self.boot();
        Ok(())
    }
}

impl ClientApi for World {
    fn login(&self) -> LoginState {
        let s = self.sim();
        if s.running.is_empty() {
            LoginState::Unknown
        } else if s.signed_in.is_some() {
            LoginState::SignedIn { persist: if s.misreport_persist { Some(false) } else { s.persist } }
        } else {
            LoginState::SignedOut
        }
    }
    fn identity(&self) -> Option<Identity> {
        let s = self.sim();
        let (puuid, _) = s.signed_in.clone()?;
        Some(Identity { riot_id: Some(format!("{puuid}#TR1")), puuid: Some(puuid), ban: s.ban })
    }
    fn probe(&self, league: bool, path: &str) -> Option<(u16, serde_json::Value)> {
        let s = self.sim();
        let (puuid, token) = s.signed_in.clone()?;
        if league {
            return None;
        }
        // Answers full of things that must not be passed on.
        Some((200, serde_json::json!({ "type": "authenticated", "subject": puuid, "accessToken": token, "path": path, "game_name": "GizliOyuncu" })))
    }
    fn auth_problem(&self) -> Option<AuthProblem> {
        let s = self.sim();
        s.auth_problem.filter(|_| (s.auth_problem_is_old || !s.typed.is_empty()) && s.signed_in.is_none())
    }
    fn quit(&self) -> bool {
        if self.sim().unkillable || self.sim().running.is_empty() {
            return false;
        }
        self.sim().quits += 1;
        self.stop(true);
        true
    }
    fn start_product(&self, product: &str) -> bool {
        self.sim().started_products.push(product.into());
        true
    }
    fn league(&self) -> Option<LeagueInfo> {
        self.sim().league.clone()
    }
    fn league_phase(&self) -> Option<String> {
        self.sim().phase.clone()
    }
}

impl LoginTyper for World {
    fn type_login(&self, username: &str, password: &str, _target: &UiTarget) -> Result<(), String> {
        let mut s = self.sim();
        if let Some(e) = s.typer_error.clone() {
            return Err(e);
        }
        s.typed.push((username.into(), password.into()));
        if !s.first_enter_lost {
            s.submit();
        }
        Ok(())
    }

    fn submit_again(&self, _target: &UiTarget) -> bool {
        let mut s = self.sim();
        s.enters_again += 1;
        s.submit();
        true
    }
}

impl Sim {
    /// Enter on the login page: signs in if the last typed password is right.
    fn submit(&mut self) {
        let Some((_, password)) = self.typed.last().cloned() else { return };
        if let Some((good, puuid)) = self.accepts.clone() {
            if good == password {
                // Named so that `boot` reads the same account back from it.
                let token = format!("{}-typed", puuid.trim_start_matches("puuid-"));
                self.signed_in = Some((puuid, token));
                self.persist = Some(self.typed_stays_signed_in);
            }
        }
    }
}

struct Fixture {
    sw: Switcher,
    w: Arc<World>,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("qsw-test-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let env = Env { riot_games_dir: root.join("riot"), data_dir: root.join("data") };
    fs::create_dir_all(&env.riot_games_dir).unwrap();
    // Point at a fake exe so nothing depends on a real install.
    let cfg = Config { riot_client_exe: Some(root.join("RiotClientServices.exe")), ..Config::default() };
    save_json(&env.data_dir.join("config.json"), &cfg).unwrap();
    let w = Arc::new(World { riot: env.riot_games_dir.clone(), sim: Mutex::new(Sim::default()) });
    let sw = switcher(env, &w);
    Fixture { sw, w, root }
}

/// A Switcher over the simulated client; calling it again on the same
/// folders is an app restart.
fn switcher(env: Env, w: &Arc<World>) -> Switcher {
    let mut sw = Switcher::new(env, w.clone(), w.clone(), w.clone());
    let ms = Duration::from_millis;
    sw.kdf = KdfParams { memory_kib: 64, passes: 1 };
    sw.timing = Timing {
        quit_timeout: ms(300),
        settle: ms(1),
        poll: ms(5),
        boot_wait: ms(200),
        signed_out_grace: ms(30),
        ui_load: ms(5),
        login_wait: ms(150),
        submit_retry: ms(60),
        game_start_wait: ms(30),
        manual_wait: ms(300),
        net_gap: ms(0),
    };
    sw
}

/// Two accounts, A and B, each captured after a hand sign-in.
fn two_accounts(f: &Fixture) {
    f.w.sign_in_by_hand("puuid-A", "A-1", true);
    f.sw.capture_new("Ana Hesap", Game::Lol).unwrap();
    f.sw.new_login().unwrap();
    f.w.sign_in_by_hand("puuid-B", "B-1", true);
    f.sw.capture_new("Smurf", Game::Lol).unwrap();
}

fn never() -> bool {
    false
}

#[test]
fn capture_quits_the_client_properly_so_the_session_reaches_disk() {
    let f = fixture("capture");
    f.w.sign_in_by_hand("puuid-A", "A-1", true);
    let a = f.sw.capture_new("Ana Hesap", Game::Lol).unwrap();
    assert_eq!(a.id, "ana-hesap");
    assert_eq!(a.puuid.as_deref(), Some("puuid-A"));
    assert_eq!(a.riot_id.as_deref(), Some("puuid-A#TR1"));
    assert_eq!(f.w.sim().quits, 1, "asked to quit, not killed");
    assert!(f.w.sim().running.is_empty());
    assert!(f.sw.env.snapshot_file("ana-hesap").is_file());
}

#[test]
fn switches_and_keeps_renewed_sessions() {
    let f = fixture("two");
    two_accounts(&f);

    let r = f.sw.switch("ana-hesap", None).unwrap();
    assert!(r.watch_login);
    assert!(f.w.live_token().contains("A-1"));
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=league_of_legends");
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.sw.view().unwrap().active.as_deref(), Some("ana-hesap"));

    // Riot renews A's session while playing; it only reaches disk on quit.
    f.w.renew("A-2");
    f.sw.switch("smurf", None).unwrap(); // by id
    assert!(f.w.live_token().contains("B-1"));
    f.sw.switch("Ana Hesap", None).unwrap(); // by name, as a desktop shortcut would
    assert!(f.w.live_token().contains("A-2"), "renewed session was kept on the way out");
}

#[test]
fn riot_client_settings_are_never_saved_or_replaced() {
    let f = fixture("settings");
    two_accounts(&f);
    let settings = f.w.riot.join("Riot Client/Config/RiotClientSettings.yaml");
    // The user turns the login music off in Riot Client.
    fs::write(&settings, "volume_settings:\n    videos:\n        muted: true\n").unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    f.sw.switch("smurf", None).unwrap();
    f.sw.new_login().unwrap(); // adding an account used to wipe the folder
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    f.sw.capture_new("Üç", Game::Lol).unwrap();
    f.sw.add_with_credentials("Dört", "u", "p", Game::Lol).unwrap();
    f.sw.switch("d-rt", None).unwrap(); // password-only: starts signed out
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(fs::read_to_string(&settings).unwrap().contains("muted: true"), "the user's choice survived");
    assert!(f.w.riot.join("Riot Client/Config/lockfile").exists(), "the running client's lockfile is left alone");
}

#[test]
fn snapshots_from_older_versions_do_not_bring_back_old_settings() {
    let f = fixture("legacy");
    // An old config.json that still lists the settings folder.
    let old = Config {
        riot_client_exe: Some(f.root.join("RiotClientServices.exe")),
        session_paths: { let mut p = Config::default().session_paths; p.push("Riot Client/Config".into()); p },
        ..Config::default()
    };
    save_json(&f.sw.env.data_dir.join("config.json"), &old).unwrap();
    let settings = f.w.riot.join("Riot Client/Config/RiotClientSettings.yaml");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, "muted: false").unwrap(); // what was there when the account was added
    f.w.sign_in_by_hand("puuid-A", "A-1", true);
    // Simulate an old capture: write the snapshot with the settings folder in it.
    f.w.stop(true);
    let snap = quickswitch_core::snapshot::Snapshot::collect(&f.w.riot, &old.session_paths).unwrap();
    assert!(snap.files.iter().any(|(p, _)| p.starts_with("Riot Client/Config/")));
    f.sw.capture_new("A", Game::Lol).unwrap();
    std::fs::write(f.sw.env.snapshot_file("a"), quickswitch_core::vault::seal_with(None, &snap.encode()).unwrap()).unwrap();

    fs::write(&settings, "muted: true").unwrap(); // the user's current choice
    f.sw.new_login().unwrap();
    f.sw.switch("a", None).unwrap();
    assert_eq!(fs::read_to_string(&settings).unwrap(), "muted: true");
    assert!(f.w.live_token().contains("A-1"), "the session itself still came back");
}

#[test]
fn a_hand_login_to_an_unknown_account_is_not_filed_under_another() {
    let f = fixture("stranger");
    two_accounts(&f);
    f.sw.switch("ana-hesap", None).unwrap();
    // The user signs out and into a third account inside Riot Client.
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    let v = f.sw.view().unwrap();
    assert_eq!(v.active, None);
    assert!(v.unsaved_session);

    f.sw.switch("smurf", None).unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(f.w.live_token().contains("A-1"), "A still holds A's session, not C's");
}

#[test]
fn already_signed_in_only_starts_the_game() {
    let f = fixture("same");
    two_accounts(&f);
    f.sw.switch("smurf", None).unwrap();
    let quits = f.w.sim().quits;
    let launches = f.w.sim().launches.len();
    let r = f.sw.switch("smurf", None).unwrap();
    assert!(!r.watch_login);
    assert_eq!(f.w.sim().quits, quits, "client was not restarted");
    assert_eq!(f.w.sim().launches.len(), launches);
    assert_eq!(f.w.sim().started_products, ["league_of_legends"]);
}

#[test]
fn refuses_during_a_match_and_touches_nothing() {
    let f = fixture("ingame");
    two_accounts(&f);
    f.sw.switch("smurf", None).unwrap();
    f.w.sim().running.push("League of Legends.exe".into());
    assert!(f.sw.view().unwrap().in_game);
    let quits = f.w.sim().quits;
    assert!(matches!(f.sw.switch("ana-hesap", None), Err(Error::InGame(_))));
    assert_eq!(f.w.sim().quits, quits);
    assert!(f.w.live_token().contains("B-1"));
}

#[test]
fn stops_if_the_client_will_not_close() {
    let f = fixture("stuck");
    two_accounts(&f);
    f.sw.switch("smurf", None).unwrap();
    f.w.sim().unkillable = true;
    assert!(matches!(f.sw.switch("ana-hesap", None), Err(Error::StillRunning(_))));
    assert!(f.w.live_token().contains("B-1"));
}

#[test]
fn killed_client_falls_back_to_the_last_saved_session() {
    let f = fixture("kill");
    two_accounts(&f);
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.renew("A-2");
    // The polite quit is refused; the client gets killed and writes nothing.
    f.w.sim().unkillable = false;
    f.w.kill(&["RiotClientServices.exe".to_string()]);
    f.sw.switch("smurf", None).unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(f.w.live_token().contains("A-1"));
}

#[test]
fn expired_session_without_password_is_flagged_then_cleared_by_a_hand_login() {
    let f = fixture("expired");
    two_accounts(&f);
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.renew("expired-A"); // what gets saved will be rejected next time
    f.sw.switch("smurf", None).unwrap();

    f.sw.switch("ana-hesap", None).unwrap();
    let out = f.sw.await_login("ana-hesap", None, &never).unwrap();
    assert!(matches!(out, LoginOutcome::NeedsLogin(_)));
    assert!(f.sw.view().unwrap().accounts[0].account.needs_login);
    assert!(f.w.sim().typed.is_empty(), "nothing is typed without a stored password");

    // Leaving and coming back must not hide the flag.
    f.sw.switch("smurf", None).unwrap();
    assert!(f.sw.view().unwrap().accounts[0].account.needs_login);

    f.sw.switch("ana-hesap", None).unwrap();
    f.w.sign_in_by_hand("puuid-A", "A-9", true);
    assert_eq!(f.sw.await_manual_login("ana-hesap", &never).unwrap(), LoginOutcome::SignedIn);
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);
    f.sw.switch("smurf", None).unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(f.w.live_token().contains("A-9"), "the new session replaced the expired one");
}

#[test]
fn expired_session_with_password_signs_in_by_typing() {
    let f = fixture("typed");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    assert!(f.sw.view().unwrap().accounts[0].has_password);
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.renew("expired-A");
    f.sw.switch("smurf", None).unwrap();

    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed, [("ana_user".to_string(), "s3cret".to_string())]);
    assert!(f.w.sim().started_products.contains(&"league_of_legends".to_string()), "game was nudged to start");
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);
}

#[test]
fn wrong_password_or_lost_focus_ends_in_needs_login() {
    let f = fixture("badpw");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "wrong").unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.renew("expired-A");
    f.sw.switch("smurf", None).unwrap();

    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(matches!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::NeedsLogin(_)));

    f.sw.switch("smurf", None).unwrap();
    f.w.sim().typer_error = Some("pencere yok".into());
    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::NeedsLogin("pencere yok".into()));
}

#[test]
fn password_only_account_starts_signed_out_and_types() {
    let f = fixture("pwonly");
    two_accounts(&f);
    let c = f.sw.add_with_credentials("Üçüncü", "c_user", "pw-c", Game::Valorant).unwrap();
    assert_eq!(c.id, "nc"); // non-ASCII letters drop out of the id
    f.w.sim().accepts = Some(("pw-c".into(), "puuid-C".into()));
    f.sw.switch(&c.id, None).unwrap();
    assert_eq!(f.w.live_token(), "", "previous account's session is not left in place");
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=valorant");
    assert_eq!(f.sw.await_login(&c.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    let v = f.sw.view().unwrap();
    assert_eq!(v.active.as_deref(), Some("nc"));
    assert_eq!(v.accounts[2].account.puuid.as_deref(), Some("puuid-C"));
    assert!(!v.accounts[2].has_session);
}

#[test]
fn a_password_only_account_waits_the_grace_out_before_typing() {
    let mut f = fixture("pw-grace");
    f.sw.timing.signed_out_grace = Duration::from_millis(600);
    f.w.sim().accepts = Some(("hunter2".into(), "puuid-P".into()));
    let acc = f.sw.add_with_credentials("Şifreli", "user", "hunter2", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    assert!(started.elapsed() >= Duration::from_millis(600), "the page gets its time: {:?}", started.elapsed());
    assert_eq!(f.w.sim().enters_again, 0, "a sign-in that goes through is left alone");
}

#[test]
fn an_enter_the_page_did_not_take_is_pressed_once_more() {
    let f = fixture("enter-again");
    {
        let mut s = f.w.sim();
        s.accepts = Some(("hunter2".into(), "puuid-P".into()));
        s.first_enter_lost = true;
    }
    let acc = f.sw.add_with_credentials("Şifreli", "user", "hunter2", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    assert_eq!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    let s = f.w.sim();
    assert_eq!((s.typed.len(), s.enters_again), (1, 1), "typed once, Enter twice");
}

#[test]
fn a_typed_sign_in_that_stays_signed_in_is_typed_only_once() {
    let f = fixture("typed-once");
    two_accounts(&f);
    {
        let mut s = f.w.sim();
        s.accepts = Some(("hunter2".into(), "puuid-P".into()));
        s.typed_stays_signed_in = true;
    }
    let acc = f.sw.add_with_credentials("Şifreli", "user", "hunter2", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    assert_eq!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed.len(), 1);

    // Away and back: the session the typed sign-in left behind is used.
    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    let v = f.sw.view().unwrap();
    assert!(v.accounts.iter().find(|a| a.account.id == acc.id).unwrap().has_session, "kept when the client closed");
    f.sw.switch(&acc.id, None).unwrap();
    assert_eq!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed.len(), 1, "no password the second time");
    assert_eq!(f.w.sim().signed_in.as_ref().unwrap().0, "puuid-P");
}

#[test]
fn a_wrong_password_is_sent_twice_at_most() {
    let f = fixture("enter-wrong");
    f.w.sim().accepts = Some(("the-right-one".into(), "puuid-P".into()));
    let acc = f.sw.add_with_credentials("Şifreli", "user", "wrong", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    assert!(matches!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::NeedsLogin(_)));
    let s = f.w.sim();
    assert_eq!((s.typed.len(), s.enters_again), (1, 1));
}

#[test]
fn an_expired_saved_session_still_gets_the_full_grace() {
    let mut f = fixture("grace-full");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.renew("expired-A");
    f.sw.switch("smurf", None).unwrap();

    f.sw.timing.signed_out_grace = Duration::from_millis(600);
    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap", None).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    assert!(started.elapsed() >= Duration::from_millis(600), "{:?}", started.elapsed());
}

#[test]
fn idle_sync_reads_the_session_files_only_when_something_was_written() {
    let f = fixture("idle-gate");
    two_accounts(&f);
    let hash = |f: &Fixture| f.sw.view().unwrap().accounts[1].account.hash.clone();
    // Smurf is signed in; the user quits the client, which renews the session.
    f.w.boot();
    f.w.renew("B-2");
    f.w.stop(true);
    assert_eq!(f.sw.sync_idle().unwrap().as_deref(), Some("smurf"), "the renewed session is filed");
    let filed = hash(&f);

    // Proof that later rounds do not read the files: content changed behind
    // an unchanged size and time of writing goes unnoticed.
    let path = f.root.join("riot").join(SETTINGS);
    let written = fs::metadata(&path).unwrap().modified().unwrap();
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, text.replace("B-2", "B-9")).unwrap();
    fs::File::options().write(true).open(&path).unwrap().set_modified(written).unwrap();
    for _ in 0..3 {
        assert_eq!(f.sw.sync_idle().unwrap(), None);
    }
    assert_eq!(hash(&f), filed);

    // The client runs again and quits with a newer session: picked up.
    f.w.boot();
    f.w.renew("B-3");
    f.w.stop(true);
    assert_eq!(f.sw.sync_idle().unwrap().as_deref(), Some("smurf"));
    let newer = hash(&f);
    assert_ne!(newer, filed);

    // An edit of the account list alone gets one fresh look, and finds nothing.
    f.sw.rename("smurf", "Smurf 2").unwrap();
    assert_eq!(f.sw.sync_idle().unwrap(), None);
    assert_eq!(hash(&f), newer);
}

#[test]
fn await_login_can_be_cancelled() {
    let f = fixture("cancel");
    two_accounts(&f);
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.sim().signed_in = None;
    assert_eq!(f.sw.await_login("ana-hesap", None, &|| true).unwrap(), LoginOutcome::Unknown);
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);
}

#[test]
fn capture_refuses_sessions_that_would_not_survive() {
    let f = fixture("persist");
    assert!(matches!(f.sw.capture_new("A", Game::Lol), Err(Error::NoSession)));
    assert!(matches!(f.sw.capture_new("  ", Game::Lol), Err(Error::EmptyField(_))));
    // Signed in without "stay signed in": quitting leaves nothing on disk.
    f.w.sign_in_by_hand("puuid-A", "A-1", false);
    assert!(matches!(f.sw.capture_new("A", Game::Lol), Err(Error::NotPersisted)));
    assert!(f.sw.view().unwrap().accounts.is_empty());
}

#[test]
fn a_wrong_stay_signed_in_report_does_not_block_capture() {
    let f = fixture("misreport");
    // The box is ticked and the client will write its session on quit, but
    // its local API claims the box is off.
    f.w.sign_in_by_hand("puuid-A", "A-1", true);
    f.w.sim().misreport_persist = true;
    assert_eq!(f.sw.view().unwrap().client.persist, Some(false), "nothing on disk yet: the report is all we have");
    let a = f.sw.capture_new("Ana", Game::Lol).unwrap();
    assert_eq!(a.puuid.as_deref(), Some("puuid-A"));
    f.sw.new_login().unwrap();
    f.sw.switch("ana", None).unwrap();
    assert!(f.w.live_token().contains("A-1"));
    // Running again with tokens on disk, the view no longer repeats the wrong report.
    f.w.sim().misreport_persist = true;
    assert_eq!(f.sw.view().unwrap().client.persist, Some(true));
}

#[test]
fn capture_rejects_duplicates() {
    let f = fixture("dup");
    two_accounts(&f);
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(matches!(f.sw.capture_new("Yine A", Game::Lol), Err(Error::AlreadySaved(_))));
    f.sw.new_login().unwrap();
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    assert!(matches!(f.sw.capture_new("smurf", Game::Lol), Err(Error::NameTaken(_))));
}

#[test]
fn idle_sync_files_a_session_after_the_user_closes_the_client() {
    let f = fixture("idle");
    two_accounts(&f);
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.renew("A-2");
    assert_eq!(f.sw.sync_idle().unwrap(), None, "never while the client runs");
    f.w.stop(true); // user quits Riot Client
    assert_eq!(f.sw.sync_idle().unwrap().as_deref(), Some("ana-hesap"));
    assert_eq!(f.sw.sync_idle().unwrap(), None);
}

#[test]
fn league_details_land_on_the_right_account() {
    let f = fixture("league");
    two_accounts(&f);
    f.w.sim().league = Some(LeagueInfo {
        puuid: Some("puuid-A".into()),
        riot_id: Some("Ana#TR1".into()),
        level: Some(212),
        rank: Some("Gold II 45 LP".into()),
        ..LeagueInfo::default()
    });
    assert!(f.sw.refresh_league().unwrap());
    assert!(!f.sw.refresh_league().unwrap(), "no change, no write");
    let v = f.sw.view().unwrap();
    assert_eq!(v.accounts[0].account.rank.as_deref(), Some("Gold II 45 LP"));
    assert_eq!(v.accounts[0].account.level, Some(212));
    assert_eq!(v.accounts[1].account.rank, None);
}

#[test]
fn editing_accounts() {
    let f = fixture("edit");
    two_accounts(&f);
    f.sw.rename("ana-hesap", "Main").unwrap();
    assert!(matches!(f.sw.rename("ana-hesap", "SMURF"), Err(Error::NameTaken(_))));
    f.sw.set_game("ana-hesap", Game::Client).unwrap();
    f.sw.set_hotkey("ana-hesap", Some("Ctrl+Alt+Digit1")).unwrap();
    assert!(matches!(f.sw.set_hotkey("smurf", Some("Ctrl+Alt+Digit1")), Err(Error::HotkeyTaken(_))));
    f.sw.set_hotkey("ana-hesap", Some("  ")).unwrap();
    f.sw.set_hotkey("smurf", Some("Ctrl+Alt+Digit1")).unwrap();
    assert!(matches!(f.sw.set_credentials("yok", "u", "p"), Err(Error::UnknownAccount(_))));
    assert!(matches!(f.sw.set_credentials("smurf", "u", ""), Err(Error::EmptyField(_))));
    f.sw.set_credentials("smurf", "u", "p").unwrap();

    let v = f.sw.view().unwrap();
    assert_eq!((v.accounts[0].account.name.as_str(), v.accounts[0].account.game), ("Main", Game::Client));
    assert_eq!(v.accounts[0].account.hotkey, None);
    assert!(v.accounts[1].has_password);

    f.sw.switch("Main", None).unwrap();
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=riot-client");

    f.sw.remove("smurf").unwrap();
    assert!(!f.sw.env.snapshot_file("smurf").exists());
    assert!(!f.sw.env.secret_file("smurf").exists());
    assert!(matches!(f.sw.switch("smurf", None), Err(Error::UnknownAccount(_))));
}

#[test]
fn app_lock_keeps_sessions_and_passwords_closed_until_unlocked() {
    let f = fixture("lock");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    assert!(matches!(f.sw.set_lock(None, Some("short")), Err(Error::WeakPassword)));
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    let sec = f.sw.view().unwrap().security;
    assert!(sec.lock_enabled && sec.unlocked);
    assert_eq!((sec.saved_sessions, sec.saved_passwords), (2, 1));

    // The app restarts: names are listed, nothing else opens.
    let sw = switcher(f.sw.env.clone(), &f.w);
    let v = sw.view().unwrap();
    assert_eq!(v.accounts.len(), 2);
    assert!(v.security.lock_enabled && !v.security.unlocked);
    f.w.boot();
    let quits = f.w.sim().quits;
    assert!(matches!(sw.switch("ana-hesap", None), Err(Error::Locked)));
    assert!(matches!(sw.new_login(), Err(Error::Locked)));
    assert!(matches!(sw.capture_new("X", Game::Lol), Err(Error::Locked)));
    assert!(matches!(sw.set_credentials("smurf", "u", "p"), Err(Error::Locked)));
    assert_eq!(f.w.sim().quits, quits, "locked: the running client was left alone");
    assert_eq!(sw.sync_idle().unwrap(), None);

    assert!(!sw.unlock("wrong horse").unwrap());
    assert!(matches!(sw.switch("ana-hesap", None), Err(Error::Locked)));
    assert!(sw.unlock("correct horse").unwrap());
    sw.switch("ana-hesap", None).unwrap();
    assert!(f.w.live_token().contains("A-1"), "session survived being re-encrypted");

    sw.lock_now();
    assert!(matches!(sw.switch("smurf", None), Err(Error::Locked)));
}

#[test]
fn changing_or_removing_the_lock_needs_the_current_password() {
    let f = fixture("relock");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    f.sw.set_lock(None, Some("first password")).unwrap();
    // Even while unlocked, the current password is asked for.
    assert!(matches!(f.sw.set_lock(None, None), Err(Error::WrongPassword)));
    assert!(matches!(f.sw.set_lock(Some("nope"), Some("second password")), Err(Error::WrongPassword)));
    f.sw.set_lock(Some("first password"), Some("second password")).unwrap();

    let sw = switcher(f.sw.env.clone(), &f.w);
    assert!(!sw.unlock("first password").unwrap());
    assert!(sw.unlock("second password").unwrap());
    sw.switch("smurf", None).unwrap();
    assert!(f.w.live_token().contains("B-1"));

    sw.set_lock(Some("second password"), None).unwrap();
    let sw = switcher(f.sw.env.clone(), &f.w);
    assert!(!sw.view().unwrap().security.lock_enabled);
    sw.switch("ana-hesap", None).unwrap();
    assert!(f.w.live_token().contains("A-1"));
    // The stored password came through both re-encryptions.
    f.w.renew("expired-A");
    sw.switch("smurf", None).unwrap();
    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    sw.switch("ana-hesap", None).unwrap();
    assert_eq!(sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed, [("ana_user".to_string(), "s3cret".to_string())]);
}

#[test]
fn wipe_removes_everything_of_ours_and_nothing_of_riots() {
    let f = fixture("wipe");
    two_accounts(&f);
    f.sw.set_credentials("smurf", "u", "p").unwrap();
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    f.sw.switch("smurf", None).unwrap();
    f.w.stop(true);
    f.sw.wipe_all().unwrap();
    assert!(!f.sw.env.data_dir.exists());
    assert!(f.w.live_token().contains("B-1"), "Riot Client's own session is untouched");
    // Starts over cleanly; config.json is recreated with defaults on demand.
    save_json(&f.sw.env.data_dir.join("config.json"), &Config { riot_client_exe: Some(f.root.join("x.exe")), ..Config::default() }).unwrap();
    let v = f.sw.view().unwrap();
    assert!(v.accounts.is_empty() && !v.security.lock_enabled && v.unsaved_session);
}

#[test]
fn another_game_can_be_asked_for_without_changing_the_accounts_own() {
    let f = fixture("othergame");
    two_accounts(&f);
    // Not signed in as A: full switch, but into VALORANT this once.
    let r = f.sw.switch("ana-hesap", Some(Game::Valorant)).unwrap();
    assert_eq!(r.game, Game::Valorant);
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=valorant");
    assert_eq!(f.sw.view().unwrap().accounts[0].account.game, Game::Lol, "the account's own game is unchanged");

    // Already signed in as A: asking for League just starts it, no restart.
    let quits = f.w.sim().quits;
    let r = f.sw.switch("ana-hesap", Some(Game::Lol)).unwrap();
    assert!(!r.watch_login);
    assert_eq!(f.w.sim().quits, quits);
    assert_eq!(f.w.sim().started_products, ["league_of_legends"]);

    // A typed login nudges the game that was asked for.
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    f.w.renew("expired-A");
    f.sw.switch("smurf", None).unwrap();
    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap", Some(Game::Valorant)).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", Some(Game::Valorant), &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().started_products.last().unwrap(), "valorant");
}

#[test]
fn reordering_accounts() {
    let f = fixture("order");
    two_accounts(&f);
    f.sw.add_with_credentials("Üç", "u", "p", Game::Lol).unwrap();
    let names = |sw: &Switcher| sw.view().unwrap().accounts.iter().map(|a| a.account.id.clone()).collect::<Vec<_>>();
    assert_eq!(names(&f.sw), ["ana-hesap", "smurf", "hesap"]);
    f.sw.reorder(&["hesap".into(), "ana-hesap".into(), "smurf".into()]).unwrap();
    assert_eq!(names(&f.sw), ["hesap", "ana-hesap", "smurf"]);
    // A partial or stale list never drops or duplicates accounts.
    f.sw.reorder(&["smurf".into(), "yok".into()]).unwrap();
    assert_eq!(names(&f.sw), ["smurf", "hesap", "ana-hesap"]);
    // The order survives a restart.
    assert_eq!(names(&switcher(f.sw.env.clone(), &f.w)), ["smurf", "hesap", "ana-hesap"]);
}

#[test]
fn champion_select_and_queue_block_switching_until_they_end() {
    let f = fixture("champselect");
    two_accounts(&f);
    f.sw.switch("smurf", None).unwrap();
    f.w.sim().running.push("LeagueClientUx.exe".into());
    for (phase, blocked) in [("Lobby", false), ("Matchmaking", true), ("ReadyCheck", true), ("ChampSelect", true), ("InProgress", true), ("EndOfGame", false)] {
        f.w.sim().phase = Some(phase.into());
        assert_eq!(f.sw.in_game(), blocked, "{phase}");
    }
    f.w.sim().phase = Some("ChampSelect".into());
    let quits = f.w.sim().quits;
    let err = f.sw.switch("ana-hesap", None).unwrap_err();
    assert!(matches!(err, Error::InMatchmaking(_)) && err.kind() == "in_game", "offered to switch afterwards");
    assert!(matches!(f.sw.new_login(), Err(Error::InMatchmaking(_))));
    assert_eq!(f.w.sim().quits, quits, "the client was not closed");
    assert!(f.w.live_token().contains("B-1"));
    // A phase left over from a closed League client is not trusted.
    f.w.sim().running.retain(|p| p != "LeagueClientUx.exe");
    assert!(!f.sw.in_game());
}

#[test]
fn window_hotkey_cannot_take_an_account_hotkey() {
    let f = fixture("winhotkey");
    two_accounts(&f);
    assert_eq!(f.sw.config().unwrap().window_hotkey, "Ctrl+Shift+Space");
    f.sw.set_hotkey("smurf", Some("Ctrl+Shift+KeyS")).unwrap();
    assert!(matches!(f.sw.set_window_hotkey("Ctrl+Shift+KeyS"), Err(Error::HotkeyTaken(_))));
    f.sw.set_window_hotkey("Ctrl+Shift+KeyQ").unwrap();
    assert_eq!(f.sw.config().unwrap().window_hotkey, "Ctrl+Shift+KeyQ");
}

#[test]
fn an_unreadable_league_client_blocks_until_the_user_insists() {
    let f = fixture("unknownphase");
    two_accounts(&f);
    f.sw.switch("smurf", None).unwrap();
    f.w.sim().running.push("LeagueClientUx.exe".into());
    f.w.sim().phase = None; // running, but its local interface does not answer
    assert!(f.sw.in_game(), "a queued switch waits");
    let quits = f.w.sim().quits;
    let err = f.sw.switch("ana-hesap", None).unwrap_err();
    assert_eq!(err.kind(), "league_unknown");
    assert!(matches!(f.sw.new_login(), Err(Error::LeagueStateUnknown)));
    assert_eq!(f.w.sim().quits, quits, "nothing was closed");
    // The user says "switch anyway".
    f.sw.switch_with("ana-hesap", None, true).unwrap();
    assert!(f.w.live_token().contains("A-1"));
    // "Anyway" never overrides a running match.
    f.w.sim().running.push("League of Legends.exe".into());
    assert!(matches!(f.sw.switch_with("smurf", None, true), Err(Error::InGame(_))));
    // Nor a phase it can read.
    f.w.sim().running.retain(|p| p != "League of Legends.exe");
    f.w.sim().running.push("LeagueClientUx.exe".into());
    f.w.sim().phase = Some("ChampSelect".into());
    assert!(matches!(f.sw.switch_with("smurf", None, true), Err(Error::InMatchmaking(_))));
}

// ---- whose session is it ------------------------------------------------------

#[test]
fn a_hand_login_to_an_unsaved_account_never_overwrites_a_saved_one() {
    let f = fixture("live-stranger");
    two_accounts(&f); // Smurf was put in place last; the client is closed
    let smurf = f.sw.view().unwrap().accounts[1].account.clone();
    // The app sits in the tray. The user opens Riot Client and signs in to
    // an account the app does not know, with "stay signed in".
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    assert!(f.sw.observe_live().unwrap(), "seen: nobody we know");
    assert!(!f.sw.observe_live().unwrap(), "and noted once");
    f.w.stop(true);
    assert_eq!(f.sw.sync_idle().unwrap(), None, "not filed under Smurf");
    let v = f.sw.view().unwrap();
    assert_eq!(v.accounts[1].account.hash, smurf.hash, "Smurf's saved session is untouched");
    assert!(v.unsaved_session && v.active.is_none(), "offered for saving instead");
    // And Smurf still signs in as Smurf.
    f.sw.switch("smurf", None).unwrap();
    assert_eq!(f.w.sim().signed_in.as_ref().unwrap().0, "puuid-B");
}

#[test]
fn a_hand_login_to_another_saved_account_is_filed_under_that_account() {
    let f = fixture("live-known");
    two_accounts(&f); // current: Smurf
    f.sw.new_login().unwrap();
    f.w.sign_in_by_hand("puuid-A", "A-7", true);
    assert!(f.sw.observe_live().unwrap());
    f.w.stop(true);
    assert_eq!(f.sw.sync_idle().unwrap().as_deref(), Some("ana-hesap"));
    assert_eq!(f.sw.view().unwrap().active.as_deref(), Some("ana-hesap"));
    f.sw.switch("smurf", None).unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    assert!(f.w.live_token().contains("A-7"), "the renewed session went to its own account");
}

#[test]
fn a_token_naming_a_saved_account_decides_where_a_closed_session_goes() {
    let f = fixture("hint");
    let ana = "b639ec2f-e2a0-509a-85f2-ffbec2526938";
    f.w.sign_in_by_hand(ana, "A-1", true);
    f.sw.capture_new("Ana Hesap", Game::Lol).unwrap();
    f.sw.new_login().unwrap();
    f.w.sign_in_by_hand("puuid-B", "B-1", true);
    f.sw.capture_new("Smurf", Game::Lol).unwrap(); // current: Smurf, client closed
    let smurf = f.sw.view().unwrap().accounts[1].account.hash.clone();

    // While the app was not running, Ana signed in by hand and quit. The
    // file does not carry the account id as such, but its id token does.
    let jwt = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJiNjM5ZWMyZi1lMmEwLTUwOWEtODVmMi1mZmJlYzI1MjY5MzgiLCJ4IjoxfQ.c2ln";
    let path = f.root.join("riot").join(SETTINGS);
    fs::write(&path, format!("psl:\n    authorization:\n        riot-client:\n            id_token: \"{jwt}\"\n            refresh_token: \"A-2\"\n")).unwrap();
    assert_eq!(f.sw.sync_idle().unwrap().as_deref(), Some("ana-hesap"));
    assert_eq!(f.sw.view().unwrap().accounts[1].account.hash, smurf);
}

#[test]
fn a_snapshot_that_signs_in_as_someone_else_is_thrown_away() {
    let f = fixture("foreign");
    two_accounts(&f);
    // The one case nothing can see: the app was not running while a third
    // account signed in by hand and quit. Its session lands on Smurf.
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    f.w.stop(true);
    assert_eq!(f.sw.sync_idle().unwrap().as_deref(), Some("smurf"));

    // The next switch to Smurf finds out, and does not leave it that way.
    f.sw.switch("ana-hesap", None).unwrap();
    f.sw.switch("smurf", None).unwrap();
    let outcome = f.sw.await_login("smurf", None, &never).unwrap();
    assert!(matches!(&outcome, LoginOutcome::NeedsLogin(why) if why.contains("yanlış kayıt silindi")), "{outcome:?}");
    let v = f.sw.view().unwrap();
    let smurf = &v.accounts[1];
    assert!(smurf.account.needs_login && !smurf.has_session);
    assert_eq!(v.security.saved_sessions, 1, "only Ana's is left");
    assert!(v.active.is_none() && v.unsaved_session, "the stranger is shown as what it is");
    // Smurf's own id is kept, so a later hand sign-in to Smurf repairs it.
    assert_eq!(smurf.account.puuid.as_deref(), Some("puuid-B"));
}

// ---- sessions that go unused ---------------------------------------------------

#[test]
fn a_session_that_went_unused_is_flagged_until_it_is_used_again() {
    let mut f = fixture("stale");
    two_accounts(&f);
    let day = 86_400;
    let t0 = 1_800_000_000u64;
    let clock = Arc::new(std::sync::atomic::AtomicU64::new(t0));
    let c = clock.clone();
    f.sw.clock = Arc::new(move || c.load(std::sync::atomic::Ordering::SeqCst));
    // Both are used today.
    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    f.sw.switch("smurf", None).unwrap();
    assert_eq!(f.sw.await_login("smurf", None, &never).unwrap(), LoginOutcome::SignedIn);

    clock.store(t0 + 6 * day, std::sync::atomic::Ordering::SeqCst);
    let v = f.sw.view().unwrap();
    assert_eq!(v.accounts[0].idle_days, Some(6));
    assert!(!v.accounts[0].stale, "six days: not yet");
    assert_eq!(v.accounts[1].idle_days, None, "signed in right now");

    clock.store(t0 + 9 * day, std::sync::atomic::Ordering::SeqCst);
    let v = f.sw.view().unwrap();
    assert!(v.accounts[0].stale && !v.accounts[1].stale);

    // Using it clears the warning; the other one starts to age.
    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn);
    clock.store(t0 + 10 * day, std::sync::atomic::Ordering::SeqCst);
    let v = f.sw.view().unwrap();
    assert!(!v.accounts[0].stale);
    assert_eq!(v.accounts[1].idle_days, Some(1));

    // Accounts that sign in by password have no session to go stale.
    let p = f.sw.add_with_credentials("Şifreli", "u", "p", Game::Lol).unwrap();
    clock.store(t0 + 90 * day, std::sync::atomic::Ordering::SeqCst);
    let v = f.sw.view().unwrap();
    let pw = v.accounts.iter().find(|a| a.account.id == p.id).unwrap();
    assert!(pw.idle_days.is_none() && !pw.stale);
}

// ---- the lock closing by itself ------------------------------------------------

#[test]
fn the_app_locks_itself_after_going_unused() {
    let f = fixture("autolock");
    two_accounts(&f);
    let min = |m: u64| Duration::from_secs(m * 60);
    assert!(!f.sw.auto_lock_if_idle(min(600)), "no lock, nothing to close");
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    assert_eq!(f.sw.view().unwrap().security.auto_lock_minutes, 15, "on by default");
    assert!(!f.sw.auto_lock_if_idle(min(14)));
    assert!(f.sw.auto_lock_if_idle(min(15)));
    assert!(matches!(f.sw.switch("ana-hesap", None), Err(Error::Locked)));
    assert!(!f.sw.auto_lock_if_idle(min(60)), "already closed");

    assert!(f.sw.unlock("correct horse").unwrap());
    f.sw.set_auto_lock_minutes(0).unwrap();
    assert!(!f.sw.auto_lock_if_idle(min(6000)), "turned off");
    f.sw.set_auto_lock_minutes(5).unwrap();
    assert!(f.sw.auto_lock_if_idle(min(5)));
}

// ---- a change of the lock that was cut short -----------------------------------

/// Every `.bin` under the data folder, by path.
fn sealed_files(f: &Fixture) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for dir in ["snapshots", "secrets"] {
        for e in fs::read_dir(f.root.join("data").join(dir)).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x == "bin") {
                out.push((e.path(), fs::read(e.path()).unwrap()));
            }
        }
    }
    out.sort();
    out
}

/// Leaves the data folder as a cut-short change of the lock would: `old`
/// still under the real names, `new` beside them as `.new`.
fn cut_short(old: &[(PathBuf, Vec<u8>)], new: &[(PathBuf, Vec<u8>)]) {
    for (path, bytes) in old {
        fs::write(path, bytes).unwrap();
    }
    for (path, bytes) in new {
        fs::write(path.with_extension("new"), bytes).unwrap();
    }
}

fn no_leftovers(f: &Fixture) -> bool {
    ["snapshots", "secrets"].iter().all(|dir| {
        fs::read_dir(f.root.join("data").join(dir)).into_iter().flatten().flatten().all(|e| e.path().extension().is_some_and(|x| x == "bin"))
    })
}

#[test]
fn a_lock_change_cut_short_is_finished_or_undone_on_the_next_start() {
    let f = fixture("relock");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    let env = || Env { riot_games_dir: f.root.join("riot"), data_dir: f.root.join("data") };
    let vault = f.root.join("data").join("vault.json");

    let open = sealed_files(&f); // sealed without a master password
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    let locked = sealed_files(&f); // sealed with it
    let vault_file = fs::read(&vault).unwrap();
    assert_eq!(open.len(), 3);

    // 1. Turning the lock on, stopped before the lock itself was written:
    //    the change did not happen, the half-made files go.
    fs::remove_file(&vault).unwrap();
    cut_short(&open, &locked);
    let sw = switcher(env(), &f.w);
    assert!(no_leftovers(&f));
    assert_eq!(sealed_files(&f), open);
    sw.switch("ana-hesap", None).unwrap();

    // 2. Turning it on, stopped after the lock was written but before the
    //    files took their names: finished once the password is given.
    cut_short(&open, &locked);
    fs::write(&vault, &vault_file).unwrap();
    let sw = switcher(env(), &f.w);
    assert!(!no_leftovers(&f), "locked: cannot tell yet, nothing is touched");
    assert!(sw.unlock("correct horse").unwrap());
    assert!(no_leftovers(&f));
    assert_eq!(sealed_files(&f), locked, "everything is under the master password");
    sw.switch("smurf", None).unwrap();

    // 3. Turning it off, stopped before the lock was removed: still on,
    //    and the unprotected copies are removed rather than used.
    cut_short(&locked, &open);
    let sw = switcher(env(), &f.w);
    assert!(sw.unlock("correct horse").unwrap());
    assert!(no_leftovers(&f));
    assert_eq!(sealed_files(&f), locked);

    // 4. Turning it off, stopped after the lock was removed: the files
    //    under the old key could never be opened again; the new ones take over.
    cut_short(&locked, &open);
    fs::remove_file(&vault).unwrap();
    let sw = switcher(env(), &f.w);
    assert!(no_leftovers(&f));
    assert_eq!(sealed_files(&f), open);
    sw.switch("ana-hesap", None).unwrap();
    assert!(sw.view().unwrap().accounts[0].has_password);
}

#[test]
fn an_account_id_in_other_letter_case_is_still_the_same_account() {
    let f = fixture("idcase");
    two_accounts(&f);
    // Ana's id was stored in upper case (as another Riot interface may
    // spell it); the client reports it in lower case.
    let path = f.sw.env.data_dir.join("accounts.bin");
    let vault = quickswitch_core::vault::Vault::new(f.sw.env.data_dir.join("vault.json"));
    let mut state: serde_json::Value = serde_json::from_slice(&vault.open(&fs::read(&path).unwrap()).unwrap()).unwrap();
    state["accounts"][0]["puuid"] = "PUUID-A".into();
    fs::write(&path, quickswitch_core::vault::seal_with(None, &serde_json::to_vec(&state).unwrap()).unwrap()).unwrap();

    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn, "accepted, not treated as someone else");
    let v = f.sw.view().unwrap();
    assert!(v.accounts[0].has_session, "the saved session was not thrown away");
    assert_eq!(v.active.as_deref(), Some("ana-hesap"));
    assert!(!f.sw.observe_live().unwrap() || f.sw.view().unwrap().active.as_deref() == Some("ana-hesap"));
}

// ---- bringing accounts over from League Accounts --------------------------------

#[test]
fn accounts_exported_by_league_accounts_come_in_as_password_accounts() {
    let f = fixture("import");
    two_accounts(&f);
    f.sw.add_with_credentials("Eski", "Already_Here", "pw", Game::Lol).unwrap();
    // The shape League Accounts writes, with fields we do not use left in.
    let file = r#"[
      {"account_id":"user_one","name":"Player One#EUW","riot_id_not_found":false,"region":"EUW1","region_display":"EUW",
       "password":"hunter2","description":"main","skip_profile_update":false,"tier":"Gold","division":"II","lp":"45","level":"212",
       "reached_last_season":"N/A","finished_last_season":"N/A","tft":{},"rank_provenance":null,"tft_provenance":null},
      {"account_id":"  user_two ","name":"","password":"p2","region":"TR1","level":""},
      {"account_id":"already_here","name":"Dup#TR1","password":"x"},
      {"account_id":"USER_ONE","name":"Again#EUW","password":"y"},
      {"account_id":"no_password","name":"Empty#TR1","password":""},
      {"account_id":"","name":"Nobody#TR1","password":"z"},
      {"account_id":"user_three","name":"Smurf","password":"p3"}
    ]"#;
    let done = f.sw.import_league_accounts(file).unwrap();
    assert_eq!((done.added, done.already_here, done.incomplete), (3, 2, 2));

    let v = f.sw.view().unwrap();
    let by_name = |n: &str| v.accounts.iter().find(|a| a.account.name == n).unwrap_or_else(|| panic!("{n}"));
    let one = by_name("Player One#EUW");
    assert!(one.has_password && !one.has_session);
    assert_eq!(one.account.riot_id.as_deref(), Some("Player One#EUW"));
    assert_eq!(one.account.level, Some(212));
    assert!(by_name("user_two").has_password, "no Riot ID: shown by its username");
    assert!(by_name("Smurf (2)").has_password, "a taken name gets a number");
    assert_eq!(v.accounts.len(), 6);

    // The imported password is the one that gets typed.
    f.w.sim().accepts = Some(("hunter2".into(), "puuid-O".into()));
    let id = one.account.id.clone();
    f.sw.switch(&id, None).unwrap();
    assert_eq!(f.sw.await_login(&id, None, &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed.last().unwrap(), &("user_one".to_string(), "hunter2".to_string()));

    // Importing the same file again adds nothing.
    let again = f.sw.import_league_accounts(file).unwrap();
    assert_eq!((again.added, again.already_here), (0, 5));
}

#[test]
fn a_file_that_is_not_an_export_is_refused_and_a_locked_app_imports_nothing() {
    let f = fixture("import-bad");
    for bad in ["", "not json", r#"{"account_id":"a","password":"b"}"#, r#"[{"account_id": 5}]"#] {
        assert!(matches!(f.sw.import_league_accounts(bad), Err(Error::BadImport)), "{bad}");
    }
    assert_eq!(f.sw.import_league_accounts("[]").unwrap().added, 0);
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    f.sw.lock_now();
    assert!(matches!(f.sw.import_league_accounts(r#"[{"account_id":"a","password":"b"}]"#), Err(Error::Locked)));
    assert!(f.sw.unlock("correct horse").unwrap());
    assert!(f.sw.view().unwrap().accounts.is_empty());
}

// ---- what Riot says about a sign-in ---------------------------------------------

#[test]
fn each_reason_a_typed_sign_in_fails_is_told_apart() {
    for (problem, words) in [
        (AuthProblem::Rejected, "kabul etmedi"),
        (AuthProblem::RateLimited, "çok fazla"),
        (AuthProblem::Captcha, "captcha"),
        (AuthProblem::Verification, "doğrulama kodu"),
    ] {
        let mut f = fixture(&format!("auth-{problem:?}"));
        // A long wait, so "at once" is told from "when it ran out" by a wide
        // margin: short sleeps last about 15 ms each on Windows.
        f.sw.timing.login_wait = Duration::from_secs(5);
        {
            let mut s = f.w.sim();
            s.accepts = Some(("right".into(), "puuid-P".into()));
            s.auth_problem = Some(problem);
        }
        let acc = f.sw.add_with_credentials("Şifreli", "user", "wrong", Game::Lol).unwrap();
        f.sw.switch(&acc.id, None).unwrap();
        let started = std::time::Instant::now();
        let outcome = f.sw.await_login(&acc.id, None, &never).unwrap();
        assert!(matches!(&outcome, LoginOutcome::NeedsLogin(why) if why.contains(words)), "{problem:?}: {outcome:?}");
        assert!(started.elapsed() < Duration::from_secs(2), "said at once, not after the wait runs out: {:?}", started.elapsed());
        assert_eq!(f.w.sim().enters_again, 0, "a refused sign-in is not sent again");
        assert!(f.sw.view().unwrap().accounts[0].account.needs_login);
    }
}

#[test]
fn a_refusal_left_over_from_an_earlier_attempt_is_not_held_against_this_one() {
    // The user typed a wrong password by hand a moment ago; Riot still shows
    // that refusal when the stored, correct password is sent.
    let f = fixture("auth-stale");
    {
        let mut s = f.w.sim();
        s.accepts = Some(("right".into(), "puuid-P".into()));
        s.auth_problem = Some(AuthProblem::Rejected);
        s.auth_problem_is_old = true;
        s.first_enter_lost = true; // so the sign-in takes a moment to go through
    }
    let acc = f.sw.add_with_credentials("Şifreli", "user", "right", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    assert_eq!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);

    // If it is still refused after this attempt has had its time, it is said.
    let mut g = fixture("auth-stale-still");
    // Room to spare, so a coarse system timer cannot run the wait out first.
    g.sw.timing.login_wait = Duration::from_secs(5);
    {
        let mut s = g.w.sim();
        s.accepts = Some(("right".into(), "puuid-P".into()));
        s.auth_problem = Some(AuthProblem::Rejected);
        s.auth_problem_is_old = true;
    }
    let acc = g.sw.add_with_credentials("Şifreli", "user", "wrong", Game::Lol).unwrap();
    g.sw.switch(&acc.id, None).unwrap();
    let outcome = g.sw.await_login(&acc.id, None, &never).unwrap();
    assert!(matches!(&outcome, LoginOutcome::NeedsLogin(why) if why.contains("kabul etmedi")), "{outcome:?}");
}

#[test]
fn a_verification_code_entered_by_hand_finishes_the_sign_in() {
    let f = fixture("auth-code");
    {
        let mut s = f.w.sim();
        s.accepts = Some(("right".into(), "puuid-P".into()));
        s.auth_problem = Some(AuthProblem::Verification);
    }
    let acc = f.sw.add_with_credentials("Şifreli", "user", "wrong", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    assert!(matches!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::NeedsLogin(_)));
    // The user types the code in Riot Client.
    f.w.sim().signed_in = Some(("puuid-P".into(), "P-1".into()));
    assert_eq!(f.sw.await_manual_login(&acc.id, &never).unwrap(), LoginOutcome::SignedIn);
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);
}

#[test]
fn a_client_that_wants_administrator_rights_is_named_as_that() {
    let f = fixture("elevated");
    two_accounts(&f);
    f.w.sim().elevated = true;
    let e = f.sw.switch("ana-hesap", None).unwrap_err();
    assert_eq!(e.kind(), "riot_elevated");
    assert!(e.to_string().contains("yönetici"));
}

#[test]
fn a_switch_says_what_it_is_doing() {
    let mut f = fixture("steps");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "u", "p").unwrap();
    let steps = Arc::new(Mutex::new(Vec::new()));
    let sink = steps.clone();
    // Every step names the account it is for; kept as "step@account".
    f.sw.on_step = Arc::new(move |step, id| sink.lock().unwrap().push(format!("{step}@{id}")));
    let seen = || std::mem::take(&mut *steps.lock().unwrap());
    let ana = |list: &[&str]| list.iter().map(|s| format!("{s}@ana-hesap")).collect::<Vec<_>>();
    f.sw.switch("ana-hesap", None).unwrap();
    f.sw.await_login("ana-hesap", None, &never).unwrap();
    assert_eq!(seen(), ana(&["closing", "restoring", "starting", "waiting", "game"]));

    // Asking for the account that is already open only starts the game.
    f.sw.switch("ana-hesap", None).unwrap();
    assert_eq!(seen(), ana(&["game"]));

    // An expired session with a stored password adds the sign-in step.
    f.sw.switch("smurf", None).unwrap();
    f.w.renew("expired-A");
    f.w.sim().accepts = Some(("p".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.stop(false);
    f.w.write_session("expired-A");
    f.w.boot();
    seen();
    f.sw.await_login("ana-hesap", None, &never).unwrap();
    assert_eq!(seen(), ana(&["waiting", "signing_in", "game"]));
}

// ---- bans -----------------------------------------------------------------------

#[test]
fn a_league_ban_is_kept_with_the_account_and_dropped_when_riot_stops_reporting_it() {
    let f = fixture("ban");
    two_accounts(&f);
    let ban = Ban { until: Some(1_900_000_000_000) };
    f.sw.switch("ana-hesap", None).unwrap();
    f.w.sim().ban = Some(ban);
    assert_eq!(f.sw.await_login("ana-hesap", None, &never).unwrap(), LoginOutcome::SignedIn, "a ban does not stop the sign-in");
    let v = f.sw.view().unwrap();
    assert_eq!(v.accounts[0].account.ban, Some(ban));
    assert_eq!(v.accounts[1].account.ban, None, "only on the account it belongs to");

    // Still shown while another account is in use.
    f.w.sim().ban = None;
    f.sw.switch("smurf", None).unwrap();
    f.sw.await_login("smurf", None, &never).unwrap();
    assert_eq!(f.sw.view().unwrap().accounts[0].account.ban, Some(ban));

    // Back on it with the ban lifted.
    f.sw.switch("ana-hesap", None).unwrap();
    f.sw.await_login("ana-hesap", None, &never).unwrap();
    assert_eq!(f.sw.view().unwrap().accounts[0].account.ban, None);
}

// ---- notes ----------------------------------------------------------------------

#[test]
fn a_note_is_kept_tidy_and_short() {
    let f = fixture("note");
    two_accounts(&f);
    f.sw.set_note("smurf", "  duo   için\n ayrılan hesap ").unwrap();
    assert_eq!(f.sw.view().unwrap().accounts[1].account.note, "duo için ayrılan hesap");
    f.sw.set_note("smurf", &"ş".repeat(500)).unwrap();
    assert_eq!(f.sw.view().unwrap().accounts[1].account.note.chars().count(), 200);
    f.sw.set_note("smurf", "").unwrap();
    assert!(f.sw.view().unwrap().accounts[1].account.note.is_empty());
    assert!(matches!(f.sw.set_note("yok", "x"), Err(Error::UnknownAccount(_))));
}

// ---- pasted lists ---------------------------------------------------------------

#[test]
fn pasted_lines_are_previewed_then_added() {
    let f = fixture("bulk");
    f.sw.add_with_credentials("Eski", "old_user", "pw", Game::Lol).unwrap();
    let lines = "
        user1--Player One#EUW--pass--with--dashes
        user2--plainpass

        OLD_USER--x
        user1--again
        just-a-name
        --nopassuser
        user3--Name#TR1--
    ";
    let preview = f.sw.bulk_add(lines, true).unwrap();
    assert_eq!((preview.added, preview.already_here, preview.incomplete), (2, 2, 3));
    assert_eq!(f.sw.view().unwrap().accounts.len(), 1, "a preview adds nothing");

    let done = f.sw.bulk_add(lines, false).unwrap();
    assert_eq!(done, preview, "the preview told the truth");
    let v = f.sw.view().unwrap();
    assert_eq!(v.accounts.iter().map(|a| a.account.name.as_str()).collect::<Vec<_>>(), ["Eski", "Player One#EUW", "user2"]);

    // The password is everything after the last separator that belongs to it.
    f.w.sim().accepts = Some(("pass--with--dashes".into(), "puuid-O".into()));
    let id = v.accounts[1].account.id.clone();
    f.sw.switch(&id, None).unwrap();
    assert_eq!(f.sw.await_login(&id, None, &never).unwrap(), LoginOutcome::SignedIn);
}

// ---- backups --------------------------------------------------------------------

#[test]
fn an_encrypted_backup_brings_everything_but_the_sessions_to_another_computer() {
    let a = fixture("backup-from");
    two_accounts(&a);
    a.sw.set_credentials("ana-hesap", "ana_user", "s3cret-pw").unwrap();
    a.sw.set_note("ana-hesap", "ana hesabım").unwrap();
    a.sw.set_hotkey("smurf", Some("Ctrl+Shift+Digit2")).unwrap();
    a.sw.add_with_credentials("Şifreli", "pw_user", "hunter2-pw", Game::Valorant).unwrap();

    assert!(matches!(a.sw.export_backup(None, "short"), Err(Error::WeakBackupPassword)));
    let file = a.sw.export_backup(None, "yedek parolası").unwrap();
    for secret in ["s3cret-pw", "hunter2-pw", "ana_user", "pw_user", "Ana Hesap", "puuid-A"] {
        assert!(!file.contains(secret), "{secret} is readable in the backup");
    }

    let b = fixture("backup-to");
    assert!(matches!(b.sw.import_backup("not a backup", "x", false), Err(Error::BadBackup)));
    assert!(matches!(b.sw.import_backup(&file, "yanlış parola", false), Err(Error::BackupNotOpened)));
    let mut damaged = file.clone();
    let at = damaged.rfind("\"data\":\"").unwrap() + 40;
    damaged.replace_range(at..at + 1, if &damaged[at..at + 1] == "0" { "1" } else { "0" });
    assert!(matches!(b.sw.import_backup(&damaged, "yedek parolası", false), Err(Error::BackupNotOpened)));

    let preview = b.sw.import_backup(&file, "yedek parolası", true).unwrap();
    assert_eq!((preview.added, preview.already_here), (3, 0));
    assert!(b.sw.view().unwrap().accounts.is_empty());
    assert_eq!(b.sw.import_backup(&file, "yedek parolası", false).unwrap(), preview);

    let v = b.sw.view().unwrap();
    let get = |name: &str| v.accounts.iter().find(|x| x.account.name == name).unwrap();
    let ana = get("Ana Hesap");
    assert!(ana.has_password && !ana.has_session, "the password came, the session did not");
    assert_eq!(ana.account.note, "ana hesabım");
    assert_eq!(ana.account.puuid.as_deref(), Some("puuid-A"));
    let smurf = get("Smurf");
    assert!(!smurf.has_password && !smurf.has_session && smurf.account.needs_login, "nothing to sign in with yet");
    assert_eq!(smurf.account.hotkey.as_deref(), Some("Ctrl+Shift+Digit2"));
    assert_eq!(get("Şifreli").account.game, Game::Valorant);

    // The restored password works, and a second restore adds nothing.
    b.w.sim().accepts = Some(("s3cret-pw".into(), "puuid-A".into()));
    let id = ana.account.id.clone();
    b.sw.switch(&id, None).unwrap();
    assert_eq!(b.sw.await_login(&id, None, &never).unwrap(), LoginOutcome::SignedIn);
    let again = b.sw.import_backup(&file, "yedek parolası", false).unwrap();
    assert_eq!((again.added, again.already_here), (0, 3));
}

#[test]
fn a_plain_export_is_what_league_accounts_reads_and_what_we_read_back() {
    let a = fixture("plain-from");
    two_accounts(&a); // session-only accounts: nothing to export for them
    assert!(matches!(a.sw.export_plain(None, "tr"), Err(Error::NothingToExport)));
    a.sw.import_league_accounts(r#"[{"account_id":"user_one","name":"One#EUW","region":"EUW","password":"p1","description":"main","level":"30"}]"#).unwrap();
    a.sw.add_with_credentials("İkinci", "user_two", "p2", Game::Lol).unwrap();

    let file = a.sw.export_plain(None, "TR").unwrap();
    let entries: Vec<serde_json::Value> = serde_json::from_str(&file).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["account_id"], "user_one");
    assert_eq!(entries[0]["password"], "p1");
    assert_eq!(entries[0]["name"], "One#EUW");
    assert_eq!(entries[0]["region"], "euw", "the region it came with");
    assert_eq!(entries[0]["description"], "main");
    assert_eq!(entries[1]["region"], "tr", "the one chosen for accounts without");

    let b = fixture("plain-to");
    assert_eq!(b.sw.import_league_accounts(&file).unwrap().added, 2);
}

#[test]
fn with_the_app_lock_on_passwords_leave_only_with_the_master_password() {
    let f = fixture("leave");
    let acc = f.sw.add_with_credentials("Şifreli", "user", "hunter2-pw", Game::Lol).unwrap();
    assert_eq!(f.sw.password_for_clipboard(&acc.id, None).unwrap().as_str(), "hunter2-pw", "no lock: the confirmation is the caller's");
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    // Unlocked and in use, yet each way out asks again.
    for wrong in [None, Some(""), Some("not it")] {
        assert!(matches!(f.sw.password_for_clipboard(&acc.id, wrong), Err(Error::WrongPassword)));
        assert!(matches!(f.sw.export_plain(wrong, "tr"), Err(Error::WrongPassword)));
        assert!(matches!(f.sw.export_backup(wrong, "yedek parolası"), Err(Error::WrongPassword)));
    }
    assert_eq!(f.sw.password_for_clipboard(&acc.id, Some("correct horse")).unwrap().as_str(), "hunter2-pw");
    assert!(f.sw.export_plain(Some("correct horse"), "tr").is_ok());
    assert!(f.sw.export_backup(Some("correct horse"), "yedek parolası").is_ok());
    f.sw.lock_now();
    assert!(matches!(f.sw.password_for_clipboard(&acc.id, Some("correct horse")), Err(Error::Locked)));
    let two = fixture("leave-none");
    two_accounts(&two);
    assert!(matches!(two.sw.password_for_clipboard("smurf", None), Err(Error::EmptyField(_))), "no password stored");
}

// ---- the log --------------------------------------------------------------------

#[test]
fn nothing_that_identifies_an_account_reaches_the_log() {
    let f = fixture("log-privacy");
    // Every value that must stay out, made easy to find.
    let (name, user, pass, puuid, token) = ("GizliHesapAdı", "gizli_kullanici", "Gizli-Sifre-123", "puuid-G", "G-gizli-token");
    f.w.sign_in_by_hand(puuid, token, true);
    f.sw.capture_new(name, Game::Lol).unwrap();
    let id = f.sw.view().unwrap().accounts[0].account.id.clone();
    f.sw.set_credentials(&id, user, pass).unwrap();
    f.sw.set_note(&id, "GizliNot").unwrap();
    f.sw.bulk_add("gizli_toplu--GizliRiot#TR1--Gizli-Toplu-Sifre", false).unwrap();
    f.sw.import_league_accounts(r#"[{"account_id":"gizli_ice","name":"Gizliİçe#TR1","password":"Gizli-Ice-Sifre"}]"#).unwrap();
    let backup = f.sw.export_backup(None, "Gizli-Yedek-Parolasi").unwrap();
    f.sw.import_backup(&backup, "Gizli-Yedek-Parolasi", false).unwrap();
    f.sw.export_plain(None, "tr").unwrap();
    f.sw.password_for_clipboard(&id, None).unwrap();

    // A session that works, one that expired and is typed, a refusal, a lock.
    f.sw.new_login().unwrap();
    f.sw.switch(&id, None).unwrap();
    f.sw.await_login(&id, None, &never).unwrap();
    f.w.stop(false);
    f.w.write_session("expired-G");
    f.w.boot();
    f.w.sim().accepts = Some((pass.into(), puuid.into()));
    f.sw.await_login(&id, None, &never).unwrap();
    f.sw.new_login().unwrap();
    f.w.sim().auth_problem = Some(AuthProblem::Rejected);
    f.w.sim().accepts = Some(("başka".into(), puuid.into()));
    f.sw.switch(&id, None).unwrap();
    f.sw.await_login(&id, None, &never).unwrap();
    f.sw.set_lock(None, Some("Gizli-Ana-Sifre")).unwrap();
    f.sw.auto_lock_if_idle(Duration::from_secs(86_400));
    f.sw.unlock("Gizli-Ana-Sifre").unwrap();

    let log = fs::read_to_string(f.root.join("data").join("quickswitch.log")).unwrap();
    assert!(log.lines().count() > 10, "the flows above do write to the log");
    let lower = log.to_lowercase();
    for secret in ["gizli", "puuid-g", "expired-g", &id] {
        assert!(!lower.contains(&secret.to_lowercase()), "the log holds {secret:?}:\n{log}");
    }
}

#[test]
fn the_end_of_the_log_can_be_read_back_whole_lines_only() {
    let f = fixture("log-tail");
    assert_eq!(f.sw.log_tail(1000), "", "no log yet");
    for i in 0..50 {
        f.sw.log(&format!("line number {i}"));
    }
    let all = f.sw.log_tail(1_000_000);
    assert_eq!(all.lines().count(), 50);
    let tail = f.sw.log_tail(200);
    assert!(tail.len() <= 200 && tail.ends_with("line number 49\n"));
    assert!(tail.lines().all(|l| l.split(' ').next().unwrap().parse::<u64>().is_ok()), "every line starts with its time: {tail}");
}

// ---- the internet -----------------------------------------------------------------

use quickswitch_core::net::{Gate, NetSettings, Reply, Transport};
use std::collections::HashMap;

/// Stands in for the internet: answers from a table of addresses and
/// remembers everything it was asked for.
#[derive(Default)]
struct Web {
    pages: Mutex<HashMap<String, (u16, Vec<u8>)>>,
    asked: Mutex<Vec<String>>,
}

struct WebTransport(Arc<Web>);

impl Transport for WebTransport {
    fn get(&self, url: &str, _max: usize) -> io::Result<Reply> {
        self.0.asked.lock().unwrap().push(url.to_string());
        let (status, body) = self.0.pages.lock().unwrap().get(url).cloned().unwrap_or((404, Vec::new()));
        Ok(Reply { status, body, location: None })
    }
}

impl Web {
    fn put(&self, url: &str, status: u16, body: impl Into<Vec<u8>>) {
        self.pages.lock().unwrap().insert(url.to_string(), (status, body.into()));
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

/// A profile page in the layout `opgg.rs` reads.
fn profile_page(description: &str, icon: u32, updated: &str) -> String {
    format!(
        r#"<html><head><meta name="description" content="{description}"/></head><body><img src="https://s.example/profile_icons/profileIcon{icon}.jpg"/><span>30</span><script>{{"initUpdatedAt": "{updated}"}}</script></body></html>"#
    )
}

const ANA_PAGE: &str = "https://op.gg/lol/summoners/tr/puuid-A-TR1";
const SMURF_PAGE: &str = "https://op.gg/lol/summoners/tr/puuid-B-TR1";
const ONLINE: NetSettings = NetSettings { decided: true, online: true, lol_ranks: true, icons: true, refresh_minutes: 30 };

/// Two accounts, a pretend internet, a clock that can be moved, and the
/// user's yes to going online.
fn online_fixture(name: &str) -> (Fixture, Arc<Web>, Arc<std::sync::atomic::AtomicU64>) {
    let mut f = fixture(name);
    let web = Arc::new(Web::default());
    f.sw.net = Gate::new(Box::new(WebTransport(web.clone())), f.root.join("data").join("network.log"));
    f.sw.timing.net_gap = Duration::ZERO;
    let clock = Arc::new(std::sync::atomic::AtomicU64::new(1_800_000_000));
    let c = clock.clone();
    f.sw.clock = Arc::new(move || c.load(std::sync::atomic::Ordering::SeqCst));
    two_accounts(&f);
    f.sw.set_net_settings(ONLINE).unwrap();
    (f, web, clock)
}

#[test]
fn nothing_is_fetched_until_the_user_has_agreed() {
    let (f, web, _) = online_fixture("net-consent");
    // Back to a fresh install: the question has not been answered.
    let mut cfg = f.sw.config().unwrap();
    cfg.net = NetSettings::default();
    save_json(&f.root.join("data").join("config.json"), &cfg).unwrap();
    assert!(!f.sw.view().unwrap().net.decided);
    assert!(f.sw.refresh_ranks(None, true).unwrap().paused);
    assert!(f.sw.icon_png(6).is_none());
    assert!(web.asked().is_empty(), "not one request");

    // Saying no is also an answer, and it is kept.
    f.sw.set_net_settings(NetSettings { online: false, ..NetSettings::default() }).unwrap();
    assert!(f.sw.view().unwrap().net.decided);
    assert!(f.sw.refresh_ranks(None, true).unwrap().paused);
    assert!(web.asked().is_empty());
}

#[test]
fn ranks_come_from_the_public_profile_and_only_the_riot_id_goes_out() {
    let (f, web, _) = online_fixture("net-ranks");
    f.sw.set_credentials("ana-hesap", "gizli_kullanici", "Gizli-Sifre-1").unwrap();
    web.put(ANA_PAGE, 200, profile_page("puuid-A#TR1 / Gold 2 45 LP / 120Win 98Lose Win rate 55%", 4567, "2027-01-15T07:00:00Z"));
    web.put(SMURF_PAGE, 200, profile_page("puuid-B#TR1 / Lv. 30", 12, "2027-01-15T07:00:00Z"));

    let done = f.sw.refresh_ranks(None, true).unwrap();
    assert_eq!((done.changed, done.not_found, done.failed, done.paused), (2, 0, 0, false));
    let v = f.sw.view().unwrap();
    let ana = &v.accounts[0].account;
    assert_eq!(ana.rank.as_deref(), Some("Gold II 45 LP"));
    assert_eq!((ana.wins, ana.losses, ana.icon, ana.level), (Some(120), Some(98), Some(4567), Some(30)));
    assert_eq!(ana.rank_from.as_deref(), Some("opgg"));
    assert_eq!(v.accounts[1].account.rank, None, "unranked is an answer too");
    assert_eq!(v.accounts[1].account.icon, Some(12));

    // What left the computer: two addresses, each a public Riot ID and a region.
    assert_eq!(web.asked(), [ANA_PAGE, SMURF_PAGE]);
    for sent in web.asked() {
        assert!(!sent.contains("gizli") && !sent.contains("Gizli") && !sent.contains("Ana"), "{sent}");
    }
    // And what was written down about it holds no account at all.
    let ledger = fs::read_to_string(f.root.join("data").join("network.log")).unwrap();
    assert_eq!(ledger.lines().count(), 2);
    assert!(!ledger.contains("puuid") && !ledger.contains("TR1"), "{ledger}");

    // Asked again, nothing is new.
    let again = f.sw.refresh_ranks(None, true).unwrap();
    assert_eq!((again.changed, again.same), (0, 2));
}

#[test]
fn the_clients_own_word_stands_until_the_page_is_about_a_later_time() {
    let (f, web, clock) = online_fixture("net-fresher");
    // Ana is signed in and the League client says Platinum, right now.
    f.sw.switch("ana-hesap", None).unwrap();
    f.sw.await_login("ana-hesap", None, &never).unwrap();
    f.w.sim().league = Some(LeagueInfo { puuid: Some("puuid-A".into()), rank: Some("Platinum IV 10 LP".into()), level: Some(99), icon: Some(7), region: Some("euw".into()), ..LeagueInfo::default() });
    assert!(f.sw.refresh_league().unwrap());
    let ana = f.sw.view().unwrap().accounts[0].account.clone();
    assert_eq!((ana.rank_from.as_deref(), ana.icon, ana.region.as_deref()), (Some("client"), Some(7), Some("euw")));

    // The page is from an hour before that: only what the client does not say is taken.
    let euw = "https://op.gg/lol/summoners/euw/puuid-A-TR1";
    web.put(euw, 200, profile_page("puuid-A#TR1 / Gold 1 90 LP / 10Win 2Lose Win rate 83%", 1, "2027-01-15T07:00:00Z"));
    f.sw.refresh_ranks(Some("ana-hesap"), false).unwrap();
    let ana = f.sw.view().unwrap().accounts[0].account.clone();
    assert_eq!(ana.rank.as_deref(), Some("Platinum IV 10 LP"), "looked up under the region the client reported");
    assert_eq!((ana.wins, ana.icon, ana.level), (Some(10), Some(7), Some(99)));

    // Days later the page is about a later time than the client's word.
    clock.fetch_add(3 * 86_400, std::sync::atomic::Ordering::SeqCst);
    web.put(euw, 200, profile_page("puuid-A#TR1 / Emerald 4 0 LP / 30Win 12Lose Win rate 71%", 1, "2027-01-17T00:00:00Z"));
    f.sw.refresh_ranks(Some("ana-hesap"), false).unwrap();
    let ana = f.sw.view().unwrap().accounts[0].account.clone();
    assert_eq!((ana.rank.as_deref(), ana.rank_from.as_deref()), (Some("Emerald IV 0 LP"), Some("opgg")));
}

#[test]
fn a_riot_id_that_does_not_exist_is_said_and_a_refusal_means_staying_away() {
    let (f, web, clock) = online_fixture("net-problems");
    web.put(SMURF_PAGE, 200, profile_page("puuid-B#TR1 / Silver 4 29 LP / 3Win 4Lose Win rate 43%", 5, "x"));
    // Ana's page does not exist.
    let done = f.sw.refresh_ranks(None, true).unwrap();
    assert_eq!((done.not_found, done.changed), (1, 1));
    assert_eq!(f.sw.view().unwrap().accounts[0].account.lookup_problem.as_deref(), Some("not_found"));

    // The site says "too many requests": the round ends and nothing more is asked for a while.
    web.put(ANA_PAGE, 429, "");
    let before = web.asked().len();
    let done = f.sw.refresh_ranks(None, true).unwrap();
    assert!(done.paused);
    assert_eq!(web.asked().len(), before + 1, "stopped at the refusal, Smurf was not asked for");
    assert!(f.sw.refresh_ranks(None, true).unwrap().paused);
    assert_eq!(web.asked().len(), before + 1, "and stays away");
    clock.fetch_add(16 * 60, std::sync::atomic::Ordering::SeqCst);
    web.put(ANA_PAGE, 200, profile_page("puuid-A#TR1 / Lv. 12", 5, "x"));
    assert!(!f.sw.refresh_ranks(None, true).unwrap().paused, "until the pause is over");
    assert_eq!(f.sw.view().unwrap().accounts[0].account.lookup_problem, None);

    // A page that is not a profile is a failure, not an answer.
    web.put(ANA_PAGE, 200, "<html>maintenance</html>");
    web.put(SMURF_PAGE, 200, "<html>maintenance</html>");
    let done = f.sw.refresh_ranks(None, true).unwrap();
    assert_eq!((done.failed, done.paused), (2, true), "two unreadable answers in a row: the site changed");
    assert_eq!(f.sw.view().unwrap().accounts[1].account.rank.as_deref(), Some("Silver IV 29 LP"), "what was known is kept");
}

#[test]
fn accounts_are_looked_up_when_due_and_only_those_that_can_be() {
    let (f, web, clock) = online_fixture("net-due");
    web.put(ANA_PAGE, 200, profile_page("puuid-A#TR1 / Lv. 1", 1, "x"));
    web.put(SMURF_PAGE, 200, profile_page("puuid-B#TR1 / Lv. 1", 1, "x"));
    // One that only plays VALORANT, one with nothing to look it up by.
    f.sw.set_game("smurf", Game::Valorant).unwrap();
    f.sw.add_with_credentials("Şifreli", "user", "pw", Game::Lol).unwrap();

    let done = f.sw.refresh_ranks(None, false).unwrap();
    assert_eq!(web.asked(), [ANA_PAGE], "League accounts with a Riot ID");
    assert_eq!(done.skipped, 1, "the one without a Riot ID is counted, not asked for");
    f.sw.refresh_ranks(None, false).unwrap();
    assert_eq!(web.asked().len(), 1, "not again within the half hour");
    clock.fetch_add(31 * 60, std::sync::atomic::Ordering::SeqCst);
    f.sw.refresh_ranks(None, false).unwrap();
    assert_eq!(web.asked().len(), 2, "again once it is due");

    // What to look an account up by can be given by hand, and is checked.
    let id = f.sw.view().unwrap().accounts[2].account.id.clone();
    assert!(matches!(f.sw.set_lookup(&id, "no tag", "tr"), Err(Error::BadRiotId)));
    assert!(matches!(f.sw.set_lookup(&id, "Yeni#TR1", "moon"), Err(Error::BadRegion)));
    f.sw.set_lookup(&id, " Yeni#TR1 ", "EUW").unwrap();
    web.put("https://op.gg/lol/summoners/euw/Yeni-TR1", 200, profile_page("Yeni#TR1 / Iron 4 0 LP / 0Win 5Lose Win rate 0%", 9, "x"));
    assert_eq!(f.sw.refresh_ranks(Some(&id), false).unwrap().changed, 1);
    assert_eq!(f.sw.view().unwrap().accounts[2].account.rank.as_deref(), Some("Iron IV 0 LP"));
}

#[test]
fn an_icon_picture_is_fetched_once_and_only_a_picture_is_kept() {
    let (f, web, _) = online_fixture("net-icons");
    let png: Vec<u8> = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3].to_vec();
    web.put("https://ddragon.leagueoflegends.com/api/versions.json", 200, r#"["16.22.1","16.21.1"]"#);
    web.put("https://ddragon.leagueoflegends.com/cdn/16.22.1/img/profileicon/4567.png", 200, png.clone());
    web.put("https://ddragon.leagueoflegends.com/cdn/16.22.1/img/profileicon/13.png", 200, "<html>not a picture</html>");

    assert_eq!(f.sw.icon_png(4567), Some(png.clone()));
    assert_eq!(f.sw.icon_png(4567), Some(png), "the second time from the folder");
    assert_eq!(web.asked().len(), 2, "the version once, the picture once");
    assert_eq!(f.sw.icon_png(13), None, "not a PNG: thrown away");
    assert!(!f.root.join("data").join("icons").join("13.png").exists());

    // With pictures turned off, what is already here still shows; nothing new is fetched.
    f.sw.set_net_settings(NetSettings { icons: false, ..ONLINE }).unwrap();
    let before = web.asked().len();
    assert!(f.sw.icon_png(4567).is_some());
    assert_eq!(f.sw.icon_png(99), None);
    assert_eq!(web.asked().len(), before);

    // A version that is not a version never becomes part of an address.
    let (g, web, _) = online_fixture("net-icons-bad");
    web.put("https://ddragon.leagueoflegends.com/api/versions.json", 200, r#"["../../evil"]"#);
    assert_eq!(g.sw.icon_png(1), None);
    assert_eq!(web.asked().len(), 1);
}

#[test]
fn a_diagnostic_bundle_says_how_things_stand_and_not_who_anyone_is() {
    let (f, web, _) = online_fixture("diagnose");
    f.sw.set_credentials("ana-hesap", "gizli_kullanici", "Gizli-Sifre-1").unwrap();
    f.sw.set_note("ana-hesap", "GizliNot").unwrap();
    web.put(ANA_PAGE, 200, profile_page("puuid-A#TR1 / Gold 2 45 LP / 1Win 1Lose Win rate 50%", 1, "x"));
    f.sw.refresh_ranks(None, true).unwrap();
    f.sw.switch("ana-hesap", None).unwrap();
    f.sw.await_login("ana-hesap", None, &never).unwrap();

    let bundle = f.sw.diagnostics("9.9.9");
    assert_eq!(bundle["quickswitch"], "9.9.9");
    assert_eq!(bundle["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(bundle["accounts"][0]["has password"], true);
    assert_eq!(bundle["accounts"][0]["rank from"], "opgg");
    assert_eq!(bundle["internet"]["online"], true);
    let session = &bundle["riot client answers"]["/rso-auth/v1/session"];
    assert_eq!(session["status"], 200);
    assert_eq!(session["shape"]["type"], "authenticated", "what state the client is in");
    assert_eq!(bundle["league client answers"]["/lol-summoner/v1/current-summoner"], "no answer");
    assert_eq!(bundle["session files"][0]["kind"], "file");
    assert!(bundle["session files"][0]["outline"].as_array().is_some_and(|o| !o.is_empty()));
    assert!(bundle["log"].as_array().is_some_and(|l| !l.is_empty()));

    let text = serde_json::to_string_pretty(&bundle).unwrap().to_lowercase();
    for secret in ["gizli", "puuid-a", "puuid-b", "a-1", "ana hesap", "smurf", "ana-hesap", "tr1"] {
        assert!(!text.contains(secret), "the bundle holds {secret:?}");
    }
}

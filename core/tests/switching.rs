//! End-to-end tests of the switching logic against a simulated Riot Client:
//! a fake `Riot Games` folder, fake processes, a fake local API and a fake
//! keyboard.

use quickswitch_core::store::{save_json, Config};
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
    league: Option<LeagueInfo>,
    /// The local API says "stay signed in" is off although it is on.
    misreport_persist: bool,
    /// League client gameflow phase, while it runs.
    phase: Option<String>,
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
        Some(Identity { riot_id: Some(format!("{puuid}#TR1")), puuid: Some(puuid) })
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
        if let Some((good, puuid)) = s.accepts.clone() {
            if good == password {
                s.signed_in = Some((puuid, "typed-session".into()));
                s.persist = Some(false);
            }
        }
        Ok(())
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
        game_start_wait: ms(30),
        manual_wait: ms(300),
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
fn password_only_account_does_not_wait_out_the_session_grace() {
    let mut f = fixture("pw-fast");
    // Long enough that waiting it out in full would be plain to see.
    f.sw.timing.signed_out_grace = Duration::from_secs(2);
    f.w.sim().accepts = Some(("hunter2".into(), "puuid-P".into()));
    let acc = f.sw.add_with_credentials("Şifreli", "user", "hunter2", Game::Lol).unwrap();
    f.sw.switch(&acc.id, None).unwrap();
    let started = std::time::Instant::now();
    assert_eq!(f.sw.await_login(&acc.id, None, &never).unwrap(), LoginOutcome::SignedIn);
    assert!(started.elapsed() < Duration::from_millis(1500), "typed after a quarter of the grace: {:?}", started.elapsed());
    assert_eq!(f.w.sim().typed.len(), 1);
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

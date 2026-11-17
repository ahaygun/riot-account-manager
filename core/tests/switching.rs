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
        fs::write(cfg.join("RiotClientSettings.yaml"), format!("settings-for: {token}\n")).unwrap();
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

    let r = f.sw.switch("ana-hesap").unwrap();
    assert!(r.watch_login);
    assert!(f.w.live_token().contains("A-1"));
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=league_of_legends");
    assert_eq!(f.sw.await_login("ana-hesap", &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.sw.view().unwrap().active.as_deref(), Some("ana-hesap"));

    // Riot renews A's session while playing; it only reaches disk on quit.
    f.w.renew("A-2");
    f.sw.switch("smurf").unwrap(); // by id
    assert!(f.w.live_token().contains("B-1"));
    f.sw.switch("Ana Hesap").unwrap(); // by name, as a desktop shortcut would
    assert!(f.w.live_token().contains("A-2"), "renewed session was kept on the way out");
}

#[test]
fn the_lockfile_is_never_stored_or_restored() {
    let f = fixture("lockfile");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
    f.w.stop(true);
    assert!(f.w.riot.join("Riot Client/Config/RiotClientSettings.yaml").is_file());
    fs::remove_file(f.w.riot.join("Riot Client/Config/lockfile")).unwrap();
    f.sw.switch("smurf").unwrap();
    assert!(!f.w.riot.join("Riot Client/Config/lockfile").exists());
    assert_eq!(
        fs::read_to_string(f.w.riot.join("Riot Client/Config/RiotClientSettings.yaml")).unwrap(),
        "settings-for: B-1\n"
    );
}

#[test]
fn a_hand_login_to_an_unknown_account_is_not_filed_under_another() {
    let f = fixture("stranger");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
    // The user signs out and into a third account inside Riot Client.
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    let v = f.sw.view().unwrap();
    assert_eq!(v.active, None);
    assert!(v.unsaved_session);

    f.sw.switch("smurf").unwrap();
    f.sw.switch("ana-hesap").unwrap();
    assert!(f.w.live_token().contains("A-1"), "A still holds A's session, not C's");
}

#[test]
fn already_signed_in_only_starts_the_game() {
    let f = fixture("same");
    two_accounts(&f);
    f.sw.switch("smurf").unwrap();
    let quits = f.w.sim().quits;
    let launches = f.w.sim().launches.len();
    let r = f.sw.switch("smurf").unwrap();
    assert!(!r.watch_login);
    assert_eq!(f.w.sim().quits, quits, "client was not restarted");
    assert_eq!(f.w.sim().launches.len(), launches);
    assert_eq!(f.w.sim().started_products, ["league_of_legends"]);
}

#[test]
fn refuses_during_a_match_and_touches_nothing() {
    let f = fixture("ingame");
    two_accounts(&f);
    f.sw.switch("smurf").unwrap();
    f.w.sim().running.push("League of Legends.exe".into());
    assert!(f.sw.view().unwrap().in_game);
    let quits = f.w.sim().quits;
    assert!(matches!(f.sw.switch("ana-hesap"), Err(Error::InGame(_))));
    assert_eq!(f.w.sim().quits, quits);
    assert!(f.w.live_token().contains("B-1"));
}

#[test]
fn stops_if_the_client_will_not_close() {
    let f = fixture("stuck");
    two_accounts(&f);
    f.sw.switch("smurf").unwrap();
    f.w.sim().unkillable = true;
    assert!(matches!(f.sw.switch("ana-hesap"), Err(Error::StillRunning(_))));
    assert!(f.w.live_token().contains("B-1"));
}

#[test]
fn killed_client_falls_back_to_the_last_saved_session() {
    let f = fixture("kill");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
    f.w.renew("A-2");
    // The polite quit is refused; the client gets killed and writes nothing.
    f.w.sim().unkillable = false;
    f.w.kill(&["RiotClientServices.exe".to_string()]);
    f.sw.switch("smurf").unwrap();
    f.sw.switch("ana-hesap").unwrap();
    assert!(f.w.live_token().contains("A-1"));
}

#[test]
fn expired_session_without_password_is_flagged_then_cleared_by_a_hand_login() {
    let f = fixture("expired");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
    f.w.renew("expired-A"); // what gets saved will be rejected next time
    f.sw.switch("smurf").unwrap();

    f.sw.switch("ana-hesap").unwrap();
    let out = f.sw.await_login("ana-hesap", &never).unwrap();
    assert!(matches!(out, LoginOutcome::NeedsLogin(_)));
    assert!(f.sw.view().unwrap().accounts[0].account.needs_login);
    assert!(f.w.sim().typed.is_empty(), "nothing is typed without a stored password");

    // Leaving and coming back must not hide the flag.
    f.sw.switch("smurf").unwrap();
    assert!(f.sw.view().unwrap().accounts[0].account.needs_login);

    f.sw.switch("ana-hesap").unwrap();
    f.w.sign_in_by_hand("puuid-A", "A-9", true);
    assert_eq!(f.sw.await_manual_login("ana-hesap", &never).unwrap(), LoginOutcome::SignedIn);
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);
    f.sw.switch("smurf").unwrap();
    f.sw.switch("ana-hesap").unwrap();
    assert!(f.w.live_token().contains("A-9"), "the new session replaced the expired one");
}

#[test]
fn expired_session_with_password_signs_in_by_typing() {
    let f = fixture("typed");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "s3cret").unwrap();
    assert!(f.sw.view().unwrap().accounts[0].has_password);
    f.sw.switch("ana-hesap").unwrap();
    f.w.renew("expired-A");
    f.sw.switch("smurf").unwrap();

    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap").unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed, [("ana_user".to_string(), "s3cret".to_string())]);
    assert!(f.w.sim().started_products.contains(&"league_of_legends".to_string()), "game was nudged to start");
    assert!(!f.sw.view().unwrap().accounts[0].account.needs_login);
}

#[test]
fn wrong_password_or_lost_focus_ends_in_needs_login() {
    let f = fixture("badpw");
    two_accounts(&f);
    f.sw.set_credentials("ana-hesap", "ana_user", "wrong").unwrap();
    f.sw.switch("ana-hesap").unwrap();
    f.w.renew("expired-A");
    f.sw.switch("smurf").unwrap();

    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    f.sw.switch("ana-hesap").unwrap();
    assert!(matches!(f.sw.await_login("ana-hesap", &never).unwrap(), LoginOutcome::NeedsLogin(_)));

    f.sw.switch("smurf").unwrap();
    f.w.sim().typer_error = Some("pencere yok".into());
    f.sw.switch("ana-hesap").unwrap();
    assert_eq!(f.sw.await_login("ana-hesap", &never).unwrap(), LoginOutcome::NeedsLogin("pencere yok".into()));
}

#[test]
fn password_only_account_starts_signed_out_and_types() {
    let f = fixture("pwonly");
    two_accounts(&f);
    let c = f.sw.add_with_credentials("Üçüncü", "c_user", "pw-c", Game::Valorant).unwrap();
    assert_eq!(c.id, "nc"); // non-ASCII letters drop out of the id
    f.w.sim().accepts = Some(("pw-c".into(), "puuid-C".into()));
    f.sw.switch(&c.id).unwrap();
    assert_eq!(f.w.live_token(), "", "previous account's session is not left in place");
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=valorant");
    assert_eq!(f.sw.await_login(&c.id, &never).unwrap(), LoginOutcome::SignedIn);
    let v = f.sw.view().unwrap();
    assert_eq!(v.active.as_deref(), Some("nc"));
    assert_eq!(v.accounts[2].account.puuid.as_deref(), Some("puuid-C"));
    assert!(!v.accounts[2].has_session);
}

#[test]
fn await_login_can_be_cancelled() {
    let f = fixture("cancel");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
    f.w.sim().signed_in = None;
    assert_eq!(f.sw.await_login("ana-hesap", &|| true).unwrap(), LoginOutcome::Unknown);
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
    f.sw.switch("ana").unwrap();
    assert!(f.w.live_token().contains("A-1"));
    // Running again with tokens on disk, the view no longer repeats the wrong report.
    f.w.sim().misreport_persist = true;
    assert_eq!(f.sw.view().unwrap().client.persist, Some(true));
}

#[test]
fn capture_rejects_duplicates() {
    let f = fixture("dup");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
    assert!(matches!(f.sw.capture_new("Yine A", Game::Lol), Err(Error::AlreadySaved(_))));
    f.sw.new_login().unwrap();
    f.w.sign_in_by_hand("puuid-C", "C-1", true);
    assert!(matches!(f.sw.capture_new("smurf", Game::Lol), Err(Error::NameTaken(_))));
}

#[test]
fn idle_sync_files_a_session_after_the_user_closes_the_client() {
    let f = fixture("idle");
    two_accounts(&f);
    f.sw.switch("ana-hesap").unwrap();
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

    f.sw.switch("Main").unwrap();
    assert_eq!(f.w.sim().launches.last().unwrap()[0], "--launch-product=riot-client");

    f.sw.remove("smurf").unwrap();
    assert!(!f.sw.env.snapshot_file("smurf").exists());
    assert!(!f.sw.env.secret_file("smurf").exists());
    assert!(matches!(f.sw.switch("smurf"), Err(Error::UnknownAccount(_))));
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
    assert!(matches!(sw.switch("ana-hesap"), Err(Error::Locked)));
    assert!(matches!(sw.new_login(), Err(Error::Locked)));
    assert!(matches!(sw.capture_new("X", Game::Lol), Err(Error::Locked)));
    assert!(matches!(sw.set_credentials("smurf", "u", "p"), Err(Error::Locked)));
    assert_eq!(f.w.sim().quits, quits, "locked: the running client was left alone");
    assert_eq!(sw.sync_idle().unwrap(), None);

    assert!(!sw.unlock("wrong horse").unwrap());
    assert!(matches!(sw.switch("ana-hesap"), Err(Error::Locked)));
    assert!(sw.unlock("correct horse").unwrap());
    sw.switch("ana-hesap").unwrap();
    assert!(f.w.live_token().contains("A-1"), "session survived being re-encrypted");

    sw.lock_now();
    assert!(matches!(sw.switch("smurf"), Err(Error::Locked)));
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
    sw.switch("smurf").unwrap();
    assert!(f.w.live_token().contains("B-1"));

    sw.set_lock(Some("second password"), None).unwrap();
    let sw = switcher(f.sw.env.clone(), &f.w);
    assert!(!sw.view().unwrap().security.lock_enabled);
    sw.switch("ana-hesap").unwrap();
    assert!(f.w.live_token().contains("A-1"));
    // The stored password came through both re-encryptions.
    f.w.renew("expired-A");
    sw.switch("smurf").unwrap();
    f.w.sim().accepts = Some(("s3cret".into(), "puuid-A".into()));
    sw.switch("ana-hesap").unwrap();
    assert_eq!(sw.await_login("ana-hesap", &never).unwrap(), LoginOutcome::SignedIn);
    assert_eq!(f.w.sim().typed, [("ana_user".to_string(), "s3cret".to_string())]);
}

#[test]
fn wipe_removes_everything_of_ours_and_nothing_of_riots() {
    let f = fixture("wipe");
    two_accounts(&f);
    f.sw.set_credentials("smurf", "u", "p").unwrap();
    f.sw.set_lock(None, Some("correct horse")).unwrap();
    f.sw.switch("smurf").unwrap();
    f.w.stop(true);
    f.sw.wipe_all().unwrap();
    assert!(!f.sw.env.data_dir.exists());
    assert!(f.w.live_token().contains("B-1"), "Riot Client's own session is untouched");
    // Starts over cleanly; config.json is recreated with defaults on demand.
    save_json(&f.sw.env.data_dir.join("config.json"), &Config { riot_client_exe: Some(f.root.join("x.exe")), ..Config::default() }).unwrap();
    let v = f.sw.view().unwrap();
    assert!(v.accounts.is_empty() && !v.security.lock_enabled && v.unsaved_session);
}

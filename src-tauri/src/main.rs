// No console window in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use quickswitch_core::{Env, Error, Game, LocalApi, LoginOutcome, Switcher, SysProcs, View, WindowTyper};
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_updater::{Update, UpdaterExt};

const TRAY_ID: &str = "main";
const TICK: Duration = Duration::from_secs(3);
/// Upkeep (filing sessions, reading rank) runs every this many ticks.
const UPKEEP_EVERY: u32 = 5;
/// A newer version is looked for shortly after start and then this often.
const UPDATE_FIRST_TICK: u32 = 4;
const UPDATE_EVERY: u32 = 6 * 60 * 60 / 3;

struct Shared {
    sw: Arc<Switcher>,
    /// Held for anything that closes the client or moves session files, so
    /// two of those never overlap.
    ops: Mutex<()>,
    /// Bumped by every switch; a login watcher from an older switch stops
    /// when it sees the number has moved on.
    generation: AtomicU64,
    /// Account to switch to once the running match ends.
    pending: Mutex<Option<String>>,
    /// Registered account hotkeys: shortcut id to account id.
    hotkeys: Mutex<HashMap<u32, String>>,
    /// This build can update itself: it was made with a signing key, so
    /// the updater is configured. Builds without one never go online.
    can_update: bool,
    /// A newer version that was found and is waiting for the user's go.
    update: Mutex<Option<Update>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Error shape the window receives: a stable `kind` to branch on and a
/// message to show.
#[derive(Debug, Clone, Serialize)]
struct CmdError {
    kind: String,
    message: String,
}

impl From<Error> for CmdError {
    fn from(e: Error) -> Self {
        CmdError { kind: e.kind().into(), message: e.to_string() }
    }
}

fn other(message: impl Into<String>) -> CmdError {
    CmdError { kind: "other".into(), message: message.into() }
}

type CmdResult<T> = Result<T, CmdError>;

/// Tells the window and the tray that something they show has changed.
fn changed(app: &AppHandle) {
    let _ = app.emit("state-changed", ());
    refresh_tray(app);
}

#[derive(Clone, Serialize)]
struct Notice {
    text: String,
    error: bool,
    /// Set for a refused switch, so the window can offer to queue it.
    kind: Option<String>,
    account: Option<String>,
}

fn notice(app: &AppHandle, text: impl Into<String>, error: bool) {
    let _ = app.emit("notice", Notice { text: text.into(), error, kind: None, account: None });
}

// ---- switching ---------------------------------------------------------------

/// The one place a switch happens, whoever asked for it.
/// `game` overrides the account's own game for this one switch.
/// `force` overrides only an unreadable League client state.
fn do_switch(app: &AppHandle, target: &str, game: Option<Game>, force: bool) -> CmdResult<String> {
    let shared = app.state::<Shared>();
    let (result, generation) = {
        let _ops = lock(&shared.ops);
        let generation = shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        (shared.sw.switch_with(target, game, force), generation)
    };
    changed(app);
    let switched = result?;
    *lock(&shared.pending) = None;
    if switched.watch_login {
        watch_login(app.clone(), switched.account.id.clone(), game, generation);
    }
    Ok(switched.account.name)
}

/// Follows the sign-in after a switch on its own thread: flags an expired
/// session, types the password if one is stored, and keeps watching for a
/// sign-in by hand.
fn watch_login(app: AppHandle, id: String, game: Option<Game>, generation: u64) {
    std::thread::spawn(move || {
        let shared = app.state::<Shared>();
        let cancelled = || shared.generation.load(Ordering::SeqCst) != generation;
        match shared.sw.await_login(&id, game, &cancelled) {
            Ok(LoginOutcome::NeedsLogin(why)) => {
                changed(&app);
                notice(&app, why, true);
                let _ = shared.sw.await_manual_login(&id, &cancelled);
                changed(&app);
            }
            Ok(_) => changed(&app),
            Err(e) => notice(&app, e.to_string(), true),
        }
    });
}

/// Switch asked for outside the window (tray, hotkey, command line). There
/// is nowhere else to show a problem, so the window comes up with it.
fn switch_in_background(app: &AppHandle, target: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = do_switch(&app, &target, None, false) {
            show_window(&app);
            let _ = app.emit("notice", Notice { text: e.message, error: true, kind: Some(e.kind), account: Some(target) });
        }
    });
}

// ---- commands called from the window -----------------------------------------

#[derive(Serialize)]
struct AppView {
    #[serde(flatten)]
    view: View,
    pending: Option<String>,
    autostart: bool,
    version: &'static str,
    update: UpdateView,
    /// Global hotkey that opens the window.
    window_hotkey: String,
}

#[derive(Serialize)]
struct UpdateView {
    /// This build is able to update itself.
    supported: bool,
    /// The user has left update checks on.
    enabled: bool,
    /// Version waiting to be installed.
    available: Option<String>,
}

#[tauri::command(async)]
fn get_state(app: AppHandle) -> CmdResult<AppView> {
    let shared = app.state::<Shared>();
    let pending = lock(&shared.pending).clone();
    let available = lock(&shared.update).as_ref().map(|u| u.version.clone());
    Ok(AppView {
        view: shared.sw.view()?,
        pending,
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        version: env!("CARGO_PKG_VERSION"),
        update: UpdateView {
            supported: shared.can_update,
            enabled: shared.sw.config().map(|c| c.check_updates).unwrap_or(true),
            available,
        },
        window_hotkey: shared.sw.config().map(|c| c.window_hotkey).unwrap_or_default(),
    })
}

#[tauri::command(async)]
fn switch_account(app: AppHandle, id: String, game: Option<Game>, force: Option<bool>) -> CmdResult<String> {
    let name = do_switch(&app, &id, game, force.unwrap_or(false))?;
    hide_window(&app);
    Ok(name)
}

#[tauri::command(async)]
fn queue_switch(app: AppHandle, id: String) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    if shared.sw.account_name(&id).is_none() {
        return Err(Error::UnknownAccount(id).into());
    }
    *lock(&shared.pending) = Some(id);
    changed(&app);
    Ok(())
}

#[tauri::command(async)]
fn cancel_queue(app: AppHandle) {
    *lock(&app.state::<Shared>().pending) = None;
    changed(&app);
}

/// Runs one operation that may close the client or move session files.
fn exclusive<T>(app: &AppHandle, f: impl FnOnce(&Switcher) -> quickswitch_core::switcher::Result<T>) -> CmdResult<T> {
    let shared = app.state::<Shared>();
    let result = {
        let _ops = lock(&shared.ops);
        // Any login watcher belongs to a session that is about to go away.
        shared.generation.fetch_add(1, Ordering::SeqCst);
        f(&shared.sw)
    };
    changed(app);
    Ok(result?)
}

/// Runs one small edit of the account list.
fn edit<T>(app: &AppHandle, f: impl FnOnce(&Switcher) -> quickswitch_core::switcher::Result<T>) -> CmdResult<T> {
    let result = f(&app.state::<Shared>().sw);
    changed(app);
    Ok(result?)
}

#[tauri::command(async)]
fn begin_add(app: AppHandle) -> CmdResult<()> {
    exclusive(&app, |sw| sw.new_login())
}

#[tauri::command(async)]
fn capture_account(app: AppHandle, name: String, game: Game) -> CmdResult<String> {
    exclusive(&app, |sw| sw.capture_new(&name, game)).map(|a| a.id)
}

#[tauri::command(async)]
fn add_with_password(app: AppHandle, name: String, username: String, password: String, game: Game) -> CmdResult<String> {
    edit(&app, |sw| sw.add_with_credentials(&name, &username, &password, game)).map(|a| a.id)
}

#[tauri::command(async)]
fn remove_account(app: AppHandle, id: String) -> CmdResult<()> {
    edit(&app, |sw| sw.remove(&id))?;
    register_hotkeys(&app);
    Ok(())
}

#[tauri::command(async)]
fn rename_account(app: AppHandle, id: String, name: String) -> CmdResult<()> {
    edit(&app, |sw| sw.rename(&id, &name))
}

#[tauri::command(async)]
fn reorder_accounts(app: AppHandle, ids: Vec<String>) -> CmdResult<()> {
    edit(&app, |sw| sw.reorder(&ids))
}

#[tauri::command(async)]
fn set_game(app: AppHandle, id: String, game: Game) -> CmdResult<()> {
    edit(&app, |sw| sw.set_game(&id, game))
}

#[tauri::command(async)]
fn set_credentials(app: AppHandle, id: String, username: String, password: String) -> CmdResult<()> {
    edit(&app, |sw| sw.set_credentials(&id, &username, &password))
}

#[tauri::command(async)]
fn clear_credentials(app: AppHandle, id: String) -> CmdResult<()> {
    edit(&app, |sw| sw.clear_credentials(&id))
}

#[tauri::command(async)]
fn set_hotkey(app: AppHandle, id: String, hotkey: Option<String>) -> CmdResult<()> {
    let hotkey = hotkey.filter(|h| !h.trim().is_empty());
    if let Some(h) = &hotkey {
        check_hotkey(h, &window_hotkey(&app)).map_err(other)?;
    }
    edit(&app, |sw| sw.set_hotkey(&id, hotkey.as_deref()))?;
    register_hotkeys(&app);
    Ok(())
}

#[tauri::command(async)]
fn create_shortcut(app: AppHandle, id: String) -> CmdResult<()> {
    let name = app.state::<Shared>().sw.account_name(&id).ok_or(Error::UnknownAccount(id.clone()))?;
    let exe = std::env::current_exe().map_err(|e| other(e.to_string()))?;
    quickswitch_core::shortcut::create_desktop_shortcut(&exe, &id, &name).map_err(|e| other(format!("Kısayol oluşturulamadı: {e}")))
}

#[tauri::command(async)]
fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let auto = app.autolaunch();
    let result = if enabled { auto.enable() } else { auto.disable() };
    changed(&app);
    result.map_err(|e| other(format!("Otomatik başlatma ayarlanamadı: {e}")))
}

// ---- app lock and data --------------------------------------------------------

#[tauri::command(async)]
fn unlock(app: AppHandle, password: String) -> CmdResult<()> {
    let ok = app.state::<Shared>().sw.unlock(&password)?;
    changed(&app);
    if ok { Ok(()) } else { Err(Error::WrongPassword.into()) }
}

#[tauri::command(async)]
fn lock_now(app: AppHandle) {
    app.state::<Shared>().sw.lock_now();
    changed(&app);
}

/// Turns the app lock on, off, or changes its password.
#[tauri::command(async)]
fn set_lock(app: AppHandle, current: Option<String>, new: Option<String>) -> CmdResult<()> {
    exclusive(&app, |sw| sw.set_lock(current.as_deref(), new.as_deref()))
}

/// Deletes everything the app has stored and returns it to a first run.
#[tauri::command(async)]
fn wipe_all(app: AppHandle) -> CmdResult<()> {
    *lock(&app.state::<Shared>().pending) = None;
    exclusive(&app, |sw| sw.wipe_all())?;
    let _ = app.autolaunch().disable();
    register_hotkeys(&app);
    changed(&app);
    Ok(())
}

/// Shows the data folder in Explorer, so the user can see what is stored.
#[tauri::command(async)]
fn open_data_dir(app: AppHandle) -> CmdResult<()> {
    let dir = app.state::<Shared>().sw.env.data_dir.clone();
    let _ = std::fs::create_dir_all(&dir);
    if cfg!(windows) {
        std::process::Command::new("explorer.exe").arg(&dir).spawn().map(|_| ()).map_err(|e| other(e.to_string()))
    } else {
        Err(other(dir.display().to_string()))
    }
}

// ---- updates --------------------------------------------------------------------
//
// The updater downloads `latest.json` from the project's GitHub releases,
// and installs a newer installer only if its signature matches the public
// key built into this app. Nothing is installed without the user's click.

/// Looks for a newer version and remembers it. Returns its version.
///
/// Async on purpose: commands already run on the async runtime, and
/// blocking on a future from inside it aborts the task. The background
/// thread, which is not on the runtime, drives this with `block_on`.
async fn find_update(app: &AppHandle) -> Result<Option<String>, String> {
    if !app.state::<Shared>().can_update {
        return Ok(None);
    }
    let updater = app.updater().map_err(|e| e.to_string())?;
    let found = updater.check().await.map_err(|e| e.to_string())?;
    let version = found.as_ref().map(|u| u.version.clone());
    *lock(&app.state::<Shared>().update) = found;
    changed(app);
    Ok(version)
}

#[tauri::command]
async fn check_update_now(app: AppHandle) -> CmdResult<Option<String>> {
    find_update(&app).await.map_err(|e| other(format!("Güncelleme denetlenemedi: {e}")))
}

#[tauri::command(async)]
fn set_update_checks(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let shared = app.state::<Shared>();
    shared.sw.set_check_updates(enabled)?;
    if !enabled {
        *lock(&shared.update) = None;
    }
    changed(&app);
    Ok(())
}

#[derive(Clone, Serialize)]
struct UpdateProgress {
    /// 0 to 100, when the size is known.
    percent: Option<u8>,
    /// The download is done and the installer is starting.
    installing: bool,
}

/// Downloads the waiting update, verifies its signature, runs the
/// installer and restarts into the new version. Reports progress as
/// `update-progress` events.
#[tauri::command]
async fn install_update(app: AppHandle) -> CmdResult<()> {
    let update = lock(&app.state::<Shared>().update).clone();
    let Some(update) = update else {
        return Err(other("Kurulacak bir güncelleme yok."));
    };
    if app.state::<Shared>().sw.in_game() {
        return Err(other("Oyun açıkken güncelleme kurulmaz."));
    }
    let mut received: u64 = 0;
    let mut last_percent = None;
    let progress_app = app.clone();
    let done_app = app.clone();
    update
        .download_and_install(
            move |chunk, total| {
                received += chunk as u64;
                let percent = total.filter(|t| *t > 0).map(|t| (received * 100 / t).min(100) as u8);
                if percent != last_percent {
                    last_percent = percent;
                    let _ = progress_app.emit("update-progress", UpdateProgress { percent, installing: false });
                }
            },
            move || {
                let _ = done_app.emit("update-progress", UpdateProgress { percent: Some(100), installing: true });
            },
        )
        .await
        .map_err(|e| other(format!("Güncelleme kurulamadı: {e}")))?;
    app.restart();
}

// ---- hotkeys -------------------------------------------------------------------

/// A hotkey must parse and must include Ctrl or Alt, so a plain letter can
/// never be taken away from every other program. (Whether Ctrl+Alt+key
/// types a character on the user's keyboard is checked by the window,
/// which sees the keyboard layout.)
fn parse_hotkey(text: &str) -> Result<Shortcut, String> {
    let has_modifier = text.split('+').any(|part| matches!(part.trim().to_ascii_lowercase().as_str(), "ctrl" | "control" | "alt"));
    if !has_modifier {
        return Err("Kısayolda Ctrl ya da Alt olmalı.".into());
    }
    Shortcut::from_str(text).map_err(|_| format!("Geçersiz kısayol: {text}"))
}

/// An account hotkey: valid, and not the one that opens the window.
fn check_hotkey(text: &str, window_hotkey: &str) -> Result<Shortcut, String> {
    let shortcut = parse_hotkey(text)?;
    if Shortcut::from_str(window_hotkey).is_ok_and(|w| w.id() == shortcut.id()) {
        return Err("Bu kısayol pencereyi açmak için kullanılıyor.".into());
    }
    Ok(shortcut)
}

fn window_hotkey(app: &AppHandle) -> String {
    app.state::<Shared>().sw.config().map(|c| c.window_hotkey).unwrap_or_default()
}

#[tauri::command(async)]
fn set_window_hotkey(app: AppHandle, hotkey: String) -> CmdResult<()> {
    parse_hotkey(&hotkey).map_err(other)?;
    edit(&app, |sw| sw.set_window_hotkey(&hotkey))?;
    register_hotkeys(&app);
    Ok(())
}

/// Drops every registered hotkey and registers the current set. A
/// combination another program already owns is skipped.
fn register_hotkeys(app: &AppHandle) {
    let shared = app.state::<Shared>();
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();
    let window = window_hotkey(app);
    if let Ok(toggle) = Shortcut::from_str(&window) {
        let _ = shortcuts.register(toggle);
    }
    let mut map = HashMap::new();
    if let Ok(view) = shared.sw.view() {
        for a in &view.accounts {
            let Some(shortcut) = a.account.hotkey.as_deref().and_then(|h| check_hotkey(h, &window).ok()) else { continue };
            if shortcuts.register(shortcut).is_ok() {
                map.insert(shortcut.id(), a.account.id.clone());
            }
        }
    }
    *lock(&shared.hotkeys) = map;
}

fn on_hotkey(app: &AppHandle, shortcut: &Shortcut) {
    let account = lock(&app.state::<Shared>().hotkeys).get(&shortcut.id()).cloned();
    match account {
        Some(id) => switch_in_background(app, id),
        None => toggle_window(app),
    }
}

// ---- window ----------------------------------------------------------------------

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn hide_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

fn toggle_window(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(w) if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) => {
            let _ = w.hide();
        }
        _ => show_window(app),
    }
}

// ---- tray --------------------------------------------------------------------------

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let view = app.state::<Shared>().sw.view().ok();
    let mut menu = MenuBuilder::new(app);
    if let Some(view) = &view {
        for a in &view.accounts {
            let active = view.active.as_deref() == Some(a.account.id.as_str());
            let mut label = if active { format!("●  {}", a.account.name) } else { format!("     {}", a.account.name) };
            if a.account.needs_login {
                label.push_str("  (giriş gerekli)");
            }
            menu = menu.item(&MenuItemBuilder::with_id(format!("acc:{}", a.account.id), label).build(app)?);
        }
        if !view.accounts.is_empty() {
            menu = menu.separator();
        }
    }
    menu.text("show", "Pencereyi aç").text("quit", "Çıkış").build()
}

fn refresh_tray(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id(TRAY_ID), build_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

// ---- background work ---------------------------------------------------------------

fn background_loop(app: AppHandle) {
    let shared = app.state::<Shared>();
    let mut tick: u32 = 0;
    // The match must be seen gone on two ticks in a row before a queued
    // switch runs, so the end-of-game screen is not cut off mid-load.
    let mut clear_ticks = 0;
    loop {
        std::thread::sleep(TICK);
        tick = tick.wrapping_add(1);

        let queued = lock(&shared.pending).clone();
        match queued {
            Some(id) if !shared.sw.in_game() => {
                clear_ticks += 1;
                if clear_ticks >= 2 {
                    clear_ticks = 0;
                    *lock(&shared.pending) = None;
                    if let Err(e) = do_switch(&app, &id, None, false) {
                        show_window(&app);
                        notice(&app, e.message, true);
                    }
                }
            }
            _ => clear_ticks = 0,
        }

        if shared.can_update && tick % UPDATE_EVERY == UPDATE_FIRST_TICK && shared.sw.config().map(|c| c.check_updates).unwrap_or(false) {
            // Offline or no release yet: simply try again next time.
            let _ = tauri::async_runtime::block_on(find_update(&app));
        }

        if tick % UPKEEP_EVERY == 0 {
            // Skip the round if a switch is in progress.
            let Ok(_ops) = shared.ops.try_lock() else { continue };
            let filed = shared.sw.sync_idle().ok().flatten().is_some();
            let ranked = shared.sw.refresh_league().unwrap_or(false);
            if filed || ranked {
                changed(&app);
            }
        }
    }
}

// ---- command line ------------------------------------------------------------------

/// `quickswitch.exe --switch <account name or id>`
fn switch_target(args: &[String]) -> Option<String> {
    let i = args.iter().position(|a| a == "--switch")?;
    args.get(i + 1).cloned()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let start_target = switch_target(&args);
    let start_hidden = start_target.is_some() || args.iter().any(|a| a == "--hidden");

    let env = Env::from_system();
    let api = Arc::new(LocalApi::new(env.riot_games_dir.clone()));
    let switcher = Switcher::new(env, Arc::new(SysProcs), api, Arc::new(WindowTyper));

    // The updater is only configured in release builds made with a signing
    // key (see .github/workflows/release.yml). Without that configuration
    // the plugin is not loaded at all.
    let context = tauri::generate_context!();
    let can_update = context.config().plugins.0.contains_key("updater");

    let mut builder = tauri::Builder::default();
    if can_update {
        builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    }

    builder
        // Must be the first of the plugins below. A second launch hands its arguments to
        // the running app and exits, so desktop shortcuts stay instant.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| match switch_target(&argv) {
            Some(target) => switch_in_background(app, target),
            None => show_window(app),
        }))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        on_hotkey(app, shortcut);
                    }
                })
                .build(),
        )
        .manage(Shared {
            sw: Arc::new(switcher),
            ops: Mutex::new(()),
            generation: AtomicU64::new(0),
            pending: Mutex::new(None),
            hotkeys: Mutex::new(HashMap::new()),
            can_update,
            update: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            switch_account,
            queue_switch,
            cancel_queue,
            begin_add,
            capture_account,
            add_with_password,
            remove_account,
            rename_account,
            reorder_accounts,
            set_game,
            set_credentials,
            clear_credentials,
            set_hotkey,
            set_window_hotkey,
            create_shortcut,
            set_autostart,
            unlock,
            lock_now,
            set_lock,
            wipe_all,
            open_data_dir,
            check_update_now,
            set_update_checks,
            install_update
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip("QuickSwitch")
                .menu(&build_menu(&handle)?)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_window(app),
                    "quit" => app.exit(0),
                    id => {
                        if let Some(acc) = id.strip_prefix("acc:") {
                            switch_in_background(app, acc.to_string());
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        show_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            register_hotkeys(&handle);

            let loop_handle = handle.clone();
            std::thread::spawn(move || background_loop(loop_handle));

            match start_target.clone() {
                Some(target) => switch_in_background(&handle, target),
                None if !start_hidden => show_window(&handle),
                None => {}
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window keeps the app in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(context)
        .expect("error while running QuickSwitch");
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: &str = "Ctrl+Shift+Space";

    #[test]
    fn hotkeys_the_window_can_produce_are_accepted() {
        for ok in ["Ctrl+Alt+Digit1", "Ctrl+Shift+KeyA", "Alt+F5", "Ctrl+Alt+Numpad3", "Ctrl+Alt+Shift+KeyQ", "Ctrl+Shift+Space"] {
            assert!(parse_hotkey(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn unsafe_hotkeys_are_refused() {
        assert!(check_hotkey("KeyA", WINDOW).is_err(), "no modifier");
        assert!(check_hotkey("Shift+KeyA", WINDOW).is_err(), "shift alone would block typing a capital A");
        assert!(check_hotkey("Ctrl+Shift+Space", WINDOW).is_err(), "the window's own");
        assert!(check_hotkey("ctrl+shift+space", WINDOW).is_err(), "the window's own, spelled differently");
        assert!(check_hotkey("Ctrl+Alt+NotAKey", WINDOW).is_err());
    }

    #[test]
    fn distinct_hotkeys_get_distinct_ids() {
        let a = parse_hotkey("Ctrl+Alt+Digit1").unwrap();
        let b = parse_hotkey("Ctrl+Alt+Digit2").unwrap();
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn switch_argument() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(switch_target(&args(&["app.exe", "--switch", "Ana Hesap"])).as_deref(), Some("Ana Hesap"));
        assert_eq!(switch_target(&args(&["app.exe", "--hidden"])), None);
        assert_eq!(switch_target(&args(&["app.exe", "--switch"])), None);
    }
}

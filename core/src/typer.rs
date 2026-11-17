//! Types a username and password into Riot Client's login window. This is
//! the fallback for when a saved session has expired.
//!
//! Keystrokes go to whatever has the keyboard focus, so the rule is: before
//! every batch, the foreground window must belong to a Riot Client UI
//! process, or nothing more is typed.
//!
//! A process counts as Riot Client only if its name matches and its
//! executable sits inside Riot Client's install folder. The name alone is
//! not enough: any program can call itself "Riot Client.exe".

use std::path::Path;

/// Which windows may receive the login, and how to reach the user.
pub struct UiTarget<'a> {
    /// Process names that own the login window.
    pub processes: &'a [String],
    /// Riot Client's install folder, when known.
    pub install_dir: Option<&'a Path>,
    /// Called with `true` when Windows will not let us bring the window
    /// forward and the user is asked to click it, and with `false` when
    /// that is over.
    pub ask_for_focus: &'a dyn Fn(bool),
    /// Stops waiting for the user's click.
    pub cancelled: &'a dyn Fn() -> bool,
    /// One line of diagnostics. Never given the username or password.
    pub log: &'a dyn Fn(&str),
}

pub trait LoginTyper: Send + Sync {
    fn type_login(&self, username: &str, password: &str, target: &UiTarget) -> Result<(), String>;
}

/// `exe` is inside `dir` (case-insensitive, as Windows paths are).
pub fn is_inside(exe: &Path, dir: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    let (exe, dir) = (norm(exe), norm(dir));
    let dir = dir.trim_end_matches('\\');
    exe.strip_prefix(dir).is_some_and(|rest| rest.starts_with('\\'))
}

/// Real implementation (Windows only; elsewhere it reports that).
#[derive(Default)]
pub struct WindowTyper;

#[cfg(not(windows))]
impl LoginTyper for WindowTyper {
    fn type_login(&self, _: &str, _: &str, _: &UiTarget) -> Result<(), String> {
        Err("Otomatik giriş yalnızca Windows'ta çalışır.".into())
    }
}

#[cfg(windows)]
impl LoginTyper for WindowTyper {
    fn type_login(&self, username: &str, password: &str, target: &UiTarget) -> Result<(), String> {
        win::type_login(username, password, target)
    }
}

#[cfg(windows)]
mod win {
    use super::{is_inside, UiTarget};
    use std::ffi::c_void;
    use std::thread::sleep;
    use std::time::Duration;

    type Hwnd = *mut c_void;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct KeybdInput {
        vk: u16,
        scan: u16,
        flags: u32,
        time: u32,
        extra: usize,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct MouseInput {
        dx: i32,
        dy: i32,
        data: u32,
        flags: u32,
        time: u32,
        extra: usize,
    }

    // The union must be as large as its biggest member (the mouse one) for
    // `size_of::<Input>()` to match what SendInput expects.
    #[repr(C)]
    #[derive(Clone, Copy)]
    union InputUnion {
        ki: KeybdInput,
        mi: MouseInput,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Input {
        kind: u32,
        u: InputUnion,
    }

    const INPUT_KEYBOARD: u32 = 1;
    const KEYEVENTF_KEYUP: u32 = 0x2;
    const KEYEVENTF_UNICODE: u32 = 0x4;
    const VK_TAB: u16 = 0x09;
    const VK_RETURN: u16 = 0x0D;
    const SW_RESTORE: i32 = 9;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: unsafe extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn GetForegroundWindow() -> Hwnd;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn BringWindowToTop(hwnd: Hwnd) -> i32;
        fn IsIconic(hwnd: Hwnd) -> i32;
        fn ShowWindow(hwnd: Hwnd, cmd: i32) -> i32;
        fn SetWindowPos(hwnd: Hwnd, after: Hwnd, x: i32, y: i32, cx: i32, cy: i32, flags: u32) -> i32;
        fn SwitchToThisWindow(hwnd: Hwnd, alt_tab: i32);
        fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn AttachThreadInput(attach: u32, attach_to: u32, attach_flag: i32) -> i32;
        fn SendInput(count: u32, inputs: *const Input, size: i32) -> u32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    /// Process ids of the Riot Client UI right now.
    fn ui_pids(target: &UiTarget) -> Vec<u32> {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let Ok(mut sys) = System::new() else { return Vec::new() };
        let what = ProcessRefreshKind::nothing().with_exe(UpdateKind::Always);
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, what);
        sys.processes()
            .iter()
            .filter(|(_, p)| {
                let name = p.name().to_string_lossy();
                if !target.processes.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    return false;
                }
                match target.install_dir {
                    // An unreadable path is treated as "not Riot Client".
                    Some(dir) => p.exe().is_some_and(|exe| is_inside(exe, dir)),
                    None => true,
                }
            })
            .map(|(pid, _)| pid.as_u32())
            .collect()
    }

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmGetWindowAttribute(hwnd: Hwnd, attribute: u32, value: *mut c_void, size: u32) -> i32;
    }

    const DWMWA_CLOAKED: u32 = 14;
    const SWP_NOMOVE: u32 = 0x2;
    const SWP_NOSIZE: u32 = 0x1;
    const HWND_TOPMOST: isize = -1;
    const HWND_NOTOPMOST: isize = -2;
    const VK_MENU: u16 = 0x12;

    struct Search {
        pids: Vec<u32>,
        found: Vec<(Hwnd, i64)>,
    }

    /// Hidden by the compositor although "visible" to older APIs.
    fn cloaked(hwnd: Hwnd) -> bool {
        let mut value = 0u32;
        // SAFETY: `value` is a u32, the size the attribute is defined with.
        let hr = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut value as *mut u32 as *mut c_void, 4) };
        hr == 0 && value != 0
    }

    unsafe extern "system" fn visit(hwnd: Hwnd, lparam: isize) -> i32 {
        // SAFETY: `lparam` is the `&mut Search` passed to EnumWindows below,
        // which outlives the enumeration.
        let search = &mut *(lparam as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if search.pids.contains(&pid) && IsWindowVisible(hwnd) != 0 && GetWindowTextLengthW(hwnd) > 0 && !cloaked(hwnd) {
            let mut r = Rect::default();
            GetWindowRect(hwnd, &mut r);
            let area = i64::from(r.right - r.left) * i64::from(r.bottom - r.top);
            search.found.push((hwnd, area));
        }
        1
    }

    /// The Riot Client window to sign in through: its largest visible,
    /// titled, uncloaked top-level window. Riot Client also owns small or
    /// hidden helper windows, which cannot be brought forward.
    fn find_window(target: &UiTarget) -> Option<Hwnd> {
        let mut search = Search { pids: ui_pids(target), found: Vec::new() };
        if search.pids.is_empty() {
            (target.log)("login: no Riot Client UI process found");
            return None;
        }
        // SAFETY: the callback only touches `search`, which lives across the call.
        unsafe { EnumWindows(visit, &mut search as *mut Search as isize) };
        (target.log)(&format!("login: {} UI process(es), {} candidate window(s)", search.pids.len(), search.found.len()));
        search.found.into_iter().filter(|(_, area)| *area > 200 * 200).max_by_key(|(_, area)| *area).map(|(h, _)| h)
    }

    fn foreground_pid() -> u32 {
        let mut pid = 0u32;
        // SAFETY: plain queries; a null foreground window yields pid 0.
        unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
        pid
    }

    fn process_name(pid: u32) -> String {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let Ok(mut sys) = System::new() else { return "?".into() };
        let pid = Pid::from_u32(pid);
        sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, ProcessRefreshKind::nothing());
        sys.process(pid).map(|p| p.name().to_string_lossy().into_owned()).unwrap_or_else(|| "?".into())
    }

    fn foreground_is_riot(target: &UiTarget) -> bool {
        let pid = foreground_pid();
        pid != 0 && ui_pids(target).contains(&pid)
    }

    fn settled(target: &UiTarget) -> bool {
        sleep(Duration::from_millis(200));
        foreground_is_riot(target)
    }

    /// Brings the window to the front. Windows refuses this to programs in
    /// the background unless one of several conditions holds, so each
    /// documented way is tried in turn, from the gentlest.
    fn focus(hwnd: Hwnd, target: &UiTarget) -> bool {
        // SAFETY (all blocks): handles come from the system and the calls
        // fail harmlessly on a stale one.
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
        }
        let steps: [(&str, &dyn Fn()); 5] = [
            ("direct", &|| unsafe {
                SetForegroundWindow(hwnd);
            }),
            // A program that produced the last input event may take the
            // foreground: a lone Alt press and release is that event.
            ("alt", &|| unsafe {
                send(&[key(VK_MENU, 0, 0), key(VK_MENU, 0, KEYEVENTF_KEYUP)]);
                SetForegroundWindow(hwnd);
                BringWindowToTop(hwnd);
            }),
            ("attach", &|| unsafe {
                let mut unused = 0u32;
                let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), &mut unused);
                let me = GetCurrentThreadId();
                AttachThreadInput(me, fg_thread, 1);
                BringWindowToTop(hwnd);
                SetForegroundWindow(hwnd);
                AttachThreadInput(me, fg_thread, 0);
            }),
            ("topmost", &|| unsafe {
                SetWindowPos(hwnd, HWND_TOPMOST as Hwnd, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
                SetWindowPos(hwnd, HWND_NOTOPMOST as Hwnd, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
                SetForegroundWindow(hwnd);
            }),
            ("switch", &|| unsafe {
                SwitchToThisWindow(hwnd, 1);
            }),
        ];
        for (name, step) in steps {
            step();
            if settled(target) {
                (target.log)(&format!("login: window brought forward ({name})"));
                return true;
            }
            (target.log)(&format!("login: '{name}' did not bring it forward; foreground is {}", process_name(foreground_pid())));
        }
        false
    }

    /// When Windows refuses every way, the user clicks the window: that is
    /// always allowed. Waits up to 45 seconds.
    fn wait_for_click(target: &UiTarget) -> bool {
        (target.ask_for_focus)(true);
        let start = std::time::Instant::now();
        let mut ok = false;
        while start.elapsed() < Duration::from_secs(45) && !(target.cancelled)() {
            if foreground_is_riot(target) {
                // Let the click finish before typing.
                sleep(Duration::from_millis(400));
                ok = foreground_is_riot(target);
                if ok {
                    break;
                }
            }
            sleep(Duration::from_millis(200));
        }
        (target.ask_for_focus)(false);
        (target.log)(if ok { "login: user brought the window forward" } else { "login: gave up waiting for the user's click" });
        ok
    }

    fn send(inputs: &[Input]) {
        // SAFETY: `inputs` is a valid slice of correctly laid out INPUT structs.
        unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<Input>() as i32) };
    }

    fn key(vk: u16, scan: u16, flags: u32) -> Input {
        Input { kind: INPUT_KEYBOARD, u: InputUnion { ki: KeybdInput { vk, scan, flags, time: 0, extra: 0 } } }
    }

    fn type_text(text: &str) {
        for unit in text.encode_utf16() {
            send(&[key(0, unit, KEYEVENTF_UNICODE), key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]);
            sleep(Duration::from_millis(12));
        }
    }

    fn press(vk: u16) {
        send(&[key(vk, 0, 0), key(vk, 0, KEYEVENTF_KEYUP)]);
    }

    pub fn type_login(username: &str, password: &str, target: &UiTarget) -> Result<(), String> {
        let lost = || {
            (target.log)(&format!("login: lost the window while typing; foreground is {}", process_name(foreground_pid())));
            "Riot Client penceresi yazarken öne çıkmadı; şifrenin başka bir yere gitmemesi için yazma durduruldu.".to_string()
        };
        let hwnd = find_window(target).ok_or("Riot Client penceresi bulunamadı.")?;
        if !focus(hwnd, target) && !wait_for_click(target) {
            return Err("Riot Client penceresi öne getirilemedi; şifre yazılmadı. Riot Client'ta elle giriş yapabilirsin.".into());
        }
        // The login page opens with the username field focused.
        type_text(username);
        sleep(Duration::from_millis(250));
        if !foreground_is_riot(target) {
            return Err(lost());
        }
        press(VK_TAB);
        sleep(Duration::from_millis(250));
        // Checked again right before the password and before Enter: a
        // password typed into another window is worse than not signing in.
        if !foreground_is_riot(target) {
            return Err(lost());
        }
        type_text(password);
        sleep(Duration::from_millis(250));
        if !foreground_is_riot(target) {
            return Err(lost());
        }
        press(VK_RETURN);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::is_inside;
    use std::path::Path;

    #[test]
    fn only_executables_inside_the_install_folder_count() {
        let dir = Path::new(r"C:\Riot Games\Riot Client");
        assert!(is_inside(Path::new(r"C:\Riot Games\Riot Client\UX\Riot Client.exe"), dir));
        assert!(is_inside(Path::new(r"c:\riot games\riot client\ux\Riot Client.exe"), dir));
        assert!(!is_inside(Path::new(r"C:\Users\x\Downloads\Riot Client.exe"), dir));
        // A sibling folder that merely starts with the same text.
        assert!(!is_inside(Path::new(r"C:\Riot Games\Riot Client Fake\Riot Client.exe"), dir));
        assert!(!is_inside(dir, dir));
    }
}

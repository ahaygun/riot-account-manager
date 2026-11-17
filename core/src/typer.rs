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

/// Which windows may receive the login.
pub struct UiTarget<'a> {
    /// Process names that own the login window.
    pub processes: &'a [String],
    /// Riot Client's install folder, when known.
    pub install_dir: Option<&'a Path>,
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
        fn ShowWindowAsync(hwnd: Hwnd, cmd: i32) -> i32;
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

    struct Search {
        pids: Vec<u32>,
        found: Hwnd,
    }

    unsafe extern "system" fn visit(hwnd: Hwnd, lparam: isize) -> i32 {
        // SAFETY: `lparam` is the `&mut Search` passed to EnumWindows below,
        // which outlives the enumeration.
        let search = &mut *(lparam as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if search.pids.contains(&pid) && IsWindowVisible(hwnd) != 0 && GetWindowTextLengthW(hwnd) > 0 {
            search.found = hwnd;
            return 0; // stop
        }
        1
    }

    /// A visible, titled top-level window of one of the UI processes.
    fn find_window(target: &UiTarget) -> Option<Hwnd> {
        let mut search = Search { pids: ui_pids(target), found: std::ptr::null_mut() };
        if search.pids.is_empty() {
            return None;
        }
        // SAFETY: the callback only touches `search`, which lives across the call.
        unsafe { EnumWindows(visit, &mut search as *mut Search as isize) };
        (!search.found.is_null()).then_some(search.found)
    }

    fn foreground_is_riot(target: &UiTarget) -> bool {
        let mut pid = 0u32;
        // SAFETY: plain queries; a null foreground window yields pid 0.
        unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
        pid != 0 && ui_pids(target).contains(&pid)
    }

    /// Brings the window to the front. Windows normally refuses this from a
    /// background program; attaching to the foreground thread's input for a
    /// moment is the documented way around it.
    fn focus(hwnd: Hwnd, target: &UiTarget) -> bool {
        for _ in 0..6 {
            // SAFETY: handles come from the system; calls fail harmlessly on stale ones.
            unsafe {
                let mut unused = 0u32;
                let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), &mut unused);
                let me = GetCurrentThreadId();
                AttachThreadInput(me, fg_thread, 1);
                ShowWindowAsync(hwnd, SW_RESTORE);
                SetForegroundWindow(hwnd);
                AttachThreadInput(me, fg_thread, 0);
            }
            sleep(Duration::from_millis(250));
            if foreground_is_riot(target) {
                return true;
            }
        }
        false
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
        let lost = || "Riot Client penceresi öne getirilemedi; şifre yazılmadı.".to_string();
        let hwnd = find_window(target).ok_or("Riot Client penceresi bulunamadı.")?;
        if !focus(hwnd, target) {
            return Err(lost());
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

//! Creates a desktop shortcut that switches to one account.

use std::io;
use std::path::Path;

/// Characters Windows does not allow in file names.
pub fn file_safe(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) || c.is_control() { ' ' } else { c }).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() { "Hesap".into() } else { cleaned }
}

/// Writes `<Desktop>\<name> - QuickSwitch.lnk` running `exe --switch <id>`.
#[cfg(windows)]
pub fn create_desktop_shortcut(exe: &Path, account_id: &str, account_name: &str) -> io::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Values travel as environment variables, never as script text, so an
    // account name can not turn into PowerShell code.
    let script = r#"
$ErrorActionPreference = 'Stop'
$desktop = [Environment]::GetFolderPath('Desktop')
$link = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $desktop ($env:QSW_NAME + ' - QuickSwitch.lnk')))
$link.TargetPath = $env:QSW_EXE
$link.Arguments = '--switch "' + $env:QSW_ID + '"'
$link.WorkingDirectory = Split-Path $env:QSW_EXE
$link.IconLocation = $env:QSW_EXE
$link.Save()
"#;
    let out = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("QSW_NAME", file_safe(account_name))
        .env("QSW_EXE", exe)
        .env("QSW_ID", account_id)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(String::from_utf8_lossy(&out.stderr).trim().to_string()))
    }
}

#[cfg(not(windows))]
pub fn create_desktop_shortcut(_exe: &Path, _account_id: &str, _account_name: &str) -> io::Result<()> {
    Err(io::Error::other("Kısayol oluşturma yalnızca Windows'ta çalışır."))
}

#[cfg(test)]
mod tests {
    use super::file_safe;

    #[test]
    fn names() {
        assert_eq!(file_safe("Ana Hesap"), "Ana Hesap");
        assert_eq!(file_safe(r#"a/b\c:d*"e"#), "a b c d e");
        assert_eq!(file_safe("???"), "Hesap");
    }
}

//! Puts a password on the clipboard in a way that keeps it out of Windows'
//! clipboard history and cloud sync, and takes it off again later.

use std::io;

/// Puts `text` on the clipboard, marked as not to be kept in the clipboard
/// history, not to be synced to other devices, and not to be picked up by
/// clipboard monitors that honour the mark. Returns a ticket for
/// `clear_if_still_ours`.
#[cfg(windows)]
pub fn set_private_text(text: &str) -> io::Result<u32> {
    use windows::core::w;
    use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};

    const CF_UNICODETEXT: u32 = 13;

    /// Hands `bytes` to the clipboard under `format`. The clipboard owns the
    /// memory once it has taken it; until then it is ours to free.
    unsafe fn put(format: u32, bytes: &[u8]) -> io::Result<()> {
        let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)).map_err(io::Error::other)?;
        let target = GlobalLock(memory) as *mut u8;
        if target.is_null() {
            let _ = GlobalFree(Some(memory));
            return Err(io::Error::other("clipboard memory could not be locked"));
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        let _ = GlobalUnlock(memory);
        if let Err(e) = SetClipboardData(format, Some(HANDLE(memory.0))) {
            let _ = GlobalFree(Some(memory));
            return Err(io::Error::other(e));
        }
        Ok(())
    }

    let wide: Vec<u8> = text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect();
    // SAFETY: the clipboard is opened and closed around the calls; every
    // buffer handed over is allocated here with the size that is copied.
    unsafe {
        // Another program may hold the clipboard for a moment.
        let mut opened = OpenClipboard(None);
        for _ in 0..10 {
            if opened.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
            opened = OpenClipboard(None);
        }
        opened.map_err(io::Error::other)?;
        let result = (|| {
            EmptyClipboard().map_err(io::Error::other)?;
            // The marks go on first: a history that looks at the text the
            // moment it arrives already finds them.
            put(RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing")), &[0])?;
            put(RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")), &0u32.to_le_bytes())?;
            put(RegisterClipboardFormatW(w!("CanUploadToCloudClipboard")), &0u32.to_le_bytes())?;
            put(CF_UNICODETEXT, &wide)
        })();
        let _ = CloseClipboard();
        result?;
        Ok(GetClipboardSequenceNumber())
    }
}

/// Empties the clipboard if nothing else has been copied since
/// `set_private_text` returned `ticket`.
#[cfg(windows)]
pub fn clear_if_still_ours(ticket: u32) {
    use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardSequenceNumber, OpenClipboard};
    // SAFETY: plain clipboard calls; opened and closed here.
    unsafe {
        if GetClipboardSequenceNumber() == ticket && OpenClipboard(None).is_ok() {
            let _ = EmptyClipboard();
            let _ = CloseClipboard();
        }
    }
}

#[cfg(not(windows))]
pub fn set_private_text(_text: &str) -> io::Result<u32> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "the clipboard is only supported on Windows"))
}

#[cfg(not(windows))]
pub fn clear_if_still_ours(_ticket: u32) {}

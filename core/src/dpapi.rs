//! Encrypts files at rest with Windows DPAPI, so they are only readable by
//! the same Windows user on the same machine.
//!
//! The extra entropy below is not a secret (the source is public). It
//! makes our files useless to generic tools that sweep a disk for DPAPI
//! blobs and decrypt them with default settings.
//!
//! On other systems (development and tests only) data is stored as-is.

use std::io;

const MAGIC: &[u8; 4] = b"QSW1";

pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    out.extend(imp::protect(plain)?);
    Ok(out)
}

pub fn unprotect(blob: &[u8]) -> io::Result<Vec<u8>> {
    let body = blob
        .strip_prefix(MAGIC)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not a snapshot file"))?;
    imp::unprotect(body)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::io;
    use std::ptr::{null, null_mut};

    #[repr(C)]
    struct DataBlob {
        cb: u32,
        pb: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;
    const ENTROPY: &[u8] = b"QuickSwitch/dpapi/v1";

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            description: *const u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            description: *mut *mut u16,
            entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    fn take(out: DataBlob) -> Vec<u8> {
        // SAFETY: on success the API returns a LocalAlloc'd buffer of `cb`
        // bytes that we own; copy it out and free it.
        unsafe {
            let v = std::slice::from_raw_parts(out.pb, out.cb as usize).to_vec();
            LocalFree(out.pb as *mut c_void);
            v
        }
    }

    fn blob(data: &[u8]) -> io::Result<DataBlob> {
        let cb = u32::try_from(data.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "snapshot too large"))?;
        Ok(DataBlob { cb, pb: data.as_ptr() as *mut u8 })
    }

    pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(plain)?;
        let entropy = blob(ENTROPY)?;
        let mut out = DataBlob { cb: 0, pb: null_mut() };
        // SAFETY: `input` and `entropy` point at data that outlives the
        // call; `out` is filled by the API.
        let ok = unsafe {
            CryptProtectData(&input, null(), &entropy, null_mut(), null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(out))
    }

    pub fn unprotect(body: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(body)?;
        let entropy = blob(ENTROPY)?;
        let mut out = DataBlob { cb: 0, pb: null_mut() };
        // SAFETY: as in `protect`.
        let ok = unsafe {
            CryptUnprotectData(&input, null_mut(), &entropy, null_mut(), null_mut(), CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(take(out))
    }
}

#[cfg(not(windows))]
mod imp {
    use std::io;
    pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
        Ok(plain.to_vec())
    }
    pub fn unprotect(body: &[u8]) -> io::Result<Vec<u8>> {
        Ok(body.to_vec())
    }
}

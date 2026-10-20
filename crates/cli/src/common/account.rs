// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The current user's id and the home the account database lists for it.

use std::path::PathBuf;

/// Bytes for the strings `getpwuid_r` copies out of the account database.
#[cfg(unix)]
const PASSWD_BUF_BYTES: usize = 16 * 1024;

/// The current user id.
#[cfg(unix)]
pub fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
pub fn uid() -> u32 {
    0
}

/// The home directory the account database lists for the current user.
#[cfg(unix)]
pub fn account_home() -> Option<PathBuf> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    let mut buf = vec![0 as libc::c_char; PASSWD_BUF_BYTES];
    // SAFETY: an all-zero passwd is a valid value to pass for getpwuid_r to fill.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call and `buf.len()` is its size.
    let rc = unsafe { libc::getpwuid_r(uid(), &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pwd.pw_dir.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, so pw_dir points at a NUL-terminated
    // string inside `buf`, which is still alive.
    let dir = unsafe { CStr::from_ptr(pwd.pw_dir) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(dir.to_bytes())))
}

#[cfg(not(unix))]
pub fn account_home() -> Option<PathBuf> {
    None
}

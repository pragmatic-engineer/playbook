// SPDX-FileCopyrightText: 2026 Igor Santos
// SPDX-License-Identifier: Apache-2.0

//! The current user's id and the home the account database lists for it.

#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

/// The current user id.
#[cfg(unix)]
pub fn uid() -> u32 {
    rustix::process::getuid().as_raw()
}

#[cfg(not(unix))]
pub fn uid() -> u32 {
    0
}

/// The home directory the account database lists for the current user.
///
/// There is no safe `getpwuid`, so this reads `/etc/passwd`. macOS keeps its
/// users in Directory Services, not that file, so when no line matches, a
/// directory under `/Users` or `/home` owned by this user stands in.
#[cfg(unix)]
pub fn account_home() -> Option<PathBuf> {
    let uid = uid();
    std::fs::read_to_string("/etc/passwd")
        .ok()
        .and_then(|text| home_from_passwd(&text, uid))
        .or_else(|| {
            let roots = [Path::new("/Users"), Path::new("/home")];
            home_by_ownership(&roots, uid, std::env::var("USER").ok().as_deref())
        })
}

/// The home field of the `/etc/passwd` line for `uid`.
#[cfg(unix)]
fn home_from_passwd(text: &str, uid: u32) -> Option<PathBuf> {
    text.lines().find_map(|line| {
        // name:password:uid:gid:gecos:home:shell
        let mut fields = line.split(':');
        let line_uid: u32 = fields.nth(2)?.parse().ok()?;
        let home = fields.nth(2)?;
        (line_uid == uid && !home.is_empty()).then(|| PathBuf::from(home))
    })
}

/// The one directory directly under `roots` that `uid` owns, or the one named
/// `hint` when several qualify. `Shared` is a folder macOS keeps for everyone.
#[cfg(unix)]
fn home_by_ownership(roots: &[&Path], uid: u32, hint: Option<&str>) -> Option<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    let owned: Vec<PathBuf> = roots
        .iter()
        .filter_map(|root| std::fs::read_dir(root).ok())
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name() != "Shared")
        .filter(|entry| entry.metadata().is_ok_and(|m| m.is_dir() && m.uid() == uid))
        .map(|entry| entry.path())
        .collect();
    match owned.as_slice() {
        [only] => Some(only.clone()),
        _ => owned
            .into_iter()
            .find(|path| hint.is_some_and(|name| path.file_name().is_some_and(|f| f == name))),
    }
}

#[cfg(not(unix))]
pub fn account_home() -> Option<PathBuf> {
    None
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const PASSWD: &str = "root:*:0:0:System Administrator:/var/root:/bin/sh\n\
        # a comment\n\
        ada:x:501:20:Ada:/Users/ada:/bin/zsh\n\
        nohome:x:502:20:No Home::/bin/false\n";

    #[test]
    fn the_passwd_line_for_the_uid_gives_its_home() {
        assert_eq!(
            home_from_passwd(PASSWD, 501),
            Some(PathBuf::from("/Users/ada"))
        );
        assert_eq!(
            home_from_passwd(PASSWD, 0),
            Some(PathBuf::from("/var/root"))
        );
    }

    #[test]
    fn an_unknown_uid_or_an_empty_home_gives_none() {
        assert_eq!(home_from_passwd(PASSWD, 777), None);
        assert_eq!(home_from_passwd(PASSWD, 502), None);
    }

    #[test]
    fn a_home_owned_by_the_user_is_found_without_a_passwd_line() {
        use std::os::unix::fs::MetadataExt;
        let root = std::env::temp_dir().join(format!("pb-account-{}", std::process::id()));
        std::fs::create_dir_all(root.join("me")).unwrap();
        std::fs::create_dir_all(root.join("Shared")).unwrap();
        let uid = std::fs::metadata(&root).unwrap().uid();

        assert_eq!(
            home_by_ownership(&[root.as_path()], uid, None),
            Some(root.join("me"))
        );

        std::fs::create_dir_all(root.join("also")).unwrap();
        assert_eq!(home_by_ownership(&[root.as_path()], uid, None), None);
        assert_eq!(
            home_by_ownership(&[root.as_path()], uid, Some("also")),
            Some(root.join("also"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn this_account_has_a_home_that_exists() {
        let home = account_home().expect("the account database lists a home");
        assert!(home.is_absolute());
        assert!(home.exists(), "{home:?}");
    }
}

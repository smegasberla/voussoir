//! Undoing an update that does not survive its own startup.
//!
//! Replacing the binary is safe by construction: the write is a rename, so a
//! crash mid-install leaves the old file runnable. Surviving *launch* is a
//! different question. A build that is the wrong architecture, misses a library
//! the old one bundled, or panics before the window exists replaces a working app
//! with a broken one, and no amount of care at install time can tell.
//!
//! The guard is a marker beside the binary. Installing writes it; a healthy
//! launch deletes it. Seeing it still there on a second launch means the build
//! written by the last update never got far enough to confirm, so it is replaced
//! with the copy the installer kept and the restored binary starts instead.
//!
//! Two launches, not one, because a first launch can fail for reasons that have
//! nothing to do with the update: no display, a locked profile, a kill from the
//! OS. Rolling back on the first failure would undo good updates.

use std::path::{Path, PathBuf};

/// What a launch should do about a pending update.
#[derive(Debug, PartialEq, Eq)]
pub enum Startup {
    /// No update is pending, or this is its first launch: carry on.
    Continue,
    /// The previous update never confirmed, and was restored. The caller should
    /// start `restored` and exit.
    RolledBack { restored: PathBuf, version: String },
}

/// The marker and the previous binary, both siblings of the running executable.
fn marker(target: &Path) -> PathBuf {
    with_suffix(target, ".voussoir-pending")
}

fn previous(target: &Path) -> PathBuf {
    with_suffix(target, ".voussoir-previous")
}

fn with_suffix(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|name| name.to_owned())
        .unwrap_or_else(|| "voussoir".into());
    name.push(suffix);
    target.with_file_name(name)
}

/// Record that `version` was installed, with the old binary kept at `.previous`.
///
/// Called by the installer after the new binary is in place. The marker carries
/// the attempt count so a launch can tell "first run of a new build" from "the
/// new build has now failed once".
pub fn record_pending(target: &Path, version: &str) -> std::io::Result<()> {
    let marker = marker(target);
    // A leftover marker belongs to an update that already resolved; keeping its
    // count would make the next first launch look like a second one.
    if marker.exists() {
        let _ = std::fs::remove_file(&marker);
    }
    std::fs::write(&marker, format!("{version}\n0\n"))
}

/// Decide what this launch should do, updating the marker as a side effect.
///
/// Kept free of any GUI or process work so the decision is testable: the caller
/// passes the path to the running binary and does the restart.
pub fn guard(target: &Path) -> Startup {
    let marker = marker(target);
    let Ok(text) = std::fs::read_to_string(&marker) else {
        return Startup::Continue;
    };
    let mut lines = text.lines();
    let version = lines.next().unwrap_or_default().trim().to_owned();
    let attempts: u32 = lines
        .next()
        .unwrap_or("0")
        .trim()
        .parse()
        .unwrap_or_default();

    if version.is_empty() {
        // An unreadable marker cannot describe an update to undo.
        let _ = std::fs::remove_file(&marker);
        return Startup::Continue;
    }

    // First launch since the install: count it and let the app try.
    if attempts == 0 {
        let _ = std::fs::write(&marker, format!("{version}\n1\n"));
        return Startup::Continue;
    }

    // Second launch without a confirmation: the build does not start.
    let saved = previous(target);
    if !saved.exists() {
        // Nothing to restore from (the installer could not keep a copy). Clearing
        // the marker is the honest move: repeating this decision every launch
        // would be a loop with no way out.
        let _ = std::fs::remove_file(&marker);
        return Startup::Continue;
    }

    match restore(&saved, target) {
        Ok(()) => {
            let _ = std::fs::remove_file(&marker);
            let _ = std::fs::remove_file(&saved);
            Startup::RolledBack {
                restored: target.to_path_buf(),
                version,
            }
        }
        Err(_) => {
            // The restore failed, so the broken build is still in place. Leave
            // the marker: a later launch may succeed, and a manual reinstall
            // should see that an update was pending.
            Startup::Continue
        }
    }
}

/// Put `saved` back over `target`, keeping the mode the running binary had.
fn restore(saved: &Path, target: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(target)
            .map(|metadata| metadata.permissions().mode())
            .unwrap_or(0o755);
        std::fs::set_permissions(saved, std::fs::Permissions::from_mode(mode))?;
    }
    // A rename over the target is atomic within a filesystem. On Windows the
    // running executable cannot be replaced, which is why the caller exits and
    // the restored file is started as a new process instead.
    #[cfg(unix)]
    {
        std::fs::rename(saved, target)
    }
    #[cfg(not(unix))]
    {
        std::fs::copy(saved, target).map(|_| ())
    }
}

/// The launch succeeded: the pending update is now the one to keep.
///
/// Called once the first window exists, which is the earliest point that proves
/// the binary got far enough to be useful. Deliberately not "after the event
/// loop returns": an app that crashes on interaction should still be kept, since
/// that is a bug to fix, not a broken install to undo.
pub fn confirm(target: &Path) {
    let _ = std::fs::remove_file(marker(target));
    let _ = std::fs::remove_file(previous(target));
}

/// Copy the running binary to `.previous` before it is replaced.
pub fn keep_previous(target: &Path) -> std::io::Result<()> {
    let saved = previous(target);
    let _ = std::fs::remove_file(&saved);
    std::fs::copy(target, &saved).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "voussoir-rollback-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A target binary, plus a `.previous` copy of what it replaced.
    fn installed(dir: &Path, old: &[u8], new: &[u8]) -> PathBuf {
        let target = dir.join("app");
        std::fs::write(&target, old).unwrap();
        keep_previous(&target).unwrap();
        std::fs::write(&target, new).unwrap();
        record_pending(&target, "2.0.0").unwrap();
        target
    }

    #[test]
    fn no_marker_means_nothing_to_do() {
        let dir = temp("clean");
        let target = dir.join("app");
        std::fs::write(&target, b"binary").unwrap();
        assert_eq!(guard(&target), Startup::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_first_launch_of_a_new_build_is_allowed() {
        let dir = temp("first");
        let target = installed(&dir, b"old", b"new");
        // One failure must not undo a good update: a launch can fail for reasons
        // unrelated to the binary.
        assert_eq!(guard(&target), Startup::Continue);
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_second_launch_without_confirmation_restores_the_previous_binary() {
        let dir = temp("rollback");
        let target = installed(&dir, b"old-binary", b"new-binary");
        assert_eq!(guard(&target), Startup::Continue);

        match guard(&target) {
            Startup::RolledBack { version, restored } => {
                assert_eq!(version, "2.0.0");
                assert_eq!(restored, target);
            }
            other => panic!("expected a rollback, got {other:?}"),
        }
        assert_eq!(std::fs::read(&target).unwrap(), b"old-binary");
        // The marker is gone, so the restored build is not rolled back again.
        assert_eq!(guard(&target), Startup::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_confirmed_launch_keeps_the_new_build_and_drops_the_copy() {
        let dir = temp("confirm");
        let target = installed(&dir, b"old", b"new");
        assert_eq!(guard(&target), Startup::Continue);
        confirm(&target);

        // No marker, and the old copy is gone, so nothing can roll back later.
        assert!(!with_suffix(&target, ".voussoir-pending").exists());
        assert!(!with_suffix(&target, ".voussoir-previous").exists());
        assert_eq!(guard(&target), Startup::Continue);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_marker_with_no_previous_copy_is_cleared_rather_than_looping() {
        let dir = temp("nocopy");
        let target = dir.join("app");
        std::fs::write(&target, b"new").unwrap();
        record_pending(&target, "2.0.0").unwrap();
        assert_eq!(guard(&target), Startup::Continue);
        // Without a copy there is nothing to restore, and repeating the decision
        // on every launch would be a loop the user cannot leave.
        assert_eq!(guard(&target), Startup::Continue);
        assert!(!with_suffix(&target, ".voussoir-pending").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_marker_is_ignored() {
        let dir = temp("garbage");
        let target = dir.join("app");
        std::fs::write(&target, b"new").unwrap();
        std::fs::write(with_suffix(&target, ".voussoir-pending"), b"\n\n").unwrap();
        assert_eq!(guard(&target), Startup::Continue);
        assert!(!with_suffix(&target, ".voussoir-pending").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

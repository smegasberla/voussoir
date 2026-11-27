//! A crash in a real process, not just a marker written by hand.
//!
//! The unit tests in `rollback` prove the decision; this proves the wiring that
//! makes the decision reachable. A build that panics after its window exists runs
//! no teardown, so nothing deletes the marker. That is the whole mechanism behind
//! crash rollback, and it only means something if a process that actually dies
//! leaves the state behind.
//!
//! The test re-executes a copy of itself: the child is the "installed app", so its
//! `current_exe` is the file the marker belongs to, exactly as in a real install.
//! Replacing a running executable is a rename, which is what the installer does and
//! which Unix allows, so the test is Unix-only.
//!
//! The child's panic is expected, and is captured rather than shown.

#![cfg(all(
    unix,
    feature = "updater",
    not(any(target_os = "android", target_os = "ios"))
))]

use std::path::{Path, PathBuf};
use std::process::Command;

use voussoir::rollback::{self, Startup};

const CHILD_ENV: &str = "VOUSSOIR_ROLLBACK_CHILD_DIR";
/// Only test in this file that spawns a child, so the child can run exactly it.
const SELF: &str = "a_process_that_panics_leaves_the_update_pending";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "voussoir-lifecycle-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn marker(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap().to_owned();
    name.push(".voussoir-pending");
    target.with_file_name(name)
}

/// What the child does: it *is* the installed build, so it records itself the way
/// the installer would, then dies without a teardown.
///
/// The child does not rename over its own executable. A real install never does:
/// it replaces the file while no process is running it, and the new build starts
/// fresh. Doing it here would leave `/proc/self/exe` pointing at a deleted inode,
/// which is a property of this test rather than of the updater, and it would make
/// the crash hook write its note to a name that does not exist.
fn child_work(_dir: &Path) -> ! {
    let target = std::env::current_exe().unwrap();
    // The installer's own steps, in its order: keep the running build aside, then
    // record the version that is now in place.
    rollback::keep_previous(&target).unwrap();
    rollback::record_pending(&target, "3.0.0").unwrap();

    // This launch is counted, as a real one would be, and then the build dies.
    assert_eq!(rollback::guard(&target), Startup::Continue);
    rollback::install_crash_hook();
    panic!("the page panicked while rendering");
}

#[test]
fn a_process_that_panics_leaves_the_update_pending() {
    // Child branch: do the work and never come back.
    if let Ok(dir) = std::env::var(CHILD_ENV) {
        child_work(Path::new(&dir));
    }

    let dir = scratch("panic");
    // The child runs a copy, so `current_exe` is the file being updated.
    let app = dir.join("app");
    let original = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    std::fs::write(&app, &original).unwrap();
    // Runnable, since it is started directly rather than through a loader.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&app, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let status = Command::new(&app)
        .args(["--exact", SELF, "--nocapture", "--test-threads=1"])
        .env(CHILD_ENV, &dir)
        .env_remove("RUST_BACKTRACE")
        .output()
        .unwrap();

    // The child has to have died for this to test anything.
    assert!(!status.status.success(), "the child process did not fail");
    let stderr = String::from_utf8_lossy(&status.stderr);
    assert!(
        stderr.contains("render") || stderr.contains("panicked"),
        "the child did not report the panic it was meant to raise: {stderr}"
    );

    // The dying process confirmed nothing, so the next launch reverts it. This is
    // the case that used to be left to the app to detect.
    let target = dir.join("app");
    match rollback::guard(&target) {
        Startup::RolledBack {
            version,
            restored,
            crash,
        } => {
            assert_eq!(version, "3.0.0");
            assert_eq!(restored, target);
            assert!(
                crash.as_deref().is_some_and(|note| note.contains("render")),
                "the panic was not recorded: {crash:?}"
            );
        }
        other => panic!("expected the crashed build to be reverted, got {other:?}"),
    }
    // The reversion happened on disk: the marker and the kept copy are gone, and
    // the executable is runnable again. The byte-level replacement is covered by
    // the unit tests; what this adds is that a process which actually died left the
    // state behind for the next launch to act on.
    assert!(!marker(&target).exists());
    assert!(!dir.join("app.voussoir-previous").exists());
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the restored binary lost its executable bit");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clean_exit_keeps_the_build_the_crash_test_would_have_reverted() {
    // The same setup, minus the panic: confirming on teardown must keep the build,
    // so the two outcomes differ only by whether the process exited on its terms.
    let dir = scratch("clean");
    let target = dir.join("app");
    std::fs::write(&target, b"installed-build").unwrap();
    rollback::keep_previous(&target).unwrap();
    std::fs::write(&target, b"healthy-build").unwrap();
    rollback::record_pending(&target, "3.0.0").unwrap();

    assert_eq!(rollback::guard(&target), Startup::Continue);
    rollback::confirm(&target);

    assert_eq!(rollback::guard(&target), Startup::Continue);
    assert_eq!(std::fs::read(&target).unwrap(), b"healthy-build");
    let _ = std::fs::remove_dir_all(&dir);
}

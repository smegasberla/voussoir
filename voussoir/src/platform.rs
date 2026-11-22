//! The few services that differ in kind, not just in implementation, per OS.
//!
//! Everything else in the shell goes through `wry` and `tao`, which already hide
//! the differences. Two things they do not: where an app may write, and how a
//! callback that originates outside the event loop (a Java menu click, a dialog
//! result) gets back into it. Both live here.
//!
//! The callback path needs the handle because a native layer owns its own thread
//! and calls in from whichever one it likes. `App::run` publishes a handle at
//! startup and `emit_signal` marshals back to the loop, so a callback never
//! touches a webview directly. On desktop nothing uses these, because muda and
//! tray-icon have their own handlers; they are public because a platform layer
//! outside this crate (the Android one) is the intended caller.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{AppHandle, Registry};

/// Take a lock, ignoring poisoning.
///
/// These statics hold plain data (a queue of lifecycle states, the current
/// activity, the pending pickers), so a panic while one was held leaves them
/// consistent. Propagating the poison would instead make the shell permanently
/// unusable after one unrelated panic, which is worse than continuing: the
/// `unwrap` this replaces panicked in exactly that case, and in a library a
/// single poisoned mutex should not be able to take down the app.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

static HANDLE: OnceLock<AppHandle> = OnceLock::new();
static REGISTRY: OnceLock<Arc<Registry>> = OnceLock::new();

/// Publish the handle so platform callbacks can reach the event loop.
///
/// Called once per process, from `App::run`. The first call wins: a second
/// activity in the same process must not redirect callbacks to a dead loop.
pub fn set_app_handle(handle: &AppHandle) {
    let _ = HANDLE.set(handle.clone());
}

/// Publish the registry `App::run` built, so an outer bootstrap can answer with
/// the same commands the webview uses.
///
/// The Android bridge is the reason: its `invoke` arrives on a Java thread and
/// cannot rebuild a registry, and building a second one would silently disagree
/// with the page about which commands exist.
pub fn set_registry(registry: &Arc<Registry>) {
    let _ = REGISTRY.set(Arc::clone(registry));
}

/// The live handle, if the shell has started.
pub fn handle() -> Option<&'static AppHandle> {
    HANDLE.get()
}

/// The live registry and handle together.
///
/// One function rather than two calls, because a caller that has one without the
/// other cannot answer anything: it is either fully started or it is not.
pub fn runtime() -> Option<(&'static Arc<Registry>, &'static AppHandle)> {
    Some((REGISTRY.get()?, HANDLE.get()?))
}

/// Raise the event a menu click is delivered as, from any thread.
pub fn menu_clicked(id: &str) {
    if let Some(handle) = handle() {
        handle.emit_signal(&format!("menu:{id}"));
    }
}

/// Raise the event a tray click is delivered as, from any thread.
pub fn tray_clicked(id: &str) {
    if let Some(handle) = handle() {
        handle.emit_signal(&format!("tray:{id}"));
    }
}

/// Lifecycle states that arrived before a page could receive them.
///
/// Android reports `onResume` from the activity, and the activity runs before
/// `App::run` has published a handle and before the first page can execute a
/// script. `AppHandle::emit` drops in both cases, so a state is recorded here and
/// flushed by [`page_loaded`], the first moment a script can run. Dropping one is
/// not harmless: a page that never hears `pause` keeps its timers running while
/// the app is in the background, which is the bug this exists to prevent.
#[derive(Default)]
struct LifecycleQueue {
    states: Vec<String>,
}

impl LifecycleQueue {
    /// Record a transition. An empty one names no state and is ignored rather
    /// than queued as a blank event the page cannot interpret.
    fn record(&mut self, state: &str) {
        if !state.is_empty() {
            self.states.push(state.to_owned());
        }
    }

    /// Take everything pending, oldest first, leaving the queue empty.
    ///
    /// Taking is what makes a flush idempotent: a second call sends nothing, so a
    /// page that reloads is not replayed a `pause` from ten minutes ago.
    fn drain(&mut self) -> Vec<String> {
        std::mem::take(&mut self.states)
    }
}

static LIFECYCLE: Mutex<LifecycleQueue> = Mutex::new(LifecycleQueue { states: Vec::new() });

/// Whether a page has finished loading at least once, so `emit` will land.
static PAGE_LOADED: AtomicBool = AtomicBool::new(false);

/// Record a lifecycle transition, from any thread.
///
/// `state` is `resume`, `pause`, `stop`, or `destroy`. Passed through rather than
/// matched against a list: an app that reports a state this crate has not seen is
/// better served by the page receiving it than by it silently vanishing. The page
/// hears it as `voussoir.listen("lifecycle", ({ state }) => ...)`.
pub fn lifecycle(state: &str) {
    lock(&LIFECYCLE).record(state);
    if PAGE_LOADED.load(Ordering::Acquire) {
        deliver_lifecycle();
    }
}

/// Announce that a page is ready, and hand it every state it missed.
///
/// Called from the page-load handler in `lib.rs`. Until the handle exists the
/// states stay queued, so a flush with no handle loses nothing.
pub fn page_loaded() {
    PAGE_LOADED.store(true, Ordering::Release);
    deliver_lifecycle();
}

/// Send the queued states to the page, or keep them if there is no page yet.
fn deliver_lifecycle() {
    let Some(handle) = handle() else { return };
    for state in lock(&LIFECYCLE).drain() {
        handle.emit("lifecycle", serde_json::json!({ "state": state }));
    }
}

/// A directory this app may write to, creating it if needed.
///
/// Per-platform, because "the user's data directory" is a different place and a
/// different permission model on each one. `None` means it could not be
/// determined or created, which is reported rather than guessed at.
pub fn data_dir(app_name: &str) -> Option<PathBuf> {
    let dir = platform_data_dir(app_name)?;
    match std::fs::create_dir_all(&dir) {
        Ok(()) => Some(dir),
        Err(error) => {
            crate::reporting::record(
                &format!("could not create {}: {error}", dir.display()),
                "platform",
            );
            None
        }
    }
}

#[cfg(target_os = "android")]
fn platform_data_dir(_app_name: &str) -> Option<PathBuf> {
    // Android has no per-app directory the process may choose: the context owns
    // it, and the path differs per install. `getFilesDir` is the one that needs
    // no permission and survives until uninstall, and it is already per-app, so
    // the app name plays no part in it.
    android::files_dir()
}

#[cfg(target_os = "ios")]
fn platform_data_dir(app_name: &str) -> Option<PathBuf> {
    // iOS sets HOME to the app container root inside the sandbox, and
    // Library/Application Support is the location Apple documents for
    // app-managed data. Falling through to the XDG branch below would write
    // outside the container layout the OS expects.
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support")
            .join(app_name),
    )
}

#[cfg(target_os = "macos")]
fn platform_data_dir(app_name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join("Library/Application Support")
            .join(app_name),
    )
}

#[cfg(target_os = "windows")]
fn platform_data_dir(app_name: &str) -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").or_else(|| std::env::var_os("LOCALAPPDATA"))?;
    Some(PathBuf::from(base).join(app_name))
}

#[cfg(not(any(
    target_os = "android",
    target_os = "macos",
    target_os = "windows",
    target_os = "ios"
)))]
fn platform_data_dir(app_name: &str) -> Option<PathBuf> {
    // XDG, which is also what the BSDs follow.
    let base = match std::env::var_os("XDG_DATA_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(base.join(app_name))
}

#[cfg(target_os = "android")]
pub use android::{
    complete_dialog, fail_dialog, http_get, init_activity, install_menu, pick_dialog, show_menu,
};

#[cfg(all(target_os = "android", feature = "tray"))]
pub use android::post_notification;

#[cfg(target_os = "android")]
mod android {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Mutex};
    use std::time::Duration;

    use jni::objects::{GlobalRef, JObject, JString};
    use jni::sys::jobject;
    use jni::JavaVM;

    /// How long a picker may stay open before its waiter gives up.
    ///
    /// The answer never arrives if the activity is destroyed while the picker is
    /// up, and a worker thread blocked on a channel would then leak for the rest
    /// of the process. Fifteen minutes is longer than a user plausibly spends in
    /// a file picker, and short enough that a dead activity is noticed.
    // A backstop, not the normal path: the activity answers every request it
    // accepts and refuses the ones it cannot with `fail_dialog` right away, so
    // this only fires when the activity itself went away mid-pick. It used to be
    // fifteen minutes, which is a parked worker for as long as a user keeps the
    // app open.
    const DIALOG_TIMEOUT: Duration = Duration::from_secs(2 * 60);

    /// The activity, replaced whenever the OS recreates it.
    ///
    /// `ndk_context` stores only the *application* context, which is enough for
    /// `getFilesDir` but not for anything that must be started from an activity:
    /// file pickers, `PopupMenu` anchoring, and notifications all need one. The
    /// reference is replaced on every create rather than kept, because a
    /// configuration change destroys the previous activity and calling into a
    /// destroyed one is what produces `DeadObjectException` on the Java side.
    static ACTIVITY: Mutex<Option<GlobalRef>> = Mutex::new(None);

    /// Record the activity so native calls can reach it later.
    ///
    /// Called from the app's activity-create hook, on the UI thread, with the
    /// activity alive. Runs once per activity instance, not once per process.
    pub fn init_activity(activity: GlobalRef) {
        *lock(&ACTIVITY) = Some(activity);
    }

    /// The activity currently in front, if the hook has run.
    fn activity() -> Option<GlobalRef> {
        lock(&ACTIVITY).clone()
    }

    /// Run `work` with the Java VM attached to this thread.
    ///
    /// Attaching is cheap when the thread is already attached, which it is for
    /// anything the UI thread calls in on; the guard detaches a thread we were
    /// the first to attach, so a worker thread does not leak a JNI attachment.
    fn with_env<T>(work: impl FnOnce(&mut jni::JNIEnv) -> Option<T>) -> Option<T> {
        let context = ndk_context::android_context();
        let raw = context.vm().cast::<jni::sys::JavaVM>();
        // SAFETY: tao initialised this context in `onCreate` and the VM outlives
        // every activity, so the pointer names a live VM for the process.
        let vm = unsafe { JavaVM::from_raw(raw) }.ok()?;
        let mut env = vm.attach_current_thread().ok()?;
        work(&mut env)
    }

    /// The application context tao stored, borrowed for the duration of a call.
    fn context<'local>() -> JObject<'local> {
        let raw = ndk_context::android_context().context() as jobject;
        // SAFETY: a global reference owned by tao's static; reading it does not
        // consume it and nothing here deletes it.
        unsafe { JObject::from_raw(raw) }
    }

    /// `Context.getFilesDir().getAbsolutePath()`, the app's private data root.
    pub fn files_dir() -> Option<PathBuf> {
        with_env(|env| {
            let context = context();
            let file = env
                .call_method(&context, "getFilesDir", "()Ljava/io/File;", &[])
                .ok()?
                .l()
                .ok()?;
            let path = env
                .call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])
                .ok()?
                .l()
                .ok()?;
            let path: String = env.get_string(&JString::from(path)).ok()?.into();
            Some(PathBuf::from(path))
        })
    }

    /// Run `work` with an environment and the current activity.
    ///
    /// Only the activity can start a file picker, show a `PopupMenu`, or post a
    /// notification, so every service below goes through here rather than keeping
    /// a second copy of the reference. `None` means the hook has not run, which
    /// is a state the caller reports rather than blocks on.
    fn with_activity<T>(work: impl FnOnce(&mut jni::JNIEnv, &JObject) -> Option<T>) -> Option<T> {
        let activity = activity()?;
        with_env(|env| work(env, activity.as_obj()))
    }

    /// Picker requests waiting for an answer, keyed by the id the activity sees.
    ///
    /// The id is assigned here rather than by the activity so the entry can be
    /// registered *before* the picker is shown: an answer then can never arrive
    /// with nobody listening for it. The answer is a `Result` because the
    /// activity can refuse to show a picker at all (only one may be outstanding
    /// at a time), and that is not the same thing as the user cancelling.
    static PENDING: Mutex<Vec<(u64, mpsc::Sender<Result<Option<String>, String>>)>> =
        Mutex::new(Vec::new());
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    /// Show a picker described by `request` and block until the user answers.
    ///
    /// `request` is JSON: the payload the desktop dialogs accept plus a `kind` of
    /// `open`, `open_many`, `save`, or `folder`. `Ok(None)` is a cancel, or a
    /// picker that never answered because its activity went away; `Err` means the
    /// activity could not even be asked, which is a state worth reporting rather
    /// than presenting to the user as "you cancelled".
    ///
    /// Blocking, so only call it from an async command's worker thread: the UI
    /// thread has to stay free to deliver the answer.
    pub fn pick_dialog(request: &str) -> Result<Option<String>, String> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::channel();
        lock(&PENDING).push((id, sender));

        let asked = with_activity(|env, activity| {
            let request = env.new_string(request).ok()?;
            env.call_method(
                activity,
                "voussoirDialog",
                "(JLjava/lang/String;)V",
                &[jni::objects::JValue::Long(id as i64), (&request).into()],
            )
            .ok()?;
            Some(())
        });

        let answer = match asked {
            // A timeout is a cancel: the page is told the user did not pick
            // anything, because there is no value to report and no error the
            // page could act on.
            Some(()) => receiver.recv_timeout(DIALOG_TIMEOUT).unwrap_or(Ok(None)),
            None => Err("no activity to show the dialog from".to_owned()),
        };
        // Drop the entry whether it was answered or not: a timeout leaves it
        // behind, and `complete_dialog` has already removed an answered one.
        lock(&PENDING).retain(|(pending, _)| *pending != id);
        answer
    }

    /// Hand a picker's answer to whoever is waiting for it.
    ///
    /// Called from the activity, on the UI thread, when the picker returns.
    /// `response` is the desktop dialogs' JSON shape, or `None` for cancel. An
    /// unknown or already-answered id is ignored: a recreated activity can
    /// deliver a result whose waiter has already given up.
    pub fn complete_dialog(id: u64, response: Option<String>) {
        finish_dialog(id, Ok(response));
    }

    /// Refuse a picker the activity could not show at all.
    ///
    /// Separate from [`complete_dialog`] because "no picker appeared" is not
    /// "the user picked nothing": the page gets an error it can show, rather
    /// than a `null` that reads as a cancel.
    pub fn fail_dialog(id: u64, message: &str) {
        finish_dialog(id, Err(message.to_owned()));
    }

    fn finish_dialog(id: u64, answer: Result<Option<String>, String>) {
        let waiting = {
            let mut pending = lock(&PENDING);
            pending
                .iter()
                .position(|(pending_id, _)| *pending_id == id)
                .map(|index| pending.remove(index))
        };
        if let Some((_, sender)) = waiting {
            let _ = sender.send(answer);
        }
    }

    /// Hand the app's menu to the activity, which keeps it until asked to show it.
    ///
    /// `groups` is JSON so the activity does not need to know what a `MenuGroup`
    /// is; it renders titles, labels, and the item ids it gets back. Clicks come
    /// back through [`super::menu_clicked`] from the activity's own listener.
    ///
    /// Installing and showing are separate because there is no menu bar to hang
    /// the menu on: a `PopupMenu` needs an anchor view and a moment, and neither
    /// exists while the app is starting up. So this is called once from
    /// `build_menu` and [`show_menu`] raises it when the page asks.
    pub fn install_menu(groups: &str) -> bool {
        with_activity(|env, activity| {
            let groups = env.new_string(groups).ok()?;
            env.call_method(
                activity,
                "voussoirInstallMenu",
                "(Ljava/lang/String;)V",
                &[(&groups).into()],
            )
            .ok()?;
            Some(true)
        })
        .unwrap_or(false)
    }

    /// Show the menu installed by [`install_menu`] as a popup.
    ///
    /// False means there was nothing to show or no activity to show it from;
    /// both are reported rather than treated as a click on nothing.
    pub fn show_menu() -> bool {
        with_activity(|env, activity| {
            env.call_method(activity, "voussoirShowMenu", "()V", &[])
                .ok()?;
            Some(true)
        })
        .unwrap_or(false)
    }

    /// Post the tray configuration as a notification.
    ///
    /// Android has no status area, so the tray's items become the notification's
    /// action buttons and `tooltip` its title. True means the activity accepted
    /// it, not that the user saw it: posting needs the notification permission,
    /// which is the app's to request, and a denied permission shows nothing while
    /// still returning from `notify`.
    ///
    /// Gated like the re-export above: without the `tray` feature nothing builds
    /// the JSON this sends.
    #[cfg(feature = "tray")]
    pub fn post_notification(tray: &str) -> bool {
        with_activity(|env, activity| {
            let tray = env.new_string(tray).ok()?;
            env.call_method(
                activity,
                "voussoirNotify",
                "(Ljava/lang/String;)V",
                &[(&tray).into()],
            )
            .ok()?;
            Some(true)
        })
        .unwrap_or(false)
    }

    /// Fetch a URL through the platform's HTTP stack.
    ///
    /// `HttpURLConnection` is already linked and already trusts the system roots,
    /// which a bundled TLS stack would have to be taught separately. Blocking, so
    /// only call it from a worker thread; the activity method must also stay off
    /// the UI thread for the same reason.
    pub fn http_get(url: &str) -> Option<String> {
        with_activity(|env, activity| {
            let url = env.new_string(url).ok()?;
            let body = env
                .call_method(
                    activity,
                    "voussoirHttpGet",
                    "(Ljava/lang/String;)Ljava/lang/String;",
                    &[(&url).into()],
                )
                .ok()?
                .l()
                .ok()?;
            if body.is_null() {
                return None;
            }
            let text: String = env.get_string(&JString::from(body)).ok()?.into();
            Some(text)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_handle_makes_callbacks_no_ops() {
        // Must not panic before `App::run` publishes one.
        menu_clicked("save");
        tray_clicked("quit");
        // Nothing in a test process publishes a registry, so the pair is never
        // complete: a callback arriving now has nothing to answer with.
        assert!(runtime().is_none());
    }

    #[test]
    fn the_data_dir_is_named_after_the_app() {
        let dir = platform_data_dir("voussoir-test-app").expect("a home directory should exist");
        assert_eq!(dir.file_name().unwrap(), "voussoir-test-app");
    }

    #[test]
    fn a_lifecycle_state_before_a_page_is_kept() {
        // The activity reports `resume` before `App::run` publishes a handle and
        // before the page can run a script. Dropping it there would leave the
        // page believing it was never resumed.
        let mut queue = LifecycleQueue::default();
        queue.record("resume");
        queue.record("pause");
        assert_eq!(queue.drain(), ["resume", "pause"]);
    }

    #[test]
    fn draining_empties_the_queue() {
        // A page that reloads must not be replayed states it already handled.
        let mut queue = LifecycleQueue::default();
        queue.record("pause");
        assert_eq!(queue.drain().len(), 1);
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn an_empty_state_is_not_queued() {
        let mut queue = LifecycleQueue::default();
        queue.record("");
        assert!(queue.drain().is_empty());
    }

    #[test]
    fn a_lifecycle_state_with_no_handle_is_queued_not_lost() {
        // Must not panic and must not vanish: `deliver_lifecycle` keeps it.
        lifecycle("resume");
        let queued = LIFECYCLE.lock().unwrap().drain();
        assert_eq!(queued, ["resume"]);
    }

    #[test]
    fn publishing_a_handle_is_idempotent() {
        let first = AppHandle::detached_for_tests();
        set_app_handle(&first);
        let second = AppHandle::detached_for_tests();
        set_app_handle(&second);
        assert!(handle().is_some());
    }
}

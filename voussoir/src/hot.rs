//! Dev-only hot swapping of the commands cdylib (`voussoir dev`).
//!
//! Compiled only with the `hot-reload` feature, which `voussoir build --release`
//! never enables, so none of this exists in a shipped app. Desktop only:
//! mobile platforms restrict or disallow loading dynamic code.
//!
//! The ABI this consumes is defined once, on the other side of the boundary,
//! in the scaffolded commands crate:
//!
//! ```text
//! voussoir_dispatch(method, payload) -> *mut c_char   UTF-8 envelope, heap-owned
//! voussoir_free(ptr)                                 frees it again
//! ```
//!
//! Swap safety rules, in order of priority:
//!
//! 1. A replaced library is **never unloaded**. In-flight calls finish against
//!    code that stays mapped for the life of the dev session. Correctness
//!    beats reclamation; this never ships, so the leak does not matter.
//! 2. A library that fails to load (or answers with a malformed envelope) is
//!    discarded and the previous one keeps serving, with a loud message —
//!    never a half-loaded state.
//! 3. Every call is wrapped in `catch_unwind` here, containing panics in the
//!    wrapper itself. A panicking *command* is contained inside the dylib's
//!    own `extern "C"` entry point — it must be, because unwinding out of an
//!    `extern "C"` fn aborts the process before any host-side catch could
//!    run. The scaffolded commands crate implements that inner layer and
//!    tests it; the e2e harness proves the host survives a real one.
//!
//! State rule: statics inside the commands crate are rebuilt on every reload and
//! do not survive it, because the library that owned them is unloaded. State that
//! must survive belongs to the shell, in [`crate::state`], which the reload does
//! not touch; the generation counter it reports is bumped by this module on every
//! successful swap.

use std::ffi::{c_char, CStr, CString};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use libloading::Library;
use serde_json::Value;

use crate::ipc::{error_envelope, panic_message};
use crate::Error;

/// The dispatch symbol the commands crate exports.
type DispatchFn = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
/// Its companion destructor, so no allocator crosses the boundary.
type FreeFn = unsafe extern "C" fn(*mut c_char);

/// One loaded generation of the commands library.
pub(crate) struct Live {
    dispatch: DispatchFn,
    free: FreeFn,
    generation: u64,
    /// When the rebuild that produced this generation *started* (epoch ms),
    /// so the reload report can state true rebuild-to-reload latency.
    built_at: Option<u128>,
}

struct State {
    live: Live,
    /// Every library ever loaded, newest last. Never drained.
    keep: Vec<Library>,
}

/// A hot-reloadable commands library plus its reload pointer directory.
pub struct HotLib {
    dir: Option<PathBuf>,
    state: RwLock<State>,
}

/// What `latest.json` in the hot directory says: point the shell at `path`.
struct Latest {
    path: PathBuf,
    generation: u64,
    built_at: Option<u128>,
}

impl HotLib {
    /// Open the initial library. `latest.json` wins when present (it is
    /// written before the shell starts), otherwise the direct artifact path
    /// from `VOUSSOIR_HOT_LIB` loads as generation 0.
    pub fn open(lib_path: &Path, dir: Option<PathBuf>) -> Result<Self, String> {
        let latest = dir.as_ref().and_then(|dir| read_latest(dir).ok());
        let (path, generation, built_at) = match latest {
            Some(latest) => (latest.path, latest.generation, latest.built_at),
            None => (lib_path.to_owned(), 0, None),
        };
        let (library, mut live) = open_library(&path)?;
        // The generation the pointer file announced is this load's identity;
        // `open_library` cannot know it.
        live.generation = generation;
        live.built_at = built_at;
        Ok(Self {
            dir,
            state: RwLock::new(State {
                live,
                keep: vec![library],
            }),
        })
    }

    /// Poll `latest.json` on a background thread and swap in each new
    /// generation. Spawns one thread and returns; it lives for the process.
    /// Associated (not a method) because the thread needs an owning `Arc`.
    pub fn watch(this: &Arc<Self>) {
        let Some(dir) = this.dir.clone() else { return };
        let this = Arc::clone(this);
        std::thread::spawn(move || {
            // The last stale pointer reported. A rollback attempt that sits in
            // `latest.json` is logged once, not on every poll tick.
            let mut reported_refusal: Option<(u64, Option<u128>)> = None;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(250));
                let Ok(latest) = read_latest(&dir) else {
                    continue;
                };
                // The live identity is read from the shell rather than from a
                // loop-local counter, so a swap that happened through any other
                // path is still what the decision compares against.
                let (live_generation, live_built_at) = {
                    let state = read_lock(&this.state);
                    (state.live.generation, state.live.built_at)
                };
                if !should_swap(&latest, live_generation, live_built_at) {
                    // A stale or restarted `latest.json` used to be skipped in
                    // silence, so a shell stuck on an old build looked exactly
                    // like a dev server that had stopped watching.
                    let pointer = (latest.generation, latest.built_at);
                    if latest.generation < live_generation && reported_refusal != Some(pointer) {
                        eprintln!(
                            "voussoir: ignoring hot gen {} (built_at {:?}): gen {live_generation} is already live, so a restarted latest.json cannot roll the shell back",
                            latest.generation, latest.built_at
                        );
                        reported_refusal = Some(pointer);
                    }
                    continue;
                }
                match open_library(&latest.path) {
                    Ok((library, live)) => {
                        let latency = latest.built_at.map(|built| now_ms().saturating_sub(built));
                        this.install(live, library);
                        // Kept as well as printed: `voussoir dev --verbose` is not
                        // the only consumer, and a reload that left the window
                        // blank is diagnosed from this record, not from scrollback.
                        crate::diagnostics::record(
                            crate::diagnostics::Kind::HotSwap,
                            format!("hot commands gen {} active", latest.generation),
                            serde_json::json!({
                                "gen": latest.generation,
                                "rebuild_to_reload_ms": latency,
                                "path": latest.path,
                            }),
                        );
                        // The shell-side state store outlives the library swap,
                        // so the generation it reports is the one that is live.
                        crate::state::note_reload();
                    }
                    Err(error) => eprintln!(
                        "voussoir: hot reload FAILED (gen {}): {error} — keeping gen {live_generation}",
                        latest.generation
                    ),
                }
            }
        });
    }

    /// Swap in a freshly loaded library, keeping the old one mapped forever.
    fn install(&self, live: Live, library: Library) {
        let mut state = write_lock(&self.state);
        state.keep.push(library);
        state.live = live;
    }

    /// Run one method against the current library and return its envelope.
    ///
    /// Never panics and never returns a non-envelope: a dylib that panics,
    /// returns null, or answers with garbage all become error envelopes the
    /// page can act on.
    pub fn call(&self, method: &str, payload: &Value) -> String {
        let Ok(method) = CString::new(method) else {
            return error_envelope(&Error::Handler("invalid command name".to_owned()));
        };
        // JSON never contains a raw NUL (control characters are escaped), but
        // guard anyway rather than unwrap on a path the page can reach.
        let Ok(payload) = CString::new(payload.to_string()) else {
            return error_envelope(&Error::Handler("payload is not encodable".to_owned()));
        };
        // Snapshot the function pointers and drop the lock before calling: a
        // slow command must not block the next reload, and the library behind
        // these pointers is in `keep` regardless.
        let (dispatch, free) = {
            let live = &read_lock(&self.state).live;
            (live.dispatch, live.free)
        };

        let reply = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            let ptr = dispatch(method.as_ptr(), payload.as_ptr());
            if ptr.is_null() {
                return None;
            }
            let text = CStr::from_ptr(ptr).to_string_lossy().into_owned();
            free(ptr);
            Some(text)
        }));

        match reply {
            Ok(Some(text)) => validate_envelope(&text),
            Ok(None) => error_envelope(&Error::Handler("hot command returned no reply".to_owned())),
            // The dylib catches its own panics — it has to: unwinding out of
            // an `extern "C"` fn aborts the process before any catch here
            // could run. This branch is for a panic in the wrapper itself.
            Err(panic) => error_envelope(&Error::Panic(panic_message(panic.as_ref()))),
        }
    }
}

/// A reply is an envelope or it is an error; a dylib that learned a different
/// shape between builds must not be able to feed raw text to the page.
fn validate_envelope(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(value) if value.get("ok").is_some() || value.get("error").is_some() => text.to_owned(),
        _ => {
            let mut shown: String = text.chars().take(200).collect();
            if shown.len() < text.len() {
                shown.push('…');
            }
            error_envelope(&Error::Handler(format!(
                "hot command returned a malformed reply: {shown}"
            )))
        }
    }
}

fn open_library(path: &Path) -> Result<(Library, Live), String> {
    let library =
        unsafe { Library::new(path) }.map_err(|error| format!("{}: {error}", path.display()))?;
    let (dispatch, free) = unsafe {
        // `Symbol` derefs to the function pointer; the copy detaches it from
        // the borrow of `library`, which then moves into `State.keep`.
        let dispatch: DispatchFn = *library
            .get::<DispatchFn>(b"voussoir_dispatch")
            .map_err(|error| format!("{}: missing voussoir_dispatch: {error}", path.display()))?;
        let free: FreeFn = *library
            .get::<FreeFn>(b"voussoir_free")
            .map_err(|error| format!("{}: missing voussoir_free: {error}", path.display()))?;
        (dispatch, free)
    };
    Ok((
        library,
        Live {
            dispatch,
            free,
            generation: 0,
            built_at: None,
        },
    ))
}

/// Load `latest.json`, the pointer file `voussoir dev` writes after each rebuild.
fn read_latest(dir: &Path) -> Result<Latest, String> {
    let text = std::fs::read_to_string(dir.join("latest.json"))
        .map_err(|error| format!("latest.json unreadable: {error}"))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|error| format!("latest.json invalid: {error}"))?;
    let path = value
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "latest.json has no path".to_owned())?;
    let generation = value
        .get("gen")
        .and_then(Value::as_u64)
        .ok_or_else(|| "latest.json has no gen".to_owned())?;
    Ok(Latest {
        path: PathBuf::from(path),
        generation,
        built_at: value.get("builtAt").and_then(Value::as_u64).map(u128::from),
    })
}

/// Whether the pointer file describes a library the shell does not already run.
///
/// Generations order rebuilds, so comparing them alone was wrong in both
/// directions. A lower generation is what a restarted dev server writes when its
/// counter starts over, and acting on it silently rolled the shell back to an old
/// build. The pointer's build timestamp is the tiebreaker: a strictly newer build
/// is accepted even at a lower generation, because that is a fresh compile rather
/// than a rollback. An equal generation is the build already live and is never
/// swapped in a second time.
fn should_swap(latest: &Latest, live_generation: u64, live_built_at: Option<u128>) -> bool {
    match latest.generation.cmp(&live_generation) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less => match (latest.built_at, live_built_at) {
            (Some(built), Some(live)) => built > live,
            (Some(_), None) => true,
            // No timestamp on the pointer means there is nothing that can prove
            // it is newer than the live build, so it is not acted on.
            (None, _) => false,
        },
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0)
}

fn read_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    // A panic while holding the lock would only come from a bug in this
    // module's own code; poisoning must not take the dev session down.
    lock.read().unwrap_or_else(|error| error.into_inner())
}

fn write_lock<T>(lock: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Registry;
    use serde_json::json;

    extern "C" fn ok_dispatch(_method: *const c_char, _payload: *const c_char) -> *mut c_char {
        CString::new(r#"{"ok":"from the dylib"}"#)
            .unwrap()
            .into_raw()
    }

    extern "C" fn null_dispatch(_method: *const c_char, _payload: *const c_char) -> *mut c_char {
        std::ptr::null_mut()
    }

    extern "C" fn garbage_dispatch(_method: *const c_char, _payload: *const c_char) -> *mut c_char {
        CString::new("definitely not json").unwrap().into_raw()
    }

    extern "C" fn free(ptr: *mut c_char) {
        if !ptr.is_null() {
            drop(unsafe { CString::from_raw(ptr) });
        }
    }

    fn hot_with(dispatch: DispatchFn) -> HotLib {
        HotLib {
            dir: None,
            state: RwLock::new(State {
                live: Live {
                    dispatch,
                    free,
                    generation: 1,
                    built_at: None,
                },
                keep: Vec::new(),
            }),
        }
    }

    #[test]
    fn a_loaded_library_answers_through_the_envelope() {
        let hot = hot_with(ok_dispatch);
        assert_eq!(
            hot.call("anything", &Value::Null),
            r#"{"ok":"from the dylib"}"#
        );
    }

    #[test]
    fn a_panicking_command_inside_the_dylib_arrives_as_an_envelope() {
        // Mirrors the scaffolded commands crate's contract: the panic is
        // caught *inside* the dylib's `extern "C"` entry point, because
        // unwinding across that boundary would abort the process — no
        // host-side catch exists that could survive it. The host's job is to
        // pass the envelope through untouched.
        extern "C" fn contained_dispatch(
            _method: *const c_char,
            _payload: *const c_char,
        ) -> *mut c_char {
            let body = match panic::catch_unwind(|| panic!("dylib kaboom")) {
                Err(payload) => error_envelope(&Error::Panic(panic_message(payload.as_ref()))),
                Ok(()) => error_envelope(&Error::Handler("unreachable".to_owned())),
            };
            CString::new(body).unwrap().into_raw()
        }
        let hot = hot_with(contained_dispatch);
        let body = hot.call("boom", &Value::Null);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"]["message"], "command panicked: dylib kaboom");
        assert_eq!(parsed["error"]["kind"], "panic");
        // And the same instance keeps serving the next call.
        let hot = hot_with(ok_dispatch);
        assert!(hot.call("ok", &Value::Null).contains("ok"));
    }

    #[test]
    fn a_null_reply_is_an_error_envelope_not_a_crash() {
        let hot = hot_with(null_dispatch);
        assert!(hot.call("x", &Value::Null).contains("no reply"));
    }

    #[test]
    fn a_malformed_reply_never_reaches_the_page_raw() {
        let hot = hot_with(garbage_dispatch);
        let body = hot.call("x", &Value::Null);
        let parsed: Value = serde_json::from_str(&body).unwrap();
        let message = parsed["error"]["message"].as_str().unwrap();
        assert!(message.contains("malformed reply"), "{message}");
        assert!(message.contains("definitely not json"), "{message}");
    }

    #[test]
    fn a_malformed_envelope_that_is_still_an_envelope_passes_through() {
        // A dylib may legitimately answer {"error": ...} without "ok".
        extern "C" fn err_dispatch(_method: *const c_char, _payload: *const c_char) -> *mut c_char {
            CString::new(r#"{"error":"from the dylib"}"#)
                .unwrap()
                .into_raw()
        }
        let hot = hot_with(err_dispatch);
        assert_eq!(hot.call("x", &Value::Null), r#"{"error":"from the dylib"}"#);
    }

    #[test]
    fn an_invalid_method_name_is_refused_before_the_boundary() {
        let hot = hot_with(ok_dispatch);
        let body = hot.call("bad\0name", &Value::Null);
        assert!(body.contains("invalid command name"), "{body}");
    }

    #[test]
    fn reload_pointers_are_read_from_the_directory() {
        let dir = std::env::temp_dir().join(format!("voussoir-hot-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("latest.json"),
            r#"{"gen":7,"path":"/tmp/commands-7.so","builtAt":1234}"#,
        )
        .unwrap();
        let latest = read_latest(&dir).unwrap();
        assert_eq!(latest.generation, 7);
        assert_eq!(latest.path, PathBuf::from("/tmp/commands-7.so"));
        assert_eq!(latest.built_at, Some(1234));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_or_broken_pointer_file_is_an_error_not_a_default() {
        let dir = std::env::temp_dir().join(format!("voussoir-hot-missing-{}", std::process::id()));
        assert!(read_latest(&dir).is_err());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("latest.json"), "not json").unwrap();
        assert!(read_latest(&dir).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_registry_reaches_the_hot_library_only_for_unregistered_names() {
        let mut reg = Registry::new();
        reg.add("static", |_handle, _payload| Ok(json!("static wins")));
        reg.set_hot(Arc::new(hot_with(ok_dispatch)));

        assert_eq!(
            reg.dispatch(
                &crate::AppHandle::detached_for_tests(),
                "static",
                Value::Null
            ),
            r#"{"ok":"static wins"}"#
        );
        assert_eq!(
            reg.dispatch(
                &crate::AppHandle::detached_for_tests(),
                "greet",
                Value::Null
            ),
            r#"{"ok":"from the dylib"}"#
        );
    }

    #[test]
    fn the_config_allowlist_still_narrows_the_hot_library() {
        let mut reg = Registry::new();
        reg.set_allowlist(Some(vec!["allowed".to_owned()]));
        reg.set_hot(Arc::new(hot_with(ok_dispatch)));
        let handle = crate::AppHandle::detached_for_tests();

        let allowed = reg.dispatch(&handle, "allowed", Value::Null);
        assert_eq!(allowed, r#"{"ok":"from the dylib"}"#);
        let narrowed = reg.dispatch(&handle, "other", Value::Null);
        assert!(narrowed.contains("unknown command"), "{narrowed}");
    }

    #[test]
    fn no_hot_library_means_unknown_command_exactly_as_before() {
        let reg = Registry::new();
        let body = reg.dispatch(
            &crate::AppHandle::detached_for_tests(),
            "greet",
            Value::Null,
        );
        let parsed: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["error"]["message"], "unknown command: greet");
        assert_eq!(parsed["error"]["kind"], "unknown");
    }
}

/// The pointer-file decision, testable without a dylib: the rollback bug was in
/// the comparison, not in the loading.
#[cfg(test)]
mod pointer_tests {
    use super::*;

    fn pointer(generation: u64, built_at: Option<u128>) -> Latest {
        Latest {
            path: PathBuf::from("/tmp/voussoir-commands.so"),
            generation,
            built_at,
        }
    }

    #[test]
    fn a_higher_generation_is_swapped_in() {
        assert!(should_swap(&pointer(3, None), 2, Some(100)));
    }

    #[test]
    fn a_stale_pointer_does_not_roll_the_shell_back() {
        assert!(!should_swap(&pointer(1, Some(50)), 7, Some(100)));
        assert!(!should_swap(&pointer(1, None), 7, Some(100)));
        assert!(
            !should_swap(&pointer(1, Some(100)), 7, Some(100)),
            "an equal timestamp is not newer than the live build"
        );
    }

    #[test]
    fn a_restarted_dev_server_is_accepted_on_a_newer_timestamp() {
        // The counter restarted at 1, but this build is newer than the live one,
        // so it is a fresh compile rather than a rollback.
        assert!(should_swap(&pointer(1, Some(101)), 7, Some(100)));
    }

    #[test]
    fn the_same_generation_is_not_reloaded_again() {
        assert!(!should_swap(&pointer(5, Some(100)), 5, Some(100)));
        assert!(
            !should_swap(&pointer(5, Some(999)), 5, Some(100)),
            "a rewritten pointer with the same gen must not swap again"
        );
    }
}

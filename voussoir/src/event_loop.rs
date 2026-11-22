//! The tao event loop: setup stays in `App::run`, this is the dispatch.
//!
//! Everything that can happen after the first window exists lives here —
//! window bookkeeping, keyboard, and every `UserEvent` another thread can
//! post — so the platform phases (Android's activity callbacks, tao's
//! UIKit suspend/resume mapping) have one match to extend instead of the
//! builder's run method.

use std::sync::Arc;

#[cfg(feature = "menu")]
use serde_json::Value;
use tao::{
    event::{ElementState, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopWindowTarget},
    keyboard::ModifiersState,
};

use crate::{
    is_devtools_shortcut, open_devtools, windows, AppHandle, BuilderFactory, CloseHook, UserEvent,
};

/// What the event-loop closure carries: the pieces `App::run` assembles
/// before handing control to tao.
pub(crate) struct LoopState {
    pub(crate) open: windows::Windows,
    pub(crate) modifiers: ModifiersState,
    pub(crate) spec: Arc<windows::Spec>,
    pub(crate) factory: BuilderFactory,
    pub(crate) handle: AppHandle,
    /// Runs once per window on the way out, with that window's label.
    pub(crate) closed: Option<std::sync::Arc<CloseHook>>,
    /// Which windows have already fired a lifecycle hook, so a duplicate event
    /// from the platform cannot run one twice.
    pub(crate) fired: std::sync::Arc<crate::Fired>,
    /// The labels of the open windows, shared with `AppHandle::windows` so a
    /// handler on any thread can see them. The loop is what keeps it true.
    pub(crate) open_labels: windows::OpenWindows,
    pub(crate) show_devtools: bool,
    #[cfg(feature = "menu")]
    pub(crate) menu_commands: std::collections::HashMap<String, String>,
    /// The live menu bar, if the app declared one. Kept here because the native
    /// object is not `Send` and a runtime update has to be applied on this
    /// thread.
    #[cfg(feature = "menu")]
    pub(crate) menu: Option<crate::menu::MenuHandle>,
    /// The live tray icon, same reasoning. Dropping it removes the icon, so the
    /// loop outliving it is what keeps the tray visible for the whole run.
    #[cfg(feature = "tray")]
    pub(crate) tray: Option<crate::tray::TrayHandle>,
}

/// Handle one event against the shared loop state.
pub(crate) fn dispatch(
    state: &mut LoopState,
    event: Event<'_, UserEvent>,
    event_loop: &EventLoopWindowTarget<UserEvent>,
    control_flow: &mut ControlFlow,
) {
    *control_flow = ControlFlow::Wait;
    match event {
        Event::WindowEvent {
            event: WindowEvent::CloseRequested,
            window_id,
            ..
        } => {
            if let Some(entry) = state.open.remove(&window_id) {
                state.open_labels.remove(&entry.label);
                announce_close(state, &entry.label);
            }
            if state.open.is_empty() {
                *control_flow = ControlFlow::Exit;
            }
        }
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(mods),
            ..
        } => state.modifiers = mods,
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput { event: key, .. },
            window_id,
            ..
        } => {
            if state.show_devtools
                && key.state == ElementState::Pressed
                && is_devtools_shortcut(key.physical_key, state.modifiers)
            {
                if let Some(entry) = state.open.get(&window_id) {
                    open_devtools(&entry.webview);
                }
            }
        }
        Event::UserEvent(UserEvent::Emit {
            label,
            event,
            payload,
        }) => {
            let script = emit_script(&event, &payload);
            for entry in state.open.values() {
                if label.as_ref().is_none_or(|l| *l == entry.label) {
                    if let Err(error) = entry.webview.evaluate_script(&script) {
                        eprintln!("voussoir: emit failed: {error}");
                    }
                }
            }
        }
        Event::UserEvent(UserEvent::OpenWindow(options)) => {
            match windows::create_window(event_loop, &state.spec, &options, &*state.factory) {
                Ok((id, entry)) => {
                    state.open_labels.insert(&entry.label);
                    windows::insert(&mut state.open, id, entry);
                }
                Err(error) => {
                    // The label was reserved before the loop ran; a window that
                    // never existed must not stay addressable.
                    if let Some(label) = &options.label {
                        state.open_labels.remove(label);
                    }
                    eprintln!("voussoir: could not open window: {error}");
                }
            }
        }
        Event::UserEvent(UserEvent::CloseWindow(label)) => {
            if let Some(id) = state
                .open
                .iter()
                .find(|(_, e)| e.label == label)
                .map(|(id, _)| *id)
            {
                if let Some(entry) = state.open.remove(&id) {
                    state.open_labels.remove(&entry.label);
                    announce_close(state, &entry.label);
                }
                if state.open.is_empty() {
                    *control_flow = ControlFlow::Exit;
                }
            }
        }
        Event::UserEvent(UserEvent::Tray(id)) => {
            state
                .handle
                .emit_to_all("tray", serde_json::json!({ "id": id }));
        }
        Event::UserEvent(UserEvent::Menu(id)) => {
            state
                .handle
                .emit_to_all("menu", serde_json::json!({ "id": id }));
            #[cfg(feature = "menu")]
            if let Some(command) = state.menu_commands.get(&id) {
                state.handle.run_command(command, Value::Null);
            }
            #[cfg(not(feature = "menu"))]
            let _ = &id;
        }
        Event::UserEvent(UserEvent::Eval { label, script }) => {
            for entry in state.open.values() {
                // A bridge reply is addressed to the window that asked: the
                // shim's pending map and id counter are per frame, so a
                // broadcast could settle another window's promise.
                if label.as_ref().is_some_and(|l| *l != entry.label) {
                    continue;
                }
                if let Err(error) = entry.webview.evaluate_script(&script) {
                    eprintln!("voussoir: eval failed: {error}");
                }
            }
        }
        #[cfg(feature = "tray")]
        Event::UserEvent(UserEvent::TrayUpdate(change)) => {
            crate::live::apply_tray(state.tray.as_ref(), &change);
        }
        #[cfg(feature = "menu")]
        Event::UserEvent(UserEvent::MenuUpdate(change)) => {
            crate::live::apply_menu(state.menu.as_ref(), &change);
        }
        Event::UserEvent(UserEvent::Signal(signal)) => {
            if signal == "devtools" {
                for entry in state.open.values() {
                    open_devtools(&entry.webview);
                }
            }
            state
                .handle
                .emit_to_all("signal", serde_json::json!({ "id": signal }));
        }
        // Mobile lifecycle: tao maps the UIKit callbacks onto
        // WindowEvent::Suspended/Resumed, and the page hears them as
        // `lifecycle` through the same queue Android's activity
        // callbacks use. Desktop never raises either, so this is
        // mobile-only in practice.
        #[cfg(any(target_os = "android", target_os = "ios"))]
        Event::WindowEvent {
            event: WindowEvent::Suspended,
            ..
        } => crate::platform::lifecycle("pause"),
        #[cfg(any(target_os = "android", target_os = "ios"))]
        Event::WindowEvent {
            event: WindowEvent::Resumed,
            ..
        } => crate::platform::lifecycle("resume"),
        _ => {}
    }
}

/// Tell the app a window is going away, if it asked.
///
/// A panicking hook must not take the event loop down with it: the window is
/// already gone and the user has work in the others.
fn announce_close(state: &LoopState, label: &str) {
    let Some(closed) = &state.closed else { return };
    if !state.fired.first_close(label) {
        return;
    }
    if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        closed(&state.handle, label)
    })) {
        eprintln!(
            "voussoir: on_window_close panicked: {}",
            crate::ipc::panic_message(payload.as_ref())
        );
    }
}

/// Build the `emit` script, serializing the payload at the boundary.
///
/// The payload used to be interpolated as pre-serialized text, which is only
/// safe while every `UserEvent::Emit` poster remembers to encode it. Serializing
/// here means a poster that passes raw text cannot turn it into page code: a
/// quote becomes `\"`, a `</script>` cannot close anything, and the page receives
/// exactly the JSON value it was given.
fn emit_script(event: &str, payload: &serde_json::Value) -> String {
    format!(
        "window.__voussoir_emit&&window.__voussoir_emit({},{})",
        serde_json::to_string(event).unwrap_or_else(|_| "\"\"".to_owned()),
        serde_json::to_string(payload).unwrap_or_else(|_| "null".to_owned())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_emit_payload_is_reserialized_at_the_boundary() {
        let script = emit_script("tick", &json!({ "text": "\"); alert(1); //" }));
        // The injection stays inside the JSON string: its quote is escaped, so
        // the call cannot be broken out of.
        assert_eq!(
            script,
            r#"window.__voussoir_emit&&window.__voussoir_emit("tick",{"text":"\"); alert(1); //"})"#
        );
    }

    #[test]
    fn an_emit_event_name_is_a_json_string() {
        let script = emit_script("a\"b", &json!(null));
        assert!(script.contains(r#"("a\"b",null)"#), "{script}");
    }
}

//! The pieces that assemble a window: the webview factory, the menu and tray
//! builders, the injected shim, and the devtools shortcut.
//!
//! These are the shell's internals rather than its API, so they live away from
//! the builder that calls them: `app.rs` says *what* an app asked for, this says
//! *how* the platform is made to do it.

use std::sync::Arc;

use tao::{
    keyboard::{KeyCode, ModifiersState},
    window::Window,
};
use wry::{http::Request, WebView, WebViewBuilder};

use crate::dispatch::{serve_async, serve_bridge};
use crate::hooks::ReadyHook;
#[cfg(feature = "menu")]
use crate::menu;
use crate::security::{self, APP_ORIGIN, PAGE_ORIGIN};
#[cfg(feature = "tray")]
use crate::tray;
use crate::{
    diagnostics, platform, platform_name, AppHandle, AssetTable, BuilderFactory, Error, Fired,
    Registry, SCHEME, SHIM_PROTOCOL,
};

/// The JS shim injected into every page: defines `window.voussoir`.
const SHIM: &str = include_str!("shim.js");

/// The invoke endpoint.
///
/// Android's WebView and Windows' WebView2 have no custom-scheme interception, so
/// wry rewrites `voussoir://localhost` to `http://voussoir.localhost` and intercepts that
/// instead. The shim must use the rewritten form there or its `fetch` never
/// reaches the handler.
#[cfg(any(target_os = "android", target_os = "windows"))]
pub(crate) fn invoke_base(dev: bool) -> &'static str {
    // A released page is served from the rewritten origin, so a relative path is
    // same-origin. In dev the page comes from the dev server, where only the
    // absolute rewritten URL still reaches the interceptor (CORS allows it).
    if cfg!(target_os = "windows") && dev {
        "http://voussoir.localhost/__voussoir/"
    } else {
        "/__voussoir/"
    }
}
/// Reached through the custom protocol in both modes, so the page's own origin
/// never matters on desktop.
#[cfg(not(any(target_os = "android", target_os = "windows")))]
pub(crate) fn invoke_base(_dev: bool) -> &'static str {
    "voussoir://localhost/__voussoir/"
}

/// Print the per-command latency table when a verbose dev session exits.
///
/// Printed rather than only recorded: the developer is looking at the terminal
/// when the app closes, and the buffer is about to go away with the process. The
/// table is empty when diagnostics were off, so this is a no-op in a normal run.
pub(crate) fn print_latency_summary(registry: &Registry) {
    if !diagnostics::enabled() {
        return;
    }
    let report = registry.latency_report();
    let Some(commands) = report.as_array() else {
        return;
    };
    if commands.is_empty() {
        return;
    }
    eprintln!("voussoir: command latency, slowest first:");
    for command in commands {
        eprintln!(
            "  {:<28} p50 {} ms  p95 {} ms  max {} ms  n {}",
            command["method"].as_str().unwrap_or("?"),
            command["p50_ms"],
            command["p95_ms"],
            command["max_ms"],
            command["count"],
        );
    }
}

/// Assemble the closure that builds a webview for any target URL.
pub(crate) fn build_factory(
    registry: Arc<Registry>,
    handle: AppHandle,
    assets: AssetTable,
    dev_origin: Option<String>,
    ready: Option<Arc<ReadyHook>>,
    devtools: bool,
    command_timeout_ms: u64,
) -> BuilderFactory {
    // Shared by every window this factory builds: a window's page can finish
    // loading more than once (a reload), and the hook is once per window.
    let fired = Arc::new(Fired::default());
    let shim = build_shim_with(dev_origin.as_deref(), command_timeout_ms);
    Arc::new(move |target: String, label: Option<String>| {
        // Taken before the protocol handler moves `label` into its own closure:
        // the ready hook needs the same name and the two cannot share it.
        let page_label = label.clone().unwrap_or_default();
        let nav_origin = dev_origin.clone();
        let protocol_registry = Arc::clone(&registry);
        let protocol_handle = handle.clone();
        let protocol_origin = dev_origin.clone();
        let bridge_label = label.clone();

        let builder = WebViewBuilder::new()
            // Debug builds always have devtools; a release build gets them only
            // with the `devtools` feature (or `full`).
            .with_devtools(devtools && cfg!(any(debug_assertions, feature = "devtools")))
            .with_initialization_script(&shim)
            .with_navigation_handler(move |url| {
                let allowed = security::navigation_allowed(&url, nav_origin.as_deref());
                if !allowed {
                    // The one place a page can learn why its link did nothing,
                    // and the only trace a developer gets that the policy, not
                    // the page, was what refused.
                    crate::diagnostics::record(
                        crate::diagnostics::Kind::Security,
                        "navigation refused",
                        serde_json::json!({ "url": url, "dev_origin": nav_origin }),
                    );
                }
                allowed
            })
            .with_asynchronous_custom_protocol(
                SCHEME.into(),
                move |_id, request: Request<Vec<u8>>, responder| {
                    serve_async(
                        Arc::clone(&protocol_registry),
                        protocol_handle.clone(),
                        assets,
                        protocol_origin.clone(),
                        label.clone(),
                        request,
                        responder,
                    )
                },
            )
            .with_ipc_handler({
                // Used wherever the shim was built with the bridge: Android, iOS
                // and Windows. The handler is registered everywhere so the shim's
                // choice is the only branch, not a platform `cfg` in two places.
                let registry = Arc::clone(&registry);
                let handle = handle.clone();
                move |request: Request<String>| {
                    serve_bridge(
                        &registry,
                        &handle,
                        bridge_label.as_deref(),
                        request.body().as_str(),
                    );
                }
            })
            .with_url(target);

        match &ready {
            Some(ready) => {
                let ready = Arc::clone(ready);
                let handle = handle.clone();
                let fired = Arc::clone(&fired);
                builder.with_on_page_load_handler(move |event, _url| {
                    if matches!(event, wry::PageLoadEvent::Finished) {
                        platform::page_loaded();
                        if fired.first_ready(&page_label) {
                            ready(&handle, &page_label);
                        }
                    }
                })
            }
            // Without a ready hook the handler still has work: a lifecycle state
            // reported before the page could run a script is queued, and this is
            // the moment it can be delivered.
            None => builder.with_on_page_load_handler(|event, _url| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    platform::page_loaded();
                }
            }),
        }
    })
}

#[cfg(feature = "menu")]
pub(crate) fn menu_command_map(
    groups: &[menu::MenuGroup],
) -> std::collections::HashMap<String, String> {
    groups
        .iter()
        .flat_map(|g| g.items.iter())
        .filter_map(|item| item.command.clone().map(|cmd| (item.id.clone(), cmd)))
        .collect()
}

#[cfg(feature = "menu")]
pub(crate) fn build_menu(
    groups: &[menu::MenuGroup],
    handle: &AppHandle,
    app_name: &str,
) -> Option<menu::MenuHandle> {
    let built = match menu::build(app_name, groups) {
        Ok(menu) => menu,
        Err(error) => {
            eprintln!("voussoir: menu could not be built: {error}");
            return None;
        }
    };

    // Desktop menus report clicks through muda's global handler. Android has no
    // menu bar: its menu is a Java `PopupMenu` that calls back through
    // `platform::menu_clicked`, so it only needs the groups in a form the
    // activity can render, and emits through `platform::handle` when a click
    // arrives.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let proxy = handle.clone();
        muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
            proxy.emit_signal(&format!("menu:{}", event.id.0));
        }));
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = handle;
        // There is no menu bar to hang the items on, so Android hands them over
        // once and shows them later by `__voussoir_show_menu`, when there is an
        // anchor view. iOS has no menu backend at all yet: the handle is inert.
        #[cfg(target_os = "android")]
        {
            let json = menu::to_json(&built.groups()).to_string();
            if !crate::platform::install_menu(&json) {
                eprintln!("voussoir: menu installed before the activity existed; it will not show");
            }
        }
    }

    Some(built)
}

#[cfg(feature = "tray")]
pub(crate) fn build_tray(
    config: &Option<tray::TrayConfig>,
    handle: &AppHandle,
) -> Option<tray::TrayHandle> {
    let config = config.as_ref()?;
    match tray::build(config, handle) {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("voussoir: tray could not be created: {error}");
            None
        }
    }
}

/// Open devtools if this build has them.
///
/// `wry` only exposes `open_devtools` when devtools are compiled in, which is
/// automatic in debug builds and requires the `devtools` feature in release. The
/// shell promises the shortcut, so it must not fail to compile without it; a
/// release build that opted out simply gets no devtools.
pub(crate) fn open_devtools(webview: &WebView) {
    #[cfg(any(debug_assertions, feature = "devtools"))]
    webview.open_devtools();
    #[cfg(not(any(debug_assertions, feature = "devtools")))]
    {
        let _ = webview;
        eprintln!("voussoir: devtools are not compiled into this build");
    }
}

/// F12 on Linux and Windows, Cmd-Alt-I on macOS.
pub(crate) fn is_devtools_shortcut(key: KeyCode, modifiers: ModifiersState) -> bool {
    #[cfg(target_os = "macos")]
    {
        key == KeyCode::KeyI && modifiers.super_key() && modifiers.alt_key()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = modifiers;
        key == KeyCode::F12
    }
}

/// Attach a webview to a window.
///
/// Three shapes, because the platforms genuinely differ:
///
/// - **Windows, macOS, Android, iOS** — the window is a handle the webview can
///   attach to directly, so `build` is enough.
/// - **GTK platforms** (Linux and the BSDs) — the webview is a GTK widget that
///   has to be added to the window's container explicitly.
pub(crate) fn build_webview(
    builder: WebViewBuilder<'_>,
    window: &Window,
) -> Result<WebView, Error> {
    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "android",
        target_os = "ios"
    ))]
    {
        Ok(builder.build(window)?)
    }
    #[cfg(voussoir_gtk_webview)]
    {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let container = window
            .default_vbox()
            .ok_or_else(|| Error::Runtime("window has no gtk container".to_owned()))?;
        Ok(builder.build_gtk(container)?)
    }
}

/// Build the JS shim with the origin allowlist baked in.
///
/// The allowlist is computed in Rust and injected as JSON, so the page side never
/// guesses which origins are legitimate.
pub(crate) fn build_shim_with(dev_origin: Option<&str>, timeout_ms: u64) -> String {
    // Where a custom-protocol POST cannot answer, the reply has to travel over
    // wry's IPC handler instead:
    // - Android's WebResourceRequest carries no body at all.
    // - iOS's WKURLSchemeHandler never hands the body to Rust, so the request
    //   arrives with an empty payload.
    // - Windows' WebView2 intercepts the rewritten origin but does not answer an
    //   embedded-mode POST, and the page waits on a promise that never settles.
    // All three already register the IPC handler, so this only picks the route.
    let bridge = cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_os = "windows"
    ));
    let mut origins = vec![APP_ORIGIN.to_owned()];
    if let Some(dev) = dev_origin {
        origins.push(dev.to_owned());
    }
    // Android's WebView and Windows' WebView2 rewrite the page origin to
    // http://voussoir.localhost, which wry intercepts. On Windows the rewrite is a
    // navigation to that URL, so it is the origin the page actually reports.
    #[cfg(any(target_os = "android", target_os = "windows"))]
    origins.push(crate::security::REWRITTEN_APP_ORIGIN.to_owned());
    let allowed = serde_json::to_string(&origins).unwrap_or_else(|_| "[]".to_owned());
    SHIM.replace("__INVOKE_BASE__", invoke_base(dev_origin.is_some()))
        .replace("__ALLOWED_ORIGINS__", &allowed)
        .replace("__BRIDGE__", if bridge { "true" } else { "false" })
        .replace("__PAGE_ORIGIN__", PAGE_ORIGIN)
        .replace("__PLATFORM__", platform_name())
        .replace("__SHIM_PROTOCOL__", &SHIM_PROTOCOL.to_string())
        .replace("__TIMEOUT__", &timeout_ms.to_string())
}

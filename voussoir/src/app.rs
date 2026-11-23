//! The builder and the entry point.
//!
//! `App` is the whole configuration surface: chain `.assets`, `.command`,
//! `.window`, `.on_ready`, then `.run()`. It lives in its own module so the
//! crate root can stay a short map of the shell instead of a thousand lines of
//! builder methods, and so `run` is read next to the loop it starts.

use std::sync::Arc;

use serde_json::Value;
use tao::{event::Event, event_loop::EventLoopBuilder, keyboard::ModifiersState};
// iOS builds its first window through `open_window` below, so the builder and
// the webview constructor are not on this path there. Importing them anyway is
// an unused-import error under `-D warnings`.
#[cfg(not(target_os = "ios"))]
use tao::window::WindowBuilder;

use crate::hooks::{CloseHook, Fired, ReadyHook};
#[cfg(feature = "hot-reload")]
use crate::hot;
#[cfg(feature = "menu")]
use crate::menu;
#[cfg(feature = "tray")]
use crate::tray;
#[cfg(not(target_os = "ios"))]
use crate::build_webview;
use crate::{
    build_factory, diagnostics, event_loop, live, platform, print_latency_summary,
    register_builtins, reporting, windows, AppHandle, AssetTable, Config, Error, Registry,
    UserEvent, WindowOptions,
};
#[cfg(feature = "menu")]
use crate::{build_menu, menu_command_map};
#[cfg(feature = "tray")]
use crate::build_tray;

/// The builder. Chain configuration, then `run()`.
pub struct App {
    config: Config,
    registry: Registry,
    assets: AssetTable,
    dev_url: Option<String>,
    entry: String,
    pub(crate) ready: Option<ReadyHook>,
    pub(crate) closed: Option<CloseHook>,
    window: WindowOptions,
    #[cfg(feature = "menu")]
    menus: Vec<menu::MenuGroup>,
    #[cfg(feature = "tray")]
    tray: Option<tray::TrayConfig>,
    want_devtools: bool,
    /// The hot commands library, attached by `hot_commands_from_env`.
    #[cfg(feature = "hot-reload")]
    hot: Option<std::sync::Arc<hot::HotLib>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        Self {
            config: Config::default(),
            registry: Registry::new(),
            assets: &[],
            dev_url: dev_url(),
            entry: String::new(),
            ready: None,
            closed: None,
            window: WindowOptions::default(),
            #[cfg(feature = "menu")]
            menus: Vec::new(),
            #[cfg(feature = "tray")]
            tray: None,
            want_devtools: false,
            #[cfg(feature = "hot-reload")]
            hot: None,
        }
    }

    /// Apply a `voussoir.json` string, normally via `include_str!` so the config is
    /// compiled in and cannot go missing at runtime.
    ///
    /// Invalid config is a programming error; it is reported at startup rather
    /// than silently ignored, because a wrong window title is a bug worth seeing.
    pub fn config(mut self, json: &str) -> Self {
        match Config::from_json(json) {
            Ok(config) => self.config = config,
            Err(message) => {
                eprintln!("voussoir: {message}");
                std::process::exit(1);
            }
        }
        self
    }

    /// The compile-time embedded frontend.
    pub fn assets(mut self, assets: AssetTable) -> Self {
        self.assets = assets;
        self
    }

    /// Register a command that runs on the UI thread.
    ///
    /// Keep these short: they block painting. For disk, network, or anything that
    /// can take longer than a frame, use `async_command`.
    pub fn command<F>(mut self, name: &str, handler: F) -> Self
    where
        F: Fn(&AppHandle, Value) -> Result<Value, Error> + Send + Sync + 'static,
    {
        self.registry.add(name, handler);
        self
    }

    /// Register a command whose argument and reply are real types.
    ///
    /// Identical on the wire to [`App::command`]; the difference is that a
    /// mismatched payload is rejected with `kind: "payload"` before the body
    /// runs, so a typo in the frontend is a diagnosable error rather than a
    /// string the handler has to defend against.
    ///
    /// ```ignore
    /// #[derive(serde::Deserialize)]
    /// struct GreetArgs { name: String }
    ///
    /// app.command_typed("greet", |_app, args: GreetArgs| {
    ///     Ok(format!("Hello, {}!", args.name))
    /// });
    /// ```
    pub fn command_typed<P, R, F>(mut self, name: &str, handler: F) -> Self
    where
        P: crate::serde::de::DeserializeOwned + Send + 'static,
        R: crate::serde::Serialize + 'static,
        F: Fn(&AppHandle, P) -> Result<R, Error> + Send + Sync + 'static,
    {
        self.registry.add_typed(name, handler);
        self
    }

    /// Register an `async fn` command. Requires the `rt` feature.
    ///
    /// The future runs on the shared runtime rather than a dedicated thread per
    /// call, so slow work does not block painting and does not grow the thread
    /// count either. The reply travels through the same envelope.
    #[cfg(feature = "rt")]
    pub fn async_fn<P, R, F, Fut>(mut self, name: &str, handler: F) -> Self
    where
        P: crate::serde::de::DeserializeOwned + Send + 'static,
        R: crate::serde::Serialize + Send + Sync + 'static,
        F: Fn(AppHandle, P) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<R, Error>> + Send + 'static,
    {
        self.registry.add_async_fn(name, handler);
        self
    }

    /// Register a command that runs on a worker thread.
    ///
    /// The page awaits the reply; the window keeps painting meanwhile, so a slow
    /// write or request no longer freezes the UI.
    pub fn async_command<F>(mut self, name: &str, handler: F) -> Self
    where
        F: Fn(AppHandle, Value) -> Result<Value, Error> + Send + Sync + 'static,
    {
        self.registry.add_async(name, handler);
        self
    }

    /// Install a plugin: run its `install` against this app.
    ///
    /// A plugin is an ordinary crate that implements [`crate::Plugin`]; there is
    /// no registry to update and no file to add. What it registers shares the
    /// allowlist and envelope with the app's own commands, so an allowlist can
    /// withhold a plugin's command exactly like any other.
    ///
    /// Order matters only for name collisions, since registration is last-wins.
    pub fn plugin<P: crate::Plugin>(self, plugin: P) -> Self {
        plugin.install(self)
    }

    /// Whether a command is registered under `name`.
    ///
    /// For a plugin deciding whether to claim a name the app may already use.
    /// It ignores the allowlist: a registered command that a config withholds is
    /// still registered, and answering otherwise would make the check depend on
    /// which window asked.
    pub fn has_command(&self, name: &str) -> bool {
        self.registry.contains(name)
    }

    /// Load a specific page at startup, relative to the served root.
    pub fn entry(mut self, path: &str) -> Self {
        self.entry = path.trim_start_matches('/').to_owned();
        self
    }

    /// Override the first window's label, title, size, or entry page.
    pub fn window(mut self, options: WindowOptions) -> Self {
        self.window = options;
        self
    }

    /// Run once per window, right after that window's page finishes loading.
    ///
    /// `window_label` is the label of the window that is ready, so an app that
    /// opens several can push each one its own initial state. This is the first
    /// point where `emit_to` is safe for that window: its listeners exist and
    /// the shim is installed. Fired exactly once per window, including windows
    /// opened later from a handler.
    pub fn on_ready<F>(mut self, handler: F) -> Self
    where
        F: Fn(&AppHandle, &str) + Send + Sync + 'static,
    {
        self.ready = Some(Box::new(handler));
        self
    }

    /// Run once per window when it is closing, before the webview is dropped.
    ///
    /// `window_label` is the window that is going away. Emissions sent from
    /// here are dropped with the window on purpose: use it to persist or to
    /// tell the remaining windows, not to talk to the one that is closing.
    pub fn on_window_close<F>(mut self, handler: F) -> Self
    where
        F: Fn(&AppHandle, &str) + Send + Sync + 'static,
    {
        self.closed = Some(Box::new(handler));
        self
    }

    /// Add a native menu group. Requires the `menu` feature.
    #[cfg(feature = "menu")]
    pub fn menu(mut self, group: menu::MenuGroup) -> Self {
        self.menus.push(group);
        self
    }

    /// Add a system tray icon. Requires the `tray` feature.
    #[cfg(feature = "tray")]
    pub fn tray(mut self, config: tray::TrayConfig) -> Self {
        self.tray = Some(config);
        self
    }

    /// Write crash and error reports to `path` as well as keeping them in memory.
    ///
    /// Call this before `run`. Without it reports still exist and are readable
    /// through `__voussoir_reports`, they are just not persisted.
    pub fn report_log(self, path: impl Into<std::path::PathBuf>) -> Self {
        reporting::set_log_file(path);
        self
    }

    /// Install an application sink that receives every crash and error record,
    /// in addition to the in-memory buffer and the optional log file.
    ///
    /// This is the seam for crash telemetry without forking the shell: the
    /// framework still uploads nothing by itself, an app forwards records to
    /// its own collector here. Called from the panic hook, so the sink must not
    /// panic and must not block for long.
    pub fn report_sink<F>(self, sink: F) -> Self
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        reporting::set_reporter(sink);
        self
    }

    /// Keep devtools available in release builds, bound to F12 / Cmd-Alt-I.
    pub fn devtools(mut self, enabled: bool) -> Self {
        self.want_devtools = enabled;
        self
    }

    /// Attach the hot-reloaded commands library `voussoir dev` prepares.
    ///
    /// Reads `VOUSSOIR_HOT_LIB` (the initial cdylib) and `VOUSSOIR_HOT_DIR` (where
    /// each rebuild publishes a new generation). This method only exists
    /// with the `hot-reload` feature, which `voussoir build --release` never
    /// enables — there is nothing here for a shipped app to reach. Every
    /// failure is loud on stderr: a silently dead command table would look
    /// like a bug in the app rather than in the tooling.
    #[cfg(feature = "hot-reload")]
    pub fn hot_commands_from_env(mut self) -> Self {
        let lib = std::env::var("VOUSSOIR_HOT_LIB").unwrap_or_default();
        if lib.is_empty() {
            eprintln!(
                "voussoir: `hot-reload` is on but VOUSSOIR_HOT_LIB is unset; run this app \
                 through `voussoir dev` (commands will not resolve)"
            );
            return self;
        }
        let dir = std::env::var("VOUSSOIR_HOT_DIR")
            .ok()
            .map(std::path::PathBuf::from);
        match hot::HotLib::open(std::path::Path::new(&lib), dir) {
            Ok(hot) => self.hot = Some(std::sync::Arc::new(hot)),
            Err(error) => eprintln!("voussoir: hot commands unavailable: {error}"),
        }
        self
    }

    /// Open the window(s) and run until the last one closes.
    ///
    /// In a debug build, `VOUSSOIR_DEV_URL` redirects the webview at a dev server so
    /// the frontend hot-reloads without touching Rust. Release builds ignore it.
    pub fn run(self) -> Result<(), Error> {
        reporting::install_panic_hook();
        // Read before anything else can log, so a startup denial is recorded
        // instead of only printed.
        diagnostics::init_from_env();

        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        let proxy = event_loop.create_proxy();

        let mut config = self.config;
        if let Some(entry) = self.window.entry.clone() {
            config.entry = entry;
        }
        let entry = if self.entry.is_empty() {
            config.entry.clone()
        } else {
            self.entry.clone()
        };

        let mut registry = self.registry;
        register_builtins(&mut registry, &config);
        registry.set_allowlist(config.commands.clone());
        registry.set_window_allowlist(config.window_commands.clone());
        // Now that every handler exists, say so if a config name matches none.
        registry.audit_allowlist();

        // Reloads must be running before the first window can call in.
        #[cfg(feature = "hot-reload")]
        if let Some(lib) = &self.hot {
            registry.set_hot(std::sync::Arc::clone(lib));
            hot::HotLib::watch(lib);
        }

        let registry = Arc::new(registry);
        // One registry of open labels, shared by the handle (so any handler can
        // read it) and the event loop (which is what opens and closes windows).
        let open = windows::OpenWindows::new();
        // The declarations are all a tray or menu update needs to be validated,
        // so the handle carries them; the native objects stay in the loop below.
        // With neither feature compiled in there is nothing to declare, and
        // `Declared::new` does not exist, so the empty form is used instead.
        let declared = {
            #[cfg(any(feature = "tray", feature = "menu"))]
            {
                Arc::new(live::Declared::new(
                    #[cfg(feature = "tray")]
                    self.tray.as_ref(),
                    #[cfg(feature = "menu")]
                    &self.menus,
                ))
            }
            #[cfg(not(any(feature = "tray", feature = "menu")))]
            {
                Arc::new(live::Declared::empty())
            }
        };
        let handle = AppHandle::new(
            proxy,
            Arc::clone(&registry),
            open.clone(),
            entry.clone(),
            declared,
        );

        // Published before anything can call back, and on every platform: an
        // outer bootstrap (the Android one) has to answer with the same registry
        // the webview uses, and a platform callback can arrive before the first
        // window exists.
        platform::set_registry(&registry);
        platform::set_app_handle(&handle);

        let spec = Arc::new(windows::Spec {
            entry,
            default_title: config.title.clone(),
            default_size: (config.width, config.height),
            dev_origin: self.dev_url.clone(),
        });

        let show_devtools = self.want_devtools || cfg!(debug_assertions);
        // Read before `config` is consumed below; the page's default deadline.
        let registry_timeout = config.command_timeout_ms;
        let factory = build_factory(
            Arc::clone(&registry),
            handle.clone(),
            self.assets,
            self.dev_url.clone(),
            self.ready.map(Arc::new),
            show_devtools,
            registry_timeout,
        );
        let closed = self.closed.map(Arc::new);

        // First window.
        let first = WindowOptions {
            entry: Some(spec.entry.clone()),
            title: self.window.title.clone(),
            width: self.window.width,
            height: self.window.height,
            label: self
                .window
                .label
                .clone()
                .or_else(|| Some(spec.entry.clone())),
            // The app's own window uses the app's own surface; a narrower lane
            // is something a command asks for when it opens a window itself.
            allowlist: None,
        };
        // The first window is built here rather than through `open_window`, so
        // its label is registered by hand; `insert` is idempotent, so the iOS
        // path (which does go through `open_window`) is unaffected.
        handle.window_registry().insert(&spec.label_for(&first));
        #[cfg(not(target_os = "ios"))]
        let window = WindowBuilder::new()
            .with_title(spec.title_for(&first))
            .with_inner_size(windows::size_for(&first, spec.default_size))
            .build(&event_loop)
            .map_err(|e| Error::Runtime(e.to_string()))?;
        #[cfg(not(target_os = "ios"))]
        let webview = build_webview(
            factory(
                spec.url_for(&spec.entry_for(&first)),
                Some(spec.label_for(&first)),
            ),
            &window,
        )?;

        // tao builds a window against `UIApplication`, which does not exist
        // until `event_loop.run` calls `UIApplicationMain`; iOS asks for the
        // first window through the event a `window.open` command uses and
        // gets it once the loop is running.
        #[cfg(target_os = "ios")]
        {
            // stdout, not the log: it is the only channel the simulator console
            // carries back to `voussoir ios verify`, and a run that never opened
            // a window used to look exactly like a page that stayed silent.
            match handle.open_window(first) {
                Ok(label) => println!("VOUSSOIR_IOS_WINDOW requested {label}"),
                Err(error) => {
                    println!("VOUSSOIR_IOS_WINDOW failed: {error}");
                    return Err(error);
                }
            }
        }

        // Both natives stay alive for the whole run: the menu is the bar itself,
        // and dropping the tray handle removes the icon from the status area. They
        // also have to be reachable from the loop, because a runtime update is
        // applied there rather than from the caller's thread.
        #[cfg(feature = "menu")]
        let (menu_handle, menu_commands) = {
            let built = build_menu(&self.menus, &handle, &config.name);
            #[cfg(not(target_os = "ios"))]
            let attached = built.as_ref().map(|built| built.attach(&window));
            #[cfg(not(target_os = "ios"))]
            if let Some(Err(error)) = attached {
                eprintln!("voussoir: menu could not be attached: {error}");
            }
            #[cfg(target_os = "ios")]
            let _ = &built;
            (built, menu_command_map(&self.menus))
        };

        #[cfg(feature = "tray")]
        let (tray_handle, tray_commands) = {
            let commands: std::collections::HashMap<String, String> = self
                .tray
                .as_ref()
                .map(|config| {
                    config
                        .items
                        .iter()
                        .filter_map(|item| {
                            item.command
                                .clone()
                                .map(|command| (item.id.clone(), command))
                        })
                        .collect()
                })
                .unwrap_or_default();
            (build_tray(&self.tray, &handle), commands)
        };

        let mut state = event_loop::LoopState {
            fired: std::sync::Arc::new(Fired::default()),
            open_labels: handle.window_registry(),
            open: {
                #[cfg(not(target_os = "ios"))]
                let open = {
                    let mut open = windows::Windows::new();
                    windows::insert(
                        &mut open,
                        window.id(),
                        windows::WindowEntry {
                            label: spec.label_for(&first),
                            window,
                            webview,
                        },
                    );
                    open
                };
                #[cfg(target_os = "ios")]
                let open = windows::Windows::new();
                open
            },
            modifiers: ModifiersState::empty(),
            spec,
            factory,
            handle,
            closed,
            show_devtools,
            #[cfg(feature = "menu")]
            menu_commands,
            #[cfg(feature = "tray")]
            tray_commands,
            #[cfg(feature = "menu")]
            menu: menu_handle,
            #[cfg(feature = "tray")]
            tray: tray_handle,
        };

        // `run` never returns, so the summary cannot live after it. The loop's
        // own teardown event is the last moment the totals are complete, and it
        // is also the only hook that fires on every platform.
        let summary_registry = Arc::clone(&registry);
        event_loop.run(move |event, event_loop, control_flow| {
            if matches!(event, Event::LoopDestroyed) {
                print_latency_summary(&summary_registry);
            }
            event_loop::dispatch(&mut state, event, event_loop, control_flow);
        });
    }
}

/// The platform name the page receives as `voussoir.platform`.
///
/// Shared frontend code uses it to hide affordances that cannot work: a second
/// window on Android, a file dialog on a platform without one, and so on.
pub fn platform_name() -> &'static str {
    #[cfg(target_os = "android")]
    {
        "android"
    }
    #[cfg(target_os = "ios")]
    {
        "ios"
    }
    #[cfg(target_os = "windows")]
    {
        "windows"
    }
    #[cfg(target_os = "macos")]
    {
        "macos"
    }
    #[cfg(any(
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    {
        "bsd"
    }
    #[cfg(target_os = "linux")]
    {
        "linux"
    }
}

fn dev_url() -> Option<String> {
    if cfg!(debug_assertions) {
        std::env::var("VOUSSOIR_DEV_URL")
            .ok()
            .filter(|s| !s.is_empty())
    } else {
        None
    }
}

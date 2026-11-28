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

#[cfg(feature = "tray")]
use crate::build_tray;
#[cfg(not(target_os = "ios"))]
use crate::build_webview;
use crate::hooks::{CloseHook, Fired, ReadyHook};
#[cfg(feature = "hot-reload")]
use crate::hot;
#[cfg(feature = "menu")]
use crate::menu;
#[cfg(feature = "tray")]
use crate::tray;
use crate::{
    build_factory, diagnostics, event_loop, live, platform, print_latency_summary,
    register_builtins, reporting, windows, AppHandle, AssetTable, Config, Error, Registry,
    UserEvent, WindowOptions,
};
#[cfg(feature = "menu")]
use crate::{build_menu, menu_command_map};

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
    ///
    /// The body is four named steps, in the order they have to happen, and it is the
    /// same rule `ipc` follows — reading one concern should not require scrolling past
    /// the next. The entry point was the last place in the crate breaking it.
    pub fn run(self) -> Result<(), Error> {
        install_startup_guards();
        // Before the loop, because this run may *be* the second attempt after an
        // update that never reached a window. It does not return when it fires: the
        // restored binary replaces this process instead of continuing in it.
        restore_after_a_failed_update();

        let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
        let proxy = event_loop.create_proxy();
        let shell = Shell::assemble(self, proxy);
        // Taken before the shell is consumed: the loop's teardown event prints the
        // latency summary, which lives on the registry the handlers hold.
        let summary = Arc::clone(&shell.registry);
        let natives = Natives::build(&shell, &event_loop)?;
        run_event_loop(event_loop, loop_state(shell, natives), summary);
        Ok(())
    }
}

// The pieces `App::run` is built from. The split is by concern rather than by size,
// so each one can be read without the others: what runs before anything else, what
// runs when the previous run failed, what the first window is made of, and what the
// loop does with it.

/// Install the hooks that must exist before anything can fail.
///
/// The order matters, and it is the second hook's reason for existing: the crash
/// hook is chained *after* the panic hook, so a crash is on record before the process
/// dies and the next launch can say which build stopped and why. Diagnostics come
/// last, because nothing above them should be able to log into a sink that is not
/// readable yet.
fn install_startup_guards() {
    reporting::install_panic_hook();
    #[cfg(all(
        feature = "updater",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    crate::rollback::install_crash_hook();
    // Read before anything else can log, so a startup denial is recorded instead of
    // only printed.
    diagnostics::init_from_env();
}

/// Revert an update installed by the previous run that never confirmed itself.
///
/// The guard counts launches and, on a second attempt with no confirmation, restores
/// the copy the installer kept. It is a no-op when no update is pending, so a normal
/// start pays one `exists` call.
#[cfg(all(
    feature = "updater",
    not(any(target_os = "android", target_os = "ios"))
))]
fn restore_after_a_failed_update() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let crate::rollback::Startup::RolledBack {
        restored,
        version,
        crash,
    } = crate::rollback::guard(&current)
    else {
        return;
    };
    match crash {
        Some(reason) => eprintln!(
            "voussoir: {version} stopped without exiting cleanly ({reason}); \
             restored the previous build"
        ),
        None => eprintln!("voussoir: {version} did not start; restored the previous build"),
    }
    // Start the restored binary as its own process: this one is running the build
    // that never reached a window.
    crate::updater::spawn_and_exit_for_rollback(restored);
}

/// Without the updater nothing can be pending, so there is nothing to revert.
#[cfg(not(all(
    feature = "updater",
    not(any(target_os = "android", target_os = "ios"))
)))]
fn restore_after_a_failed_update() {}

/// Everything a window needs, resolved from the builder and the configuration.
///
/// This is the seam between "what the app asked for" and "what the loop runs". The
/// builder is consumed here rather than read at the point of use, so the fields a
/// window is created from are named once instead of being reached for through `self`
/// throughout the loop's setup.
struct Shell {
    /// The app's declared name, which the menu bar takes as its title. Read only by
    /// the menu, so a shell without it does not carry the field.
    #[cfg(feature = "menu")]
    config: Config,
    closed: Option<CloseHook>,
    /// Debug builds always have devtools; a release build gets them only with the
    /// `devtools` feature. Settled here so the factory and the loop cannot disagree.
    show_devtools: bool,
    /// The menu declarations, kept because the bar is built once a window exists to
    /// attach it to.
    #[cfg(feature = "menu")]
    menus: Vec<menu::MenuGroup>,
    /// The tray declaration, kept for the same reason: the icon outlives this call and
    /// its clicks route through the loop.
    #[cfg(feature = "tray")]
    tray: Option<tray::TrayConfig>,
    /// The window the app asks for when nothing else does.
    first: WindowOptions,
    registry: Arc<Registry>,
    handle: AppHandle,
    /// Built once, after the handle exists, and shared by every window the app opens
    /// — the first included.
    factory: crate::BuilderFactory,
    spec: Arc<windows::Spec>,
}

impl Shell {
    /// Resolve the builder into what the loop needs.
    ///
    /// Everything that must be true *before the first window can call in* happens
    /// here: the entry page is settled, the built-ins and allowlists are folded into
    /// the registry, reloads are watching, and the handle is published. A platform
    /// callback can arrive before the first window exists — Android's activity start,
    /// a menu click delivered early — which is why the handle is published rather
    /// than kept local.
    fn assemble(app: App, proxy: tao::event_loop::EventLoopProxy<UserEvent>) -> Self {
        let mut config = app.config;
        if let Some(entry) = app.window.entry.clone() {
            config.entry = entry;
        }
        let entry = if app.entry.is_empty() {
            config.entry.clone()
        } else {
            app.entry.clone()
        };

        let mut registry = app.registry;
        register_builtins(&mut registry, &config);
        registry.set_allowlist(config.commands.clone());
        registry.set_window_allowlist(config.window_commands.clone());
        // Now that every handler exists, say so if a config name matches none.
        registry.audit_allowlist();

        // Reloads must be running before the first window can call in.
        #[cfg(feature = "hot-reload")]
        if let Some(lib) = &app.hot {
            registry.set_hot(Arc::clone(lib));
            hot::HotLib::watch(lib);
        }

        let registry = Arc::new(registry);
        // One registry of open labels, shared by the handle (so any handler can read
        // it) and the event loop (which is what opens and closes windows).
        let open = windows::OpenWindows::new();
        // The declarations are all a tray or menu update needs to be validated, so the
        // handle carries them; the native objects stay in the loop. With neither
        // feature compiled in there is nothing to declare, and `Declared::new` does not
        // exist, so the empty form is used instead.
        let declared = {
            #[cfg(any(feature = "tray", feature = "menu"))]
            {
                Arc::new(live::Declared::new(
                    #[cfg(feature = "tray")]
                    app.tray.as_ref(),
                    #[cfg(feature = "menu")]
                    &app.menus,
                ))
            }
            #[cfg(not(any(feature = "tray", feature = "menu")))]
            {
                Arc::new(live::Declared::empty())
            }
        };
        let handle = AppHandle::new(proxy, Arc::clone(&registry), open, entry.clone(), declared);

        // Published before anything can call back, and on every platform: an outer
        // bootstrap (the Android one) has to answer with the same registry the webview
        // uses, and a platform callback can arrive before the first window exists.
        platform::set_registry(&registry);
        platform::set_app_handle(&handle);

        let spec = Arc::new(windows::Spec {
            entry: entry.clone(),
            default_title: config.title.clone(),
            default_size: (config.width, config.height),
            dev_origin: app.dev_url.clone(),
        });

        // Debug builds always have devtools; a release build gets them only with the
        // `devtools` feature (or `full`).
        let show_devtools = app.want_devtools || cfg!(debug_assertions);
        let factory = build_factory(crate::shell::FactoryParts {
            registry: Arc::clone(&registry),
            handle: handle.clone(),
            assets: app.assets,
            dev_origin: app.dev_url.clone(),
            ready: app.ready.map(Arc::new),
            devtools: show_devtools,
            command_timeout_ms: config.command_timeout_ms,
            snapshot: crate::snapshot::Request::from_env(),
        });

        // The app's own window uses the app's own surface: a narrower allowlist is
        // something a command asks for when it opens a window itself. The label
        // defaults to the entry page, which is what `__voussoir_close` and every
        // emitted event address.
        let first = WindowOptions {
            entry: Some(entry),
            title: app.window.title.clone(),
            width: app.window.width,
            height: app.window.height,
            label: app
                .window
                .label
                .clone()
                .or_else(|| Some(spec.entry.clone())),
            allowlist: None,
        };

        Self {
            #[cfg(feature = "menu")]
            config,
            closed: app.closed,
            show_devtools,
            #[cfg(feature = "menu")]
            menus: app.menus,
            #[cfg(feature = "tray")]
            tray: app.tray,
            first,
            registry,
            handle,
            factory,
            spec,
        }
    }
}

/// The native objects a run keeps alive, and the tables their clicks resolve
/// through.
///
/// The menu and the tray have to outlive the first window: the menu *is* the bar, and
/// dropping the tray handle removes the icon from the status area. They also have to
/// reach the loop, because a runtime update is applied there rather than from the
/// caller's thread.
struct Natives {
    /// Every open window, keyed by tao's id.
    open: windows::Windows,
    #[cfg(feature = "menu")]
    menu: Option<menu::MenuHandle>,
    #[cfg(feature = "menu")]
    menu_commands: std::collections::HashMap<String, String>,
    #[cfg(feature = "tray")]
    tray: Option<tray::TrayHandle>,
    #[cfg(feature = "tray")]
    tray_commands: std::collections::HashMap<String, String>,
}

impl Natives {
    /// Build the first window, its webview, then the natives that hang off it.
    ///
    /// The order is forced by what each one needs: a webview attaches to a window, a
    /// menu attaches to a window, and a tray only has to outlive one.
    fn build(
        shell: &Shell,
        event_loop: &tao::event_loop::EventLoop<UserEvent>,
    ) -> Result<Self, Error> {
        let (mut open, window) = first_window(shell, event_loop)?;
        let label = shell.spec.label_for(&shell.first);
        let webview = match &window {
            Some(window) => Some(build_webview(
                (shell.factory)(
                    shell.spec.url_for(&shell.spec.entry_for(&shell.first)),
                    Some(label.clone()),
                ),
                window,
            )?),
            None => None,
        };

        // Attaching a menu is a window operation on Linux and Windows, and a no-op on
        // macOS (the bar belongs to the app) and iOS.
        #[cfg(feature = "menu")]
        let (menu, menu_commands) = {
            let built = build_menu(&shell.menus, &shell.handle, &shell.config.name);
            if let (Some(built), Some(window)) = (&built, &window) {
                if let Err(error) = built.attach(window) {
                    eprintln!("voussoir: menu could not be attached: {error}");
                }
            }
            (built, menu_command_map(&shell.menus))
        };

        #[cfg(feature = "tray")]
        let (tray, tray_commands) = {
            let commands = shell
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
            (build_tray(&shell.tray, &shell.handle), commands)
        };

        if let (Some(window), Some(webview)) = (window, webview) {
            windows::insert(
                &mut open,
                window.id(),
                windows::WindowEntry {
                    label,
                    window,
                    webview,
                },
            );
        }

        Ok(Self {
            open,
            #[cfg(feature = "menu")]
            menu,
            #[cfg(feature = "menu")]
            menu_commands,
            #[cfg(feature = "tray")]
            tray,
            #[cfg(feature = "tray")]
            tray_commands,
        })
    }
}

/// Build the app's first window, or put in a request for one where it cannot exist
/// yet.
///
/// Returns the window when this platform builds it before the loop runs, and `None`
/// on iOS, where tao builds against `UIApplication` — which does not exist until
/// `event_loop.run` calls `UIApplicationMain`. There the window is requested through
/// the same event a `window.open` command uses, and arrives once the loop is running.
///
/// On every other platform the window is built here rather than through
/// `open_window`, so its label is registered by hand. Not on iOS: that call reserves
/// the label itself and refuses one that is already open, and registering it first
/// made the app reject its own first window, so no page ever loaded.
#[cfg(not(target_os = "ios"))]
fn first_window(
    shell: &Shell,
    event_loop: &tao::event_loop::EventLoop<UserEvent>,
) -> Result<(windows::Windows, Option<tao::window::Window>), Error> {
    let spec = &shell.spec;
    let first = &shell.first;
    shell
        .handle
        .window_registry()
        .insert(&spec.label_for(first));
    let window = WindowBuilder::new()
        .with_title(spec.title_for(first))
        .with_inner_size(windows::size_for(first, spec.default_size))
        .build(event_loop)
        .map_err(|e| Error::Runtime(e.to_string()))?;
    Ok((windows::Windows::new(), Some(window)))
}

#[cfg(target_os = "ios")]
fn first_window(
    shell: &Shell,
    _event_loop: &tao::event_loop::EventLoop<UserEvent>,
) -> Result<(windows::Windows, Option<tao::window::Window>), Error> {
    shell
        .handle
        .open_window(shell.first.clone())
        .map_err(|e| Error::Runtime(e.to_string()))?;
    Ok((windows::Windows::new(), None))
}

/// The loop state both halves of setup feed into.
fn loop_state(shell: Shell, natives: Natives) -> event_loop::LoopState {
    event_loop::LoopState {
        fired: Arc::new(Fired::default()),
        open_labels: shell.handle.window_registry(),
        open: natives.open,
        modifiers: ModifiersState::empty(),
        spec: shell.spec,
        factory: shell.factory,
        handle: shell.handle,
        closed: shell.closed.map(Arc::new),
        show_devtools: shell.show_devtools,
        #[cfg(feature = "menu")]
        menu_commands: natives.menu_commands,
        #[cfg(feature = "tray")]
        tray_commands: natives.tray_commands,
        #[cfg(feature = "menu")]
        menu: natives.menu,
        #[cfg(feature = "tray")]
        tray: natives.tray,
    }
}

/// Run the tao loop until the last window closes.
fn run_event_loop(
    event_loop: tao::event_loop::EventLoop<UserEvent>,
    mut state: event_loop::LoopState,
    summary: Arc<Registry>,
) {
    // The loop never returns, so the summary cannot live after it. The loop's own
    // teardown event is the last moment the totals are complete, and it is also the
    // only hook that fires on every platform.
    #[cfg(all(
        feature = "updater",
        not(any(target_os = "android", target_os = "ios"))
    ))]
    let pending = std::env::current_exe().ok();
    event_loop.run(move |event, event_loop, control_flow| {
        if matches!(event, Event::LoopDestroyed) {
            print_latency_summary(&summary);
            // The pending update is confirmed here, not when the first window
            // appears: only a deliberate shutdown proves the build runs. A build that
            // crashes during use never reaches this, so its marker survives and the
            // next launch reverts it.
            #[cfg(all(
                feature = "updater",
                not(any(target_os = "android", target_os = "ios"))
            ))]
            if let Some(target) = &pending {
                crate::rollback::confirm(target);
            }
        }
        event_loop::dispatch(&mut state, event, event_loop, control_flow);
    });
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

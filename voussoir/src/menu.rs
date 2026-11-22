//! Optional native menu bar.
//!
//! Builds the menu a desktop app is expected to have — Cut/Copy/Paste/Select All
//! and Quit from the platform, plus app-defined items — and turns clicks into
//! events the page receives as `voussoir.listen("menu", ({id}) => ...)`.
//!
//! Predefined items are used wherever possible because they carry the platform's
//! own labels and shortcuts, which is also what makes Cmd-C and Ctrl-C work in the
//! webview without any key handling of ours.
//!
//! Android has no menu bar and no system edit shortcuts to borrow: the platform
//! expects a navigation drawer or an overflow button, both of which are app
//! chrome rather than OS chrome. So the same `MenuGroup`s are kept and handed to
//! the activity, which shows them as a `PopupMenu`; nothing is attached to the
//! window, and `accelerator` is documentation only there.

use crate::WindowOptions;

/// One app-defined menu item.
#[derive(Debug, Clone)]
pub struct MenuItem {
    /// Identifier sent to the page when the item is clicked.
    pub id: String,
    pub label: String,
    /// Optional accelerator, e.g. `"CmdOrCtrl+S"`.
    pub accelerator: Option<String>,
    /// Optional command run when the item is clicked, so a menu item can be
    /// wired straight to a registered command.
    pub command: Option<String>,
    /// Whether the item is a check item, and its initial state.
    ///
    /// `None` is a plain item. Toggling a plain item is reported as an error
    /// rather than silently doing nothing, because a menu that ignores a check
    /// looks broken instead of unresponsive.
    pub checked: Option<bool>,
}

impl MenuItem {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            accelerator: None,
            command: None,
            checked: None,
        }
    }

    pub fn accelerator(mut self, accel: &str) -> Self {
        self.accelerator = Some(accel.to_owned());
        self
    }

    pub fn command(mut self, command: &str) -> Self {
        self.command = Some(command.to_owned());
        self
    }

    /// Declare this item as a check item, initially checked.
    pub fn check(mut self) -> Self {
        self.checked = Some(true);
        self
    }

    /// Declare this item as a check item with an explicit initial state.
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }
}

/// A named top-level menu holding app items.
#[derive(Debug, Clone)]
pub struct MenuGroup {
    pub title: String,
    pub items: Vec<MenuItem>,
}

impl MenuGroup {
    pub fn new(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            items: Vec::new(),
        }
    }

    pub fn item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }
}

/// The groups as the activity renders them: a title and its items per group.
///
/// JSON rather than a JNI call per field, because the activity builds a
/// `PopupMenu` out of this and has no reason to know what a `MenuGroup` is.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn to_json(groups: &[MenuGroup]) -> serde_json::Value {
    serde_json::json!(groups
        .iter()
        .map(|group| {
            serde_json::json!({
                "title": group.title,
                "items": group
                    .items
                    .iter()
                    .map(|item| serde_json::json!({ "id": item.id, "label": item.label }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>())
}

/// A built menu, kept alive for as long as the app runs.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub struct MenuHandle {
    pub(crate) menu: muda::Menu,
    /// Check items by id, so a runtime toggle does not walk the whole bar.
    checks: std::collections::HashMap<String, muda::CheckMenuItem>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl MenuHandle {
    /// Attach the menu to a window.
    ///
    /// On Linux it goes into the window's GTK container; on Windows it becomes
    /// the window's menu bar; on macOS it becomes the application menu, which is
    /// a single global menu rather than a per-window one.
    pub fn attach(&self, window: &tao::window::Window) -> Result<(), String> {
        #[cfg(voussoir_gtk_menu)]
        {
            use tao::platform::unix::WindowExtUnix;

            // The menu goes into the window's own vertical box, not the bare
            // `GtkWindow`. `init_for_gtk_window(window, None)` hands muda the window
            // itself, and GTK permits one child per `GtkBin`: tao has already put a
            // box there for the webview, so the bar is refused with "attempting to
            // add a widget with type GtkMenuBar to a GtkApplicationWindow" and the
            // window never maps. Passing the box is what makes the bar a sibling of
            // the webview instead of a second child of the window.
            let container = window.default_vbox();
            self.menu
                .init_for_gtk_window(window.gtk_window(), container)
                .map_err(|e| e.to_string())?;
        }
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::WindowExtWindows;
            // SAFETY: the handle is the window's own and outlives this call.
            unsafe {
                self.menu
                    .init_for_hwnd(window.hwnd() as isize)
                    .map_err(|e| e.to_string())?;
            }
        }
        #[cfg(target_os = "macos")]
        {
            // `init_for_nsapp` installs the whole bar, and the first submenu of a
            // macOS menu bar *is* the application menu. There is no per-window
            // attach on macOS; the window argument is only meaningful elsewhere.
            self.menu.init_for_nsapp();
        }
        let _ = window;
        Ok(())
    }

    /// Check or uncheck one app-defined item.
    ///
    /// Runs on whichever thread muda's menu lives on, which is the UI thread;
    /// `AppHandle::set_menu_checked` queues it there rather than calling this
    /// from a handler.
    pub fn set_checked(&self, id: &str, checked: bool) -> Result<(), String> {
        match self.checks.get(id) {
            Some(item) => {
                item.set_checked(checked);
                Ok(())
            }
            None => Err(format!(
                "menu item {id:?} is not a check item; declare it with .check() or .checked(..)"
            )),
        }
    }
}

/// A built menu on mobile, which is the groups themselves.
///
/// There is nothing to build on Android: a `PopupMenu` is constructed per
/// display, from the activity, when the app asks for it. iOS has no menu
/// system at all; `build` still succeeds so an app declaring menus on every
/// platform compiles, and keeping the groups here means an app can declare
/// one menu and get the platform's own presentation where one exists.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub struct MenuHandle {
    /// Behind a lock because a runtime toggle has to edit the groups the activity
    /// renders, which happens after the menu was handed over.
    groups: std::sync::Mutex<Vec<MenuGroup>>,
}

#[cfg(any(target_os = "android", target_os = "ios"))]
impl MenuHandle {
    /// No-op: mobile menus are not attached to anything.
    pub fn attach(&self, _window: &tao::window::Window) -> Result<(), String> {
        Ok(())
    }

    /// The declared groups, for the activity to render.
    pub fn groups(&self) -> Vec<MenuGroup> {
        self.groups
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Check or uncheck one item and re-hand the groups to the activity, because
    /// a `PopupMenu` is rebuilt from them on every display.
    pub fn set_checked(&self, id: &str, checked: bool) -> Result<(), String> {
        let mut groups = self
            .groups
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut found = false;
        for group in groups.iter_mut() {
            for item in group.items.iter_mut() {
                if item.id == id {
                    if item.checked.is_none() {
                        return Err(format!(
                            "menu item {id:?} is not a check item; declare it with .check() or .checked(..)"
                        ));
                    }
                    item.checked = Some(checked);
                    found = true;
                }
            }
        }
        if !found {
            return Err(format!("no menu item is named {id:?}"));
        }
        let json = to_json(&groups).to_string();
        drop(groups);
        crate::platform::install_menu(&json);
        Ok(())
    }
}

/// Create a menu with the platform-standard edit items plus the given groups.
///
/// `app_name` is used for the macOS application menu's title, About item, and
/// Quit item; other platforms use it only to label the quit entry.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn build(_app_name: &str, groups: &[MenuGroup]) -> Result<MenuHandle, String> {
    validate(groups)?;
    Ok(MenuHandle {
        groups: std::sync::Mutex::new(groups.to_vec()),
    })
}

/// Create a menu with the platform-standard edit items plus the given groups.
///
/// `app_name` is used for the macOS application menu's title, About item, and
/// Quit item; other platforms use it only to label the quit entry.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn build(app_name: &str, groups: &[MenuGroup]) -> Result<MenuHandle, String> {
    use muda::{Menu, MenuItem as MudaItem, PredefinedMenuItem, Submenu};

    validate(groups)?;

    let menu = Menu::new();

    // On macOS the first submenu is the application menu: it holds About,
    // Services, Hide, and Quit, and the OS expects it before anything else.
    #[cfg(target_os = "macos")]
    {
        let app_menu = Submenu::new(app_name, true);
        for item in [
            PredefinedMenuItem::about(Some(&format!("About {app_name}")), None),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::services(Some("Services")),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::hide(Some(&format!("Hide {app_name}"))),
            PredefinedMenuItem::hide_others(Some("Hide Others")),
            PredefinedMenuItem::separator(),
            PredefinedMenuItem::quit(Some(&format!("Quit {app_name}"))),
        ] {
            app_menu.append(&item).map_err(|e| e.to_string())?;
        }
        menu.append(&app_menu).map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app_name;

    let edit = Submenu::new("Edit", true);
    for item in [
        PredefinedMenuItem::cut(Some("Cut")),
        PredefinedMenuItem::copy(Some("Copy")),
        PredefinedMenuItem::paste(Some("Paste")),
        PredefinedMenuItem::select_all(Some("Select All")),
    ] {
        edit.append(&item).map_err(|e| e.to_string())?;
    }
    menu.append(&edit).map_err(|e| e.to_string())?;

    let mut checks = std::collections::HashMap::new();
    for group in groups {
        let submenu = Submenu::new(&group.title, true);
        for item in &group.items {
            let accel = item.accelerator.as_deref().and_then(parse_accelerator);
            match item.checked {
                Some(checked) => {
                    let menu_item =
                        muda::CheckMenuItem::with_id(&item.id, &item.label, true, checked, accel);
                    submenu.append(&menu_item).map_err(|e| e.to_string())?;
                    checks.insert(item.id.clone(), menu_item);
                }
                None => {
                    let menu_item = MudaItem::with_id(&item.id, &item.label, true, accel);
                    submenu.append(&menu_item).map_err(|e| e.to_string())?;
                }
            }
        }
        menu.append(&submenu).map_err(|e| e.to_string())?;
    }

    // Quit is a top-level item outside macOS, where it does not belong to an
    // application menu.
    #[cfg(not(target_os = "macos"))]
    {
        menu.append(&PredefinedMenuItem::separator())
            .map_err(|e| e.to_string())?;
        menu.append(&PredefinedMenuItem::quit(Some(&format!("Quit {app_name}"))))
            .map_err(|e| e.to_string())?;
    }

    Ok(MenuHandle { menu, checks })
}

/// Reject declarations that would make a menu broken or ambiguous.
///
/// Duplicate ids are the dangerous case: both items report the same event, the
/// check map keeps only the last, and the page cannot tell which was clicked.
/// An accelerator muda cannot parse is worse than no accelerator, because it
/// looks declared and does nothing.
pub fn validate(groups: &[MenuGroup]) -> Result<(), String> {
    let mut seen = std::collections::HashMap::new();
    for group in groups {
        if group.title.trim().is_empty() {
            return Err("a menu group needs a title".to_owned());
        }
        for item in &group.items {
            if item.id.trim().is_empty() {
                return Err(format!("menu item {:?} has an empty id", item.label));
            }
            if seen.insert(item.id.clone(), ()).is_some() {
                return Err(format!(
                    "menu items share the id {:?}; each item needs its own",
                    item.id
                ));
            }
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            if let Some(accelerator) = &item.accelerator {
                if parse_accelerator(accelerator).is_none() {
                    return Err(format!(
                        "menu item {:?} has an accelerator muda cannot parse: {accelerator:?}",
                        item.id
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Parse an accelerator string, tolerating the two spellings muda accepts.
///
/// Android has no accelerators: a `PopupMenu` shows labels only, so the strings
/// stay as documentation instead of becoming key bindings.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn parse_accelerator(text: &str) -> Option<muda::accelerator::Accelerator> {
    use std::str::FromStr;
    muda::accelerator::Accelerator::from_str(text).ok()
}

/// The payload the page receives when a menu item is clicked.
pub fn event_payload(id: &str) -> serde_json::Value {
    serde_json::json!({ "id": id })
}

/// Options for a window opened from a menu item.
pub fn window_options(title: &str) -> WindowOptions {
    WindowOptions {
        title: Some(title.to_owned()),
        ..WindowOptions::default()
    }
}

/// Accelerators declared by a set of groups, for the unit tests and for docs.
pub fn accelerators(groups: &[MenuGroup]) -> Vec<(String, String)> {
    groups
        .iter()
        .flat_map(|g| g.items.iter())
        .filter_map(|item| {
            item.accelerator
                .as_ref()
                .map(|accel| (item.id.clone(), accel.clone()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group() -> MenuGroup {
        MenuGroup::new("File")
            .item(
                MenuItem::new("save", "Save")
                    .accelerator("CmdOrCtrl+S")
                    .command("save"),
            )
            .item(MenuItem::new("open", "Open...").accelerator("CmdOrCtrl+O"))
    }

    #[test]
    fn groups_and_items_chain_in_order() {
        let g = group();
        assert_eq!(g.title, "File");
        assert_eq!(g.items.len(), 2);
        assert_eq!(g.items[0].id, "save");
        assert_eq!(g.items[0].command.as_deref(), Some("save"));
        assert_eq!(g.items[1].accelerator.as_deref(), Some("CmdOrCtrl+O"));
        assert!(g.items[1].command.is_none());
    }

    #[test]
    fn duplicate_and_empty_ids_are_refused() {
        let duplicate = [
            MenuGroup::new("File").item(MenuItem::new("save", "Save")),
            MenuGroup::new("Edit").item(MenuItem::new("save", "Save again")),
        ];
        let error = validate(&duplicate).unwrap_err();
        assert!(error.contains("share the id"), "{error}");

        let empty = [MenuGroup::new("File").item(MenuItem::new("", "Nameless"))];
        assert!(validate(&empty).unwrap_err().contains("empty id"));

        let untitled = [MenuGroup::new("  ").item(MenuItem::new("a", "A"))];
        assert!(validate(&untitled).unwrap_err().contains("title"));
        assert!(validate(&[group()]).is_ok());
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn an_unparsable_accelerator_is_reported() {
        let groups = [MenuGroup::new("File")
            .item(MenuItem::new("save", "Save").accelerator("CmdOrCtrl+NotAKey"))];
        let error = validate(&groups).unwrap_err();
        assert!(error.contains("cannot parse"), "{error}");
    }

    #[test]
    fn check_items_are_declared_explicitly() {
        let g = MenuGroup::new("View")
            .item(MenuItem::new("wrap", "Wrap Lines").check())
            .item(MenuItem::new("grid", "Show Grid").checked(false))
            .item(MenuItem::new("save", "Save"));
        assert_eq!(g.items[0].checked, Some(true));
        assert_eq!(g.items[1].checked, Some(false));
        assert_eq!(
            g.items[2].checked, None,
            "an item without .check() stays plain, so ticking it can be reported"
        );
    }

    /// The mobile handle keeps the groups and can toggle one, which is what
    /// stands in for a native menu on Android and iOS.
    #[cfg(any(target_os = "android", target_os = "ios"))]
    #[test]
    fn a_mobile_menu_toggle_updates_the_groups() {
        let built = build(
            "Notes",
            &[MenuGroup::new("View")
                .item(MenuItem::new("wrap", "Wrap Lines").check())
                .item(MenuItem::new("save", "Save"))],
        )
        .expect("a menu without accelerators always builds");

        built
            .set_checked("wrap", false)
            .expect("declared check item");
        let groups = built.groups();
        assert_eq!(groups[0].items[0].checked, Some(false));

        // A plain item cannot be ticked, and an unknown id is named in the error
        // rather than silently ignored.
        let error = built.set_checked("save", true).unwrap_err();
        assert!(error.contains("not a check item"), "{error}");
        let error = built.set_checked("nope", true).unwrap_err();
        assert!(error.contains("no menu item"), "{error}");
    }

    #[test]
    fn accelerators_are_collected() {
        let accels = accelerators(&[group()]);
        assert_eq!(accels.len(), 2);
        assert_eq!(accels[0], ("save".to_owned(), "CmdOrCtrl+S".to_owned()));
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[test]
    fn shortcut_strings_parse() {
        assert!(parse_accelerator("CmdOrCtrl+S").is_some());
        assert!(parse_accelerator("Command+Shift+Z").is_some());
        assert!(parse_accelerator("Ctrl+Alt+Delete").is_some());
        // Garbage must not panic, just not produce an accelerator.
        assert!(parse_accelerator("nonsense+++").is_none());
    }

    #[test]
    fn the_event_payload_names_the_item() {
        assert_eq!(event_payload("save"), serde_json::json!({ "id": "save" }));
    }

    #[test]
    fn menu_windows_carry_the_given_title() {
        let options = window_options("Settings");
        assert_eq!(options.title.as_deref(), Some("Settings"));
        assert!(options.entry.is_none());
    }

    #[test]
    #[cfg_attr(
        target_os = "macos",
        ignore = "muda needs an initialized NSApplication on the main thread; unit tests run on worker threads without one, so this asserts on every other platform and the macOS path is covered by the desktop e2e"
    )]
    fn a_menu_builds_on_every_platform() {
        // Desktop wraps the groups in a native menu; Android keeps them for the
        // activity to render. Either way declaring one menu must not fail.
        assert!(build("Voussoir", &[group()]).is_ok());
    }

    #[cfg(target_os = "android")]
    #[test]
    fn android_keeps_the_groups_for_the_activity() {
        let built = build("Voussoir", &[group()]).expect("a menu should build");
        assert_eq!(built.groups().len(), 1);
        assert_eq!(built.groups()[0].title, "File");
        assert_eq!(built.groups()[0].items[0].id, "save");
    }
}

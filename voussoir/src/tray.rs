//! System tray icon with a context menu.
//!
//! Minimal by design: an icon, a tooltip, and menu items that route through the
//! same command/event path as everything else. Clicking an item emits a `tray`
//! event with `{"id": ...}` and optionally runs the command it names, so a tray
//! menu needs no separate plumbing in the app.
//!
//! On Linux this needs `libappindicator` (or ayatana-appindicator) at build time;
//! the icon lives in the desktop's status area and its menu is owned by the
//! environment.
//!
//! Android has no status area and no background tray, so the same configuration
//! becomes a notification: the tooltip is its title and each item an action
//! button. It is not equivalent, and is not presented as such: a notification can
//! be dismissed, and posting one needs the notification permission, so the tray's
//! "always there" promise does not carry over.

use serde_json::json;

use crate::{AppHandle, Error};

/// One tray menu item.
#[derive(Debug, Clone)]
pub struct TrayItem {
    /// Identifier sent to the page.
    pub id: String,
    pub label: String,
    /// Optional command run when the item is clicked.
    pub command: Option<String>,
    /// Whether the item is a check item, and its initial state.
    ///
    /// `None` is a plain item. `Some(false)`/`Some(true)` makes it a check item,
    /// which is what `AppHandle::set_tray_checked` toggles at runtime; a plain
    /// item has no visual state to toggle, so asking to check one is an error
    /// rather than a silent no-op.
    pub checked: Option<bool>,
}

impl TrayItem {
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            command: None,
            checked: None,
        }
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

/// Everything needed to create a tray icon.
#[derive(Debug, Clone)]
pub struct TrayConfig {
    pub tooltip: String,
    /// RGBA pixels and their dimensions. A 32x32 or 22x22 icon is typical.
    pub icon: Option<(Vec<u8>, u32, u32)>,
    pub items: Vec<TrayItem>,
    /// Quit the app when the last window closes. Defaults to true; a tray app
    /// often wants false so it can keep running in the background.
    pub quit_on_last_window: bool,
}

impl Default for TrayConfig {
    fn default() -> Self {
        Self {
            tooltip: "Voussoir".to_owned(),
            icon: None,
            items: Vec::new(),
            quit_on_last_window: true,
        }
    }
}

impl TrayConfig {
    pub fn new(tooltip: &str) -> Self {
        Self {
            tooltip: tooltip.to_owned(),
            ..Self::default()
        }
    }

    pub fn icon(mut self, rgba: Vec<u8>, width: u32, height: u32) -> Self {
        self.icon = Some((rgba, width, height));
        self
    }

    pub fn item(mut self, item: TrayItem) -> Self {
        self.items.push(item);
        self
    }

    /// Replace the tooltip declared at build time.
    pub fn tooltip(mut self, text: &str) -> Self {
        self.tooltip = text.to_owned();
        self
    }

    pub fn keep_running(mut self) -> Self {
        self.quit_on_last_window = false;
        self
    }

    /// Reject a configuration the platform would reject late, or mangle silently.
    ///
    /// An icon whose byte count does not match its dimensions fails inside the
    /// platform library, where the message names neither the icon nor the reason;
    /// a duplicate id makes two items indistinguishable in the event the page
    /// receives, and an empty one cannot be addressed at all.
    pub fn validate(&self) -> Result<(), String> {
        if let Some((rgba, width, height)) = &self.icon {
            let expected = (*width as usize)
                .checked_mul(*height as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| "tray icon dimensions overflow".to_owned())?;
            if *width == 0 || *height == 0 || rgba.len() != expected {
                return Err(format!(
                    "tray icon is {width}x{height} and needs {expected} RGBA bytes, got {}",
                    rgba.len()
                ));
            }
        }
        let mut seen = std::collections::HashMap::new();
        for item in &self.items {
            if item.id.trim().is_empty() {
                return Err(format!("tray item {:?} has an empty id", item.label));
            }
            if seen.insert(item.id.clone(), ()).is_some() {
                return Err(format!(
                    "tray items share the id {:?}; each item needs its own",
                    item.id
                ));
            }
        }
        Ok(())
    }
}

/// A live tray icon. Dropping it removes the icon from the status area.
///
/// Held by the event loop rather than by the app, because the platform objects
/// behind it are not `Send`. The methods on it therefore run on the UI thread,
/// reached through the matching `AppHandle` calls.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub struct TrayHandle {
    icon: tray_icon::TrayIcon,
    /// The check items by id, so a runtime toggle does not have to walk the menu.
    checks: std::collections::HashMap<String, tray_icon::menu::CheckMenuItem>,
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl TrayHandle {
    /// Replace the hover tooltip.
    pub fn set_tooltip(&self, text: &str) -> Result<(), Error> {
        self.icon
            .set_tooltip(Some(text))
            .map_err(|error| Error::Runtime(format!("tray tooltip: {error}")))
    }

    /// Check or uncheck one item.
    pub fn set_checked(&self, id: &str, checked: bool) -> Result<(), Error> {
        match self.checks.get(id) {
            Some(item) => {
                item.set_checked(checked);
                Ok(())
            }
            None => Err(Error::Handler(format!(
                "tray item {id:?} is not a check item; declare it with .check() or .checked(..)"
            ))),
        }
    }

    /// Show the tray's own menu, for a page that has a button of its own.
    ///
    /// This is the programmatic counterpart to a click on the icon, which is what
    /// an app needs when the affordance is in the page rather than in the status
    /// area (a "tray menu" button in a settings window, for instance).
    pub fn popup(&self) {
        self.icon.show_menu();
    }
}

/// A live tray on Android, which is a notification that has already been posted.
///
/// There is nothing to keep alive: the notification outlives this value, and the
/// OS owns it from the moment it is posted.
#[cfg(target_os = "android")]
pub struct TrayHandle {
    _private: (),
}

#[cfg(target_os = "android")]
impl TrayHandle {
    /// A posted notification cannot be edited on Android; the tray is re-posted
    /// by the app instead. Reported, never a silent no-op.
    pub fn set_tooltip(&self, _text: &str) -> Result<(), Error> {
        Err(Error::Handler(
            "a posted notification cannot be retitled; build the tray after the title is known"
                .to_owned(),
        ))
    }

    pub fn set_checked(&self, _id: &str, _checked: bool) -> Result<(), Error> {
        Err(Error::Handler(
            "notification actions carry no check state on Android".to_owned(),
        ))
    }

    /// There is no tray surface to pop up; the action buttons are on the
    /// notification the OS already shows.
    pub fn popup(&self) {}
}

/// Build the tray icon and wire its menu to the app's event loop.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn build(config: &TrayConfig, handle: &AppHandle) -> Result<TrayHandle, Error> {
    use tray_icon::menu::{CheckMenuItem, Menu, MenuItem as TrayMenuItem};

    config.validate().map_err(Error::Handler)?;
    use tray_icon::{Icon, TrayIconBuilder};

    let menu = Menu::new();
    let mut checks = std::collections::HashMap::new();
    for item in &config.items {
        match item.checked {
            Some(checked) => {
                // A check item needs a stable id on its own, which is also the
                // key `set_checked` looks up.
                let menu_item = CheckMenuItem::with_id(&item.id, &item.label, true, checked, None);
                menu.append(&menu_item)
                    .map_err(|e| Error::Runtime(e.to_string()))?;
                checks.insert(item.id.clone(), menu_item);
            }
            None => {
                let menu_item = TrayMenuItem::with_id(&item.id, &item.label, true, None);
                menu.append(&menu_item)
                    .map_err(|e| Error::Runtime(e.to_string()))?;
            }
        }
    }

    let mut builder = TrayIconBuilder::new()
        .with_tooltip(&config.tooltip)
        .with_menu(Box::new(menu));

    if let Some((rgba, width, height)) = &config.icon {
        let icon = Icon::from_rgba(rgba.clone(), *width, *height)
            .map_err(|e| Error::Runtime(format!("invalid tray icon: {e}")))?;
        builder = builder.with_icon(icon);
    }

    let icon = builder
        .build()
        .map_err(|e| Error::Runtime(format!("tray icon failed: {e}")))?;

    let proxy = handle.clone();
    tray_icon::menu::MenuEvent::set_event_handler(Some(
        move |event: tray_icon::menu::MenuEvent| {
            proxy.emit_signal(&format!("tray:{}", event.id.0));
        },
    ));

    Ok(TrayHandle { icon, checks })
}

/// Post the tray as a notification, with its items as action buttons.
///
/// The configuration is handed over as JSON rather than as individual JNI calls
/// so the activity decides how a notification is built; a failure to reach the
/// activity is reported, because a tray that silently did nothing would look like
/// a missing feature rather than a broken one.
#[cfg(target_os = "android")]
pub fn build(config: &TrayConfig, _handle: &AppHandle) -> Result<TrayHandle, Error> {
    config.validate().map_err(Error::Handler)?;

    let request = json!({
        "title": config.tooltip,
        "items": config
            .items
            .iter()
            .map(|item| json!({ "id": item.id, "label": item.label }))
            .collect::<Vec<_>>(),
    });

    if crate::platform::post_notification(&request.to_string()) {
        Ok(TrayHandle { _private: () })
    } else {
        Err(Error::Handler(
            "the tray could not be posted as a notification: the activity is not available"
                .to_owned(),
        ))
    }
}

/// A live tray on iOS, which does not exist.
///
/// iOS has no status area a tray can occupy and no background service an app
/// can own. `build` reports that honestly rather than silently doing
/// nothing: an app that wants a visible affordance on iOS uses the page's
/// own UI, the way the scaffold hides its tray on mobile.
#[cfg(target_os = "ios")]
pub struct TrayHandle {
    _private: (),
}

#[cfg(target_os = "ios")]
impl TrayHandle {
    pub fn set_tooltip(&self, _text: &str) -> Result<(), Error> {
        Err(Error::Handler(
            "the system tray does not exist on iOS; use in-page UI instead".to_owned(),
        ))
    }

    pub fn set_checked(&self, _id: &str, _checked: bool) -> Result<(), Error> {
        Err(Error::Handler(
            "the system tray does not exist on iOS; use in-page UI instead".to_owned(),
        ))
    }

    pub fn popup(&self) {}
}

#[cfg(target_os = "ios")]
pub fn build(_config: &TrayConfig, _handle: &AppHandle) -> Result<TrayHandle, Error> {
    Err(Error::Handler(
        "the system tray does not exist on iOS; use in-page UI instead".to_owned(),
    ))
}

/// The event payload the page receives for a tray click.
pub fn event_payload(id: &str) -> serde_json::Value {
    json!({ "id": id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_must_match_its_dimensions() {
        let good = TrayConfig::new("Notes").icon(vec![0; 4 * 16 * 16], 16, 16);
        assert!(good.validate().is_ok());
        let bad = TrayConfig::new("Notes").icon(vec![0; 7], 16, 16);
        let error = bad.validate().unwrap_err();
        assert!(error.contains("16x16") && error.contains("7"), "{error}");
        assert!(TrayConfig::new("Notes").icon(vec![0; 4], 0, 1).validate().is_err());
    }

    #[test]
    fn ids_must_be_unique_and_non_empty() {
        let duplicate = TrayConfig::new("Notes")
            .item(TrayItem::new("open", "Open"))
            .item(TrayItem::new("open", "Open again"));
        assert!(duplicate.validate().unwrap_err().contains("share the id"));
        let empty = TrayConfig::new("Notes").item(TrayItem::new("", "Nameless"));
        assert!(empty.validate().unwrap_err().contains("empty id"));
    }

    #[test]
    fn config_builder_is_chainable_and_keeps_order() {
        let config = TrayConfig::new("Notes")
            .icon(vec![0; 4 * 16 * 16], 16, 16)
            .item(TrayItem::new("open", "Open").command("show"))
            .item(TrayItem::new("quit", "Quit"))
            .keep_running();

        assert_eq!(config.tooltip, "Notes");
        assert!(!config.quit_on_last_window);
        assert_eq!(config.items.len(), 2);
        assert_eq!(config.items[0].id, "open");
        assert_eq!(config.items[0].command.as_deref(), Some("show"));
        assert!(config.items[1].command.is_none());
        let (rgba, w, h) = config.icon.unwrap();
        assert_eq!((rgba.len(), w, h), (4 * 16 * 16, 16, 16));
    }

    #[test]
    fn defaults_quit_with_the_last_window() {
        let config = TrayConfig::default();
        assert!(config.quit_on_last_window);
        assert!(config.items.is_empty());
        assert!(config.icon.is_none());
    }

    #[test]
    fn check_items_are_declared_explicitly() {
        let config = TrayConfig::new("Notes")
            .tooltip("Notes (3 unsaved)")
            .item(TrayItem::new("auto", "Autosave").check())
            .item(TrayItem::new("mute", "Mute").checked(false))
            .item(TrayItem::new("quit", "Quit"));

        assert_eq!(config.tooltip, "Notes (3 unsaved)");
        assert_eq!(config.items[0].checked, Some(true));
        assert_eq!(config.items[1].checked, Some(false));
        assert_eq!(
            config.items[2].checked, None,
            "an item without .check() stays plain, so toggling it can be reported"
        );
    }

    #[test]
    fn the_event_payload_names_the_item() {
        assert_eq!(event_payload("open"), json!({ "id": "open" }));
    }
}

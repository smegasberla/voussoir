# Per-window allowlists

A window that renders untrusted content should not be able to call the whole app.
Voussoir lets you narrow one window's command surface, at startup or when the
window is opened, and the narrowing can only ever remove.

## Theory: the confused-deputy problem

If your app opens a window showing someone else's content, that content runs in a
window that can call your commands. It cannot read your files itself, so it asks
the window for what it wants, and the window obliges. The content has used your
app's authority without ever holding it.

This is the classic confused deputy. The defence is not to harden the content;
it is to make sure the deputy has less authority when doing someone else's
bidding. In an IPC-shaped app, that means **the command surface is per window**.

The alternative design, a single global surface, cannot express this, and that is
why the allowlist is per window rather than a global list only.

## In `voussoir.json`

```json
{
  "commands": ["greet", "save", "delete"],
  "windows": { "preview": { "commands": ["greet"] } }
}
```

A window labelled `preview` can call `greet` and nothing else.

**The intersection rule is the important part:** the effective list is the
window's list **and** the global list. A `windows` entry can therefore never grant
a command the app withheld, which means a config file cannot be used to escalate.

```
effective(preview) = global ∩ windows.preview
                   = {greet, save, delete} ∩ {greet}
                   = {greet}
```

Add `"delete"` to `preview` and it is still not grantable, because the
intersection is what applies.

### The default that surprises everyone

**Omitting `commands` altogether allows everything**, built-ins included. A config
like `{"name": "Notes"}` permits every command the app registered plus every
built-in the enabled features add. That is the scaffold's default and the right one
for development: a new command works the moment you add it.

It is the wrong default for an app you ship, because a command registered by a
dependency you barely thought about is now reachable from a page that renders a
stranger's text. Before shipping, write the list out explicitly: it becomes the
inventory of what your app can do, which is a useful document on its own.

`"commands": []` is **not** the same as omitting the key. Omitting means "no
restriction"; an empty array means "nothing at all", built-ins included. If every
command answers `unknown`, check for an empty array first.

### What the built-ins are called

Built-in commands count against the allowlist like yours, and their names are ugly
on purpose: nobody is meant to type them.

| Name | What it does |
| --- | --- |
| `__voussoir_version` | the crate version |
| `__voussoir_open_window` | opens a window, returns its label |
| `__voussoir_windows` | lists the open labels |
| `__voussoir_close_window` | closes one |
| `__voussoir_devtools` | opens devtools when the feature allows it |
| `__voussoir_data_dir` | the per-app data directory |
| `__voussoir_diagnostics` | the recorded diagnostic entries |
| `__voussoir_diagnostics_enabled` | whether recording is on |
| `__voussoir_reports` | the report buffer |
| `__voussoir_tray_tooltip`, `__voussoir_tray_checked`, `__voussoir_tray_popup` | tray state |
| `__voussoir_menu_checked` | menu check state |
| `__voussoir_show_menu` | Android only: raises the menu |
| `storage.get`, `storage.set`, `storage.delete`, `storage.keys`, `storage.path` | with the `storage` feature |

You do not normally list these. But a window narrowed to `["render"]` also cannot
call `__voussoir_open_window` or `__voussoir_windows`, so `window.voussoir.windows()`
in that page rejects. That is usually what you want; it is worth knowing before you
spend twenty minutes on it.

### The four cases, as a table

Assume `"commands": ["a", "b", "c"]`.

| Request | Window entry for that label | Result |
| --- | --- | --- |
| `a` from a window | none | allowed |
| `d` from a window | none | denied, `kind: "unknown"` |
| `a` from a window | `["b"]` | denied |
| `b` from a window | `["b"]` | allowed |
| `d` from a window | `["b", "d"]` | denied: the global list wins |
| `a` from a **menu item** | `["b"]` | allowed: no label means no window check |
| `d` from a menu item | any | denied |

Row 5 says a window list cannot grant anything. Row 6 is the asymmetry that costs
people an hour: **a menu item's command is dispatched with no window label**, so it
is checked against `commands` and never against a window's list. A menu item wired
to a command you removed from `commands` fails **silently**, because the reply is
discarded. If a menu item does nothing and nothing appears in the terminal, that is
where to look.

## At runtime

```rust
let label = app.open_window(WindowOptions {
    title: Some("Preview".to_owned()),
    allowlist: Some(vec!["greet".to_owned()]),
    ..Default::default()
})?;
```

The narrowing is registered **before the window exists**, so its page can never
make a call it will not be allowed to keep making. That ordering is the point: a
window that is briefly wide open and then narrowed would have a window of
opportunity, and a page loads fast.

From the page, the same thing:

```ts
const { label } = await window.voussoir.invoke<{ label: string }>(
  "__voussoir_open_window",
  { label: "preview", title: "Preview", allowlist: ["greet"] },
);
```

The built-in answers `{"label": "preview"}`, so the page can address the window it
just opened even if the shell had to generate a unique label.

## Where the label comes from

The shell reads the calling window's label from the webview the request arrived
on, **never from the payload**. A page cannot claim another window's permissions
because there is no field for one.

This is worth pausing on, because it is the one property that a page cannot
influence. If the label came from the payload, a narrowed window would simply
send `{"window": "main"}` and the whole mechanism would be decoration.

## Theory: what a denial looks like, and why it looks wrong

`kind: "unknown"`, the same answer an unregistered command gets.

A narrowed window cannot probe for the commands it is missing, which is the point:
a preview window that can distinguish "not allowed" from "does not exist" has
learned part of the app's surface.

The cost is real and worth knowing: during development, a typo in an allowlist
looks exactly like a typo in a command name. Both answer `unknown`.

The crate narrows that gap by auditing the allowlist automatically at startup: it
checks every name in `commands` and in every `windows` entry against the registry,
and logs the ones no handler can answer. You do not call it; it runs as part of
`App::run`. A config that names a command you renamed therefore says so on stderr
instead of looking like a missing command.

It cannot catch the other case, a command that exists and is withheld, because
that is the mechanism working.

## Patterns

**A preview window** — the motivating case:

```json
{
  "commands": ["greet", "save", "preview.render"],
  "windows": { "preview": { "commands": ["preview.render"] } }
}
```

`preview.render` returns HTML and cannot touch the disk; `save` stays in the main
window.

**A plugin's commands per window** — narrowed without the plugin cooperating:

```json
{ "windows": { "settings": { "commands": ["sqlite.query", "storage.get"] } } }
```

**An empty list** — a window with no surface at all, which is what you want for
anything purely decorative or fully untrusted:

```json
{ "windows": { "help": { "commands": [] } } }
```

Note the difference from omitting the key: absent means "inherit the global
list"; `[]` means "nothing".

## Testing it

The `ipc` unit tests cover the intersection rule and the per-window lookup
directly, and `bun scripts/verify.ts` builds a window with a strict list, calls
out of it, and asserts the denial arrives over the real transport.

A useful assertion for an app is the envelope a denied call produces, because
that is what the page will see. The crate's own `ipc` tests do it this way:

```rust
#[test]
fn preview_window_cannot_reach_delete() {
    let mut registry = Registry::new();
    registry.set_allowlist(Some(vec!["greet".to_owned(), "delete".to_owned()]));
    registry.set_window_allowlist(HashMap::from([(
        "preview".to_owned(),
        vec!["greet".to_owned()],
    )]));
    // ... register the handlers, then dispatch as "preview":
    assert!(dispatch(&registry, "preview", "greet").contains(r#""ok""#));
    assert!(dispatch(&registry, "preview", "delete").contains(r#""kind":"unknown""#));
}
```

`Registry` has no `allows(label, name)` predicate: the intersection is applied
inside dispatch, so a denial is observable as `kind: "unknown"` rather than as a
boolean. Assert the envelope, which is also what the page branches on.

The startup audit is the other half of the testing story. At startup the registry
checks every name in both lists against the handlers that exist and records a
diagnostic for each name nothing can answer:

```
voussoir: [app] allowlist names an unknown command command=save_document
voussoir: [app] window allowlist names an unknown command window=preview command=rendr
```

That turns a typo into a line on the terminal instead of a page that mysteriously
receives `unknown`. **It is skipped entirely under hot reload**, because a name
might belong to the command library, which cannot be asked what it exports. So in
`dev` a typo only shows up as `unknown` at runtime.

## Failure modes, in the order people hit them

| Symptom | Cause | Fix |
| --- | --- | --- |
| every command answers `unknown` | `"commands": []` | remove the key, or list what you need |
| one command answers `unknown` in a second window | that window's list omits it | add it there |
| one command answers `unknown` everywhere | absent from `commands` | add it to the global list |
| `windows()` rejects in one window | the list has no `__voussoir_windows` | allow it explicitly |
| a menu item does nothing, no error | the command is not in `commands`; the reply is discarded | add it, and watch the terminal in `dev` |
| a list change had no effect | the app was not rebuilt | `voussoir.json` is compiled in |
| the startup warning names a command you removed | stale config entry | delete the name |

## Next

- [Multi-window](multi-window.md) — labels, listing and closing.
- [Security model](security.md) — the rest of the policy.

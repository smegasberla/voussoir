# Errors the page can branch on

A failure in a Voussoir app is data, not a string. Every rejection from `invoke`
carries a `kind` you can switch on, a `method` naming the command, and a
`message` written for a human.

## Theory: why error codes are not enough, and error strings are worse

Two common mistakes, and what this design does instead:

- **Matching on message text.** `if (message.includes("not found"))` breaks the
  first time someone rewords a message, and rewordings are not breaking changes.
- **A single `Error`.** `if (err)` tells you something failed but not whether to
  retry, hide a button, or show a dialog.

The middle path is a small, stable enum of classes. Voussoir calls it `kind`, and
the list is small enough to read once and exhaustive enough to switch on without a
default branch.

## The seven kinds

```ts
try {
  await window.voussoir.invoke("save", { path });
} catch (error) {
  const { kind, method, message } = error as {
    kind: "unknown" | "payload" | "handler" | "panic" | "busy" | "timeout" | "runtime";
    method: string;
    message: string;
  };
  if (kind === "unknown") hideSaveButton();
  else if (kind === "busy") retryLater();
  else showToast(`${method}: ${message}`);
}
```

| `kind` | Raised by | What the page should do |
| --- | --- | --- |
| `unknown` | no handler, or the allowlist withheld it | treat the command as absent; do not retry |
| `payload` | the request did not match the typed argument | a bug in the page; log it |
| `handler` | the handler returned `Err` | show the message; it is meant for a user |
| `panic` | the handler panicked | show the message, keep the window; report it |
| `busy` | every worker-thread slot was taken | retry shortly; nothing was started |
| `timeout` | the shim stopped waiting (raised in the page, not in Rust) | the command may still be running; do not assume it failed |
| `runtime` | the webview or event loop failed | fatal for the window; offer a restart |

**`unknown` covers both a missing command and a denied one on purpose.** A page
must not be able to probe which commands exist but were withheld, so the two are
indistinguishable from inside the webview. If your page needs to know whether a
feature exists, ask about the feature, not about the command.

## Theory: three kinds are not yours

Of the seven, only three are produced by code you write:

| Produced by | Kinds |
| --- | --- |
| your handler | `handler`, and `payload` if you return `Error::Payload` |
| the registry | `unknown`, `payload`, `busy` |
| the shim (page side) | `timeout` |
| the shell | `runtime`, `panic` |

`panic` deserves a note: the shell catches unwinds at the handler boundary, so a
panicking command becomes an error envelope and **the window survives**. That is a
guarantee, not a hope; `scripts/verify.ts` asserts it. Write the page as if a
command can panic, because in development it will.

`timeout` is the one kind that can arrive for a command that never reached Rust at
all, which is why it is worth treating differently: the work may still be running
when the page gives up.

## Writing an error

```rust
fn save(payload: &Value) -> Result<Value, voussoir::Error> {
    let path = payload
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| voussoir::Error::Payload("save needs a `path`".to_owned()))?;

    std::fs::write(path, "…")
        .map_err(|e| voussoir::Error::Handler(format!("could not write {path}: {e}")))?;

    Ok(voussoir::serde_json::json!({ "ok": true }))
}
```

The rule for choosing between them:

- **`Payload`** — the *caller* is wrong. A missing field, a wrong type, a value
  out of range. The page has a bug, and the message is for the developer.
- **`Handler`** — the *world* is wrong. The disk is full, the file vanished, the
  network refused. The message is for the user, so write it as a sentence.

A `String` error from a `Result<_, String>` becomes `handler`. That is a good
default, and it means a quick command does not need to import `Error` at all.

## Theory: separate "user" from "developer" messages

The `message` field is shown to a real person, so a bare `EACCES` is a failure of
the app, not of the framework. Two habits:

- **Handler messages are for users.** "The file could not be saved: permission
  denied" beats "os error 13".
- **Detail goes in the log, not in the message.** Do not paste both into one string.

There is no public `record` function to call: the reporting module is private, and
the three seams that exist are all methods or commands, on purpose.

| You want | Use |
| --- | --- |
| to read reports from the page | the `__voussoir_reports` command |
| to forward every report to your own collector | `App::report_sink(\|line\| ...)` |
| to persist reports to a file | `App::report_log(path)` |

```rust
App::new()
    .report_log("/tmp/notes-reports.log")
    .report_sink(|line| eprintln!("[notes] {line}"))
```

`report_sink` is called from the panic hook, so the closure must not panic and must
not block for long. Without `report_log` reports are still kept in memory and
readable through `__voussoir_reports`; they are just not persisted.

## Retrying: the only kind worth retrying

```ts
async function invokeWithRetry<T>(method: string, payload: unknown, tries = 3): Promise<T> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await window.voussoir.invoke<T>(method, payload);
    } catch (error) {
      const { kind } = error as { kind: string };
      if (kind !== "busy" || attempt >= tries) throw error;
      await new Promise((r) => setTimeout(r, 50 * attempt));
    }
  }
}
```

Retrying anything else is usually wrong:

- `unknown` will be `unknown` again.
- `payload` is a bug in the caller.
- `handler` may be transient, but you cannot tell from the kind alone, so the
  command should say so in its message.
- `timeout` must not be retried blindly: the first attempt may still be running.

## Deadlines

The default deadline is `command_timeout_ms` from `voussoir.json` (30 seconds
unless you change it). `0` disables it. A page can override it per call:

```ts
await window.voussoir.invoke("export", payload, { timeout: 300_000 });
```

**Why a deadline at all?** Because a promise that never settles is
indistinguishable from a frozen UI, and the user will force-quit. A `timeout` is
a better failure than a hang, even when the work is legitimate and long.

## Next

- [Commands & payloads](ipc-contract.md) — the envelope these kinds arrive in.
- [Troubleshooting](troubleshooting.md) — what each kind usually means in practice.

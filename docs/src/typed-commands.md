# Typed commands

`Value` in, `Value` out is the wire format, but it is a poor API to write by hand:
the handler has to re-check every field, and a wrong shape becomes a string the
page cannot act on. `command_typed` moves deserialization in front of the handler
and answers a malformed request with `kind: "payload"`, naming the command.

## Theory: where validation belongs

There are three places a payload can be checked, and only one of them is good:

1. **In the page**, before calling. Cheap and useless: the page is the untrusted
   side, and a second caller will skip it.
2. **Inside the handler**, by hand. Correct, but repeated in every command, and
   the failure is a `String` the page cannot branch on.
3. **In front of the handler**, by the registry. Done once, uniformly, and the
   failure becomes a typed error the page can act on.

`command_typed` is (3). The handler never sees a malformed request, so it can be
written as if the types are true.

## A typed command

```rust
// `voussoir` re-exports serde, so an app never adds serde to its own
// Cargo.toml. That is also why each derive needs the `crate` attribute below.
use voussoir::serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(crate = "voussoir::serde")]
struct EchoArgs {
    text: String,
    repeat: Option<u32>,
}

#[derive(Serialize)]
#[serde(crate = "voussoir::serde")]
struct EchoReply {
    text: String,
    count: u32,
}

App::new().command_typed("echo", |_app: &AppHandle, args: EchoArgs| {
    let count = args.repeat.unwrap_or(1);
    Ok(EchoReply {
        text: args.text.repeat(count as usize),
        count,
    })
})
```

The page sees no difference: `invoke("echo", { text: "hi", repeat: 2 })` resolves
to `{"text": "hihi", "count": 2}`.

What changed is the failure path. Send `repeat: "two"` and the handler never
runs; the page gets:

```json
{"error": {"message": "...", "kind": "payload", "method": "echo"}}
```

**`#[serde(crate = "voussoir::serde")]` is not optional.** A derive macro generates
paths like `::serde::Deserializer`, which resolve only if `serde` is a direct
dependency. Your app depends on `voussoir` alone (the scaffold's `Cargo.toml` has no
serde line), so the derive has to be told where the crate lives. Without the
attribute the error is `could not find serde in the crate root`, which reads like a
missing dependency and is not one.

**Read the three parts of `EchoArgs` carefully, because they teach the whole
model:**

- `text: String` is required. A missing field is a `payload` error, not a
  default.
- `repeat: Option<u32>` is optional. `null` or absent both deserialize to `None`.
- The type is strict: `"2"` (a string) is not a `u32`. That strictness is the
  point, not an accident to work around.

## Theory: why the envelope does not change

A typed command and a plain one are indistinguishable on the wire. That is
deliberate:

- You can convert a command to typed without breaking any client that already
  calls it.
- The page never needs to know which kind it is calling.
- The allowlist, per-window gating and diagnostics apply identically.

What does change is the *meaning* of `payload`: it now says "the request shape
was wrong", rather than "the handler decided to say no". That is a better error,
because it points at the caller.

`scripts/verify.ts` asserts both halves over the real transport: a bad shape comes
back as `payload/typed_echo`, and the same command with the right shape answers.

## Theory: why typed commands cannot live in the hot-reloaded table

The command crate is a `cdylib` in development. Rust types do not cross that
boundary: the shell cannot instantiate a closure with a concrete argument type in
a library it loaded at runtime. So the table's entries take `&Value`, and the
typing has to happen on the side where both the concrete type and the shell are
visible.

Practically:

| You want | Put it |
| --- | --- |
| typed argument, does not hot-reload | `.command_typed(...)` on `App` in `main.rs` |
| hot-reloadable, dynamic payload | the table, taking `&Value` |
| both | two commands: table entry validates by hand, `App` wrapper is typed |

That third row is more common than it sounds, and it is fine: the wrapper is a
few lines, and the logic underneath is what changes often.

## Async typed commands

```rust
// Requires the `rt` feature: voussoir = { features = ["full", "rt"] }
App::new().async_fn("lookup", |_app, args: LookupArgs| async move {
    // `?` needs a From impl; Error has none for io::Error, and a database
    // error is not one either, so convert explicitly and say what failed.
    let row = db
        .get(args.id)
        .await
        .map_err(|e| Error::Handler(format!("lookup failed: {e}")))?;
    Ok(row)
})
```

The argument is deserialized **on the calling thread**, before a task is spawned.
So a malformed request is answered immediately and costs no scheduler work; only
the actual body moves to the runtime.

## Returning rich errors

A typed handler can return `voussoir::Error` instead of `String`, which is how a
command distinguishes a bad payload from a real failure:

```rust
use voussoir::Error;

fn parse(payload: &Value) -> Result<u32, Error> {
    payload
        .get("count")
        .and_then(Value::as_u64)
        .map(|n| n as u32)
        .ok_or_else(|| Error::Payload("count must be a number".to_owned()))
}
```

`Error::Payload` becomes `kind: "payload"`. `Error::Handler` becomes
`kind: "handler"`. A plain `String` error is `handler`, which is the right default
for "something went wrong" and the wrong one for "your request was malformed".

## Guidance

- **Type the argument when the page is yours.** It is a few lines and it turns a
  class of bug into a wire error.
- **Leave `Value` for genuinely dynamic payloads**, such as a plugin registry or
  a proxy command that forwards to another handler.
- **Do not validate twice.** If the type says `String`, do not also check
  `is_some()` inside the handler.
- **Give the structs names that match the page's fields.** The page's object keys
  are the field names, so `#[serde(rename_all = "camelCase")]` is the usual fix
  when TypeScript code uses `camelCase` and Rust uses `snake_case`.

## Next

- [Errors the page can branch on](errors.md) — the other side of `kind`.
- [Commands & payloads](ipc-contract.md) — the envelope itself.

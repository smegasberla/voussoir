//! A minimal Voussoir app: embedded assets, one command round-trip, one event.
//!
//! Run: `cargo run --example hello`
//! Headless proof: `VOUSSOIR_SELFTEST=1 cargo run --example hello` exits 0 after the
//! JS -> Rust -> JS round-trip prints `SELFTEST OK`.

use voussoir::serde_json::{json, Value};
use voussoir::{App, AssetTable, Error};

/// `greet` arguments.
#[derive(voussoir::serde::Deserialize)]
#[serde(crate = "voussoir::serde")]
struct GreetArgs {
    name: Option<String>,
}

static ASSETS: AssetTable = &[
    ("index.html", include_bytes!("hello/index.html")),
    ("style.css", include_bytes!("hello/style.css")),
    ("main.js", include_bytes!("hello/main.js")),
];

fn main() -> Result<(), Error> {
    let started = std::time::Instant::now();
    let selftest = std::env::var_os("VOUSSOIR_SELFTEST").is_some();
    let bench = std::env::var_os("VOUSSOIR_BENCH").is_some();

    App::new()
        .config(include_str!("hello/voussoir.json"))
        .assets(ASSETS)
        // Typed: `{}` or `{"name":"..."}` only. Anything else is rejected with a
        // `payload` error naming the command, before this body runs.
        .command_typed("greet", move |_app, args: GreetArgs| {
            if selftest {
                // Proves the page's call reached Rust. Without this, a run that
                // printed READY and died is indistinguishable from a page that
                // never loaded its script, and those need different fixes.
                println!("SELFTEST INVOKED");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            let name = args.name.as_deref().unwrap_or("world");
            Ok(json!(format!("Hello, {name}! (from Rust)")))
        })
        .command("report", move |_app, payload: Value| {
            let greeting = payload
                .get("greeting")
                .and_then(Value::as_str)
                .unwrap_or("");
            if selftest && greeting.contains("from Rust") {
                println!("SELFTEST OK: {greeting}");
                std::process::exit(0);
            }
            Ok(json!(null))
        })
        .on_ready(move |app, _window| {
            if selftest {
                // Printed before the round trip is attempted. A run that dies
                // without this line never opened its window; a run that prints
                // it and still fails got as far as a working webview and lost
                // the page's call instead. Those need different fixes, and a
                // silent SIGTERM used to be indistinguishable between them.
                println!("SELFTEST READY");
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
            if bench {
                println!("READY {}", started.elapsed().as_millis());
                use std::io::Write;
                let _ = std::io::stdout().flush();
                std::process::exit(0);
            }
            std::thread::spawn({
                let app = app.clone();
                move || {
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    app.emit("tick", json!(1));
                }
            });
        })
        .run()
}

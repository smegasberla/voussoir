//! Registry dispatch behind the transport seam.
//!
//! Both transports land here: the custom protocol (`serve_async`, every
//! platform) and the Android bridge (`serve_bridge`, where a POST cannot carry
//! a body). The envelope, the per-window allowlist and the security headers
//! are decided in this one module, so a new transport joins by calling in
//! rather than by re-implementing the rules.

use std::borrow::Cow;
use std::sync::Arc;

use serde_json::Value;
use wry::http::{header, Request, Response};

use crate::assets::{content_type, lookup, AssetTable};
use crate::ipc;
use crate::security::content_security_policy;
use crate::transport;
use crate::{AppHandle, Registry};

/// Answer a bridge message: parse `{id, method, payload}`, run the command, then
/// resolve the page's promise by evaluating `__voussoir_resolve(id, envelope)`.
///
/// Used on Android, where a custom-protocol POST cannot carry a body. The
/// envelope is identical to the protocol transport, so a command behaves the same
/// on both platforms.
pub(crate) fn serve_bridge(
    registry: &Arc<Registry>,
    handle: &AppHandle,
    label: Option<&str>,
    message: &str,
) {
    let parsed: Value = match serde_json::from_str(message) {
        Ok(value) => value,
        Err(_) => {
            crate::diagnostics::record(
                crate::diagnostics::Kind::Bridge,
                "bridge message is not valid JSON",
                serde_json::json!({ "bytes": message.len() }),
            );
            return;
        }
    };

    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = parsed
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        crate::diagnostics::record(
            crate::diagnostics::Kind::Bridge,
            "bridge message has no method",
            serde_json::json!({ "keys": parsed.as_object().map(|map| map.keys().cloned().collect::<Vec<_>>()) }),
        );
        return;
    };
    let payload = parsed.get("payload").cloned().unwrap_or(Value::Null);

    // Owned, because the reply runs after this function returns and only the
    // requesting window may receive it.
    let target = label.map(str::to_owned);
    let dispatch_target = target.clone();
    let script_handle = handle.clone();
    registry.dispatch_for_window(
        handle,
        dispatch_target.as_deref(),
        &method,
        payload,
        move |body| {
            // `body` is already an envelope; hand it to the page as JSON, not as a
            // string, so the shim does not have to parse twice.
            let script = format!(
                "window.__voussoir_resolve&&window.__voussoir_resolve({}, {})",
                id, body
            );
            // Only the window that asked: the shim's pending map and id counter are
            // per frame, so evaluating in every window could resolve another
            // window's promise with this reply. `None` is a caller with no window
            // (the desktop bridge fallback) and keeps the broadcast behavior.
            match &target {
                Some(label) => script_handle.eval_window(label, &script),
                None => script_handle.eval_all(&script),
            }
        },
    );
}

/// Serve one request, replying through wry's async responder so async commands
/// and slow handlers do not block the UI thread.
pub(crate) fn serve_async(
    registry: Arc<Registry>,
    handle: AppHandle,
    assets: AssetTable,
    dev_origin: Option<String>,
    label: Option<String>,
    request: Request<Vec<u8>>,
    responder: wry::RequestAsyncResponder,
) {
    let dev = dev_origin.is_some();
    let mut builder = base_headers(dev);
    builder = grant_fetch(builder, dev_origin.as_deref());

    if request.method() == "OPTIONS" {
        responder.respond(respond_or_500(
            builder.status(204).body(Cow::Borrowed(&[][..])),
        ));
        return;
    }

    let path = request.uri().path().to_owned();
    // Every request the webview makes, and every reply it gets. A page that
    // never runs its script is otherwise invisible: the window looks healthy and
    // only the command that never arrives is missing, which is a different fault
    // from a page that ran and had its call refused. Recorded only when
    // diagnostics are on, so a release build pays nothing.
    crate::diagnostics::record(
        crate::diagnostics::Kind::Dispatch,
        "request",
        serde_json::json!({ "method": request.method().as_str(), "path": &path }),
    );

    if let Some(method) = ipc::method_from_path(&path) {
        if request.method() != "POST" {
            responder.respond(json_error(builder, 405, "invoke requires POST"));
            return;
        }
        let payload = match transport::parse_payload(request.body()) {
            Ok(value) => value,
            Err(message) => {
                responder.respond(json_error(builder, 400, &message));
                return;
            }
        };
        // The response is built here, not from `builder`, so the CORS header has
        // to be re-applied: in dev the page is served from a different origin and
        // without it the browser rejects the reply before the page ever sees it.
        let cors_origin = dev_origin.clone();
        registry.dispatch_for_window(&handle, label.as_deref(), &method, payload, move |body| {
            let mut response = base_headers(dev);
            response = grant_fetch(response, cors_origin.as_deref());
            responder.respond(respond_or_500(
                response
                    .header(header::CONTENT_TYPE, "application/json")
                    .status(200)
                    .body(Cow::Owned(body.into_bytes())),
            ));
        });
        return;
    }

    let response = match lookup(assets, &path) {
        Some((name, bytes)) => respond_or_500(
            builder
                .header(header::CONTENT_TYPE, content_type(name))
                .status(200)
                .body(bytes),
        ),
        None => respond_or_500(
            builder
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .status(404)
                .body(Cow::Borrowed(b"not found".as_slice())),
        ),
    };
    responder.respond(response);
}

/// Finish a response, falling back to a bare 500 if the builder rejects it.
///
/// Every response here uses a literal status and literal header names, so the
/// builder cannot fail today, and that was previously asserted with `expect`.
/// A panic is still the wrong answer on the IPC path: panic containment would
/// turn it into a dead command for the page, while a 500 is a reply the page can
/// report. `json_error` takes its status as an argument, so this is where a
/// future dynamic status would land rather than a new panic.
fn respond_or_500(
    built: Result<Response<Cow<'static, [u8]>>, wry::http::Error>,
) -> Response<Cow<'static, [u8]>> {
    built.unwrap_or_else(|_| {
        let mut response = Response::new(Cow::Borrowed(b"internal error".as_slice()));
        *response.status_mut() = wry::http::StatusCode::INTERNAL_SERVER_ERROR;
        response
    })
}

fn base_headers(dev: bool) -> wry::http::response::Builder {
    Response::builder()
        .header(
            header::CONTENT_SECURITY_POLICY,
            content_security_policy(dev),
        )
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
}

/// Grant the page permission to read a response it fetched itself.
///
/// In dev the page lives on the dev server, so the grant names it. In a bundled
/// app the page and the response share a scheme, but a custom-scheme origin is
/// opaque on WKWebView: it has no tuple origin for `'self'` or a same-origin
/// check to match. Without an explicit grant the page's own
/// `<script type="module">` is refused there, the page never runs, and every
/// command call silently never happens, while Chromium and GTK, which do match,
/// keep working. The handler only serves this app's own bundle to its own
/// webview and no request is credentialed, so a wildcard grants nothing a
/// foreign page could use.
fn grant_fetch(
    builder: wry::http::response::Builder,
    dev_origin: Option<&str>,
) -> wry::http::response::Builder {
    match dev_origin {
        Some(origin) => with_cors(builder, origin),
        None => builder.header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
    }
}

fn with_cors(builder: wry::http::response::Builder, origin: &str) -> wry::http::response::Builder {
    builder
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin)
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "POST, GET, OPTIONS")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "content-type")
}

fn json_error(
    response: wry::http::response::Builder,
    status: u16,
    message: &str,
) -> Response<Cow<'static, [u8]>> {
    let body = format!(r#"{{"error":{}}}"#, serde_json::Value::from(message));
    respond_or_500(
        response
            .header(header::CONTENT_TYPE, "application/json")
            .status(status)
            .body(Cow::Owned(body.into_bytes())),
    )
}

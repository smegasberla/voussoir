//! The seam between a page's bytes and a `Registry` call.
//!
//! A desktop build reaches this through wry's custom protocol
//! (`POST voussoir://localhost/__voussoir/<method>`), a mobile build through a native
//! message handler that receives `{method, payload}` as one string. Everything
//! after parsing is shared, so the two cannot answer a page differently.
//!
//! It lives in its own module rather than in the crate root because the envelope
//! is the contract: it should be readable without scrolling past a window loop.

use crate::{AppHandle, Registry};
use serde_json::Value;

/// The endpoint path decoder, re-exported here because this module *is* the seam.
///
/// A page's URL reaches it before anything else does — before the allowlist, before
/// the handler table — so it belongs next to the parsers below rather than behind
/// `ipc`. The nightly fuzz harness in `fuzz/` reaches it through this re-export,
/// which is also what an out-of-tree harness would use.
pub use crate::ipc::method_from_path;

/// Which transport a page's calls arrive through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// wry custom protocol: `POST voussoir://localhost/__voussoir/<method>`.
    Protocol,
    /// A native message handler that receives `{method, payload}` as a
    /// string and answers with an envelope.
    MessageBridge,
}

/// Build the response envelope for one request on any transport.
///
/// This is the single place that decides what a page is told, so desktop and
/// mobile cannot drift apart.
pub fn respond(registry: &Registry, handle: &AppHandle, method: &str, payload: Value) -> String {
    registry.dispatch(handle, method, payload)
}

/// Parse a payload body, treating an empty body as JSON `null`.
pub fn parse_payload(body: &[u8]) -> Result<Value, String> {
    if body.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(body).map_err(|_| "payload is not valid JSON".to_owned())
}

/// The method name and payload a mobile bridge receives in one string,
/// mirroring the protocol's path-plus-body shape.
pub fn parse_message(message: &str) -> Result<(String, Value), String> {
    let value: Value =
        serde_json::from_str(message).map_err(|_| "message is not valid JSON".to_owned())?;
    let method = value
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| "message has no method".to_owned())?
        .to_owned();
    let payload = value.get("payload").cloned().unwrap_or(Value::Null);
    Ok((method, payload))
}

//! Navigation and security policy for the webview.
//!
//! A desktop app is not a browser. The page may only ever load from the app's
//! own origin (or the dev server in a debug build); anything else would replace
//! the UI with a foreign page and strand the user there, with no chrome to get
//! back. These rules are pure functions so they are testable without a window.

/// The origin pages are served from in production.
pub const APP_ORIGIN: &str = "voussoir://localhost";

/// On Android and on Windows, wry cannot intercept a custom scheme, so it
/// rewrites `voussoir://localhost/x` to `http://voussoir.localhost/x` and intercepts
/// that instead. The page therefore *is* the app's own origin, it just spells it
/// differently.
pub const REWRITTEN_APP_ORIGIN: &str = "http://voussoir.localhost";

/// The origin the page is actually served from on this target.
///
/// Anything that needs to name the app's own origin from the page side (the
/// invoke endpoint, a test fixture) must use this, not [`APP_ORIGIN`]: on
/// Windows and Android the two differ, and the page can only reach the handler
/// through the rewritten one.
#[cfg(any(target_os = "android", target_os = "windows"))]
pub const PAGE_ORIGIN: &str = REWRITTEN_APP_ORIGIN;
/// See above; the custom scheme survives on every other target.
#[cfg(not(any(target_os = "android", target_os = "windows")))]
pub const PAGE_ORIGIN: &str = APP_ORIGIN;

/// Whether `url` may be loaded in the app's webview.
///
/// Allowed:
/// - the app origin over the custom protocol
/// - `about:blank` (initial document, and what some libraries use)
/// - the dev-server origin, only when one was configured (debug builds)
///
/// Everything else is refused. `dev_origin` is `None` in release builds, so a
/// release binary cannot be redirected anywhere at runtime.
pub fn navigation_allowed(url: &str, dev_origin: Option<&str>) -> bool {
    if url == "about:blank" {
        return true;
    }
    if let Some(dev) = dev_origin {
        if url == dev || url.starts_with(&format!("{dev}/")) {
            return true;
        }
    }
    [APP_ORIGIN, REWRITTEN_APP_ORIGIN]
        .iter()
        .any(|origin| url == *origin || url.starts_with(&format!("{origin}/")))
}

/// The Content-Security-Policy served with every response.
///
/// `script-src 'self'` matters most: without it a compromised or careless page
/// can pull a remote script into a context that holds the IPC bridge. `unsafe-inline`
/// for styles only, because inline `<style>` is normal in bundled CSS output and
/// style tags cannot reach the bridge.
///
/// In dev the policy is relaxed: Bun's HMR client uses inline scripts and an
/// eval-based module loader, and `ws:` for the reload channel. That relaxation
/// is scoped to the dev origin, never to a shipped app. The only mobile branch
/// is Android's, documented below — and since the C2 measurement it serves the
/// same release policy as desktop, just spelled for its rewritten origin.
/// The Android release policy, spelled out where every target can test it:
/// the branch in [`csp`] is compile-time, so an assertion inside it only ever
/// runs on Android. The string itself is what the emulator measurement
/// proved the WebView accepts with the bridge intact.
const ANDROID_RELEASE_CSP: &str = "default-src 'self'; \
                                   script-src 'self'; \
                                   style-src 'self' 'unsafe-inline'; \
                                   img-src 'self' data: blob:; \
                                   font-src 'self' data:; \
                                   connect-src 'self'; \
                                   object-src 'none'; \
                                   base-uri 'none'; \
                                   form-action 'none'";

fn csp(dev: bool) -> &'static str {
    if cfg!(target_os = "android") {
        // On Android wry has no custom-scheme interception, so the page is
        // served from http://voussoir.localhost and `invoke` is a same-origin http
        // request (`connect-src 'self'` covers it). The bridge shim is wry's
        // injected initialization script, which is not subject to the page
        // CSP: measured 2026-11-19 on the android-36 emulator (system WebView
        // 133.0.6943.137), a scaffolded app whose page calls a command on load
        // round-tripped with `script-src 'self'` and no 'unsafe-inline'. This
        // branch therefore matches the desktop release policy; inline scripts
        // in an app's own page are blocked, exactly as they are on desktop.
        ANDROID_RELEASE_CSP
    } else if dev {
        // Dev-only relaxation, scoped to the dev origin and never shipped: Bun's
        // HMR client uses inline scripts and an eval-based module loader, and
        // `ws:` carries the reload channel.
        "default-src 'self'; \
         script-src 'self' 'unsafe-inline' 'unsafe-eval' voussoir:; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data: blob:; \
         font-src 'self' data:; \
         connect-src 'self' voussoir: http://voussoir.localhost ws: http://127.0.0.1:* http://localhost:*; \
         object-src 'none'; \
         base-uri 'none'; \
         form-action 'none'"
    } else {
        // The scheme is named next to 'self' on purpose. A custom-scheme origin
        // is opaque in some engines, and 'self' there does not reliably match a
        // <script type="module"> the page loads from its own origin: WKWebView
        // and WebView2 refused /main.js, so the page never ran and every command
        // call vanished while the window looked healthy. Naming `voussoir:` is
        // what dev mode already did, which is why dev worked and this did not.
        "default-src 'self'; \
         script-src 'self' voussoir:; \
         style-src 'self' voussoir: 'unsafe-inline'; \
         img-src 'self' voussoir: data: blob:; \
         font-src 'self' voussoir: data:; \
         connect-src 'self' voussoir:; \
         object-src 'none'; \
         base-uri 'none'; \
         form-action 'none'; \
         frame-ancestors 'none'"
    }
}

/// The CSP to attach to a response, or `None` if the caller asked for none.
pub fn content_security_policy(dev: bool) -> &'static str {
    csp(dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_origin_is_allowed() {
        assert!(navigation_allowed("voussoir://localhost/", None));
        assert!(navigation_allowed("voussoir://localhost/index.html", None));
        assert!(navigation_allowed(
            "voussoir://localhost/assets/app.js",
            None
        ));
        assert!(navigation_allowed("about:blank", None));
    }

    #[test]
    fn remote_pages_are_refused() {
        for url in [
            "http://example.com",
            "https://example.com/",
            "http://127.0.0.1:5601/escaped",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,<h1>hi</h1>",
            "voussoir:/etc/passwd",
            "voussoir://evil.com/",
        ] {
            assert!(!navigation_allowed(url, None), "{url} must be refused");
        }
    }

    #[test]
    fn prefix_match_does_not_leak_across_hosts() {
        // A naive starts_with("voussoir://localhost") would wrongly allow these.
        assert!(!navigation_allowed("voussoir://localhost.evil.com/", None));
        assert!(!navigation_allowed("voussoir://localhost@evil.com/", None));
    }

    #[test]
    fn ios_serves_the_desktop_policy_because_the_scheme_handler_exists_there() {
        // WKURLSchemeHandler intercepts voussoir://localhost natively on iOS, so
        // the page is the app's own origin exactly as on desktop, and the
        // rewritten-origin branch Android needs has no counterpart. Pin the
        // consequence instead of maintaining an iOS copy of the branch: the
        // policy iOS serves is the desktop release one, script-src tight, with
        // `voussoir:` reachable for the same-origin scheme requests.
        if !cfg!(target_os = "ios") {
            return;
        }
        assert!(content_security_policy(false).contains("script-src 'self'"));
        assert!(content_security_policy(false).contains("connect-src 'self' voussoir:"));
        assert!(navigation_allowed("voussoir://localhost/index.html", None));
    }

    #[test]
    fn the_rewritten_origin_is_still_the_app() {
        // wry serves the page from this origin on Android and on Windows;
        // refusing it would block the app's own first navigation and its
        // same-origin invoke requests.
        assert!(navigation_allowed(
            "http://voussoir.localhost/index.html",
            None
        ));
        assert!(navigation_allowed("http://voussoir.localhost/", None));
        // It must not become a hole: other http hosts stay refused.
        assert!(!navigation_allowed(
            "http://voussoir.localhost.evil.com/",
            None
        ));
        assert!(!navigation_allowed("http://example.com/", None));
        assert!(!navigation_allowed(
            "http://voussoir.localhost@evil.com/",
            None
        ));
    }

    #[test]
    fn dev_origin_is_allowed_only_when_configured() {
        let dev = Some("http://127.0.0.1:5177");
        assert!(navigation_allowed("http://127.0.0.1:5177/", dev));
        assert!(navigation_allowed("http://127.0.0.1:5177/index.html", dev));
        // Different port and different host stay refused.
        assert!(!navigation_allowed("http://127.0.0.1:5178/", dev));
        assert!(!navigation_allowed("http://example.com/", dev));
        // And without a dev origin, the same URL is refused outright.
        assert!(!navigation_allowed("http://127.0.0.1:5177/", None));
    }

    #[test]
    fn release_csp_forbids_remote_script() {
        let policy = content_security_policy(false);
        assert!(policy.contains("script-src 'self'"));
        assert!(
            !policy.contains("http://"),
            "release CSP must not allow http"
        );
        assert!(!policy.contains("'unsafe-eval'"));
        assert!(policy.contains("object-src 'none'"));
        assert!(policy.contains("frame-ancestors 'none'"));
    }

    #[test]
    fn android_policy_matches_the_desktop_release_script_src() {
        if !cfg!(target_os = "android") {
            // The branch is compile-time, so this assertion can only hold on
            // Android — where the CI emulator job compiles and runs it.
            return;
        }
        let policy = content_security_policy(false);
        assert!(policy.contains("script-src 'self'"));
        assert!(
            !policy.contains("script-src 'self' 'unsafe-inline'"),
            "the shim survives without it (verified on an emulator 2026-11-19)"
        );
        assert!(policy.contains("connect-src 'self'"));
        assert!(policy.contains("object-src 'none'"));
        assert!(policy.contains("base-uri 'none'"));
    }

    #[test]
    fn the_android_release_string_is_pinned_where_every_target_runs() {
        // The exact string the branch ships: a policy edit must be made in two
        // places, on purpose.
        assert_eq!(
            ANDROID_RELEASE_CSP,
            "default-src 'self'; \
             script-src 'self'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data: blob:; \
             font-src 'self' data:; \
             connect-src 'self'; \
             object-src 'none'; \
             base-uri 'none'; \
             form-action 'none'"
        );
        // And through the properties the guarantee rests on, on every target.
        let policy = ANDROID_RELEASE_CSP;
        assert!(policy.contains("script-src 'self'"));
        assert!(
            !policy.contains("script-src 'self' 'unsafe-inline'"),
            "the shim survives without it (verified on an emulator 2026-11-19)"
        );
        assert!(policy.contains("connect-src 'self'"));
        assert!(policy.contains("object-src 'none'"));
        assert!(policy.contains("base-uri 'none'"));
        assert!(policy.contains("form-action 'none'"));
    }

    #[test]
    fn dev_csp_allows_what_hmr_needs_and_nothing_more() {
        let policy = content_security_policy(true);
        assert!(policy.contains("ws:"));
        assert!(policy.contains("http://127.0.0.1:*"));
        assert!(policy.contains("object-src 'none'"));
        assert!(
            !policy.contains("https://"),
            "dev CSP must not open the internet"
        );
    }
}

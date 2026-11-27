//! Reading the feed over the network.
//!
//! Every rule here is about a hostile or hung endpoint: a deadline, a body cap,
//! and a URL that has to be trusted before a connection opens. Nothing in this
//! module knows what a manifest is, which is what makes it small enough to reason
//! about on its own.

use crate::Error;

/// Fetch a URL as text. Kept separate so the comparison logic stays testable
/// without a network.
///
/// On Android this goes through the activity's `HttpURLConnection`, because a
/// bundled TLS stack has to carry its own root certificates while the platform
/// one already trusts the system store.
///
/// The timeouts and the cap are not optional. ureq sets no default timeout, and
/// this command runs on a worker thread: a hung endpoint parked that worker
/// forever, which from the page looks like a command that never answers. The
/// endpoint answers with a version and a note, so a body past the cap is not
/// that document and reading it would let one bad endpoint cost an unbounded
/// allocation.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn fetch(url: &str) -> Result<String, Error> {
    fetch_limited(url, FETCH_TIMEOUT, BODY_CAP)
}

/// How long one update check may take, from connect to the last body byte.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The largest body an update check will read.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
const BODY_CAP: u64 = 64 * 1024;

/// [`fetch`] with the bounds as parameters, so a test can use a short deadline
/// and a tiny cap against a local socket instead of waiting out production
/// values.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn fetch_limited(url: &str, timeout: std::time::Duration, cap: u64) -> Result<String, Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout))
        .build()
        .new_agent();
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| Error::Handler(format!("update check failed: {e}")))?;
    response
        .body_mut()
        .with_config()
        .limit(cap)
        .read_to_string()
        .map_err(|e| Error::Handler(format!("update check failed: {e}")))
}

/// Fetch a URL as text through the activity's HTTP stack.
///
/// Android ships a curated certificate store the app cannot see, and a bundled
/// one has to be updated with the app, so the platform's `HttpURLConnection` is
/// the honest choice. It blocks, which is fine: `update.check` is async.
#[cfg(target_os = "android")]
fn fetch(url: &str) -> Result<String, Error> {
    crate::platform::http_get(url).ok_or_else(|| {
        Error::Handler("update check failed: no activity can make the request".to_owned())
    })
}

/// On iOS the honest answer is that there is nothing to fetch: apps update
/// through the App Store, and a bundled TLS stack would be the kind of
/// dynamic-policy surface Apple reviews line by line. `update.check` stays
/// registered so an app declaring the feature on every platform compiles,
/// and the version comparison stays testable; the endpoint fetch waits for
/// the Swift glue (IOS-PLAN Phase I-3) if a diagnostic fetch is ever wanted.
#[cfg(target_os = "ios")]
fn fetch(_url: &str) -> Result<String, Error> {
    Err(Error::Handler(
        "update checks are diagnostic on iOS; the App Store owns updates".to_owned(),
    ))
}

/// Tests for the fetch bounds. They use a local socket and real HTTP framing,
/// because the failure being fixed is a network call that never returns and a
/// body that never ends.
#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod fetch_tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    fn serve_once(response: String) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut scratch = [0u8; 1024];
                let _ = stream.read(&mut scratch);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        addr
    }

    fn ok_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn a_body_under_the_cap_is_read() {
        let addr = serve_once(ok_response(r#"{"version":"1.2.0"}"#));
        let body = fetch_limited(&format!("http://{addr}/"), Duration::from_secs(5), BODY_CAP)
            .expect("a well-framed response must be read");
        assert_eq!(body, r#"{"version":"1.2.0"}"#);
    }

    #[test]
    fn a_body_over_the_cap_is_refused() {
        let body = "x".repeat(4096);
        let addr = serve_once(ok_response(&body));
        let error =
            fetch_limited(&format!("http://{addr}/"), Duration::from_secs(5), 64).unwrap_err();
        assert!(
            error.to_string().contains("update check failed"),
            "the page must see a failed check, not a truncated document: {error}"
        );
    }

    #[test]
    fn a_hung_endpoint_times_out_instead_of_parking_the_worker() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
        let addr = listener.local_addr().expect("local addr");
        std::thread::spawn(move || {
            // Accept, then say nothing: this is what a hung endpoint does.
            if let Ok((stream, _)) = listener.accept() {
                std::thread::sleep(Duration::from_secs(30));
                drop::<TcpStream>(stream);
            }
        });
        let started = Instant::now();
        let error = fetch_limited(
            &format!("http://{addr}/"),
            Duration::from_millis(150),
            BODY_CAP,
        )
        .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the check must give up at its deadline, took {:?}",
            started.elapsed()
        );
        assert!(error.to_string().contains("update check failed"), "{error}");
    }
}

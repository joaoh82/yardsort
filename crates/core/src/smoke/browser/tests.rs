//! The `browser` step against a real browser, driving a page the test serves itself. Skipped
//! with a reason where no Chromium-family browser is found, as the finder reports — the same way
//! the runner's tests skip when `bun` is missing — so CI without a browser is not a failure.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::smoke::exec::{Stop, Watch};
use crate::smoke::{BrowserAction as BA, Screenshots};

/// A stop signal the test never raises.
struct NoWatch;
impl Watch for NoWatch {
    fn stop(&self) -> Option<Stop> {
        None
    }
    fn output_grew(&self, _step_id: &str) {}
}

/// A one-page app on a free port: `/` greets and offers a button to `/done`, which says so. Two
/// real pages, so `goto`, `click`, a navigation and `expect_url` are all exercised. Returns the
/// port and a flag that stops the server when the test drops it.
fn serve() -> (u16, Arc<AtomicBool>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    std::thread::spawn(move || {
        while !flag.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let mut buf = [0u8; 1024];
                    let read = socket.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_owned();
                    let body = if path.starts_with("/done") {
                        "<!doctype html><title>Done</title><h1>All done</h1>"
                    } else {
                        "<!doctype html><title>Home</title><h1>Welcome</h1>\
                         <input id=q><button onclick=\"location.href='/done'\">Go</button>"
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\
                         Connection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes());
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    });
    (port, stop)
}

#[test]
fn a_browser_step_visits_a_page_acts_on_it_and_screenshots_it() {
    let Some(browser) = find_browser("") else {
        eprintln!(
            "skipped: no Chromium-family browser found ({})",
            looked_for()
        );
        return;
    };
    let (port, stop) = serve();
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let profile = tmp.path().join("profile");

    let actions = vec![
        BA::Goto {
            url: format!("http://127.0.0.1:{port}/"),
        },
        BA::ExpectText {
            text: "Welcome".to_owned(),
        },
        BA::Fill {
            selector: "#q".to_owned(),
            text: "hi".to_owned(),
        },
        BA::Click {
            selector: "text=Go".to_owned(),
        },
        BA::WaitFor {
            selector: None,
            text: Some("All done".to_owned()),
        },
        BA::ExpectUrl {
            pattern: "**/done".to_owned(),
        },
        BA::ExpectTitle {
            title: "Done".to_owned(),
        },
        BA::Screenshot {
            name: "done".to_owned(),
        },
    ];
    let plan = Plan {
        browser: &browser,
        profile: &profile,
        output_dir: &output_dir,
        step_id: "sign_in",
        actions: &actions,
        screenshots: Screenshots::OnFailure,
        viewport: None,
        headed: false,
    };
    let done = carry_out(&plan, &NoWatch, Instant::now() + Duration::from_secs(60));
    stop.store(true, Ordering::Relaxed);

    assert!(done.passed, "the step should pass; note: {:?}", done.note);
    assert_eq!(
        done.screenshots.len(),
        1,
        "the `screenshot` action saved one"
    );
    assert!(
        std::path::Path::new(&done.screenshots[0]).is_file(),
        "the screenshot file was written"
    );
    assert!(
        done.url.as_deref().unwrap_or("").ends_with("/done"),
        "ended on the /done page, was {:?}",
        done.url
    );
    assert_eq!(done.title.as_deref(), Some("Done"));
}

#[test]
fn a_failed_expectation_fails_the_step_and_shoots_it() {
    let Some(browser) = find_browser("") else {
        eprintln!(
            "skipped: no Chromium-family browser found ({})",
            looked_for()
        );
        return;
    };
    let (port, stop) = serve();
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let profile = tmp.path().join("profile");

    let actions = vec![
        BA::Goto {
            url: format!("http://127.0.0.1:{port}/"),
        },
        // The page never says this: the step waits, then fails.
        BA::ExpectText {
            text: "This text is not on the page".to_owned(),
        },
    ];
    let plan = Plan {
        browser: &browser,
        profile: &profile,
        output_dir: &output_dir,
        step_id: "missing",
        actions: &actions,
        screenshots: Screenshots::OnFailure,
        viewport: None,
        headed: false,
    };
    // A short deadline so the wait gives up quickly.
    let done = carry_out(&plan, &NoWatch, Instant::now() + Duration::from_secs(8));
    stop.store(true, Ordering::Relaxed);

    assert!(!done.passed, "a missing expectation should fail");
    assert!(
        done.note
            .as_deref()
            .unwrap_or("")
            .contains("expected the text"),
        "the note says what was expected: {:?}",
        done.note
    );
    // On failure, with the default `on_failure`, one screenshot of the page as it was.
    assert_eq!(done.screenshots.len(), 1, "a failure screenshot was saved");
}

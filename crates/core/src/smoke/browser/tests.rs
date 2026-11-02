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

/// A stop signal that fires a cancel once `at` has passed, to end a step mid-flight.
struct CancelAt {
    at: Instant,
}
impl Watch for CancelAt {
    fn stop(&self) -> Option<Stop> {
        (Instant::now() >= self.at).then_some(Stop::Cancelled)
    }
    fn output_grew(&self, _step_id: &str) {}
}

/// A one-page app on a free port. `/` greets and carries an input, a `<select>`, a key-listening
/// input, a hidden element and a button to `/done`; each control records what it received into a
/// visible span, so the test can check that `fill`, `select` and `press` actually did something —
/// not only that the step passed. `/done` says it is done. Returns the port and a flag that stops
/// the server when the test drops it.
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
                    // `set_nonblocking` on the listener is inherited by accepted sockets on
                    // macOS and Windows (but not Linux), which would make this `read` return
                    // `WouldBlock` at once and serve the wrong page. Clear it per socket.
                    let _ = socket.set_nonblocking(false);
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
                        "<!doctype html><title>Done</title><h1>All done</h1>".to_owned()
                    } else {
                        HOME.to_owned()
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

/// The home page. Each control echoes what it got into a visible span, so a later `expect_text`
/// proves the action worked, not merely that nothing errored.
const HOME: &str = "<!doctype html><title>Home</title><h1>Welcome</h1>\
    <input id=q oninput=\"echo('out','filled:'+this.value)\">\
    <span id=out></span>\
    <select id=color onchange=\"echo('chosen','chosen:'+this.value)\">\
    <option value=red>Red</option><option value=blue>Blue</option></select>\
    <span id=chosen></span>\
    <span id=pressed></span>\
    <div id=secret style=\"display:none\">a secret</div>\
    <button onclick=\"location.href='/done'\">Go</button>\
    <script>function echo(id,t){document.getElementById(id).textContent=t;}\
    document.addEventListener('keydown',function(ev){\
    if(ev.key==='Enter')echo('pressed','pressed:Enter');});</script>";

/// Make the output directory and a profile folder under a fresh temp dir.
fn scratch() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let output_dir = tmp.path().join("out");
    std::fs::create_dir_all(&output_dir).unwrap();
    let profile = tmp.path().join("profile");
    (tmp, output_dir, profile)
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
    let _turn = crate::smoke::test_turns::browser();
    let (port, stop) = serve();
    let (_tmp, output_dir, profile) = scratch();

    let actions = vec![
        BA::Goto {
            url: format!("http://127.0.0.1:{port}/"),
        },
        BA::ExpectText {
            text: "Welcome".to_owned(),
        },
        BA::ExpectVisible {
            selector: "#q".to_owned(),
        },
        BA::ExpectHidden {
            selector: "#secret".to_owned(),
        },
        // Fill, then check the input event carried the value — the controlled-input path.
        BA::Fill {
            selector: "#q".to_owned(),
            text: "hi".to_owned(),
        },
        BA::ExpectText {
            text: "filled:hi".to_owned(),
        },
        // Select, then check the change event fired with the chosen value.
        BA::Select {
            selector: "#color".to_owned(),
            value: "blue".to_owned(),
        },
        BA::ExpectText {
            text: "chosen:blue".to_owned(),
        },
        // Press Enter, then check the keydown reached the page with the right key.
        BA::Press {
            key: "Enter".to_owned(),
        },
        BA::ExpectText {
            text: "pressed:Enter".to_owned(),
        },
        // Click by text, follow the navigation, and check where it landed.
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
    let done = carry_out(&plan, &NoWatch, Duration::from_secs(60));
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
    let _turn = crate::smoke::test_turns::browser();
    let (port, stop) = serve();
    let (_tmp, output_dir, profile) = scratch();

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
    // Short enough that the wait gives up, and long enough for a loaded runner to load the page.
    let done = carry_out(&plan, &NoWatch, Duration::from_secs(20));
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
    // On failure, with the default `on_failure`, one screenshot of the page as it was — taken
    // on its own budget, after the step's deadline has already passed.
    assert_eq!(done.screenshots.len(), 1, "a failure screenshot was saved");
}

#[test]
fn a_cancel_ends_a_browser_step() {
    let Some(browser) = find_browser("") else {
        eprintln!(
            "skipped: no Chromium-family browser found ({})",
            looked_for()
        );
        return;
    };
    let _turn = crate::smoke::test_turns::browser();
    let (port, stop) = serve();
    let (_tmp, output_dir, profile) = scratch();

    let actions = vec![
        BA::Goto {
            url: format!("http://127.0.0.1:{port}/"),
        },
        // Waits for text that never appears; the cancel must end it well before the deadline.
        BA::WaitFor {
            selector: None,
            text: Some("this never appears".to_owned()),
        },
    ];
    let plan = Plan {
        browser: &browser,
        profile: &profile,
        output_dir: &output_dir,
        step_id: "waiting",
        actions: &actions,
        screenshots: Screenshots::Never,
        viewport: None,
        headed: false,
    };
    // A generous deadline, but a cancel after a second and a half: the step ends on the cancel.
    let watch = CancelAt {
        at: Instant::now() + Duration::from_millis(1500),
    };
    let started = Instant::now();
    let done = carry_out(&plan, &watch, Duration::from_secs(120));
    stop.store(true, Ordering::Relaxed);

    assert!(
        matches!(done.stopped, Some(Stop::Cancelled)),
        "a cancel ends the step; note {:?}",
        done.note
    );
    assert_eq!(done.into_step_end().status, "cancelled");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the cancel ended it promptly, not at the deadline"
    );
}

#[test]
fn every_step_screenshots_each_action_and_never_skips_the_failure_shot() {
    let Some(browser) = find_browser("") else {
        eprintln!(
            "skipped: no Chromium-family browser found ({})",
            looked_for()
        );
        return;
    };
    let _turn = crate::smoke::test_turns::browser();
    let (port, stop) = serve();
    let (_tmp, output_dir, profile) = scratch();
    let url = format!("http://127.0.0.1:{port}/");

    // `every_step`: a shot after each action that is not itself a `screenshot`.
    let actions = vec![
        BA::Goto { url: url.clone() },
        BA::ExpectText {
            text: "Welcome".to_owned(),
        },
    ];
    let plan = Plan {
        browser: &browser,
        profile: &profile,
        output_dir: &output_dir,
        step_id: "every",
        actions: &actions,
        screenshots: Screenshots::EveryStep,
        viewport: None,
        headed: false,
    };
    let done = carry_out(&plan, &NoWatch, Duration::from_secs(30));
    assert!(done.passed, "note: {:?}", done.note);
    assert_eq!(
        done.screenshots.len(),
        2,
        "every_step shot each of the two actions"
    );

    // `never`: no shot even on failure.
    let actions = vec![
        BA::Goto { url },
        BA::ExpectText {
            text: "not on the page".to_owned(),
        },
    ];
    let plan = Plan {
        browser: &browser,
        profile: &profile,
        output_dir: &output_dir,
        step_id: "never",
        actions: &actions,
        screenshots: Screenshots::Never,
        viewport: None,
        headed: false,
    };
    let done = carry_out(&plan, &NoWatch, Duration::from_secs(8));
    stop.store(true, Ordering::Relaxed);
    assert!(!done.passed, "the expectation should fail");
    assert!(
        done.screenshots.is_empty(),
        "never took no screenshot, even on failure"
    );
}

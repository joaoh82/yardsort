//! A `browser` step: drive a Chromium found on the machine over the DevTools protocol.
//!
//! The runner is synchronous, so this is too. Chromium is launched headless (or headed, for
//! watching) with a debug port on `127.0.0.1`; the port and WebSocket path it writes into its
//! profile folder are read back, and a small JSON-RPC client over a blocking WebSocket speaks
//! the dozen methods a page needs. No async runtime, and the port is up only while the step
//! runs. The transport is all inside [`Conn`]; the actions do not know how the browser is
//! reached, so a pipe could replace it later without touching them.
//!
//! Most actions are carried out by evaluating a little JavaScript on the page — finding an
//! element by a CSS selector or `text=…`, checking it is visible, clicking or filling it. Every
//! action that needs an element waits for it, up to the step's timeout, so a file needs no
//! `sleep`. Keys (`press`) go through the input domain, so `Enter` submits a form as a real one
//! would.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use super::exec::{StepEnd, Stop, Watch};
use super::{BrowserAction, Screenshots, Viewport};
use crate::process::Group;

/// A viewport when the file names none: a laptop-ish window.
const DEFAULT_VIEWPORT: Viewport = Viewport {
    width: 1280,
    height: 800,
};

/// How long to wait for Chromium to write its `DevToolsActivePort` file before giving up. The
/// step's own clock does not start until the browser is up, so a slow launch does not eat into
/// it; this caps the launch itself.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a single poll of the page waits between looks.
const POLL: Duration = Duration::from_millis(100);

/// A budget of its own for a screenshot and for reading the page's address and title, so a
/// failure shot is still taken after the step's deadline has already passed.
const SHOT_BUDGET: Duration = Duration::from_secs(10);

/// How long to wait for the WebSocket to connect and finish its handshake, so a browser that
/// accepts the connection and then stalls cannot block the run thread forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a blocked read wakes to check the stop signal and the deadline.
const READ_TICK: Duration = Duration::from_millis(200);

/// What a `browser` step needs to run: the binary, where to keep its profile and screenshots,
/// the actions (variables already filled in), and how it should behave.
pub struct Plan<'a> {
    pub browser: &'a Path,
    pub profile: &'a Path,
    pub output_dir: &'a Path,
    pub step_id: &'a str,
    pub actions: &'a [BrowserAction],
    pub screenshots: Screenshots,
    pub viewport: Option<Viewport>,
    pub headed: bool,
}

/// What a `browser` step came to, before the runner turns it into a step row.
pub struct Done {
    passed: bool,
    note: Option<String>,
    url: Option<String>,
    title: Option<String>,
    screenshots: Vec<String>,
    stopped: Option<Stop>,
}

impl Done {
    fn failed(note: impl Into<String>) -> Self {
        Self {
            passed: false,
            note: Some(note.into()),
            url: None,
            title: None,
            screenshots: Vec::new(),
            stopped: None,
        }
    }

    /// The run was cancelled or timed out before, or while, the browser came up.
    fn stopped(stop: Stop) -> Self {
        Self {
            passed: false,
            note: None,
            url: None,
            title: None,
            screenshots: Vec::new(),
            stopped: Some(stop),
        }
    }

    pub fn into_step_end(self) -> StepEnd {
        let mut outputs = BTreeMap::new();
        if let Some(url) = &self.url {
            outputs.insert("url".to_owned(), url.clone());
        }
        if let Some(title) = &self.title {
            outputs.insert("title".to_owned(), title.clone());
        }
        let (status, note) = match (&self.stopped, self.passed) {
            (Some(Stop::Cancelled), _) => {
                ("cancelled", Some("Cancelled while this ran.".to_owned()))
            }
            (Some(_), _) => (
                "failed",
                Some("The run's time limit passed while this ran.".to_owned()),
            ),
            (None, true) => ("passed", None),
            (None, false) => ("failed", self.note),
        };
        StepEnd {
            status,
            exit: None,
            outputs,
            note,
            screenshots: self.screenshots,
            stopped: self.stopped,
        }
    }
}

/// A stop signal that never fires, for best-effort work that must still run after the step's
/// deadline has passed — a failure screenshot, the page's address and title for the record.
struct NoStop;
impl Watch for NoStop {
    fn stop(&self) -> Option<Stop> {
        None
    }
    fn output_grew(&self, _step_id: &str) {}
}

/// Carry out a `browser` step: launch the browser, run the actions, and close it. The step's
/// `timeout` clock starts once the browser is up, so a slow launch does not eat into it;
/// launching has [`LAUNCH_TIMEOUT`] of its own and honours a cancel meanwhile.
pub fn carry_out(plan: &Plan<'_>, watch: &dyn Watch, timeout: Duration) -> Done {
    if let Err(why) = std::fs::create_dir_all(plan.profile) {
        return Done::failed(format!("Cannot make the browser's profile folder: {why}"));
    }
    let viewport = plan.viewport.unwrap_or(DEFAULT_VIEWPORT);
    let launched = match launch(plan.browser, plan.profile, plan.headed, viewport, watch) {
        Ok(launched) => launched,
        Err(LaunchErr::Stopped(stop)) => return Done::stopped(stop),
        Err(LaunchErr::Failed(why)) => return Done::failed(why),
    };
    // Record the browser's pid and start fingerprint on the step now, so that if Yardsort dies
    // mid-step the restart's `recover` can kill it: the process outlives the app on Unix, where
    // nothing else reaps it (Windows has the job object).
    watch.browser_launched(
        plan.step_id,
        launched.group.id(),
        launched.group.fingerprint().as_deref(),
    );
    // `launched` is killed when it drops, whatever `run` returns.
    let deadline = Instant::now() + timeout;
    run(plan, watch, deadline, viewport, &launched)
}

/// A running browser, killed when this is dropped — so an early return never leaves one behind.
struct Launched {
    group: Group,
    port: u16,
    browser_ws: String,
}

impl Drop for Launched {
    fn drop(&mut self) {
        self.group.kill_tree();
    }
}

/// Why a launch did not yield a running browser: the run was stopped, or it failed.
enum LaunchErr {
    Stopped(Stop),
    Failed(String),
}

/// Launch the browser with its sandbox on. The sandbox often cannot start in a CI container or
/// under a Linux with unprivileged user namespaces restricted; only there, after a first launch
/// that failed with the sandbox, do we fall back to turning it off — never on a first try, so a
/// developer's own machine keeps the sandbox a `goto` to an arbitrary page relies on.
fn launch(
    browser: &Path,
    profile: &Path,
    headed: bool,
    viewport: Viewport,
    watch: &dyn Watch,
) -> Result<Launched, LaunchErr> {
    match try_launch(browser, profile, headed, viewport, true, watch) {
        Ok(launched) => Ok(launched),
        Err(LaunchErr::Failed(first)) => {
            match try_launch(browser, profile, headed, viewport, false, watch) {
                Ok(launched) => Ok(launched),
                // The first error is the one to report: the sandbox fallback is a last resort.
                Err(LaunchErr::Failed(_)) => Err(LaunchErr::Failed(first)),
                Err(stopped) => Err(stopped),
            }
        }
        Err(stopped) => Err(stopped),
    }
}

fn try_launch(
    browser: &Path,
    profile: &Path,
    headed: bool,
    viewport: Viewport,
    sandbox: bool,
    watch: &dyn Watch,
) -> Result<Launched, LaunchErr> {
    let mut command = Command::new(browser);
    command
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("--remote-debugging-port=0")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-sync")
        .arg("--disable-extensions")
        .arg("--disable-gpu")
        // Harmless, and keeps a small-/dev/shm CI box from crashing the renderer.
        .arg("--disable-dev-shm-usage")
        .arg(format!(
            "--window-size={},{}",
            viewport.width, viewport.height
        ))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !sandbox {
        command.arg("--no-sandbox");
    }
    if !headed {
        command.arg("--headless=new");
    }
    command.arg("about:blank");
    let mut group = Group::spawn(&mut command).map_err(|error| {
        LaunchErr::Failed(format!("`{}` could not start: {error}", browser.display()))
    })?;
    match devtools_endpoint(profile, &mut group, watch) {
        Ok((port, browser_ws)) => Ok(Launched {
            group,
            port,
            browser_ws,
        }),
        Err(why) => {
            group.kill_tree();
            Err(why)
        }
    }
}

/// Read the port and browser WebSocket path Chromium writes once its debugging server is up,
/// giving up after [`LAUNCH_TIMEOUT`], when the browser exits, or when the run is stopped.
fn devtools_endpoint(
    profile: &Path,
    group: &mut Group,
    watch: &dyn Watch,
) -> Result<(u16, String), LaunchErr> {
    let file = profile.join("DevToolsActivePort");
    let until = Instant::now() + LAUNCH_TIMEOUT;
    loop {
        if let Ok(text) = std::fs::read_to_string(&file) {
            let mut lines = text.lines();
            if let (Some(port), Some(path)) = (lines.next(), lines.next()) {
                if let Ok(port) = port.trim().parse::<u16>() {
                    return Ok((port, path.trim().to_owned()));
                }
            }
        }
        if let Ok(Some(status)) = group.try_wait() {
            return Err(LaunchErr::Failed(format!(
                "The browser exited before it was ready ({status})."
            )));
        }
        if let Some(stop) = watch.stop() {
            return Err(LaunchErr::Stopped(stop));
        }
        if Instant::now() >= until {
            return Err(LaunchErr::Failed(
                "The browser did not become ready within 30s.".to_owned(),
            ));
        }
        std::thread::sleep(POLL);
    }
}

/// Open a page and run the step's actions on it.
fn run(
    plan: &Plan<'_>,
    watch: &dyn Watch,
    deadline: Instant,
    viewport: Viewport,
    browser: &Launched,
) -> Done {
    let base = format!("ws://127.0.0.1:{}", browser.port);
    let mut client = match Conn::connect(&format!("{base}{}", browser.browser_ws)) {
        Ok(conn) => conn,
        Err(why) => return Done::failed(format!("Could not reach the browser: {why}")),
    };
    let target = match client.call(
        "Target.createTarget",
        json!({ "url": "about:blank" }),
        watch,
        deadline,
    ) {
        Ok(result) => match result.pointer("/targetId").and_then(Value::as_str) {
            Some(id) => id.to_owned(),
            None => return Done::failed("The browser opened no page."),
        },
        Err(CallError::Stopped(stop)) => return Done::stopped(stop),
        Err(CallError::Failed(why)) => {
            return Done::failed(format!("Could not open a page: {why}"))
        }
    };
    let mut page = match Conn::connect(&format!("{base}/devtools/page/{target}")) {
        Ok(conn) => conn,
        Err(why) => return Done::failed(format!("Could not reach the page: {why}")),
    };
    // `Page.enable` lets a `javascriptDialogOpening` event reach us, so `Conn::call` can dismiss
    // an `alert()`/`confirm()` instead of blocking on it forever.
    let _ = page.call("Page.enable", json!({}), watch, deadline);
    let _ = page.call(
        "Emulation.setDeviceMetricsOverride",
        json!({
            "width": viewport.width,
            "height": viewport.height,
            "deviceScaleFactor": 1,
            "mobile": false,
        }),
        watch,
        deadline,
    );

    let mut shots: Vec<String> = Vec::new();
    for (index, action) in plan.actions.iter().enumerate() {
        if let Some(stop) = watch.stop() {
            return done_stopped(stop, &mut page, deadline, shots);
        }
        match one(&mut page, action, watch, deadline) {
            Act::Did => {}
            Act::Stopped(stop) => return done_stopped(stop, &mut page, deadline, shots),
            Act::Failed(why) => {
                // On failure, a shot unless the file said never — this is the default, so the
                // record shows what the page looked like when the step gave up. It has a budget
                // of its own, since the step's deadline has usually just passed.
                if plan.screenshots != Screenshots::Never {
                    if let Ok(path) = shot(&mut page, plan, "failure") {
                        shots.push(path);
                    }
                }
                return Done {
                    passed: false,
                    note: Some(why),
                    screenshots: shots,
                    ..where_now(&mut page, deadline)
                };
            }
        }
        // An explicit `screenshot` already took its own; `every_step` takes one besides.
        if plan.screenshots == Screenshots::EveryStep
            && !matches!(action, BrowserAction::Screenshot { .. })
        {
            if let Ok(path) = shot(&mut page, plan, &format!("step-{}", index + 1)) {
                shots.push(path);
            }
        }
        if let BrowserAction::Screenshot { name } = action {
            match shot(&mut page, plan, name) {
                Ok(path) => shots.push(path),
                Err(why) => {
                    return Done {
                        passed: false,
                        note: Some(why),
                        screenshots: shots,
                        ..where_now(&mut page, deadline)
                    }
                }
            }
        }
    }
    Done {
        passed: true,
        note: None,
        screenshots: shots,
        ..where_now(&mut page, deadline)
    }
}

/// What carrying out one action came to.
enum Act {
    Did,
    Stopped(Stop),
    Failed(String),
}

/// Carry out one action, waiting up to `deadline` for anything it needs.
fn one(page: &mut Conn, action: &BrowserAction, watch: &dyn Watch, deadline: Instant) -> Act {
    match action {
        BrowserAction::Goto { url } => goto(page, url, watch, deadline),
        BrowserAction::Click { selector } => {
            act_on(page, selector, "click", watch, deadline, |e| {
                format!("{e}.scrollIntoView({{block:'center'}}); {e}.click();")
            })
        }
        BrowserAction::Fill { selector, text } => {
            let value = js(text);
            act_on(page, selector, "fill", watch, deadline, move |e| {
                // Assign through the prototype's own `value` setter, not `{e}.value=…`. A React
                // (or similar) controlled input shadows the element's `value` with a tracker;
                // writing the property directly leaves the tracker thinking nothing changed, so
                // the `input` event looks like a no-op and the framework's state stays empty.
                // The native setter writes past the tracker, the way a real keypress would.
                format!(
                    "{e}.focus(); \
                     {{ const d=Object.getOwnPropertyDescriptor(Object.getPrototypeOf({e}),'value'); \
                     if(d&&d.set){{ d.set.call({e},{value}); }} else {{ {e}.value={value}; }} }} \
                     {e}.dispatchEvent(new Event('input',{{bubbles:true}})); \
                     {e}.dispatchEvent(new Event('change',{{bubbles:true}}));"
                )
            })
        }
        BrowserAction::Select { selector, value } => select(page, selector, value, watch, deadline),
        BrowserAction::Press { key } => press(page, key, watch, deadline),
        BrowserAction::WaitFor { selector, text } => {
            wait_for(page, selector, text, watch, deadline)
        }
        BrowserAction::ExpectText { text } => {
            let body = format!(
                "(()=>{{ const b=document.body; return b?b.innerText.includes({t}):false; }})()",
                t = js(text)
            );
            expect(page, &body, watch, deadline, || {
                format!("expected the text {text:?} on the page, and it did not appear")
            })
        }
        BrowserAction::ExpectVisible { selector } => {
            let body = visible_js(selector);
            expect(page, &body, watch, deadline, || {
                format!("expected {selector} to be visible, and it was not")
            })
        }
        BrowserAction::ExpectHidden { selector } => {
            let body = format!(
                "(()=>{{ {PRELUDE} const e=__find({s}); return !e || !__vis(e); }})()",
                s = js(selector)
            );
            expect(page, &body, watch, deadline, || {
                format!("expected {selector} to be hidden, and it was visible")
            })
        }
        BrowserAction::ExpectUrl { pattern } => {
            let regex = match glob(pattern) {
                Ok(regex) => regex,
                Err(why) => return Act::Failed(why),
            };
            let body = "(()=>location.href)()".to_owned();
            expect_value(
                page,
                &body,
                watch,
                deadline,
                move |v| v.as_str().is_some_and(|url| regex.is_match(url)),
                || format!("expected the address to match {pattern}"),
            )
        }
        BrowserAction::ExpectTitle { title } => {
            let want = title.clone();
            let body = "(()=>document.title)()".to_owned();
            expect_value(
                page,
                &body,
                watch,
                deadline,
                move |v| v.as_str().is_some_and(|t| t.contains(&want)),
                || format!("expected the title to contain {title:?}"),
            )
        }
        // Handled by the caller; never reaches here.
        BrowserAction::Screenshot { .. } => Act::Did,
    }
}

/// `goto`: navigate, then wait until the document has finished loading. Only `errorText` is
/// checked, so a page that answers with a 404 or 500 is still "gone to": the guide says to assert
/// what the page shows with `expect_*`. A load that never completes within the step's time is a
/// failure, so a step does not pass on a page that never finished.
fn goto(page: &mut Conn, url: &str, watch: &dyn Watch, deadline: Instant) -> Act {
    match page.call("Page.navigate", json!({ "url": url }), watch, deadline) {
        Ok(result) => {
            if let Some(error) = result.get("errorText").and_then(Value::as_str) {
                return Act::Failed(format!("could not go to {url}: {error}"));
            }
        }
        Err(CallError::Stopped(stop)) => return Act::Stopped(stop),
        Err(CallError::Failed(why)) => return Act::Failed(format!("could not go to {url}: {why}")),
    }
    let body = "(()=>document.readyState==='complete')()";
    match poll(page, body, watch, deadline, |v| v == &Value::Bool(true)) {
        Poll::Ready => Act::Did,
        Poll::Stopped(stop) => Act::Stopped(stop),
        Poll::TimedOut(_) => Act::Failed(format!("{url} did not finish loading in time")),
        Poll::Failed(why) => Act::Failed(why),
    }
}

/// `click`/`fill`: wait for a visible element, then do something to it (`do_js` is given the
/// element variable name and returns the statements to run).
fn act_on(
    page: &mut Conn,
    selector: &str,
    verb: &str,
    watch: &dyn Watch,
    deadline: Instant,
    do_js: impl Fn(&str) -> String,
) -> Act {
    let body = format!(
        "(()=>{{ {PRELUDE} const e=__find({s}); if(!e) return 'missing'; \
         if(!__vis(e)) return 'hidden'; {stmts} return 'ok'; }})()",
        s = js(selector),
        stmts = do_js("e")
    );
    match poll(page, &body, watch, deadline, |v| {
        v == &Value::String("ok".into())
    }) {
        Poll::Ready => Act::Did,
        Poll::Stopped(stop) => Act::Stopped(stop),
        Poll::TimedOut(last) => Act::Failed(match last.as_str() {
            Some("hidden") => format!("could not {verb} {selector}: it stayed hidden"),
            _ => format!("could not {verb} {selector}: no such element appeared"),
        }),
        Poll::Failed(why) => Act::Failed(why),
    }
}

fn select(
    page: &mut Conn,
    selector: &str,
    value: &str,
    watch: &dyn Watch,
    deadline: Instant,
) -> Act {
    let body = format!(
        "(()=>{{ {PRELUDE} const e=__find({s}); if(!e) return 'missing'; e.value={v}; \
         e.dispatchEvent(new Event('change',{{bubbles:true}})); return e.value==={v}?'ok':'badvalue'; }})()",
        s = js(selector),
        v = js(value)
    );
    match poll(page, &body, watch, deadline, |v| {
        v == &Value::String("ok".into())
    }) {
        Poll::Ready => Act::Did,
        Poll::Stopped(stop) => Act::Stopped(stop),
        Poll::TimedOut(last) => Act::Failed(match last.as_str() {
            Some("badvalue") => format!("{value:?} is not an option of {selector}"),
            _ => format!("could not select in {selector}: no such element appeared"),
        }),
        Poll::Failed(why) => Act::Failed(why),
    }
}

/// `press`: a key to the focused element, through the input domain so it behaves as a real one.
fn press(page: &mut Conn, key: &str, watch: &dyn Watch, deadline: Instant) -> Act {
    let (dom_key, vk, code, text) = key_event(key);
    let mut down = json!({
        "type": "keyDown", "key": dom_key, "code": code, "windowsVirtualKeyCode": vk,
    });
    if let Some(text) = &text {
        down["text"] = json!(text);
    }
    let up = json!({
        "type": "keyUp", "key": dom_key, "code": code, "windowsVirtualKeyCode": vk,
    });
    for message in [down, up] {
        match page.call("Input.dispatchKeyEvent", message, watch, deadline) {
            Ok(_) => {}
            Err(CallError::Stopped(stop)) => return Act::Stopped(stop),
            Err(CallError::Failed(why)) => {
                return Act::Failed(format!("could not press {key}: {why}"))
            }
        }
    }
    Act::Did
}

fn wait_for(
    page: &mut Conn,
    selector: &Option<String>,
    text: &Option<String>,
    watch: &dyn Watch,
    deadline: Instant,
) -> Act {
    if let Some(selector) = selector {
        let body = visible_js(selector);
        return expect(page, &body, watch, deadline, || {
            format!("waited for {selector}, and it did not appear")
        });
    }
    if let Some(text) = text {
        let body = format!(
            "(()=>{{ const b=document.body; return b?b.innerText.includes({t}):false; }})()",
            t = js(text)
        );
        return expect(page, &body, watch, deadline, || {
            format!("waited for the text {text:?}, and it did not appear")
        });
    }
    Act::Did
}

/// Poll a boolean expression until it is true; a timeout fails with `why`.
fn expect(
    page: &mut Conn,
    body: &str,
    watch: &dyn Watch,
    deadline: Instant,
    why: impl Fn() -> String,
) -> Act {
    expect_value(
        page,
        body,
        watch,
        deadline,
        |v| v == &Value::Bool(true),
        why,
    )
}

/// Poll an expression until `ready` holds of its value; a timeout fails with `why`.
fn expect_value(
    page: &mut Conn,
    body: &str,
    watch: &dyn Watch,
    deadline: Instant,
    ready: impl Fn(&Value) -> bool,
    why: impl Fn() -> String,
) -> Act {
    match poll(page, body, watch, deadline, ready) {
        Poll::Ready => Act::Did,
        Poll::Stopped(stop) => Act::Stopped(stop),
        Poll::TimedOut(_) => Act::Failed(why()),
        Poll::Failed(why) => Act::Failed(why),
    }
}

/// The end of a poll of the page. `TimedOut` keeps the last value seen, for a message that says
/// how the element stood when the wait gave up.
enum Poll {
    Ready,
    Stopped(Stop),
    TimedOut(Value),
    Failed(String),
}

/// Evaluate `body` on the page over and over until `ready` holds of the result, the run is
/// stopped, or the deadline passes.
fn poll(
    page: &mut Conn,
    body: &str,
    watch: &dyn Watch,
    deadline: Instant,
    ready: impl Fn(&Value) -> bool,
) -> Poll {
    let mut last = Value::Null;
    loop {
        if let Some(stop) = watch.stop() {
            return Poll::Stopped(stop);
        }
        match eval(page, body, watch, deadline) {
            Ok(value) => {
                if ready(&value) {
                    return Poll::Ready;
                }
                last = value;
            }
            Err(CallError::Stopped(stop)) => return Poll::Stopped(stop),
            // A page navigating out from under us (after a `click`, say) tears down the context
            // the expression ran in. That is transient: keep polling, the new page is coming.
            Err(CallError::Failed(why)) if is_navigating(&why) => {}
            Err(CallError::Failed(why)) => return Poll::Failed(why),
        }
        if Instant::now() >= deadline {
            return Poll::TimedOut(last);
        }
        std::thread::sleep(POLL);
    }
}

/// Whether a `Runtime.evaluate` error is the page navigating underneath us, not a real failure.
fn is_navigating(why: &str) -> bool {
    why.contains("Execution context was destroyed")
        || why.contains("Cannot find context with specified id")
        || why.contains("Inspected target navigated or closed")
}

/// The page's address and title now, for the step's record. Best effort: a page that will not
/// answer leaves them out.
fn where_now(page: &mut Conn, _deadline: Instant) -> Done {
    // A budget of its own, not the step's: a failed or timed-out step still records where it was.
    let until = Instant::now() + Duration::from_secs(2);
    let url = eval(page, "(()=>location.href)()", &NoStop, until)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned));
    let title = eval(page, "(()=>document.title)()", &NoStop, until)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned));
    Done {
        passed: true,
        note: None,
        url,
        title,
        screenshots: Vec::new(),
        stopped: None,
    }
}

/// A run stopped mid-step: keep where the page was, for the record.
fn done_stopped(stop: Stop, page: &mut Conn, deadline: Instant, shots: Vec<String>) -> Done {
    Done {
        stopped: Some(stop),
        passed: false,
        note: None,
        screenshots: shots,
        ..where_now(page, deadline)
    }
}

/// Save a PNG of the page under the run's output, named after the step and `name`. It has a
/// budget of its own and never stops for a cancel or the step's deadline, since the one shot that
/// matters most is the failure shot taken once the deadline has already passed.
fn shot(page: &mut Conn, plan: &Plan<'_>, name: &str) -> Result<String, String> {
    let result = page
        .call(
            "Page.captureScreenshot",
            json!({ "format": "png", "captureBeyondViewport": false }),
            &NoStop,
            Instant::now() + SHOT_BUDGET,
        )
        .map_err(|error| error.into_message())?;
    let data = result
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| "the browser returned no screenshot".to_owned())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| format!("the screenshot was not readable: {error}"))?;
    let path = plan
        .output_dir
        .join(format!("{}.{}.png", plan.step_id, safe(name)));
    std::fs::write(&path, bytes).map_err(|error| {
        format!(
            "could not save the screenshot to {}: {error}",
            path.display()
        )
    })?;
    Ok(path.to_string_lossy().into_owned())
}

/// Evaluate a JavaScript expression on the page and return its value.
fn eval(
    page: &mut Conn,
    expression: &str,
    watch: &dyn Watch,
    deadline: Instant,
) -> Result<Value, CallError> {
    let result = page.call(
        "Runtime.evaluate",
        json!({
            "expression": expression,
            "returnByValue": true,
            "awaitPromise": true,
        }),
        watch,
        deadline,
    )?;
    if let Some(exception) = result.get("exceptionDetails") {
        let text = exception
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| exception.get("text").and_then(Value::as_str))
            .unwrap_or("the page script failed");
        return Err(CallError::Failed(text.to_owned()));
    }
    Ok(result
        .pointer("/result/value")
        .cloned()
        .unwrap_or(Value::Null))
}

/// Why a DevTools call did not return a value: the run was stopped (a cancel or the run's time
/// limit, seen while waiting for the reply), or it failed.
enum CallError {
    Stopped(Stop),
    Failed(String),
}

impl CallError {
    /// The reason in words, for a place that only wants a message (a stop reads as its own text).
    fn into_message(self) -> String {
        match self {
            CallError::Stopped(Stop::Cancelled) => "the run was cancelled".to_owned(),
            CallError::Stopped(_) => "the run's time limit passed".to_owned(),
            CallError::Failed(why) => why,
        }
    }
}

/// A blocking WebSocket to one DevTools endpoint, with a JSON-RPC call over it.
struct Conn {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    next_id: i64,
}

impl Conn {
    /// Connect to a DevTools WebSocket, bounded by [`CONNECT_TIMEOUT`] for both the TCP connect
    /// and the handshake, so a browser that accepts and then stalls cannot hang the run thread.
    fn connect(url: &str) -> Result<Self, String> {
        let authority = url
            .strip_prefix("ws://")
            .and_then(|rest| rest.split('/').next())
            .ok_or_else(|| format!("`{url}` is not a ws:// address"))?;
        let addr = authority
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .ok_or_else(|| format!("could not resolve `{authority}`"))?;
        let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT)
            .map_err(|error| error.to_string())?;
        // Bound the handshake; tightened to a short tick once it is up, so a `call` wakes often
        // enough to check for a stop.
        let _ = stream.set_read_timeout(Some(CONNECT_TIMEOUT));
        let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
        let (mut socket, _response) = tungstenite::client(url, MaybeTlsStream::Plain(stream))
            .map_err(|error| format!("the DevTools handshake did not complete: {error}"))?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            let _ = stream.set_read_timeout(Some(READ_TICK));
        }
        Ok(Self { socket, next_id: 0 })
    }

    /// Send one method and wait for its reply, ignoring the events that arrive meanwhile. Wakes
    /// every [`READ_TICK`] to honour a cancel or the run's time limit (`watch`) and the step's
    /// `deadline` — so a `goto` to a server that never answers, or a page stuck in a script, does
    /// not ignore `ys test cancel` until the whole timeout. A page that opens an `alert()` or
    /// `confirm()` is dismissed, since it would otherwise block every later call.
    fn call(
        &mut self,
        method: &str,
        params: Value,
        watch: &dyn Watch,
        deadline: Instant,
    ) -> Result<Value, CallError> {
        self.next_id += 1;
        let id = self.next_id;
        let message = json!({ "id": id, "method": method, "params": params }).to_string();
        self.socket
            .send(Message::text(message))
            .map_err(|error| CallError::Failed(error.to_string()))?;
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    let value: Value = match serde_json::from_str(text.as_str()) {
                        Ok(value) => value,
                        Err(_) => continue,
                    };
                    if value.get("method").and_then(Value::as_str)
                        == Some("Page.javascriptDialogOpening")
                    {
                        // Dismiss it, or the page — and this call — would wait on it forever.
                        self.next_id += 1;
                        let dismiss = json!({
                            "id": self.next_id,
                            "method": "Page.handleJavaScriptDialog",
                            "params": { "accept": false },
                        })
                        .to_string();
                        let _ = self.socket.send(Message::text(dismiss));
                        continue;
                    }
                    if value.get("id").and_then(Value::as_i64) != Some(id) {
                        continue; // an event, or another call's reply
                    }
                    if let Some(error) = value.get("error") {
                        let message = error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("the browser refused the request");
                        return Err(CallError::Failed(message.to_owned()));
                    }
                    return Ok(value.get("result").cloned().unwrap_or(Value::Null));
                }
                Ok(_) => continue,
                Err(tungstenite::Error::Io(error))
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    if let Some(stop) = watch.stop() {
                        return Err(CallError::Stopped(stop));
                    }
                    if Instant::now() >= deadline {
                        return Err(CallError::Failed(format!(
                            "`{method}` did not answer in time"
                        )));
                    }
                }
                Err(error) => return Err(CallError::Failed(error.to_string())),
            }
        }
    }
}

/// JavaScript that resolves a selector (CSS, or `text=…`) to an element and says if it is seen.
const PRELUDE: &str = "function __vis(e){ if(!e) return false; const s=getComputedStyle(e); \
    if(s.visibility==='hidden'||s.display==='none'||s.opacity==='0') return false; \
    return e.getClientRects().length>0; } \
    function __find(sel){ if(sel.startsWith('text=')){ const t=sel.slice(5).trim(); \
    const skip={SCRIPT:1,STYLE:1,NOSCRIPT:1,TEMPLATE:1}; \
    const all=[...document.querySelectorAll('body *')].filter(e=>!skip[e.tagName]); \
    const hit=all.filter(e=>(e.textContent||'').includes(t)); \
    const leaf=hit.filter(e=>!hit.some(o=>o!==e&&e.contains(o))); \
    return leaf.find(e=>__vis(e))||leaf[0]||null; } \
    return document.querySelector(sel); }";

/// `(()=>{ … __vis(__find(sel)) … })()` — whether a selector's element is visible.
fn visible_js(selector: &str) -> String {
    format!(
        "(()=>{{ {PRELUDE} return __vis(__find({s})); }})()",
        s = js(selector)
    )
}

/// A string as a JavaScript string literal, so a selector or value with quotes or newlines is
/// safe to embed in an expression.
fn js(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

/// A screenshot name reduced to letters, digits, `-` and `_`, so it is a safe file name.
fn safe(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "shot".to_owned()
    } else {
        cleaned
    }
}

/// The named keys `press` knows, beyond a single printable character. A multi-character name
/// outside this list is caught when the file is checked, so `press: F5` is not silently sent as
/// `F`. Kept beside [`key_event`], which must handle each of these.
pub const KEYS: &[&str] = &[
    "Enter",
    "Tab",
    "Escape",
    "Esc",
    "Backspace",
    "Delete",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Space",
];

/// A `press` key as the input domain wants it: the DOM `key` value, a virtual-key code, a code,
/// and the text it types, if any. The handful of keys a smoke test presses, and single printable
/// characters. The DOM `key` is returned so `Space` arrives as `" "`, not the word `"Space"`.
fn key_event(key: &str) -> (String, i64, &'static str, Option<String>) {
    match key {
        "Enter" => ("Enter".to_owned(), 13, "Enter", Some("\r".to_owned())),
        "Tab" => ("Tab".to_owned(), 9, "Tab", None),
        "Escape" | "Esc" => ("Escape".to_owned(), 27, "Escape", None),
        "Backspace" => ("Backspace".to_owned(), 8, "Backspace", None),
        "Delete" => ("Delete".to_owned(), 46, "Delete", None),
        "ArrowUp" => ("ArrowUp".to_owned(), 38, "ArrowUp", None),
        "ArrowDown" => ("ArrowDown".to_owned(), 40, "ArrowDown", None),
        "ArrowLeft" => ("ArrowLeft".to_owned(), 37, "ArrowLeft", None),
        "ArrowRight" => ("ArrowRight".to_owned(), 39, "ArrowRight", None),
        "Home" => ("Home".to_owned(), 36, "Home", None),
        "End" => ("End".to_owned(), 35, "End", None),
        "PageUp" => ("PageUp".to_owned(), 33, "PageUp", None),
        "PageDown" => ("PageDown".to_owned(), 34, "PageDown", None),
        " " | "Space" => (" ".to_owned(), 32, "Space", Some(" ".to_owned())),
        other => {
            let ch = other.chars().next().unwrap_or(' ');
            let vk = ch.to_ascii_uppercase() as i64;
            (ch.to_string(), vk, "", Some(ch.to_string()))
        }
    }
}

/// A glob over the page address (`**/done`, `*.html`) as an anchored regex: `**` spans any
/// characters including `/`, `*` any but `/`, `?` one.
fn glob(pattern: &str) -> Result<regex::Regex, String> {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    out.push_str(".*");
                } else {
                    out.push_str("[^/]*");
                }
            }
            '?' => out.push('.'),
            other => out.push_str(&regex::escape(&other.to_string())),
        }
    }
    out.push('$');
    regex::Regex::new(&out)
        .map_err(|error| format!("`{pattern}` is not a valid address glob: {error}"))
}

/// Where Chromium-family browsers are looked for, in words, for the message when none is found.
pub fn looked_for() -> String {
    let names = CANDIDATES.join(", ");
    format!("{names} on your PATH, and the usual install folders")
}

/// Why a `browser` step cannot run, in words, given the project's `browser` setting: a named path
/// that is not there is its own problem, distinct from finding none at all.
pub fn missing_browser(setting: &str) -> String {
    let setting = setting.trim();
    if setting.is_empty() {
        format!("no browser was found (looked for {})", looked_for())
    } else {
        format!("the browser set for this project, `{setting}`, is not a file on this machine")
    }
}

/// Candidate browser program names, Chrome before Chromium before Edge.
#[cfg(target_os = "macos")]
const CANDIDATES: &[&str] = &["google-chrome", "chromium", "microsoft-edge"];
#[cfg(target_os = "linux")]
const CANDIDATES: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
];
#[cfg(target_os = "windows")]
const CANDIDATES: &[&str] = &["chrome", "msedge"];

/// The usual absolute install locations, checked after PATH.
#[cfg(target_os = "macos")]
const PLACES: &[&str] = &[
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
];
#[cfg(target_os = "linux")]
const PLACES: &[&str] = &[
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/snap/bin/chromium",
];
#[cfg(target_os = "windows")]
const PLACES: &[&str] = &[
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
];

/// The browser a `browser` step should use. A project that names one is taken at its word: that
/// path if it is a file, and `None` otherwise — never a silent fall back to a different browser,
/// since the guide promises the named one is used. With nothing named, the first found on PATH,
/// then in the usual places. `None` when the machine has none.
pub fn find_browser(setting: &str) -> Option<PathBuf> {
    let setting = setting.trim();
    if !setting.is_empty() {
        let named = PathBuf::from(setting);
        return named.is_file().then_some(named);
    }
    for name in CANDIDATES {
        if let Ok(path) = which::which(name) {
            return Some(path);
        }
    }
    for place in PLACES {
        let path = PathBuf::from(place);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests;

//! The dashboard as a browser meets it: served for real on 127.0.0.1 by a stand-in host, asked
//! over real HTTP. Who is let in, who is refused, and that what the page asks for is done by
//! real programs: a shell, git, and stand-ins for the vendor's installer, `gh` and `rustunnel`.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use browser::{Method, RequestBuilder, Response, StatusCode, COOKIE, HOST, ORIGIN, SET_COOKIE};
use host_dashboard::{Dashboard, Options};
use serde_json::{json, Value};
use yardsort_core::program::testing::executable;

/// A browser's part, as little of one as the tests need: one request over plain HTTP/1.1 to
/// 127.0.0.1, with exactly the headers it was given, and the whole answer read.
mod browser {
    use std::collections::BTreeMap;
    use std::io::{Read, Write};

    pub const COOKIE: &str = "cookie";
    pub const HOST: &str = "host";
    pub const ORIGIN: &str = "origin";
    pub const SET_COOKIE: &str = "set-cookie";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Method {
        Get,
        Post,
        Delete,
    }

    impl Method {
        fn verb(self) -> &'static str {
            match self {
                Self::Get => "GET",
                Self::Post => "POST",
                Self::Delete => "DELETE",
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusCode(pub u16);

    impl StatusCode {
        pub const OK: Self = Self(200);
        pub const PERMANENT_REDIRECT: Self = Self(308);
        pub const BAD_REQUEST: Self = Self(400);
        pub const UNAUTHORIZED: Self = Self(401);
        pub const FORBIDDEN: Self = Self(403);
    }

    pub struct RequestBuilder {
        method: Method,
        port: u16,
        path: String,
        headers: BTreeMap<String, String>,
        body: Option<String>,
    }

    impl RequestBuilder {
        /// A request to `address`, `http://127.0.0.1:<port>`, addressed to it by that name.
        pub fn new(method: Method, address: &str, path: &str) -> Self {
            let authority = address.trim_start_matches("http://");
            let port = authority.rsplit(':').next().unwrap().parse().unwrap();
            Self {
                method,
                port,
                path: path.to_owned(),
                headers: BTreeMap::from([("host".to_owned(), authority.to_owned())]),
                body: None,
            }
        }

        /// Set a header, in place of one of the same name.
        pub fn header(mut self, name: &str, value: impl AsRef<str>) -> Self {
            self.headers
                .insert(name.to_ascii_lowercase(), value.as_ref().to_owned());
            self
        }

        pub fn json(mut self, body: &serde_json::Value) -> Self {
            self.body = Some(body.to_string());
            self.header("content-type", "application/json")
        }

        pub fn send(self) -> std::io::Result<Response> {
            let mut stream = std::net::TcpStream::connect(("127.0.0.1", self.port))?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
            let body = self.body.unwrap_or_default();
            let mut request = format!("{} {} HTTP/1.1\r\n", self.method.verb(), self.path);
            for (name, value) in &self.headers {
                request.push_str(&format!("{name}: {value}\r\n"));
            }
            request.push_str(&format!(
                "content-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            ));
            stream.write_all(request.as_bytes())?;
            let mut answer = Vec::new();
            stream.read_to_end(&mut answer)?;
            let answer = String::from_utf8_lossy(&answer).into_owned();
            let (head, body) = answer.split_once("\r\n\r\n").expect("a whole answer");
            let mut lines = head.split("\r\n");
            let status = lines
                .next()
                .unwrap()
                .split(' ')
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            let headers = lines
                .filter_map(|line| line.split_once(": "))
                .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
                .collect();
            Ok(Response {
                status: StatusCode(status),
                headers,
                body: body.to_owned(),
            })
        }
    }

    pub struct Response {
        status: StatusCode,
        headers: BTreeMap<String, String>,
        body: String,
    }

    impl Response {
        pub fn status(&self) -> StatusCode {
            self.status
        }

        pub fn headers(&self) -> &BTreeMap<String, String> {
            &self.headers
        }

        pub fn text(self) -> std::io::Result<String> {
            Ok(self.body)
        }

        pub fn json(self) -> serde_json::Result<serde_json::Value> {
            serde_json::from_str(&self.body)
        }
    }
}

/// A dashboard on a port of its own, with a home, a `bin` and a browser.
struct Served {
    // Dropped first: the dashboard stops before its runtime and its folder go.
    dashboard: Option<Dashboard>,
    runtime: tokio::runtime::Runtime,
    home: tempfile::TempDir,
    source: Arc<common::StandIn>,
    /// The session's cookie, once signed in.
    cookie: Option<String>,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

impl Served {
    fn start() -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("bin")).unwrap();
        std::fs::write(
            home.path().join("settings.toml"),
            format!("[dashboard]\nport = {}\n", free_port()),
        )
        .unwrap();
        Self::start_in(home)
    }

    fn start_in(home: tempfile::TempDir) -> Self {
        Self::start_with(home, None)
    }

    /// A host that lets the page sign it in and pair devices, as `yardsort-host` does.
    fn start_remote() -> Self {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("bin")).unwrap();
        std::fs::write(
            home.path().join("settings.toml"),
            format!("[dashboard]\nport = {}\n", free_port()),
        )
        .unwrap();
        Self::start_with(home, Some(common::StandInRemote::default()))
    }

    fn start_with(home: tempfile::TempDir, remote: Option<common::StandInRemote>) -> Self {
        let mut source = common::StandIn::new(home.path(), &home.path().join("bin"));
        source.remote = remote;
        let source = Arc::new(source);
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let dashboard = runtime
            .block_on(Dashboard::start(
                source.clone(),
                Options {
                    state_dir: home.path().join("dashboard"),
                },
            ))
            .expect("the dashboard starts");
        Self {
            dashboard: Some(dashboard),
            runtime,
            home,
            source,
            cookie: None,
        }
    }

    fn dashboard(&self) -> &Dashboard {
        self.dashboard.as_ref().unwrap()
    }

    fn bin(&self) -> PathBuf {
        self.home.path().join("bin")
    }

    /// Stop the dashboard and give back its home, to start another on.
    fn stop(mut self) -> tempfile::TempDir {
        let dashboard = self.dashboard.take().unwrap();
        self.runtime.block_on(dashboard.stop());
        self.home
    }

    /// A request as the page makes it: with its header, and the session's cookie if there is one.
    fn page(&self, method: Method, path: &str) -> RequestBuilder {
        let request = RequestBuilder::new(method, &self.dashboard().address(), path)
            .header("x-yardsort-dashboard", "1");
        match &self.cookie {
            Some(cookie) => request.header(COOKIE, cookie),
            None => request,
        }
    }

    fn get(&self, path: &str) -> Value {
        let response = self.page(Method::Get, path).send().unwrap();
        assert_eq!(response.status(), StatusCode::OK, "GET {path}");
        response.json().unwrap()
    }

    fn post(&self, path: &str, body: Value) -> Response {
        self.page(Method::Post, path).json(&body).send().unwrap()
    }

    /// Sign this browser in with a code from the host.
    fn sign_in(&mut self) {
        let code = self.dashboard().sign_in().code;
        let response = self.post("/api/session", json!({ "code": code }));
        assert_eq!(response.status(), StatusCode::OK);
        self.cookie = Some(cookie_of(&response));
    }

    /// Follow a job to its end: how it ended, and everything it printed.
    fn finished(&self, job: &Value) -> (String, String) {
        let id = job["id"].as_u64().expect("a job");
        let since = Instant::now();
        loop {
            let reading = self.get(&format!("/api/jobs/{id}?from=0"));
            if reading["state"] != "running" {
                return (
                    reading["state"].as_str().unwrap().to_owned(),
                    reading["output"].as_str().unwrap().to_owned(),
                );
            }
            assert!(since.elapsed() < Duration::from_secs(30), "{reading}");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

/// The `name=value` of the cookie a response sets.
fn cookie_of(response: &Response) -> String {
    let set = &response.headers()[SET_COOKIE];
    set.split(';').next().unwrap().to_owned()
}

fn refusal(response: Response) -> (StatusCode, String) {
    let status = response.status();
    let body: Value = response.json().unwrap();
    (
        status,
        body["error"].as_str().unwrap_or_default().to_owned(),
    )
}

#[test]
fn the_page_is_served_to_anyone_and_nothing_else_is_until_a_browser_signs_in() {
    let served = Served::start();
    let page = served.page(Method::Get, "/").send().unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let policy = page.headers()["content-security-policy"].clone();
    assert!(policy.contains("default-src 'none'") && policy.contains("frame-ancestors 'none'"));
    assert_eq!(page.headers()["cache-control"], "no-store");
    assert!(page
        .text()
        .unwrap()
        .contains("<title>Yardsort host</title>"));
    for file in ["/app.js", "/app.css", "/favicon.svg"] {
        assert_eq!(
            served.page(Method::Get, file).send().unwrap().status(),
            StatusCode::OK
        );
    }

    assert_eq!(served.get("/api/session")["signedIn"], false);
    for path in [
        "/api/status",
        "/api/agents",
        "/api/git",
        "/api/jobs",
        "/api/jobs/1",
    ] {
        let refused = refusal(served.page(Method::Get, path).send().unwrap());
        assert_eq!(
            refused,
            (StatusCode::UNAUTHORIZED, "signed_out".into()),
            "{path}"
        );
    }
    let refused = refusal(served.post("/api/commands", json!({ "command": "touch ran" })));
    assert_eq!(refused, (StatusCode::UNAUTHORIZED, "signed_out".into()));
    assert!(!served.home.path().join("ran").exists());
    // A cookie of the right name and a made-up value is nobody's session.
    let forged = served
        .page(Method::Get, "/api/status")
        .header(COOKIE, "yardsort_dashboard_00000000=0000")
        .send()
        .unwrap();
    assert_eq!(forged.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn a_code_from_the_host_signs_one_browser_in_once_and_signing_out_ends_it() {
    let mut served = Served::start();
    let wrong = refusal(served.post("/api/session", json!({ "code": "0000-0000-0000-0000" })));
    assert_eq!(wrong, (StatusCode::UNAUTHORIZED, "wrong_code".into()));

    let offered = served.dashboard().sign_in();
    assert_eq!(
        offered.local,
        format!("{}/#code={}", served.dashboard().address(), offered.code)
    );
    assert_eq!(offered.public, None, "no tunnel, no public address");
    let response = served.post("/api/session", json!({ "code": offered.code }));
    assert_eq!(response.status(), StatusCode::OK);
    let set = response.headers()[SET_COOKIE].clone();
    // Not for scripts, not for other sites, and not marked for HTTPS on plain loopback.
    assert!(
        set.contains("HttpOnly") && set.contains("SameSite=Strict"),
        "{set}"
    );
    assert!(!set.contains("Secure"), "{set}");
    served.cookie = Some(cookie_of(&response));

    let status = served.get("/api/status");
    assert_eq!(status["host"]["program"], "yardsort-host");
    assert_eq!(status["host"]["remote"]["name"], "Build box");
    assert_eq!(status["dashboard"]["address"], served.dashboard().address());
    assert_eq!(status["dashboard"]["signedIn"], 1);
    assert_eq!(status["dashboard"]["tunnel"]["state"], "off");
    assert_eq!(status["user"], "yardsort");
    assert_eq!(status["shell"], "sh -c");
    assert!(status["machine"]["memory"]["total"].as_u64().unwrap() > 0);

    // The code is spent: a second browser cannot use it.
    let again = refusal(served.post("/api/session", json!({ "code": offered.code })));
    assert_eq!(again, (StatusCode::UNAUTHORIZED, "wrong_code".into()));

    let out = served.page(Method::Delete, "/api/session").send().unwrap();
    assert_eq!(out.status(), StatusCode::OK);
    let refused = refusal(served.page(Method::Get, "/api/status").send().unwrap());
    assert_eq!(refused, (StatusCode::UNAUTHORIZED, "signed_out".into()));
}

#[test]
fn a_request_addressed_to_another_name_is_refused_whatever_it_carries() {
    let mut served = Served::start();
    served.sign_in();
    // What a page on another site gets a browser to send, once its name resolves to 127.0.0.1.
    for path in ["/", "/api/status", "/api/session"] {
        let rebound = served
            .page(Method::Get, path)
            .header(HOST, "evil.example.test")
            .send()
            .unwrap();
        assert_eq!(
            refusal(rebound),
            (StatusCode::FORBIDDEN, "wrong_address".into()),
            "{path}"
        );
    }
    // A forwarded port is another port on a loopback name: that is still this dashboard.
    let forwarded = served
        .page(Method::Get, "/api/status")
        .header(HOST, "localhost:9000")
        .send()
        .unwrap();
    assert_eq!(forwarded.status(), StatusCode::OK);
}

#[test]
fn a_change_is_taken_only_from_the_dashboards_own_page() {
    let mut served = Served::start();
    served.sign_in();
    let cookie = served.cookie.clone().unwrap();
    let address = served.dashboard().address();
    let command = json!({ "command": "touch ran" });

    // A form or a link on another site: the cookie would not even be sent, but were it, there
    // is no header.
    let bare = RequestBuilder::new(Method::Post, &address, "/api/commands")
        .header(COOKIE, &cookie)
        .json(&command)
        .send()
        .unwrap();
    assert_eq!(
        refusal(bare),
        (StatusCode::FORBIDDEN, "not_the_page".into())
    );
    // A script on another site that somehow added the header says where it is from.
    let elsewhere = served
        .page(Method::Post, "/api/commands")
        .header(ORIGIN, "https://evil.example.test")
        .json(&command)
        .send()
        .unwrap();
    assert_eq!(
        refusal(elsewhere),
        (StatusCode::FORBIDDEN, "not_the_page".into())
    );
    // Signing in is a change too.
    let code = served.dashboard().sign_in().code;
    let sign_in = RequestBuilder::new(Method::Post, &address, "/api/session")
        .json(&json!({ "code": code }))
        .send()
        .unwrap();
    assert_eq!(
        refusal(sign_in),
        (StatusCode::FORBIDDEN, "not_the_page".into())
    );

    std::thread::sleep(Duration::from_millis(200));
    assert!(!served.home.path().join("ran").exists());
    assert_eq!(served.get("/api/jobs"), json!([]));

    // The page itself, saying it is the page.
    let own = served
        .page(Method::Post, "/api/commands")
        .header(ORIGIN, &address)
        .json(&command)
        .send()
        .unwrap();
    assert_eq!(own.status(), StatusCode::OK);
    let (state, _) = served.finished(&own.json().unwrap());
    assert_eq!(state, "succeeded");
    assert!(served.home.path().join("ran").exists());
}

#[test]
fn a_command_runs_in_the_home_as_a_shell_would_run_it_and_can_be_stopped() {
    let mut served = Served::start();
    served.sign_in();
    let job: Value = served
        .post(
            "/api/commands",
            json!({ "command": "echo one && pwd; echo two >&2; exit 4" }),
        )
        .json()
        .unwrap();
    assert_eq!(job["kind"], "command");
    let (state, output) = served.finished(&job);
    assert_eq!(state, "failed");
    assert!(
        output.starts_with("$ echo one && pwd; echo two >&2; exit 4\n"),
        "{output}"
    );
    let home = std::fs::canonicalize(served.home.path()).unwrap();
    assert!(
        output.contains(&format!("one\n{}\n", home.display()))
            || output.contains(&format!("one\n{}\n", served.home.path().display())),
        "{output}"
    );
    assert!(
        output.contains("two\n") && output.contains("Ended with"),
        "{output}"
    );

    // One that waits for an answer gets none: nothing is on its input.
    let asks: Value = served
        .post(
            "/api/commands",
            json!({ "command": "read answer; echo \"got [$answer]\"" }),
        )
        .json()
        .unwrap();
    assert!(served.finished(&asks).1.contains("got []"));

    let sleeps: Value = served
        .post(
            "/api/commands",
            json!({ "command": "echo started; sleep 600" }),
        )
        .json()
        .unwrap();
    let id = sleeps["id"].as_u64().unwrap();
    let since = Instant::now();
    while !served.get(&format!("/api/jobs/{id}"))["output"]
        .as_str()
        .unwrap()
        .contains("started")
    {
        assert!(since.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(20));
    }
    let stopped: Value = served
        .post(&format!("/api/jobs/{id}/stop"), json!({}))
        .json()
        .unwrap();
    assert_eq!(stopped["stopped"], true);
    assert_eq!(served.finished(&sleeps).0, "stopped");

    let jobs = served.get("/api/jobs");
    let titles: Vec<&str> = jobs
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles.len(), 3);
    assert_eq!(titles[0], "echo started; sleep 600", "the newest first");

    for not_a_command in ["", "   "] {
        let refused = served.post("/api/commands", json!({ "command": not_a_command }));
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    }
}

#[test]
fn the_commit_identity_is_read_from_git_and_set_in_this_users_own_config() {
    let mut served = Served::start();
    served.sign_in();
    let before = served.get("/api/git");
    assert!(before["version"]
        .as_str()
        .unwrap()
        .starts_with("git version"));
    assert_eq!(before["name"], Value::Null);
    // Whether there is a GitHub CLI is the machine's to say (a CI runner has one in /usr/bin).
    assert_eq!(before["github"]["signedIn"], false);
    assert_eq!(before["projectsDir"], "~/Projects");

    let saved = served.post(
        "/api/git/identity",
        json!({ "name": "Build Box", "email": "box@example.invalid" }),
    );
    assert_eq!(saved.status(), StatusCode::OK);
    let after = served.get("/api/git");
    assert_eq!(after["name"], "Build Box");
    assert_eq!(after["email"], "box@example.invalid");
    let kept = std::fs::read_to_string(served.home.path().join(".gitconfig")).unwrap();
    assert!(kept.contains("email = box@example.invalid"), "{kept}");

    let refused = served.post(
        "/api/git/identity",
        json!({ "name": "", "email": "x@example.invalid" }),
    );
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let said: Value = refused.json().unwrap();
    assert!(
        said["message"].as_str().unwrap().contains("needs a name"),
        "{said}"
    );
}

/// A `gh` that is signed in once it was given a token, which it keeps in its home.
fn stand_in_gh(bin: &Path) {
    executable(
        bin.join("gh"),
        r#"#!/bin/sh
case "$*" in
  --version) echo 'gh version 9.9.9 (stand-in)';;
  'auth status --hostname github.com')
    test -e "$HOME/gh-token" || { echo 'You are not logged into any GitHub hosts.' >&2; exit 1; }
    echo 'Logged in to github.com account build-box';;
  'auth login --hostname github.com --with-token') read token; echo "$token" > "$HOME/gh-token";;
  'auth setup-git --hostname github.com') touch "$HOME/gh-setup-git";;
  'auth login --hostname github.com --git-protocol https --web')
    echo '! First copy your one-time code: 1A2B-3C4D' >&2
    echo "browser=$BROWSER prompt=$GH_PROMPT_DISABLED" >&2
    touch "$HOME/gh-token";;
  'auth logout --hostname github.com') rm -f "$HOME/gh-token"; echo 'Logged out' >&2;;
  *) echo "unexpected: $*" >&2; exit 9;;
esac
"#,
    );
}

#[test]
fn the_github_cli_is_signed_in_with_a_token_on_its_input_or_from_a_browser_and_out_again() {
    let mut served = Served::start();
    stand_in_gh(&served.bin());
    served.sign_in();
    let github = |served: &Served| served.get("/api/git")["github"].clone();
    assert_eq!(github(&served)["signedIn"], false);
    assert_eq!(github(&served)["version"], "gh version 9.9.9 (stand-in)");

    let job: Value = served
        .post(
            "/api/git/github/token",
            json!({ "token": "github_pat_example" }),
        )
        .json()
        .unwrap();
    let (state, output) = served.finished(&job);
    assert_eq!(state, "succeeded", "{output}");
    assert!(!output.contains("github_pat_example"), "{output}");
    assert!(
        output.contains("$ gh auth login --hostname github.com --with-token"),
        "{output}"
    );
    let kept = std::fs::read_to_string(served.home.path().join("gh-token")).unwrap();
    assert_eq!(kept.trim(), "github_pat_example");
    assert!(
        served.home.path().join("gh-setup-git").exists(),
        "git was told to push through gh"
    );
    assert_eq!(github(&served)["signedIn"], true);
    assert!(github(&served)["detail"]
        .as_str()
        .unwrap()
        .contains("account build-box"));
    // The token is in no job's title either.
    assert!(!served
        .get("/api/jobs")
        .to_string()
        .contains("github_pat_example"));

    let out: Value = served
        .post("/api/git/github/sign-out", json!({}))
        .json()
        .unwrap();
    assert_eq!(served.finished(&out).0, "succeeded");
    assert_eq!(github(&served)["signedIn"], false);

    // From a browser: the code it prints is in the job's output, for the person to type.
    let web: Value = served
        .post("/api/git/github/sign-in", json!({}))
        .json()
        .unwrap();
    let (state, output) = served.finished(&web);
    assert_eq!(state, "succeeded", "{output}");
    assert!(output.contains("one-time code: 1A2B-3C4D"), "{output}");
    // Told there is no browser here to open, and nobody to answer a question.
    assert!(output.contains("browser=false prompt=1"), "{output}");
    assert_eq!(github(&served)["signedIn"], true);

    let refused = served.post("/api/git/github/token", json!({ "token": "two words" }));
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn a_repository_is_cloned_by_the_host_and_listed_as_a_project() {
    let mut served = Served::start();
    served.sign_in();
    assert_eq!(served.get("/api/status")["projects"], json!([]));
    let job: Value = served
        .post(
            "/api/projects/clone",
            json!({ "remote": "https://me:s3cret@github.com/example/my-service.git" }),
        )
        .json()
        .unwrap();
    // The password an address may carry is in no title and no output.
    assert_eq!(
        job["title"],
        "Clone https://github.com/example/my-service.git"
    );
    let (state, output) = served.finished(&job);
    assert_eq!(state, "succeeded", "{output}");
    assert!(!output.contains("s3cret"), "{output}");
    assert!(
        output.contains("Cloned into ~/Projects/my-service"),
        "{output}"
    );
    let projects = served.get("/api/git")["projects"].clone();
    assert_eq!(projects[0]["name"], "my-service");
    assert_eq!(projects[0]["path"], "~/Projects/my-service");

    let missing: Value = served
        .post(
            "/api/projects/clone",
            json!({ "remote": "https://github.com/example/missing" }),
        )
        .json()
        .unwrap();
    let (state, output) = served.finished(&missing);
    assert_eq!(state, "failed");
    assert!(output.ends_with("repository not found.\n"), "{output}");
    assert_eq!(
        served.get("/api/git")["projects"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn an_agent_is_installed_by_its_vendors_installer_into_local_bin_and_then_found() {
    let mut served = Served::start();
    // What the network would have given: an installer that puts a `grok` where it was told to.
    executable(
        served.bin().join("curl"),
        r#"#!/bin/sh
while [ $# -gt 2 ]; do if [ "$1" = -o ]; then out=$2; fi; shift; done
test "$2" = https://x.ai/cli/install.sh || { echo "asked for $2" >&2; exit 22; }
cat > "$out" <<'INSTALLER'
mkdir -p "$GROK_BIN_DIR"
printf '#!/bin/sh\necho "grok 0.0.0 (stand-in)"\n' > "$GROK_BIN_DIR/grok"
chmod 755 "$GROK_BIN_DIR/grok"
echo "installed by the stand-in"
INSTALLER
"#,
    );
    served.sign_in();
    let agent = |served: &Served, id: &str| {
        served
            .get("/api/agents")
            .as_array()
            .unwrap()
            .iter()
            .find(|agent| agent["id"] == id)
            .cloned()
            .unwrap()
    };
    let before = agent(&served, "grok");
    assert_eq!(before["found"], Value::Null);
    assert_eq!(before["install"]["from"], "xAI's installer");
    assert_eq!(before["install"]["terms"], "https://x.ai/cli");
    assert_eq!(agent(&served, "codex")["install"]["from"], "npm");

    let job: Value = served
        .post("/api/agents/grok/install", json!({}))
        .json()
        .unwrap();
    assert_eq!(job["title"], "Install Grok");
    let (state, output) = served.finished(&job);
    assert_eq!(state, "succeeded", "{output}");
    // The vendor and their terms are named before anything runs.
    assert!(
        output.starts_with("Installing Grok with xAI's own installer"),
        "{output}"
    );
    assert!(output.contains("installed by the stand-in"), "{output}");
    assert!(output.contains("grok 0.0.0 (stand-in)"), "{output}");
    // This home's ~/.local/bin is not on the stand-in host's PATH, and the job says so.
    assert!(
        output.contains(".local/bin is not on PATH here"),
        "{output}"
    );
    assert!(served.home.path().join(".local/bin/grok").exists());
    // The installer that was downloaded is not left behind.
    let left: Vec<_> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with(&format!("yardsort-install-grok-{}-", std::process::id())))
        .collect();
    assert_eq!(left, Vec::<String>::new());

    let unknown = served.post("/api/agents/aider/install", json!({}));
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn an_agent_on_the_path_is_found_with_its_version() {
    let mut served = Served::start();
    executable(
        served.bin().join("claude"),
        "#!/bin/sh\necho '9.9.9 (stand-in)'\n",
    );
    served.sign_in();
    let agents = served.get("/api/agents");
    let claude = agents
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "claude")
        .unwrap();
    assert_eq!(claude["found"], "~/bin/claude");
    assert_eq!(claude["version"], "9.9.9 (stand-in)");
    assert_eq!(claude["enabled"], true);
}

#[test]
fn the_tunnel_is_kept_in_the_settings_and_its_address_becomes_one_of_the_dashboards() {
    let mut served = Served::start();
    executable(
        served.bin().join("rustunnel"),
        "#!/bin/sh\necho '{\"event\":\"tunnel_ready\",\"public_url\":\"https://build-box.edge.example.test\"}'\nexec sleep 600\n",
    );
    served.sign_in();
    let public = "build-box.edge.example.test";
    let through = |served: &Served, method: Method, path: &str, proto: &str| {
        served
            .page(method, path)
            .header(HOST, public)
            .header("x-forwarded-proto", proto)
            .send()
            .unwrap()
    };
    // Before there is a tunnel, its name is nobody's.
    let early = through(&served, Method::Get, "/api/status", "https");
    assert_eq!(
        refusal(early),
        (StatusCode::FORBIDDEN, "wrong_address".into())
    );

    let refused = served.post(
        "/api/tunnel",
        json!({ "enabled": true, "subdomain": "--token" }),
    );
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let set = served.post(
        "/api/tunnel",
        json!({ "enabled": true, "subdomain": "Build-Box", "region": "", "server": null }),
    );
    assert_eq!(set.status(), StatusCode::OK);
    let settings = std::fs::read_to_string(served.home.path().join("settings.toml")).unwrap();
    assert!(
        settings.contains("[dashboard.tunnel]\nenabled = true\nsubdomain = \"build-box\""),
        "{settings}"
    );

    let since = Instant::now();
    loop {
        let tunnel = served.get("/api/status")["dashboard"]["tunnel"].clone();
        if tunnel["state"] == "ready" {
            assert_eq!(tunnel["url"], format!("https://{public}"));
            break;
        }
        assert!(since.elapsed() < Duration::from_secs(20), "{tunnel}");
        std::thread::sleep(Duration::from_millis(30));
    }
    // A sign-in link now comes for the public address too.
    let offered = served.dashboard().sign_in();
    assert_eq!(
        offered.public,
        Some(format!("https://{public}/#code={}", offered.code))
    );

    // Through the tunnel over HTTPS: this dashboard, and the cookie it gives is for HTTPS only.
    assert_eq!(
        through(&served, Method::Get, "/api/status", "https").status(),
        StatusCode::OK
    );
    let signed_in =
        RequestBuilder::new(Method::Post, &served.dashboard().address(), "/api/session")
            .header("x-yardsort-dashboard", "1")
            .header(HOST, public)
            .header("x-forwarded-proto", "https")
            .header(ORIGIN, format!("https://{public}"))
            .json(&json!({ "code": offered.code }))
            .send()
            .unwrap();
    assert_eq!(signed_in.status(), StatusCode::OK);
    assert!(signed_in.headers()[SET_COOKIE].contains("; Secure"));

    // Over the plain HTTP the tunnel's server also answers: sent to HTTPS, and nothing taken.
    let plain = through(&served, Method::Get, "/api/status?x=1", "http");
    assert_eq!(plain.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        plain.headers()["location"],
        format!("https://{public}/api/status?x=1")
    );
    let plain = through(&served, Method::Post, "/api/sessions/end", "http");
    assert_eq!(refusal(plain), (StatusCode::FORBIDDEN, "not_https".into()));
    assert_eq!(served.get("/api/status")["dashboard"]["signedIn"], 2);

    let off = served.post(
        "/api/tunnel",
        json!({ "enabled": false, "subdomain": "build-box" }),
    );
    assert_eq!(off.status(), StatusCode::OK);
    let standing: Value = off.json().unwrap();
    assert_eq!(standing["tunnel"]["state"], "off");
    assert_eq!(
        standing["tunnelWanted"]["subdomain"], "build-box",
        "kept for the next time"
    );
    let gone = through(&served, Method::Get, "/api/status", "https");
    assert_eq!(
        refusal(gone),
        (StatusCode::FORBIDDEN, "wrong_address".into())
    );
}

#[test]
fn a_tunnel_the_settings_ask_for_is_started_with_the_dashboard() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("bin")).unwrap();
    executable(
        home.path().join("bin/rustunnel"),
        "#!/bin/sh\necho \"$*\" > \"$HOME/tunnel-args\"\necho '{\"event\":\"tunnel_ready\",\"public_url\":\"https://b.example.test\"}'\nexec sleep 600\n",
    );
    let port = free_port();
    std::fs::write(
        home.path().join("settings.toml"),
        format!(
            "[dashboard]\nport = {port}\n[dashboard.tunnel]\nenabled = true\nregion = \"eu\"\n"
        ),
    )
    .unwrap();
    let served = Served::start_in(home);
    let since = Instant::now();
    while served.dashboard().standing().tunnel
        != (host_dashboard::tunnel::State::Ready {
            url: "https://b.example.test".into(),
        })
    {
        assert!(since.elapsed() < Duration::from_secs(20));
        std::thread::sleep(Duration::from_millis(30));
    }
    let args = std::fs::read_to_string(served.home.path().join("tunnel-args")).unwrap();
    assert_eq!(
        args.trim(),
        format!("http {port} --local-host 127.0.0.1 --json --no-inspect --region eu")
    );
}

#[test]
fn a_browser_stays_signed_in_when_the_host_restarts_and_every_browser_can_be_signed_out() {
    let mut served = Served::start();
    served.sign_in();
    let cookie = served.cookie.clone();
    let mut again = Served::start_in(served.stop());
    again.cookie = cookie;
    assert_eq!(again.get("/api/session")["signedIn"], true);
    assert_eq!(again.get("/api/status")["dashboard"]["signedIn"], 1);

    let waiting = again.dashboard().sign_in().code;
    let ended: Value = again.post("/api/sessions/end", json!({})).json().unwrap();
    assert_eq!(ended["ended"], 1);
    assert_eq!(again.get("/api/session")["signedIn"], false);
    // The codes not yet used went with it.
    let refused = refusal(again.post("/api/session", json!({ "code": waiting })));
    assert_eq!(refused, (StatusCode::UNAUTHORIZED, "wrong_code".into()));
    assert_eq!(again.dashboard().sign_everyone_out(), Ok(0));
}

#[test]
fn a_port_that_is_taken_is_stepped_past_and_a_run_of_them_is_said() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("bin")).unwrap();
    // A port with the next one free too, held by something else.
    let (held, port) = loop {
        let held = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = held.local_addr().unwrap().port();
        if port < u16::MAX - 20 && std::net::TcpListener::bind(("127.0.0.1", port + 1)).is_ok() {
            break (held, port);
        }
    };
    std::fs::write(
        home.path().join("settings.toml"),
        format!("[dashboard]\nport = {port}\n"),
    )
    .unwrap();
    let served = Served::start_in(home);
    assert_eq!(served.dashboard().port(), port + 1);
    assert_eq!(served.get("/api/session")["signedIn"], false);

    // Every one of the ten taken: it says which, and where to set another.
    let home = served.stop();
    let all: Vec<_> = (port + 1..port + 10)
        .filter_map(|port| std::net::TcpListener::bind(("127.0.0.1", port)).ok())
        .collect();
    let source = Arc::new(common::StandIn::new(home.path(), &home.path().join("bin")));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let refused = runtime.block_on(Dashboard::start(
        source,
        Options {
            state_dir: home.path().join("dashboard"),
        },
    ));
    drop((held, all));
    let Err(why) = refused else {
        // Something else let one of them go in between: not what this test is about.
        return;
    };
    assert!(
        why.contains(&format!("{port} to {}", port + 9)) && why.contains("[dashboard]"),
        "{why}"
    );
}

#[test]
fn the_dashboard_listens_on_this_computer_alone() {
    let served = Served::start();
    let port = served.dashboard().port();
    assert_eq!(
        served.dashboard().address(),
        format!("http://127.0.0.1:{port}")
    );
    // Any other address this computer has: nothing answers there.
    let others: Vec<std::net::IpAddr> = std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|socket| {
            // Sends nothing: connecting a UDP socket only picks the address it would leave by.
            socket.connect("192.0.2.1:9")?;
            socket.local_addr()
        })
        .map(|address| address.ip())
        .into_iter()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect();
    for ip in others {
        let reached = std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::new(ip, port),
            Duration::from_millis(500),
        );
        assert!(reached.is_err(), "the dashboard answered at {ip}:{port}");
    }
    let _ = served.source;
}

#[test]
fn where_the_app_serves_the_page_it_neither_pairs_nor_signs_in_and_says_where_to() {
    let mut served = Served::start();
    for path in ["/api/remote/pair", "/api/remote/sign-in"] {
        let refused =
            refusal(served.post(path, json!({ "server": "https://remote.example.test" })));
        assert_eq!(
            refused,
            (StatusCode::UNAUTHORIZED, "signed_out".into()),
            "{path}"
        );
    }
    served.sign_in();
    let remote = served.get("/api/remote");
    assert_eq!(remote["available"], false);
    assert_eq!(remote["pairing"]["state"], "idle");
    let refused = served.post("/api/remote/pair", json!({}));
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let said: Value = refused.json().unwrap();
    assert!(
        said["message"]
            .as_str()
            .unwrap()
            .contains("Settings → Remote"),
        "{said}"
    );
}

#[test]
fn a_signed_in_browser_pairs_a_device_with_the_answer_it_chose() {
    let mut served = Served::start_remote();
    // Signed out, nothing is offered, read or answered.
    for path in ["/api/remote/pair", "/api/remote/pair/answer"] {
        let refused = refusal(served.post(path, json!({ "grant": "grant-1", "allow": true })));
        assert_eq!(
            refused,
            (StatusCode::UNAUTHORIZED, "signed_out".into()),
            "{path}"
        );
    }
    let refused = refusal(served.page(Method::Get, "/api/remote").send().unwrap());
    assert_eq!(refused, (StatusCode::UNAUTHORIZED, "signed_out".into()));
    // Nor from another site's page, signed in or not.
    served.sign_in();
    let elsewhere = served
        .page(Method::Post, "/api/remote/pair")
        .header(ORIGIN, "https://evil.example")
        .json(&json!({}))
        .send()
        .unwrap();
    assert_eq!(
        refusal(elsewhere),
        (StatusCode::FORBIDDEN, "not_the_page".into())
    );
    assert_eq!(served.get("/api/remote")["pairing"]["state"], "idle");

    let offered: Value = served.post("/api/remote/pair", json!({})).json().unwrap();
    assert_eq!(offered["state"], "offered");
    assert_eq!(offered["code"], "ABCD-1234");
    assert_eq!(offered["qr"], json!(["101", "010", "101"]));
    assert_eq!(offered["lapsesInMs"], 300_000);

    let remote = served.source.remote.as_ref().unwrap();
    remote.asks("Ana's phone", true);
    let asked = served.get("/api/remote");
    assert_eq!(asked["available"], true);
    assert_eq!(asked["pairing"]["state"], "asked");
    assert_eq!(asked["pairing"]["device"], "Ana's phone");
    assert_eq!(asked["pairing"]["wantsControl"], true);
    assert_eq!(asked["pairing"]["projects"][1]["name"], "gadgets");

    // An answer for another device decides nothing.
    let wrong = served.post(
        "/api/remote/pair/answer",
        json!({ "grant": "grant-2", "allow": true }),
    );
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert!(remote.answers.lock().unwrap().is_empty());

    let answered: Value = served
        .post(
            "/api/remote/pair/answer",
            json!({ "grant": "grant-1", "allow": true, "control": false, "projects": ["p2"] }),
        )
        .json()
        .unwrap();
    assert_eq!(answered["state"], "answered");
    assert_eq!(answered["allowed"], true);
    let answers = remote.answers.lock().unwrap();
    assert_eq!(answers.len(), 1);
    assert!(answers[0].allow && !answers[0].control && !answers[0].workflows);
    assert_eq!(answers[0].projects.as_deref(), Some(&["p2".to_owned()][..]));
}

#[test]
fn a_signed_in_browser_signs_the_host_in_by_the_address_pasted_and_out_again() {
    let mut served = Served::start_remote();
    let refused = refusal(served.post(
        "/api/remote/sign-in",
        json!({ "server": "https://remote.example.test" }),
    ));
    assert_eq!(refused, (StatusCode::UNAUTHORIZED, "signed_out".into()));
    served.sign_in();

    let waiting: Value = served
        .post(
            "/api/remote/sign-in",
            json!({ "server": " https://remote.example.test ", "name": "Build box" }),
        )
        .json()
        .unwrap();
    assert_eq!(waiting["state"], "waiting");
    assert_eq!(
        waiting["url"], "https://remote.example.test/v1/login?name=Build box",
        "the server and the name are taken trimmed"
    );
    assert_eq!(waiting["port"], 45999);

    let wrong = served.post(
        "/api/remote/sign-in/pasted",
        json!({ "address": "http://127.0.0.1:1/signed-in?code=bad" }),
    );
    assert_eq!(wrong.status(), StatusCode::BAD_REQUEST);
    assert_eq!(served.get("/api/remote")["signingIn"]["state"], "waiting");

    let done: Value = served
        .post(
            "/api/remote/sign-in/pasted",
            json!({ "address": "http://127.0.0.1:45999/signed-in?code=good" }),
        )
        .json()
        .unwrap();
    assert_eq!(done["state"], "signedIn");
    assert_eq!(done["serverUrl"], "https://remote.example.test");

    let out = served.post("/api/remote/sign-out", json!({}));
    assert_eq!(out.status(), StatusCode::OK);
    assert!(*served
        .source
        .remote
        .as_ref()
        .unwrap()
        .signed_out
        .lock()
        .unwrap());
}

//! The dashboard over HTTP: its page, and what the page asks.
//!
//! Every request passes [`guard`] first, which is where the dashboard decides who it is talking
//! to. In order:
//!
//! 1. **The address it was sent to** must be a loopback name or the tunnel's own. A page on
//!    another site can make a browser send a request to 127.0.0.1 under a name of its own
//!    (DNS rebinding); such a request is addressed to that name, and is refused.
//! 2. **Through the tunnel, only HTTPS.** The tunnel's server also answers plain HTTP; a request
//!    that came that way is sent to the HTTPS address, or refused if it carries anything.
//! 3. **A request that changes something** must carry a header no form or link can add, and
//!    must not come from another site's page.
//! 4. **Everything the page asks**, but whether it is signed in and the sign-in itself, needs a
//!    session.

use std::path::PathBuf;
use std::sync::{Arc, PoisonError};

use axum::extract::{Path, Query, Request, State};
use axum::http::header::{self, HeaderMap, HeaderName, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use yardsort_core::env::ShellEnv;
use yardsort_core::program::Program;
use yardsort_core::settings::TunnelSettings;

use crate::jobs::{Step, Summary, Work};
use crate::{agents, git, system, Inner, Project};

/// What a request that changes something must carry. A page on another site cannot add it
/// without the browser asking this server first, and this server never says yes.
const OURS: HeaderName = HeaderName::from_static("x-yardsort-dashboard");
/// The longest command line taken.
const LONGEST_COMMAND: usize = 8 * 1024;

type Shared = State<Arc<Inner>>;

pub fn router(inner: Arc<Inner>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/app.js", get(script))
        .route("/app.css", get(style))
        .route("/favicon.svg", get(icon))
        .route("/api/session", get(session).post(sign_in).delete(sign_out))
        .route("/api/sessions/end", post(sign_everyone_out))
        .route("/api/status", get(status))
        .route("/api/agents", get(list_agents))
        .route("/api/agents/{id}/install", post(install_agent))
        .route("/api/git", get(read_git))
        .route("/api/git/identity", post(set_identity))
        .route("/api/git/github/sign-in", post(github_sign_in))
        .route("/api/git/github/token", post(github_token))
        .route("/api/git/github/sign-out", post(github_sign_out))
        .route("/api/projects/clone", post(clone_project))
        .route("/api/commands", post(run_command))
        .route("/api/jobs", get(list_jobs))
        .route("/api/jobs/{id}", get(read_job))
        .route("/api/jobs/{id}/stop", post(stop_job))
        .route("/api/tunnel", post(set_tunnel))
        .fallback(|| async { refuse(StatusCode::NOT_FOUND, "not_found", "There is nothing here.") })
        .layer(axum::middleware::from_fn_with_state(inner.clone(), guard))
        .with_state(inner)
}

// ── Who is asking ───────────────────────────────────────────────────────────────────────────

/// How a request reached the dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Via {
    /// Straight to 127.0.0.1, or through a port forwarded to it.
    Loopback,
    Tunnel,
}

async fn guard(State(inner): Shared, request: Request, next: Next) -> Response {
    let headers = request.headers();
    let Some(via) = addressed_to(headers, inner.tunnel.host().as_deref()) else {
        return refuse(
            StatusCode::FORBIDDEN,
            "wrong_address",
            "The dashboard answers at its own address only.",
        );
    };
    let reads = matches!(*request.method(), Method::GET | Method::HEAD);
    if via == Via::Tunnel && forwarded_proto(headers) != Some("https") {
        if !reads {
            return refuse(
                StatusCode::FORBIDDEN,
                "not_https",
                "Through the tunnel, the dashboard answers over HTTPS only.",
            );
        }
        let host = headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();
        let to = format!(
            "https://{host}{}",
            request.uri().path_and_query().map_or("/", |p| p.as_str())
        );
        return match HeaderValue::from_str(&to) {
            Ok(to) => (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, to)]).into_response(),
            Err(_) => refuse(StatusCode::BAD_REQUEST, "bad_request", "Not an address."),
        };
    }
    if !reads && (!ours(headers) || from_elsewhere(headers)) {
        return refuse(
            StatusCode::FORBIDDEN,
            "not_the_page",
            "Only the dashboard's own page may ask that.",
        );
    }
    let path = request.uri().path();
    let open = path == "/api/session" && matches!(*request.method(), Method::GET | Method::POST);
    if path.starts_with("/api/") && !open {
        let signed_in =
            cookie(headers, &cookie_name(&inner)).is_some_and(|token| inner.auth.allows(token));
        if !signed_in {
            return refuse(
                StatusCode::UNAUTHORIZED,
                "signed_out",
                "Sign in first, with a code from this computer.",
            );
        }
    }
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        // Nothing but its own files, and nothing may frame it or be framed by it.
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; \
             connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::REFERRER_POLICY, "no-referrer"),
        // What the page is told is how things stand now, and may hold what a job printed.
        (header::CACHE_CONTROL, "no-store"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    response
}

/// Which of the dashboard's addresses a request names, if it names one. `tunnel` is the
/// tunnel's host name while there is one.
fn addressed_to(headers: &HeaderMap, tunnel: Option<&str>) -> Option<Via> {
    let host = headers
        .get(header::HOST)?
        .to_str()
        .ok()?
        .to_ascii_lowercase();
    let name = host_name(&host);
    // Any port: an SSH forward listens on whichever one was free on the other computer.
    if matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
        return Some(Via::Loopback);
    }
    (Some(name) == tunnel).then_some(Via::Tunnel)
}

/// `host` without its port.
fn host_name(host: &str) -> &str {
    match host.rsplit_once(':') {
        // Not the colons inside an IPv6 address in brackets.
        Some((name, port)) if !port.contains(']') => name,
        _ => host,
    }
}

/// The scheme the tunnel's server was asked over. It sets this header itself, over whatever the
/// request carried, so it is believed only for a request that came through the tunnel.
fn forwarded_proto(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("x-forwarded-proto")?
        .to_str()
        .ok()
        .map(str::trim)
}

fn ours(headers: &HeaderMap) -> bool {
    headers.get(OURS).is_some_and(|value| value == "1")
}

/// Whether a request says it comes from a page that is not at the address it was sent to.
/// Browsers say where a request that changes something comes from; one that does not say is not
/// a browser's, and has the header a page could not have added.
fn from_elsewhere(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) else {
        return false;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let from = origin.split_once("://").map_or(origin, |(_, rest)| rest);
    !from.eq_ignore_ascii_case(host)
}

fn cookie_name(inner: &Inner) -> String {
    format!("yardsort_dashboard_{}", inner.auth.instance())
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

// ── Answers ─────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct Refusal<'a> {
    error: &'a str,
    message: &'a str,
}

fn refuse(status: StatusCode, error: &str, message: &str) -> Response {
    (status, Json(Refusal { error, message })).into_response()
}

/// A refusal of what was asked, in words for the person who asked.
struct Failed(String);

impl IntoResponse for Failed {
    fn into_response(self) -> Response {
        refuse(StatusCode::BAD_REQUEST, "refused", &self.0)
    }
}

type Answer<T> = Result<Json<T>, Failed>;

/// Run `work` off the runtime: everything a [`crate::Source`] does may block.
async fn blocking<T: Send + 'static>(
    inner: &Arc<Inner>,
    work: impl FnOnce(&Inner) -> Result<T, String> + Send + 'static,
) -> Result<T, Failed> {
    let inner = inner.clone();
    tokio::task::spawn_blocking(move || work(&inner))
        .await
        .map_err(|error| Failed(format!("That could not be done: {error}")))?
        .map_err(Failed)
}

// ── The page ────────────────────────────────────────────────────────────────────────────────

async fn page() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../ui/index.html"),
    )
}

async fn script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../ui/app.js"),
    )
}

async fn style() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../ui/app.css"),
    )
}

async fn icon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml")],
        include_str!("../ui/favicon.svg"),
    )
}

// ── Signing in ──────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    signed_in: bool,
}

async fn session(State(inner): Shared, headers: HeaderMap) -> Json<Session> {
    Json(Session {
        signed_in: cookie(&headers, &cookie_name(&inner))
            .is_some_and(|token| inner.auth.allows(token)),
    })
}

#[derive(Deserialize)]
struct Code {
    code: String,
}

async fn sign_in(State(inner): Shared, headers: HeaderMap, Json(typed): Json<Code>) -> Response {
    let Some(token) = inner.auth.redeem(&typed.code) else {
        return refuse(
            StatusCode::UNAUTHORIZED,
            "wrong_code",
            "That code is not one this computer gave, or it was used already, or it is more \
             than ten minutes old. Ask for another.",
        );
    };
    // Only ever sent back over HTTPS when it was given over HTTPS.
    let secure = match addressed_to(&headers, inner.tunnel.host().as_deref()) {
        Some(Via::Tunnel) => "; Secure",
        _ => "",
    };
    let cookie = format!(
        "{}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={}{secure}",
        cookie_name(&inner),
        crate::SESSION_GOOD_FOR.as_secs()
    );
    (
        [(header::SET_COOKIE, cookie)],
        Json(Session { signed_in: true }),
    )
        .into_response()
}

async fn sign_out(State(inner): Shared, headers: HeaderMap) -> Response {
    let name = cookie_name(&inner);
    if let Some(token) = cookie(&headers, &name) {
        inner.auth.end(token);
    }
    (
        [(
            header::SET_COOKIE,
            format!("{name}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
        Json(Session { signed_in: false }),
    )
        .into_response()
}

#[derive(Serialize)]
struct Ended {
    ended: usize,
}

async fn sign_everyone_out(State(inner): Shared) -> Json<Ended> {
    Json(Ended {
        ended: inner.auth.end_all(),
    })
}

// ── How things stand ────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    host: crate::About,
    machine: system::Machine,
    dashboard: crate::Standing,
    projects: Vec<Project>,
    /// The user the host runs as, whose commands the dashboard runs.
    user: Option<String>,
    /// How a command's line is run here, as a person would write it: `sh -c`.
    shell: &'static str,
    /// Why the login shell's environment could not be used, when it could not.
    environment_problem: Option<String>,
    /// Why `settings.toml` was not used, when it was not.
    settings_problem: Option<String>,
    /// Now, in milliseconds since 1970, by this computer's clock.
    now: i64,
}

async fn status(State(inner): Shared) -> Answer<Status> {
    blocking(&inner, |inner| {
        let env = inner.source.env();
        let settings = inner.source.settings();
        let mut places: Vec<(&str, PathBuf)> = Vec::new();
        if let Some(home) = env.home_dir() {
            places.push(("Home", home));
        }
        if let Ok(root) = yardsort_core::workspaces::worktree_root(&settings.get().workspaces, &env)
        {
            places.push(("Workspaces", root));
        }
        Ok(Status {
            host: inner.source.about(),
            machine: inner.machine.read(&places),
            dashboard: inner.standing(),
            projects: projects(inner, &env),
            user: env
                .get(if cfg!(windows) { "USERNAME" } else { "USER" })
                .map(str::to_owned),
            shell: if cfg!(windows) { "cmd.exe /C" } else { "sh -c" },
            environment_problem: env.warning.clone(),
            settings_problem: settings.problem(),
            now: yardsort_core::store::now_ms(),
        })
    })
    .await
    .map(Json)
}

// ── Agents ──────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Agent {
    id: String,
    name: String,
    command: String,
    /// Turned on in Settings: one turned off is offered to nobody.
    enabled: bool,
    /// Where its program is, as agents are started.
    found: Option<String>,
    version: Option<String>,
    /// How Yardsort installs it here, when it can.
    install: Option<Install>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Install {
    /// `Anthropic's installer`, or `npm`.
    from: String,
    /// What is fetched: the installer's address, or the package.
    what: String,
    /// The vendor whose terms installing accepts, and where they are.
    vendor: Option<String>,
    terms: Option<String>,
}

async fn list_agents(State(inner): Shared) -> Answer<Vec<Agent>> {
    blocking(&inner, |inner| {
        let env = inner.source.env();
        let cwd = env.home_dir().unwrap_or_default();
        let harnesses =
            yardsort_core::harness::resolve_all(&inner.source.settings().get().harnesses);
        // Each asked for its version at once: seven programs, some slow to start.
        let found: Vec<Option<PathBuf>> = harnesses
            .iter()
            .map(|h| env.find_program(&h.def.command, &cwd))
            .collect();
        let versions: Vec<Option<String>> = std::thread::scope(|scope| {
            let asked: Vec<_> = found
                .iter()
                .map(|program| {
                    let env = &env;
                    scope.spawn(move || program.as_ref().and_then(|p| version(inner, env, p)))
                })
                .collect();
            asked
                .into_iter()
                .map(|thread| thread.join().ok().flatten())
                .collect()
        });
        Ok(harnesses
            .into_iter()
            .zip(found)
            .zip(versions)
            .map(|((harness, found), version)| {
                let install = agents::HARNESSES
                    .iter()
                    .find(|known| known.id == harness.def.id && harness.builtin && cfg!(unix))
                    .map(|known| match &known.source {
                        agents::Source::Installer {
                            url, vendor, terms, ..
                        } => Install {
                            from: format!("{vendor}'s installer"),
                            what: (*url).to_owned(),
                            vendor: Some((*vendor).to_owned()),
                            terms: Some((*terms).to_owned()),
                        },
                        agents::Source::Npm { package } => Install {
                            from: "npm".into(),
                            what: (*package).to_owned(),
                            vendor: None,
                            terms: None,
                        },
                    });
                Agent {
                    id: harness.def.id,
                    name: harness.def.label,
                    command: harness.def.command,
                    enabled: harness.def.enabled,
                    found: found.map(|path| from_home(&env, &path.display().to_string())),
                    version,
                    install,
                }
            })
            .collect())
    })
    .await
    .map(Json)
}

/// A program's version, asked once for each time the program changes.
fn version(inner: &Inner, env: &ShellEnv, program: &std::path::Path) -> Option<String> {
    let changed = std::fs::metadata(program).and_then(|m| m.modified()).ok();
    let known = inner
        .versions
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|known| known.program == program && known.changed == changed)
        .map(|known| known.version.clone());
    if let Some(version) = known {
        return version;
    }
    let version = git::version_of(env, program);
    let mut versions = inner
        .versions
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    versions.retain(|known| known.program != program);
    versions.push(crate::KnownVersion {
        program: program.to_path_buf(),
        changed,
        version: version.clone(),
    });
    version
}

async fn install_agent(State(inner): Shared, Path(id): Path<String>) -> Answer<Summary> {
    blocking(&inner, move |inner| {
        let env = inner.source.env();
        let home = env
            .home_dir()
            .ok_or("This user has no home to install into.")?;
        let plan = agents::install(&id, &home, None)?;
        let mut steps = Vec::new();
        for run in &plan.runs {
            let program =
                Program::find(&env, run.program).ok_or_else(|| format!("{}.", run.needs))?;
            let mut command = program.command(&home);
            command.args(&run.args).envs(run.env.iter().cloned());
            steps.push(Step::new(command).shown(run.shown()));
        }
        let title = format!("Install {}", plan.harness.name);
        let downloaded = plan.downloaded.clone();
        let notice = plan.notice.clone();
        let work = Work::task(move |output| {
            output.line(&notice);
            Ok(())
        })
        .and(steps)
        .then({
            let env = env.clone();
            move |output| {
                // Where agents are looked for is the login shell's PATH, which the installer may
                // just have added to: this run's own copy of it is from before.
                let path = env.get("PATH").map(std::ffi::OsString::from);
                for line in agents::report(&plan, path.as_deref(), |program| {
                    git::version_of(&env, program)
                }) {
                    output.line(&line);
                }
                Ok(())
            }
        })
        .after(move || {
            if let Some(script) = downloaded {
                let _ = std::fs::remove_file(script);
            }
        });
        inner.jobs.start("agent", &title, work)
    })
    .await
    .map(Json)
}

// ── Git ─────────────────────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GitState {
    #[serde(flatten)]
    git: git::Git,
    projects: Vec<Project>,
    /// Where a clone goes.
    projects_dir: Option<String>,
}

async fn read_git(State(inner): Shared) -> Answer<GitState> {
    blocking(&inner, |inner| {
        let env = inner.source.env();
        Ok(GitState {
            git: git::read(&env),
            projects: projects(inner, &env),
            projects_dir: yardsort_core::remote::start::projects_dir(
                &inner.source.settings().get().remote,
                &env,
            )
            .map(|dir| from_home(&env, &dir.display().to_string())),
        })
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct Identity {
    name: String,
    email: String,
}

#[derive(Serialize)]
struct Done {
    done: bool,
}

async fn set_identity(State(inner): Shared, Json(identity): Json<Identity>) -> Answer<Done> {
    blocking(&inner, move |inner| {
        git::set_identity(&inner.source.env(), &identity.name, &identity.email)
    })
    .await
    .map(|()| Json(Done { done: true }))
}

async fn github_sign_in(State(inner): Shared) -> Answer<Summary> {
    blocking(&inner, |inner| {
        let steps = git::sign_in(&inner.source.env())?;
        inner
            .jobs
            .start("github", "Sign in to GitHub", Work::steps(steps))
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct Token {
    token: String,
}

async fn github_token(State(inner): Shared, Json(token): Json<Token>) -> Answer<Summary> {
    blocking(&inner, move |inner| {
        let steps = git::sign_in_with_token(&inner.source.env(), &token.token)?;
        inner.jobs.start(
            "github",
            "Sign in to GitHub with a token",
            Work::steps(steps),
        )
    })
    .await
    .map(Json)
}

async fn github_sign_out(State(inner): Shared) -> Answer<Summary> {
    blocking(&inner, |inner| {
        let steps = git::sign_out(&inner.source.env())?;
        inner
            .jobs
            .start("github", "Sign out of GitHub", Work::steps(steps))
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct Clone {
    remote: String,
}

async fn clone_project(State(inner): Shared, Json(clone): Json<Clone>) -> Answer<Summary> {
    let remote = clone.remote.trim().to_owned();
    if remote.is_empty() || remote.len() > 2048 || remote.chars().any(char::is_control) {
        return Err(Failed(
            "Give the repository's address, such as https://github.com/you/my-service.".into(),
        ));
    }
    let source = inner.source.clone();
    let title = format!("Clone {}", without_password(&remote));
    inner
        .jobs
        .start(
            "clone",
            &title,
            Work::task(move |output| {
                output.line(&format!("Cloning {}…", without_password(&remote)));
                let project = source.clone_project(&remote)?;
                output.line(&format!(
                    "Cloned into {}, and added as the project {}.",
                    from_home(&source.env(), &project.path),
                    project.name
                ));
                Ok(())
            }),
        )
        .map(Json)
        .map_err(Failed)
}

/// The projects here, each with its folder as a person would write it.
fn projects(inner: &Inner, env: &ShellEnv) -> Vec<Project> {
    inner
        .source
        .projects()
        .into_iter()
        .map(|project| Project {
            path: from_home(env, &project.path),
            ..project
        })
        .collect()
}

/// A path as a person would write it: `~/Projects/my-service` for one in this user's home.
fn from_home(env: &ShellEnv, path: &str) -> String {
    let Some(home) = env.home_dir() else {
        return path.to_owned();
    };
    match std::path::Path::new(path).strip_prefix(&home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Ok(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        Err(_) => path.to_owned(),
    }
}

/// An address as it may be shown: without the user and password some carry before the host.
fn without_password(remote: &str) -> String {
    match remote.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
            let host = authority
                .rsplit_once('@')
                .map_or(authority, |(_, host)| host);
            format!("{scheme}://{host}/{path}")
        }
        None => remote.to_owned(),
    }
}

// ── Commands and jobs ───────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct CommandLine {
    command: String,
}

async fn run_command(State(inner): Shared, Json(line): Json<CommandLine>) -> Answer<Summary> {
    let line = line.command.trim().to_owned();
    if line.is_empty() {
        return Err(Failed("Type a command to run.".into()));
    }
    if line.len() > LONGEST_COMMAND || line.contains('\0') {
        return Err(Failed("That is too long to be one command.".into()));
    }
    blocking(&inner, move |inner| {
        let env = inner.source.env();
        let home = env.home_dir().unwrap_or_else(std::env::temp_dir);
        // The one place a line is handed to a shell: the line is the person's own, and running
        // it as their shell would is what they asked for. The shell itself is still argv.
        let (shell, flag) = if cfg!(windows) {
            ("cmd.exe", "/C")
        } else {
            ("sh", "-c")
        };
        let program = Program::find(&env, shell)
            .ok_or_else(|| format!("There is no {shell} on this user's PATH."))?;
        let mut command = program.command(&home);
        command.arg(flag).arg(&line);
        let title: String = line.chars().take(120).collect();
        inner.jobs.start(
            "command",
            &title,
            Work::steps(vec![Step::new(command).shown(line)]),
        )
    })
    .await
    .map(Json)
}

async fn list_jobs(State(inner): Shared) -> Json<Vec<Summary>> {
    Json(inner.jobs.list())
}

#[derive(Deserialize)]
struct From {
    #[serde(default)]
    from: u64,
}

async fn read_job(State(inner): Shared, Path(id): Path<u64>, Query(from): Query<From>) -> Response {
    match inner.jobs.read(id, from.from) {
        Some(reading) => Json(reading).into_response(),
        None => refuse(StatusCode::NOT_FOUND, "not_found", "No such job."),
    }
}

#[derive(Serialize)]
struct Stopped {
    stopped: bool,
}

async fn stop_job(State(inner): Shared, Path(id): Path<u64>) -> Json<Stopped> {
    Json(Stopped {
        stopped: inner.jobs.stop(id),
    })
}

// ── The tunnel ──────────────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Tunnel {
    enabled: bool,
    subdomain: Option<String>,
    region: Option<String>,
    server: Option<String>,
}

async fn set_tunnel(State(inner): Shared, Json(wanted): Json<Tunnel>) -> Answer<crate::Standing> {
    blocking(&inner, move |inner| {
        inner.set_tunnel(TunnelSettings {
            enabled: wanted.enabled,
            subdomain: wanted.subdomain,
            region: wanted.region,
            server: wanted.server,
        })?;
        Ok(inner.standing())
    })
    .await
    .map(Json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.append(*name, HeaderValue::from_str(value).unwrap());
        }
        headers
    }

    #[test]
    fn only_a_loopback_name_or_the_tunnels_own_is_this_dashboards_address() {
        let tunnel = Some("build-box.edge.example.test");
        let via = |host: &str| addressed_to(&headers(&[("host", host)]), tunnel);
        for loopback in [
            "127.0.0.1:28680",
            "localhost:9000",
            "LOCALHOST",
            "[::1]:28680",
            "127.0.0.1",
        ] {
            assert_eq!(via(loopback), Some(Via::Loopback), "{loopback}");
        }
        assert_eq!(via("Build-Box.edge.example.test"), Some(Via::Tunnel));
        assert_eq!(via("build-box.edge.example.test:443"), Some(Via::Tunnel));
        // A name of someone else's that resolves here, and a tunnel that is not this one's.
        for other in [
            "evil.example.test:28680",
            "other.edge.example.test",
            "127.0.0.1.evil.test",
            "",
        ] {
            assert_eq!(via(other), None, "{other}");
        }
        assert_eq!(addressed_to(&HeaderMap::new(), tunnel), None);
        assert_eq!(
            addressed_to(&headers(&[("host", "build-box.edge.example.test")]), None),
            None,
            "no tunnel, no public name"
        );
    }

    #[test]
    fn a_request_from_another_sites_page_is_told_apart_by_its_origin() {
        let same = headers(&[
            ("host", "127.0.0.1:28680"),
            ("origin", "http://127.0.0.1:28680"),
        ]);
        assert!(!from_elsewhere(&same));
        let other = headers(&[
            ("host", "127.0.0.1:28680"),
            ("origin", "https://evil.example.test"),
        ]);
        assert!(from_elsewhere(&other));
        // The same host on another port is another site.
        let port = headers(&[
            ("host", "127.0.0.1:28680"),
            ("origin", "http://127.0.0.1:3000"),
        ]);
        assert!(from_elsewhere(&port));
        assert!(
            !from_elsewhere(&headers(&[("host", "127.0.0.1:28680")])),
            "not a browser"
        );
        assert!(from_elsewhere(&headers(&[
            ("host", "127.0.0.1:28680"),
            ("origin", "null")
        ])));
    }

    #[test]
    fn a_cookie_is_found_among_others_by_its_whole_name() {
        let jar = headers(&[
            (
                "cookie",
                "other=1; yardsort_dashboard_ab12cd34=token; third=3",
            ),
            ("cookie", "yardsort_dashboard_ffffffff=another"),
        ]);
        assert_eq!(cookie(&jar, "yardsort_dashboard_ab12cd34"), Some("token"));
        assert_eq!(cookie(&jar, "yardsort_dashboard_ffffffff"), Some("another"));
        assert_eq!(cookie(&jar, "yardsort_dashboard"), None);
    }

    #[test]
    fn a_path_in_the_home_is_written_from_the_home() {
        use std::collections::BTreeMap;
        let home = if cfg!(windows) {
            "C:\\Users\\box"
        } else {
            "/home/box"
        };
        let env = ShellEnv {
            vars: BTreeMap::from([(
                if cfg!(windows) { "USERPROFILE" } else { "HOME" }.to_owned(),
                home.to_owned(),
            )]),
            source: yardsort_core::env::EnvSource::Process,
            warning: None,
        };
        let sep = std::path::MAIN_SEPARATOR;
        assert_eq!(
            from_home(&env, &format!("{home}{sep}Projects{sep}my-service")),
            format!("~{sep}Projects{sep}my-service")
        );
        assert_eq!(from_home(&env, home), "~");
        // Another user's folder of a like name is not this home.
        assert_eq!(
            from_home(&env, &format!("{home}er{sep}x")),
            format!("{home}er{sep}x")
        );
        assert_eq!(from_home(&env, "/srv/repos/x"), "/srv/repos/x");
    }

    #[test]
    fn an_address_is_shown_without_the_password_in_it() {
        assert_eq!(
            without_password("https://me:ghp_secret@github.com/me/repo.git"),
            "https://github.com/me/repo.git"
        );
        assert_eq!(
            without_password("https://github.com/me/repo"),
            "https://github.com/me/repo"
        );
        assert_eq!(
            without_password("git@github.com:me/repo.git"),
            "git@github.com:me/repo.git"
        );
    }
}

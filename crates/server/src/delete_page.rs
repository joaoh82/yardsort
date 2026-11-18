//! The server's own pages for deleting an account, for a person with no app to hand. Its address
//! is the one the app stores are given (docs/design/46-remote-r7.md §10).
//!
//! ```text
//! GET  /account/delete           what goes and what stays, and a button to sign in again
//! GET  /account/delete/start     a verifier for this browser, in a cookie, then the provider
//! GET  /account/delete/confirm   back from the provider: type `delete`
//! POST /account/delete/confirm   the account signed in as, deleted
//! ```
//!
//! The browser plays the device's part: the verifier whose SHA-256 started the confirming
//! sign-in stays in an `HttpOnly`, `SameSite=Lax` cookie scoped to these pages, so the code the
//! provider's answer brings back works only in the browser that started it. A form posted from
//! another site carries neither the cookie nor, with an `Origin`, this server's own; it deletes
//! nothing. The pages load nothing and run no script.

use axum::extract::rejection::{FormRejection, QueryRejection};
use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use remote_proto::account::Secret;
use remote_proto::delete::{AccountDeleted, CONFIRMATION, FRESH_SIGN_IN_MS};
use remote_proto::ErrorCode;
use serde::Deserialize;

use crate::http::{now_ms, Service};
use crate::records::Records;
use crate::tokens;

const COOKIE: &str = "yardsort_delete";

pub(crate) async fn intro<R: Records>(State(service): State<Service<R>>) -> Response {
    let name = escape(&service.describe().name);
    let issuer = escape(&issuer_host(&service.describe().issuer));
    html(
        StatusCode::OK,
        "Delete your account",
        &format!(
            "<p>This deletes your account on {name}: every phone and computer signed in to it, \
             which phones get notifications and how, and every project, note and workspace name \
             it syncs. Each of them is signed out.</p>\
             <p><strong>Nothing on your computers or phones is deleted.</strong> Their projects, \
             workspaces and agents carry on, and Yardsort keeps working there without an \
             account.</p>\
             <p>It cannot be undone. To keep a copy of what the server holds first, export it from \
             the app: <em>Settings → Remote → Your account</em>.</p>\
             <p>To confirm it is you, sign in again at {issuer}.</p>\
             <form method=\"get\" action=\"delete/start\">\
             <button type=\"submit\">Sign in again to delete</button></form>"
        ),
    )
}

pub(crate) async fn start<R: Records>(State(service): State<Service<R>>) -> Response {
    let verifier = tokens::random();
    let Ok(back) = service.base_url().join("account/delete/confirm") else {
        return trouble();
    };
    match service
        .begin(&tokens::s256(&verifier), &back, true, now_ms())
        .await
    {
        Ok(to) => {
            let mut response = Redirect::to(to.as_str()).into_response();
            if let Ok(cookie) = HeaderValue::from_str(&cookie(&service, &verifier, LOGIN_SECONDS)) {
                response.headers_mut().insert(header::SET_COOKIE, cookie);
            }
            response
        }
        Err(error) => again(StatusCode::SERVICE_UNAVAILABLE, &error.message),
    }
}

/// How long the cookie is kept: the sign-in, then the confirmation's own five minutes.
const LOGIN_SECONDS: i64 = (crate::LOGIN_TTL_MS + FRESH_SIGN_IN_MS) / 1000;

#[derive(Deserialize)]
pub(crate) struct Back {
    code: Option<String>,
    error: Option<String>,
}

pub(crate) async fn confirm<R: Records>(
    State(_service): State<Service<R>>,
    query: Result<Query<Back>, QueryRejection>,
) -> Response {
    match query {
        Ok(Query(Back {
            code: Some(code), ..
        })) => form(StatusCode::OK, &code, None),
        Ok(Query(Back { error: Some(_), .. })) => again(
            StatusCode::UNAUTHORIZED,
            "You were not signed in again, so nothing was deleted.",
        ),
        _ => again(
            StatusCode::BAD_REQUEST,
            "This page needs the answer from signing in again.",
        ),
    }
}

#[derive(Deserialize)]
pub(crate) struct Typed {
    code: String,
    confirm: String,
}

pub(crate) async fn delete<R: Records>(
    State(service): State<Service<R>>,
    headers: HeaderMap,
    typed: Result<Form<Typed>, FormRejection>,
) -> Response {
    let Ok(Form(typed)) = typed else {
        return again(StatusCode::BAD_REQUEST, "This form is incomplete.");
    };
    if !same_origin(&service, &headers) {
        return again(
            StatusCode::FORBIDDEN,
            "This form was sent from another site, so nothing was deleted.",
        );
    }
    if typed.confirm.trim().to_lowercase() != CONFIRMATION {
        return form(
            StatusCode::BAD_REQUEST,
            &typed.code,
            Some("Type delete to confirm. Nothing was deleted."),
        );
    }
    let Some(verifier) = cookie_value(&headers) else {
        return again(
            StatusCode::UNAUTHORIZED,
            "This browser did not start this deletion, so nothing was deleted.",
        );
    };
    let now = now_ms();
    let account = match service
        .confirmed(&Secret::new(typed.code), &Secret::new(verifier), now)
        .await
    {
        Ok(account) => account,
        Err(error) => return again(StatusCode::UNAUTHORIZED, &error.message),
    };
    let mut response = match service.delete(&account, now).await {
        Ok(deleted) => done(&deleted),
        Err(error) if error.code == ErrorCode::Unauthenticated => html(
            StatusCode::OK,
            "Your account was deleted",
            &para(&error.message),
        ),
        Err(error) => again(StatusCode::INTERNAL_SERVER_ERROR, &error.message),
    };
    if let Ok(cleared) = HeaderValue::from_str(&cookie(&service, "", 0)) {
        response.headers_mut().insert(header::SET_COOKIE, cleared);
    }
    response
}

fn done(deleted: &AccountDeleted) -> Response {
    let things = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    html(
        StatusCode::OK,
        "Your account was deleted",
        &format!(
            "<p>{}, {} and {} were deleted, and every device was signed out. Nothing on them was \
             deleted.</p><p>You can close this window.</p>",
            things(deleted.devices, "device", "devices"),
            things(deleted.hosts, "computer", "computers"),
            things(deleted.records, "synced record", "synced records"),
        ),
    )
}

fn form(status: StatusCode, code: &str, problem: Option<&str>) -> Response {
    let problem = problem.map(para).unwrap_or_default();
    html(
        status,
        "Delete your account",
        &format!(
            "{problem}<p>Type <strong>{CONFIRMATION}</strong> to delete the account you just \
             signed in as. This cannot be undone.</p>\
             <form method=\"post\" action=\"confirm\">\
             <input type=\"hidden\" name=\"code\" value=\"{}\">\
             <label>Confirmation <input name=\"confirm\" autocomplete=\"off\" \
             autocapitalize=\"none\" spellcheck=\"false\" required></label>\
             <button type=\"submit\">Delete my account</button></form>",
            escape(code)
        ),
    )
}

/// A page that says what went wrong, with a way to start over.
fn again(status: StatusCode, message: &str) -> Response {
    html(
        status,
        "Delete your account",
        &format!(
            "{}<p><a href=\"{}\">Start again</a></p>",
            para(message),
            if status == StatusCode::BAD_REQUEST {
                "../delete"
            } else {
                "delete"
            }
        ),
    )
}

fn trouble() -> Response {
    again(
        StatusCode::INTERNAL_SERVER_ERROR,
        "This server cannot show this page as it is set up.",
    )
}

fn html(status: StatusCode, title: &str, body: &str) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; \
                 frame-ancestors 'none'; base-uri 'none'",
            ),
        ],
        format!(
            "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
             <title>{title}</title><style>{STYLE}</style></head>\
             <body><main><h1>{title}</h1>{body}</main></body></html>"
        ),
    )
        .into_response()
}

const STYLE: &str =
    "body{font:16px/1.5 system-ui,sans-serif;margin:0;background:#fff;color:#1b1b1f}\
main{max-width:34rem;margin:0 auto;padding:2rem 1rem}h1{font-size:1.4rem}\
label{display:block;margin:1rem 0}input:not([type=hidden]){display:block;margin-top:.25rem;\
padding:.5rem;font:inherit;width:100%;box-sizing:border-box}button{font:inherit;padding:.5rem 1rem;\
background:#b3261e;color:#fff;border:0;border-radius:6px;cursor:pointer}\
@media (prefers-color-scheme:dark){body{background:#141218;color:#e6e0e9}}";

fn para(text: &str) -> String {
    format!("<p>{}</p>", escape(text))
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// The provider's host, which is what a person recognises.
fn issuer_host(issuer: &str) -> String {
    url::Url::parse(issuer)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| issuer.to_owned())
}

fn cookie<R: Records>(service: &crate::ControlService<R>, value: &str, max_age: i64) -> String {
    let base = service.base_url();
    let secure = if base.scheme() == "https" {
        "; Secure"
    } else {
        ""
    };
    format!(
        "{COOKIE}={value}; Path={}account/delete; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}",
        base.path()
    )
}

fn cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, value)| *name == COOKIE && !value.is_empty())
        .map(|(_, value)| value.to_owned())
}

/// No `Origin`, or this server's own. Browsers send one with every form post now.
fn same_origin<R: Records>(service: &crate::ControlService<R>, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return true;
    };
    let ours = service.base_url().origin().ascii_serialization();
    origin.to_str().is_ok_and(|origin| origin == ours)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_the_cookie_is_found_among_others() {
        assert_eq!(
            escape("<a href=\"x\">'&'</a>"),
            "&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=1; yardsort_delete=abc; more=2"),
        );
        assert_eq!(cookie_value(&headers).as_deref(), Some("abc"));
        headers.insert(header::COOKIE, HeaderValue::from_static("yardsort_delete="));
        assert_eq!(cookie_value(&headers), None);
        assert_eq!(
            issuer_host("https://auth.example.com/oauth"),
            "auth.example.com"
        );
    }
}

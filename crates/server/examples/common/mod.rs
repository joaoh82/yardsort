//! What the example tools share: signing in through ZITADEL's API without a browser, and
//! opening one when there is a person to use it.

use url::Url;

/// The provider's half of a sign-in, without a browser: what ZITADEL's login pages do, through
/// its session API. Ends by following the server's redirect to the loopback listener, as a
/// browser would.
pub async fn through_zitadel_api(http: &reqwest::Client, login: Url) -> Result<(), String> {
    let env = |name: &str| std::env::var(name).map_err(|_| format!("{name} is not set"));
    let token = env("ZITADEL_LOGIN_CLIENT_TOKEN")?;
    let login_name = env("YARDSORT_SIGN_IN_AS")?;
    let password = env("YARDSORT_SIGN_IN_PASSWORD")?;

    // The server sends the browser to the provider, which sends it to its login pages with the
    // id of this sign-in request.
    let authorize = redirected(http, login).await?;
    let login_page = redirected(http, authorize.clone()).await?;
    let request = login_page
        .query_pairs()
        .find(|(k, _)| k == "authRequest")
        .map(|(_, v)| v.into_owned())
        .ok_or_else(|| format!("ZITADEL sent the browser to {login_page}, with no authRequest"))?;
    let zitadel = |path: &str| authorize.join(path).map_err(|e| e.to_string());
    let api = |url: Url, body: serde_json::Value| {
        let http = http.clone();
        let token = token.clone();
        async move {
            let response = http
                .post(url.clone())
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())?;
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            if !status.is_success() {
                return Err(format!("ZITADEL {}: {status} {body}", url.path()));
            }
            Ok(body)
        }
    };

    let session = api(
        zitadel("/v2/sessions")?,
        serde_json::json!({ "checks": {
            "user": { "loginName": login_name },
            "password": { "password": password },
        }}),
    )
    .await?;
    let field = |value: &serde_json::Value, name: &str| {
        value[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("ZITADEL's answer has no {name}"))
    };
    let (session_id, session_token) = (
        field(&session, "sessionId")?,
        field(&session, "sessionToken")?,
    );
    let user = http
        .get(zitadel(&format!("/v2/sessions/{session_id}"))?)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(id) = user["session"]["factors"]["user"]["id"].as_str() {
        println!("Signed in at ZITADEL as user {id}.");
    }
    let finished = api(
        zitadel(&format!("/v2/oidc/auth_requests/{request}"))?,
        serde_json::json!({ "session": {
            "sessionId": session_id,
            "sessionToken": session_token,
        }}),
    )
    .await?;
    let callback = Url::parse(&field(&finished, "callbackUrl")?).map_err(|e| e.to_string())?;

    // The server's callback, then its redirect to this tool's listener.
    let device = redirected(http, callback).await?;
    http.get(device).send().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Where a response sends the browser next.
pub async fn redirected(http: &reqwest::Client, url: Url) -> Result<Url, String> {
    let response = http
        .get(url.clone())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let location = response
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            format!(
                "{} answered {}, not a redirect",
                url.path(),
                response.status()
            )
        })?;
    url.join(location).map_err(|e| e.to_string())
}

/// Best effort; the address is printed either way.
pub fn open(url: &str) {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(windows) {
        // Not `cmd /c start`: cmd would read the `&`s in the query as command separators.
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = command
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

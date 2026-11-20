//! The journal: every revocation and deletion, written outside the database as it happens, so
//! that a database restored from a backup cannot reopen what was closed after the backup was
//! taken (docs/design/46-remote-r7.md §12).
//!
//! **What is closed.** A device revoked (by its person, by the operator, or because a copy of its
//! refresh credential was presented), a grant revoked, a host removed, an account deleted. Each
//! is an [`Entry`] holding ids and a time and nothing else: no name, no subject, no email.
//!
//! **How an entry travels.** The store writes it in the same step as what it records, as a row
//! of `journal` that is not yet written out ([`Records::journal_unwritten`]). Every instance
//! sends those rows to the journal every few seconds and marks them written
//! ([`start_flushing`]). A row is never deleted: the records keep every key they hold, written
//! out or replayed, and that is how a restored database is told apart from a current one.
//!
//! **A restored database** holds fewer keys than the journal. Every instance asks the journal
//! for its keys from a little before the latest the records hold ([`Journal::check`]) when it
//! starts and every five minutes after; one the records do not hold means the records are behind,
//! and the instance answers every request with 503 until `yardsort-server reconcile` replays
//! the journal ([`reconcile`]) and says so with [`Signal::Journal`].
//!
//! **Where it lives**: a bucket of an S3-compatible object store ([`S3`]; Tigris on Fly.io), or
//! a directory ([`Dir`]) for a self-hoster whose backups are kept apart from it. Keys sort by
//! time: `journal/<ms, 16 digits>-<uuid>.json`. Nothing here deletes or overwrites an object.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::records::{Records, RecordsError, RevokeReason, Signal};
use crate::service::ControlService;

/// Where every key begins.
pub const PREFIX: &str = "journal/";

/// How often an instance sends what is waiting.
pub const FLUSH_EVERY: Duration = Duration::from_secs(5);

/// How often an instance asks the journal whether its records are behind.
pub const CHECK_EVERY: Duration = Duration::from_secs(5 * 60);

/// How far before the latest key the records hold a check starts listing. Instances' clocks
/// differ by a little, and a key is made from the clock of the instance that closed something.
pub const CHECK_MARGIN_MS: i64 = 60 * 60_000;

/// The rows sent in one go.
const FLUSH_BATCH: usize = 100;

/// The keys asked about in one query.
const KNOWN_BATCH: usize = 500;

/// One thing closed, as the journal keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The format, 1.
    pub v: u32,
    /// When it was closed, in milliseconds since the epoch.
    pub at: i64,
    #[serde(flatten)]
    pub closed: Closed,
}

/// What was closed. Ids only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "what",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Closed {
    /// These devices of the account were revoked, for this reason.
    DevicesRevoked {
        account: String,
        devices: Vec<String>,
        reason: Reason,
    },
    GrantRevoked {
        account: String,
        grant: String,
    },
    HostRemoved {
        account: String,
        host: String,
    },
    AccountDeleted {
        account: String,
    },
}

/// [`RevokeReason`], as the journal spells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Reason {
    ByOwner,
    RefreshReused,
    ByOperator,
}

impl From<RevokeReason> for Reason {
    fn from(reason: RevokeReason) -> Self {
        match reason {
            RevokeReason::ByOwner => Self::ByOwner,
            RevokeReason::RefreshReused => Self::RefreshReused,
            RevokeReason::ByOperator => Self::ByOperator,
        }
    }
}

impl From<Reason> for RevokeReason {
    fn from(reason: Reason) -> Self {
        match reason {
            Reason::ByOwner => Self::ByOwner,
            Reason::RefreshReused => Self::RefreshReused,
            Reason::ByOperator => Self::ByOperator,
        }
    }
}

impl Entry {
    pub fn new(at: i64, closed: Closed) -> Self {
        Self { v: 1, at, closed }
    }

    /// A key of its own: its time first, so keys sort by it, then a random part.
    pub fn key(&self) -> String {
        format!(
            "{PREFIX}{:016}-{}.json",
            self.at.max(0),
            uuid::Uuid::new_v4().simple()
        )
    }

    pub fn body(&self) -> String {
        serde_json::to_string(self).expect("an entry is plain data")
    }

    /// `None` for anything a newer server wrote that this one cannot read.
    pub fn parse(body: &[u8]) -> Option<Self> {
        serde_json::from_slice::<Self>(body)
            .ok()
            .filter(|entry| entry.v == 1)
    }
}

/// A row of the records' `journal`: a key and its entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub body: String,
}

impl Row {
    pub fn of(entry: &Entry) -> Self {
        Self {
            key: entry.key(),
            body: entry.body(),
        }
    }
}

/// The time a key was made, or `None` for a key not of this form.
pub fn key_time(key: &str) -> Option<i64> {
    key.strip_prefix(PREFIX)?.get(..16)?.parse().ok()
}

/// The first key that could have been made at `at`: every key made then or later sorts after it.
pub fn floor_key(at: i64) -> String {
    format!("{PREFIX}{:016}", at.max(0))
}

/// The journal's own failure: unreachable, refused, or answering something unreadable.
#[derive(Debug, thiserror::Error)]
#[error("the journal: {0}")]
pub struct JournalError(pub String);

/// Where the journal lives.
#[derive(Debug, Clone)]
pub enum Store {
    Dir(Dir),
    S3(S3),
}

impl Store {
    /// Write an entry under its key. One already there is left as it is: keys are never reused,
    /// so it is this entry, written before.
    pub async fn put(&self, key: &str, body: &str) -> Result<(), JournalError> {
        match self {
            Self::Dir(dir) => dir.put(key, body).await,
            Self::S3(s3) => s3.put(key, body).await,
        }
    }

    /// Every key after `after`, or every key, in order.
    pub async fn list(&self, after: Option<&str>) -> Result<Vec<String>, JournalError> {
        match self {
            Self::Dir(dir) => dir.list(after).await,
            Self::S3(s3) => s3.list(after).await,
        }
    }

    pub async fn get(&self, key: &str) -> Result<Vec<u8>, JournalError> {
        match self {
            Self::Dir(dir) => dir.get(key).await,
            Self::S3(s3) => s3.get(key).await,
        }
    }

    /// Where it is, for a log line: the bucket or the directory, never a credential.
    pub fn describe(&self) -> String {
        match self {
            Self::Dir(dir) => format!("the directory {}", dir.root.display()),
            Self::S3(s3) => format!("the bucket {} at {}", s3.bucket, s3.endpoint),
        }
    }
}

/// A journal in a directory: a file per key.
#[derive(Debug, Clone)]
pub struct Dir {
    root: PathBuf,
}

impl Dir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn path(&self, key: &str) -> Result<PathBuf, JournalError> {
        let name = key
            .strip_prefix(PREFIX)
            .filter(|name| {
                !name.is_empty() && !name.contains(['/', '\\']) && !name.starts_with('.')
            })
            .ok_or_else(|| JournalError(format!("{key:?} is not a journal key")))?;
        Ok(self.root.join(name))
    }

    async fn put(&self, key: &str, body: &str) -> Result<(), JournalError> {
        let path = self.path(key)?;
        let root = self.root.clone();
        let body = body.to_owned();
        tokio::task::spawn_blocking(move || {
            use std::io::Write;
            std::fs::create_dir_all(&root)?;
            if path.exists() {
                return Ok(());
            }
            // Whole or not at all: written beside it, then moved into place.
            let partial = root.join(format!(
                ".{}.partial",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            let mut file = std::fs::File::create(&partial)?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&partial, &path)
        })
        .await
        .map_err(|e| JournalError(e.to_string()))?
        .map_err(|e| JournalError(format!("{}: {e}", self.root.display())))
    }

    async fn list(&self, after: Option<&str>) -> Result<Vec<String>, JournalError> {
        let root = self.root.clone();
        let after = after.map(str::to_owned);
        tokio::task::spawn_blocking(move || {
            let entries = match std::fs::read_dir(&root) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
                Err(e) => return Err(e),
            };
            let mut keys = Vec::new();
            for entry in entries {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || !name.ends_with(".json") {
                    continue;
                }
                let key = format!("{PREFIX}{name}");
                if after.as_deref().is_none_or(|after| key.as_str() > after) {
                    keys.push(key);
                }
            }
            keys.sort();
            Ok(keys)
        })
        .await
        .map_err(|e| JournalError(e.to_string()))?
        .map_err(|e: std::io::Error| JournalError(format!("{}: {e}", self.root.display())))
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, JournalError> {
        let path = self.path(key)?;
        let shown = path.display().to_string();
        tokio::task::spawn_blocking(move || std::fs::read(path))
            .await
            .map_err(|e| JournalError(e.to_string()))?
            .map_err(|e| JournalError(format!("{shown}: {e}")))
    }
}

/// How a bucket is named in a request's URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Addressing {
    /// `https://<bucket>.<endpoint host>/<key>`: what AWS and Tigris expect. Tigris refuses
    /// the path for buckets made since February 2025.
    Virtual,
    /// `https://<endpoint host>/<bucket>/<key>`: for a store at an address, such as MinIO on
    /// this machine, where a name before the host cannot resolve.
    Path,
}

impl Addressing {
    /// Path for an endpoint at an IP address or `localhost`, by name otherwise.
    pub fn for_endpoint(endpoint: &Url) -> Self {
        match endpoint.host() {
            Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => Self::Path,
            Some(url::Host::Domain(name)) if name.eq_ignore_ascii_case("localhost") => Self::Path,
            _ => Self::Virtual,
        }
    }
}

/// A journal in a bucket of an S3-compatible store, signed with AWS Signature Version 4.
#[derive(Clone)]
pub struct S3 {
    pub endpoint: Url,
    pub bucket: String,
    pub addressing: Addressing,
    pub region: String,
    pub access_key: String,
    secret_key: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3")
            .field("endpoint", &self.endpoint.as_str())
            .field("bucket", &self.bucket)
            .field("addressing", &self.addressing)
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

/// The SHA-256 of nothing, which a request without a body is signed with.
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

impl S3 {
    pub fn new(
        endpoint: Url,
        bucket: String,
        addressing: Addressing,
        region: String,
        access_key: String,
        secret_key: String,
        http: reqwest::Client,
    ) -> Self {
        Self {
            endpoint,
            bucket,
            addressing,
            region,
            access_key,
            secret_key,
            http,
        }
    }

    /// The URL of a key, or of the bucket for `None`.
    pub fn url(&self, key: Option<&str>) -> Url {
        let mut url = self.endpoint.clone();
        let mut path = self.endpoint.path().trim_end_matches('/').to_owned();
        match self.addressing {
            Addressing::Path => {
                path.push('/');
                path.push_str(&uri_encode(&self.bucket, false));
            }
            Addressing::Virtual => {
                let host = format!(
                    "{}.{}",
                    self.bucket,
                    self.endpoint.host_str().unwrap_or_default()
                );
                // A bucket name that is not a host name is refused by the store when asked.
                let _ = url.set_host(Some(&host));
            }
        }
        if let Some(key) = key {
            path.push('/');
            path.push_str(&uri_encode(key, false));
        } else if path.is_empty() {
            path.push('/');
        }
        url.set_path(&path);
        url.set_query(None);
        url
    }

    async fn send(
        &self,
        method: reqwest::Method,
        key: Option<&str>,
        query: &[(&str, &str)],
        body: Option<&str>,
        extra: &[(&str, &str)],
    ) -> Result<reqwest::Response, JournalError> {
        let url = self.url(key);
        let host = match url.port() {
            Some(port) => format!("{}:{port}", url.host_str().unwrap_or_default()),
            None => url.host_str().unwrap_or_default().to_owned(),
        };
        let payload = body.map_or_else(|| EMPTY_SHA256.to_owned(), |b| hex(&Sha256::digest(b)));
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let amz_date = amz_date(now);
        let mut headers: Vec<(String, String)> = vec![
            ("host".into(), host),
            ("x-amz-content-sha256".into(), payload.clone()),
            ("x-amz-date".into(), amz_date.clone()),
        ];
        headers.extend(
            extra
                .iter()
                .map(|(k, v)| (k.to_lowercase(), (*v).to_owned())),
        );
        let authorization = sign(&Signing {
            method: method.as_str(),
            path: url.path(),
            query,
            headers: &headers,
            payload: &payload,
            amz_date: &amz_date,
            region: &self.region,
            access_key: &self.access_key,
            secret_key: &self.secret_key,
        });
        let mut url = url;
        if !query.is_empty() {
            url.set_query(Some(&canonical_query(query)));
        }
        let mut request = self
            .http
            .request(method, url)
            .header("authorization", authorization)
            .timeout(Duration::from_secs(30));
        for (name, value) in headers.iter().filter(|(name, _)| name != "host") {
            request = request.header(name.as_str(), value.as_str());
        }
        if let Some(body) = body {
            request = request.body(body.to_owned());
        }
        request
            .send()
            .await
            .map_err(|e| JournalError(format!("{}: {e}", self.bucket)))
    }

    async fn put(&self, key: &str, body: &str) -> Result<(), JournalError> {
        // Never over an object already there: a store that keeps conditional writes refuses with
        // 412, which is this entry written before. One that does not writes the same bytes again.
        let response = self
            .send(
                reqwest::Method::PUT,
                Some(key),
                &[],
                Some(body),
                &[("content-type", "application/json"), ("if-none-match", "*")],
            )
            .await?;
        match response.status().as_u16() {
            200..=299 | 412 => Ok(()),
            status => Err(refused(status, response).await),
        }
    }

    async fn list(&self, after: Option<&str>) -> Result<Vec<String>, JournalError> {
        let mut keys = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![("list-type", "2"), ("prefix", PREFIX)];
            if let Some(token) = &token {
                query.push(("continuation-token", token.as_str()));
            } else if let Some(after) = after {
                query.push(("start-after", after));
            }
            let response = self
                .send(reqwest::Method::GET, None, &query, None, &[])
                .await?;
            if !response.status().is_success() {
                return Err(refused(response.status().as_u16(), response).await);
            }
            let text = response
                .text()
                .await
                .map_err(|e| JournalError(format!("{}: {e}", self.bucket)))?;
            let page = parse_listing(&text)
                .ok_or_else(|| JournalError(format!("{}: an unreadable listing", self.bucket)))?;
            keys.extend(page.keys);
            match page.next {
                Some(next) => token = Some(next),
                None => break,
            }
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }

    async fn get(&self, key: &str) -> Result<Vec<u8>, JournalError> {
        let response = self
            .send(reqwest::Method::GET, Some(key), &[], None, &[])
            .await?;
        if !response.status().is_success() {
            return Err(refused(response.status().as_u16(), response).await);
        }
        Ok(response
            .bytes()
            .await
            .map_err(|e| JournalError(format!("{}: {e}", self.bucket)))?
            .to_vec())
    }
}

async fn refused(status: u16, response: reqwest::Response) -> JournalError {
    let text = response.text().await.unwrap_or_default();
    let code = between(&text, "<Code>", "</Code>").unwrap_or("no reason given");
    JournalError(format!("refused with {status}: {code}"))
}

/// One page of `ListObjectsV2`: its keys, and the token for the next if there is one.
#[derive(Debug, PartialEq, Eq)]
pub struct Listing {
    pub keys: Vec<String>,
    pub next: Option<String>,
}

/// The parts of a `ListObjectsV2` answer the journal needs. Keys are the journal's own, which
/// hold nothing XML escapes; one that does is unescaped all the same.
pub fn parse_listing(xml: &str) -> Option<Listing> {
    if !xml.contains("<ListBucketResult") {
        return None;
    }
    let mut keys = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<Key>") {
        let after = &rest[start + 5..];
        let end = after.find("</Key>")?;
        keys.push(unescape(&after[..end]));
        rest = &after[end + 6..];
    }
    let truncated = between(xml, "<IsTruncated>", "</IsTruncated>") == Some("true");
    let next = between(xml, "<NextContinuationToken>", "</NextContinuationToken>")
        .filter(|_| truncated)
        .map(unescape);
    if truncated && next.is_none() {
        return None;
    }
    Some(Listing { keys, next })
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(&text[start..end])
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// What a request is signed over.
pub struct Signing<'a> {
    pub method: &'a str,
    /// Already encoded, as it is sent.
    pub path: &'a str,
    pub query: &'a [(&'a str, &'a str)],
    /// Lowercase names. Every one is signed.
    pub headers: &'a [(String, String)],
    /// The body's SHA-256, in hex.
    pub payload: &'a str,
    /// `YYYYMMDDTHHMMSSZ`.
    pub amz_date: &'a str,
    pub region: &'a str,
    pub access_key: &'a str,
    pub secret_key: &'a str,
}

/// The `Authorization` header of AWS Signature Version 4, for S3.
pub fn sign(s: &Signing<'_>) -> String {
    let mut headers: Vec<(String, String)> = s
        .headers
        .iter()
        .map(|(name, value)| (name.to_lowercase(), value.trim().to_owned()))
        .collect();
    headers.sort();
    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed_headers}\n{}",
        s.method,
        s.path,
        canonical_query(s.query),
        s.payload
    );
    let date = &s.amz_date[..8];
    let scope = format!("{date}/{}/s3/aws4_request", s.region);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        s.amz_date,
        hex(&Sha256::digest(canonical.as_bytes()))
    );
    let key = [s.region, "s3", "aws4_request"].iter().fold(
        hmac(format!("AWS4{}", s.secret_key).as_bytes(), date.as_bytes()),
        |key, part| hmac(&key, part.as_bytes()),
    );
    let signature = hex(&hmac(&key, to_sign.as_bytes()));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        s.access_key
    )
}

fn canonical_query(query: &[(&str, &str)]) -> String {
    let mut pairs: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| (uri_encode(k, true), uri_encode(v, true)))
        .collect();
    pairs.sort();
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

/// Percent-encoding as Signature Version 4 wants it: everything but the unreserved characters,
/// and `/` too unless it is a path.
fn uri_encode(text: &str, slash: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            b'/' if !slash => out.push('/'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes a key of any length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `YYYYMMDDTHHMMSSZ` for seconds since the epoch, in UTC.
pub fn amz_date(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let (year, month, day) = civil(days);
    let rest = secs % 86_400;
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The calendar date of a day since the epoch (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = u32::try_from(doy - (153 * mp + 2) / 5 + 1).unwrap_or(1);
    let month = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).unwrap_or(1);
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// What a check of the journal found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// No journal is configured.
    None,
    /// The records hold every key the journal has.
    Current,
    /// The journal has keys the records do not: they were restored, and need reconciling.
    Behind,
    /// The journal could not be asked. What was found last stands.
    Unknown,
}

const NONE: u8 = 0;
const CURRENT: u8 = 1;
const BEHIND: u8 = 2;

/// An instance's journal: where it is, and whether its records were found behind it.
#[derive(Debug)]
pub struct Journal {
    store: Option<Store>,
    behind: AtomicBool,
    standing: AtomicU8,
}

impl Default for Journal {
    fn default() -> Self {
        Self::none()
    }
}

impl Journal {
    /// No journal: nothing is written out, and the records are never found behind. Entries wait
    /// in the records for a journal to be configured.
    pub fn none() -> Self {
        Self {
            store: None,
            behind: AtomicBool::new(false),
            standing: AtomicU8::new(NONE),
        }
    }

    pub fn new(store: Store) -> Self {
        Self {
            store: Some(store),
            behind: AtomicBool::new(false),
            standing: AtomicU8::new(CURRENT),
        }
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    /// Whether requests are refused until the records are reconciled.
    pub fn behind(&self) -> bool {
        self.behind.load(Ordering::SeqCst)
    }

    /// Whether the records are behind the journal: any key from a little before the latest they
    /// hold that they do not. Remembered for [`Journal::behind`], except when the journal could
    /// not be asked, which leaves what was found before.
    pub async fn check<R: Records>(&self, records: &R) -> Standing {
        let Some(store) = &self.store else {
            return Standing::None;
        };
        let standing = match behind(store, records).await {
            Ok(true) => Standing::Behind,
            Ok(false) => Standing::Current,
            Err(error) => {
                tracing::warn!("{error}: whether the records are behind it is not known");
                return Standing::Unknown;
            }
        };
        let was = self
            .behind
            .swap(standing == Standing::Behind, Ordering::SeqCst);
        self.standing.store(
            if standing == Standing::Behind {
                BEHIND
            } else {
                CURRENT
            },
            Ordering::SeqCst,
        );
        match (was, standing) {
            (false, Standing::Behind) => tracing::error!(
                "the journal holds revocations or deletions these records do not: they were \
                 restored from a backup. Every request is refused until `yardsort-server \
                 reconcile` has replayed it"
            ),
            (true, Standing::Current) => tracing::info!("the records are reconciled: serving"),
            _ => {}
        }
        standing
    }

    /// Send what the records hold that is not written out yet. How many were sent.
    pub async fn flush<R: Records>(&self, records: &R) -> Result<usize, String> {
        let Some(store) = &self.store else {
            return Ok(0);
        };
        flush(store, records).await
    }
}

async fn behind<R: Records>(store: &Store, records: &R) -> Result<bool, String> {
    let latest = records.journal_latest().await.map_err(|e| e.to_string())?;
    let after = latest
        .as_deref()
        .and_then(key_time)
        .map(|at| floor_key(at - CHECK_MARGIN_MS));
    let keys = store
        .list(after.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    Ok(!unknown(records, &keys)
        .await
        .map_err(|e| e.to_string())?
        .is_empty())
}

/// The keys of these the records do not hold, in order.
async fn unknown<R: Records>(records: &R, keys: &[String]) -> Result<Vec<String>, RecordsError> {
    let mut missing = Vec::new();
    for chunk in keys.chunks(KNOWN_BATCH) {
        let known = records.journal_known(chunk).await?;
        missing.extend(chunk.iter().filter(|k| !known.contains(*k)).cloned());
    }
    Ok(missing)
}

async fn flush<R: Records>(store: &Store, records: &R) -> Result<usize, String> {
    let mut sent = 0;
    loop {
        let rows = records
            .journal_unwritten(FLUSH_BATCH)
            .await
            .map_err(|e| e.to_string())?;
        if rows.is_empty() {
            return Ok(sent);
        }
        let mut written = Vec::with_capacity(rows.len());
        let mut failed = None;
        for row in &rows {
            match store.put(&row.key, &row.body).await {
                Ok(()) => written.push(row.key.clone()),
                Err(error) => {
                    failed = Some(error.to_string());
                    break;
                }
            }
        }
        records
            .journal_written(&written)
            .await
            .map_err(|e| e.to_string())?;
        sent += written.len();
        if let Some(error) = failed {
            return Err(error);
        }
        if rows.len() < FLUSH_BATCH {
            return Ok(sent);
        }
    }
}

/// What `reconcile` did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// Entries the records did not hold, replayed.
    pub replayed: usize,
    /// Of those, the ones that closed something the records still had open.
    pub changed: usize,
    pub devices: usize,
    pub grants: usize,
    pub hosts: usize,
    pub accounts: usize,
    /// Entries a newer server wrote, which this one cannot read and left alone.
    pub unreadable: Vec<String>,
    /// Entries the records held that were not yet in the journal, sent first.
    pub sent: usize,
}

/// Bring restored records up to date with the journal: send what they hold that the journal
/// does not, then replay, in order, every entry of the journal they do not hold. Each replay is
/// one step with noting its key, so running this again, or after it stopped half-way, replays
/// only what is left.
pub async fn reconcile<R: Records>(
    store: &Store,
    records: &R,
    now: i64,
) -> Result<Reconciled, String> {
    let mut done = Reconciled {
        sent: flush(store, records).await?,
        ..Reconciled::default()
    };
    let keys = store.list(None).await.map_err(|e| e.to_string())?;
    for key in unknown(records, &keys).await.map_err(|e| e.to_string())? {
        let body = store.get(&key).await.map_err(|e| e.to_string())?;
        let Some(entry) = Entry::parse(&body) else {
            done.unreadable.push(key);
            continue;
        };
        let changed = records
            .replay(
                &Row {
                    key: key.clone(),
                    body: entry.body(),
                },
                &entry,
                now,
            )
            .await
            .map_err(|e| format!("replaying {key}: {e}"))?;
        done.replayed += 1;
        if changed {
            done.changed += 1;
            match &entry.closed {
                Closed::DevicesRevoked { devices, .. } => done.devices += devices.len(),
                Closed::GrantRevoked { .. } => done.grants += 1,
                Closed::HostRemoved { .. } => done.hosts += 1,
                Closed::AccountDeleted { .. } => done.accounts += 1,
            }
        }
    }
    // Every instance asks again now, and the devices replayed shut lose their live sockets.
    for signal in [Signal::Journal, Signal::RecheckAll] {
        if let Err(error) = records.signal(signal).await {
            tracing::warn!("telling the running servers: {error}");
        }
    }
    Ok(done)
}

/// Send what is waiting every [`FLUSH_EVERY`], and ask whether the records are behind every
/// [`CHECK_EVERY`] and whenever [`Signal::Journal`] is heard, for as long as the service lives.
/// The first check is made at once.
pub(crate) fn start<R: Records + Clone + 'static>(service: Weak<ControlService<R>>) {
    let Some(strong) = service.upgrade() else {
        return;
    };
    if strong.journal.store().is_none() {
        return;
    }
    let mut signals = strong.records.signals();
    let journal = Arc::clone(&strong.journal);
    drop(strong);
    let flushing = service.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(FLUSH_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(service) = flushing.upgrade() else {
                return;
            };
            if let Err(error) = journal.flush(&service.records).await {
                tracing::warn!("{error}: what is waiting is sent again shortly");
            }
        }
    });
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(CHECK_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => {}
                heard = signals.recv() => match heard {
                    Ok(Signal::Journal) => {}
                    Ok(_) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                },
            }
            let Some(service) = service.upgrade() else {
                return;
            };
            service.journal.check(&service.records).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    /// AWS's own example of signing a GET of an object, in "Authenticating Requests: Using the
    /// Authorization Header (AWS Signature Version 4)".
    #[test]
    fn a_get_is_signed_as_aws_example_signs_it() {
        let authorization = sign(&Signing {
            method: "GET",
            path: "/test.txt",
            query: &[],
            headers: &headers(&[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY_SHA256),
                ("x-amz-date", "20130524T000000Z"),
            ]),
            payload: EMPTY_SHA256,
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: SECRET,
        });
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request, \
             SignedHeaders=host;range;x-amz-content-sha256;x-amz-date, \
             Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    /// The same page's example of a listing, which signs a query.
    #[test]
    fn a_listing_is_signed_with_its_query_sorted() {
        let authorization = sign(&Signing {
            method: "GET",
            path: "/",
            query: &[("prefix", "J"), ("max-keys", "2")],
            headers: &headers(&[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("x-amz-content-sha256", EMPTY_SHA256),
                ("x-amz-date", "20130524T000000Z"),
            ]),
            payload: EMPTY_SHA256,
            amz_date: "20130524T000000Z",
            region: "us-east-1",
            access_key: "AKIAIOSFODNN7EXAMPLE",
            secret_key: SECRET,
        });
        assert!(
            authorization.ends_with(
                "Signature=34b48302e7b5fa45bde8084f4b7868a86f0a534bc59db6670ed5711ef69dc6f7"
            ),
            "{authorization}"
        );
    }

    #[test]
    fn dates_are_written_in_utc() {
        assert_eq!(amz_date(0), "19700101T000000Z");
        assert_eq!(amz_date(1_369_353_600), "20130524T000000Z");
        // 2024-02-29T23:59:59Z, a leap day.
        assert_eq!(amz_date(1_709_251_199), "20240229T235959Z");
    }

    #[test]
    fn keys_sort_by_time_and_say_it() {
        let early = Entry::new(
            5,
            Closed::AccountDeleted {
                account: "a".into(),
            },
        )
        .key();
        let late = Entry::new(
            1_800_000_000_000,
            Closed::AccountDeleted {
                account: "a".into(),
            },
        )
        .key();
        assert!(early < late);
        assert!(early.starts_with("journal/0000000000000005-"));
        assert_eq!(key_time(&late), Some(1_800_000_000_000));
        assert!(floor_key(1_800_000_000_000) < late);
        assert!(floor_key(1_800_000_000_001) > late);
        assert_eq!(key_time("journal/x"), None);
    }

    #[test]
    fn an_entry_reads_back_and_names_ids_only() {
        let entry = Entry::new(
            42,
            Closed::DevicesRevoked {
                account: "acc".into(),
                devices: vec!["d1".into()],
                reason: Reason::ByOperator,
            },
        );
        let body = entry.body();
        assert_eq!(
            body,
            r#"{"v":1,"at":42,"what":"devicesRevoked","account":"acc","devices":["d1"],"reason":"byOperator"}"#
        );
        assert_eq!(Entry::parse(body.as_bytes()), Some(entry));
        assert_eq!(
            Entry::parse(br#"{"v":2,"at":1,"what":"somethingNew"}"#),
            None
        );
        assert_eq!(Entry::parse(b"not json"), None);
    }

    #[test]
    fn a_listing_is_read_page_by_page() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Name>b</Name><Prefix>journal/</Prefix>
<KeyCount>2</KeyCount><MaxKeys>2</MaxKeys><IsTruncated>true</IsTruncated>
<Contents><Key>journal/1-a.json</Key><Size>3</Size></Contents>
<Contents><Key>journal/2-b&amp;.json</Key><Size>3</Size></Contents>
<NextContinuationToken>tok/en=</NextContinuationToken></ListBucketResult>"#;
        assert_eq!(
            parse_listing(xml),
            Some(Listing {
                keys: vec!["journal/1-a.json".into(), "journal/2-b&.json".into()],
                next: Some("tok/en=".into()),
            })
        );
        let last = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
        assert_eq!(
            parse_listing(last),
            Some(Listing {
                keys: vec![],
                next: None
            })
        );
        assert_eq!(
            parse_listing("<Error><Code>AccessDenied</Code></Error>"),
            None
        );
        assert_eq!(
            parse_listing("<ListBucketResult><IsTruncated>true</IsTruncated></ListBucketResult>"),
            None
        );
    }

    #[test]
    fn a_bucket_is_named_in_the_host_unless_the_store_is_at_an_address() {
        let bucket = |endpoint: &str| {
            let endpoint = Url::parse(endpoint).unwrap();
            S3::new(
                endpoint.clone(),
                "yardsort-journal".into(),
                Addressing::for_endpoint(&endpoint),
                "auto".into(),
                "key".into(),
                "secret".into(),
                crate::oidc::http_client().unwrap(),
            )
        };
        let tigris = bucket("https://fly.storage.tigris.dev");
        assert_eq!(tigris.addressing, Addressing::Virtual);
        assert_eq!(
            tigris.url(Some("journal/1-a.json")).as_str(),
            "https://yardsort-journal.fly.storage.tigris.dev/journal/1-a.json"
        );
        assert_eq!(
            tigris.url(None).as_str(),
            "https://yardsort-journal.fly.storage.tigris.dev/"
        );
        let minio = bucket("http://127.0.0.1:9000");
        assert_eq!(minio.addressing, Addressing::Path);
        assert_eq!(
            minio.url(Some("journal/1-a.json")).as_str(),
            "http://127.0.0.1:9000/yardsort-journal/journal/1-a.json"
        );
        assert_eq!(
            minio.url(None).as_str(),
            "http://127.0.0.1:9000/yardsort-journal"
        );
        assert_eq!(
            Addressing::for_endpoint(&Url::parse("http://localhost:9000").unwrap()),
            Addressing::Path
        );
    }

    #[test]
    fn a_path_keeps_its_slashes_and_a_query_does_not() {
        assert_eq!(uri_encode("journal/a b.json", false), "journal/a%20b.json");
        assert_eq!(uri_encode("a/b=c", true), "a%2Fb%3Dc");
    }

    #[tokio::test]
    async fn a_directory_keeps_each_key_once_and_lists_after_one() {
        let root = tempfile::tempdir().unwrap();
        let dir = Store::Dir(Dir::new(root.path().join("journal")));
        assert_eq!(dir.list(None).await.unwrap(), Vec::<String>::new());
        dir.put("journal/2-b.json", "two").await.unwrap();
        dir.put("journal/1-a.json", "one").await.unwrap();
        dir.put("journal/1-a.json", "again").await.unwrap();
        assert_eq!(dir.get("journal/1-a.json").await.unwrap(), b"one");
        assert_eq!(
            dir.list(None).await.unwrap(),
            vec!["journal/1-a.json", "journal/2-b.json"]
        );
        assert_eq!(
            dir.list(Some("journal/1-a.json")).await.unwrap(),
            vec!["journal/2-b.json"]
        );
        assert!(dir.put("journal/../x.json", "no").await.is_err());
        assert!(dir.put("elsewhere/x.json", "no").await.is_err());
    }
}

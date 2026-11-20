use std::collections::HashMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ureq::http::header::{ACCEPT, RANGE};
use ureq::http::{HeaderMap, Response, StatusCode};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::typestate::WithoutBody;
use ureq::{Agent, Body, Proxy, RequestBuilder};

use crate::env::{Paths, remove_if_present, write_atomic};
use crate::log::fail;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

const API_CACHE_TTL: Duration = Duration::from_secs(600);
const ATTEMPTS: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(2);
const CHUNK: usize = 65536;
const PICTURE_LIMIT: u64 = 8 * 1024 * 1024;

static CACHE_LOCK: Mutex<()> = Mutex::new(());

static AGENT: LazyLock<Agent> = LazyLock::new(|| {
    Agent::config_builder()
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .user_agent("Throwback")
        .http_status_as_error(false)
        .timeout_connect(Some(HTTP_TIMEOUT))
        .timeout_per_read(Some(HTTP_TIMEOUT))
        .timeout_per_write(Some(HTTP_TIMEOUT))
        .build()
        .new_agent()
});

#[derive(Debug)]
pub struct CancelledError;

impl fmt::Display for CancelledError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("cancelled")
    }
}

impl std::error::Error for CancelledError {}

#[derive(Debug)]
pub struct RateLimitError {
    pub minutes: u64,
}

impl RateLimitError {
    fn from_headers(headers: &HeaderMap) -> Self {
        let seconds = |name: &str| -> f64 {
            let text = headers.get(name).and_then(|value| value.to_str().ok());
            text.and_then(|text| text.parse().ok()).unwrap_or_default()
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0.0, |now| now.as_secs_f64());
        let minutes = |seconds: f64| (seconds / 60.0).ceil() as u64;
        let minutes = match minutes(seconds("x-ratelimit-reset") - now) {
            0 => minutes(seconds("retry-after")),
            wait => wait,
        };
        Self { minutes }
    }
}

impl fmt::Display for RateLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GitHub rate limit reached, try again ")?;
        match self.minutes {
            0 => formatter.write_str("later"),
            1 => formatter.write_str("in 1 minute"),
            minutes => write!(formatter, "in {minutes} minutes"),
        }
    }
}

impl std::error::Error for RateLimitError {}

pub(crate) fn download_error(message: &str, error: anyhow::Error) -> anyhow::Error {
    if error.is::<RateLimitError>() || error.is::<CancelledError>() {
        return error;
    }
    anyhow!(fail(message, format!("{error:#}")))
}

#[derive(Deserialize, Serialize)]
struct Entry {
    fetched: SystemTime,
    data: Value,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

fn send(request: RequestBuilder<WithoutBody>) -> Result<Response<Body>> {
    let response = request
        .config()
        .proxy(Proxy::try_from_env())
        .build()
        .call()?;
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let headers = response.headers();
    let spent = headers
        .get("x-ratelimit-remaining")
        .is_some_and(|left| left == "0");
    let wait = headers
        .get("retry-after")
        .is_some_and(|wait| !wait.is_empty());
    if status == StatusCode::TOO_MANY_REQUESTS || status == StatusCode::FORBIDDEN && (spent || wait)
    {
        return Err(RateLimitError::from_headers(headers).into());
    }
    Err(ureq::Error::StatusCode(status.as_u16()).into())
}

fn fetch(url: &str, range: Option<String>) -> Result<Response<Body>> {
    let request = AGENT.get(url).header(ACCEPT, "application/octet-stream");
    send(match range {
        Some(range) => request.header(RANGE, range),
        None => request,
    })
}

fn guard() -> MutexGuard<'static, ()> {
    CACHE_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn load(file: &Path) -> HashMap<String, Entry> {
    fs::read(file)
        .ok()
        .and_then(|text| serde_json::from_slice(&text).ok())
        .unwrap_or_default()
}

fn cached(file: &Path, api_url: &str) -> Option<Value> {
    let _guard = guard();
    let entry = load(file).remove(api_url)?;
    let age = entry.fetched.elapsed().ok()?;
    (age < API_CACHE_TTL).then_some(entry.data)
}

fn store(file: &Path, api_url: &str, data: Value) -> Result<()> {
    let _guard = guard();
    let mut cache = load(file);
    let fetched = SystemTime::now();
    cache.insert(api_url.to_owned(), Entry { fetched, data });
    Ok(write_atomic(file, &serde_json::to_string(&cache)?)?)
}

pub fn invalidate_api_cache(paths: &Paths) -> Result<()> {
    let _guard = guard();
    Ok(remove_if_present(&paths.api_cache_file())?)
}

pub(crate) fn bytes(url: &str) -> Result<Vec<u8>> {
    Ok(send(AGENT.get(url))?.body_mut().read_to_vec()?)
}

fn capped(url: &str, limit: u64) -> Result<Vec<u8>> {
    let mut response = send(AGENT.get(url))?;
    let body = response.body_mut().with_config().limit(limit);
    Ok(body.read_to_vec()?)
}

pub fn github_media(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host = host.to_ascii_lowercase();
    host == "github.com"
        || host.ends_with(".githubusercontent.com")
        || host.ends_with(".githubassets.com")
}

pub fn fetch_picture(url: &str) -> Result<Vec<u8>> {
    if !github_media(url) {
        bail!("{url} is not hosted on GitHub");
    }
    capped(url, PICTURE_LIMIT)
}

pub(crate) fn json<T: DeserializeOwned>(url: &str) -> Result<T> {
    Ok(serde_json::from_slice(&bytes(url)?)?)
}

pub fn github_json<T: DeserializeOwned>(
    paths: &Paths,
    api_url: &str,
    bypass_ttl: bool,
) -> Result<T> {
    let file = paths.api_cache_file();
    if !bypass_ttl && let Some(data) = cached(&file, api_url) {
        return Ok(serde_json::from_value(data)?);
    }
    let data: Value = json(api_url)?;
    let parsed = T::deserialize(&data);
    store(&file, api_url, data)?;
    Ok(parsed?)
}

pub fn github_asset(paths: &Paths, api_url: &str, suffix: &str) -> Result<(String, String)> {
    for bypass_ttl in [false, true] {
        let release: Release = github_json(paths, api_url, bypass_ttl)?;
        let mut assets = release.assets.into_iter();
        if let Some(asset) = assets.find(|asset| asset.name.ends_with(suffix)) {
            return Ok((release.tag_name, asset.url));
        }
    }
    bail!("no release asset matching *{suffix}")
}

fn via_part(dest: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    let part = dest.with_added_extension("part");
    let written = File::create(&part)
        .map_err(Into::into)
        .and_then(|mut file| write(&mut file));
    let outcome = written.and_then(|()| Ok(fs::rename(&part, dest)?));
    if outcome.is_err() {
        let _ = fs::remove_file(&part);
    }
    outcome
}

fn download(
    url: &str,
    file: &mut File,
    on_progress: Option<&dyn Fn(f64)>,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<()> {
    let mut done = file.stream_position()?;
    let mut response = fetch(url, (done > 0).then(|| format!("bytes={done}-")))?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        file.set_len(0)?;
        file.rewind()?;
        done = 0;
    }
    let total = response.body().content_length().map(|left| done + left);
    let report = |done: u64| {
        if let (Some(on_progress), Some(total @ 1..)) = (on_progress, total) {
            on_progress(done as f64 / total as f64);
        }
    };
    report(done);
    let mut body = response.body_mut().as_reader();
    let mut chunk = vec![0; CHUNK];
    loop {
        let count = body.read(&mut chunk)?;
        if count == 0 {
            return Ok(());
        }
        if cancelled.is_some_and(|cancelled| cancelled()) {
            return Err(CancelledError.into());
        }
        file.write_all(&chunk[..count])?;
        done += count as u64;
        report(done);
    }
}

fn retryable(error: &anyhow::Error) -> bool {
    let refused = matches!(
        error.downcast_ref(),
        Some(ureq::Error::StatusCode(400..=499))
    );
    !(refused || error.is::<RateLimitError>() || error.is::<CancelledError>())
}

pub type Progress<'a> = Option<&'a dyn Fn(f64)>;

pub fn fetch_to(
    url: &str,
    dest: &Path,
    on_progress: Progress,
    cancelled: Option<&dyn Fn() -> bool>,
) -> Result<()> {
    via_part(dest, |file| {
        let mut attempt = 1;
        loop {
            match download(url, file, on_progress, cancelled) {
                Err(error) if attempt < ATTEMPTS && retryable(&error) => attempt += 1,
                outcome => return outcome,
            }
            if cancelled.is_some_and(|cancelled| cancelled()) {
                return Err(CancelledError.into());
            }
            thread::sleep(RETRY_DELAY);
        }
    })
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;
    use std::fmt::Display;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use serde_json::{Value, json};

    use crate::env::Paths;

    #[derive(Clone)]
    pub(crate) struct Request {
        pub(crate) index: usize,
        pub(crate) path: String,
        headers: HashMap<String, String>,
    }

    impl Request {
        pub(crate) fn header(&self, name: &str) -> Option<&str> {
            self.headers.get(name).map(String::as_str)
        }
    }

    pub(crate) struct Reply {
        status: u16,
        headers: String,
        body: Vec<u8>,
        sent: usize,
    }

    impl Reply {
        pub(crate) fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
            let body = body.into();
            Self {
                status,
                headers: String::new(),
                sent: body.len(),
                body,
            }
        }

        pub(crate) fn rate_limit() -> Self {
            Self::new(403, []).header("Retry-After", 120)
        }

        pub(crate) fn header(mut self, name: &str, value: impl Display) -> Self {
            self.headers += &format!("{name}: {value}\r\n");
            self
        }

        pub(crate) fn cut(mut self, sent: usize) -> Self {
            self.sent = sent;
            self
        }

        fn encode(&self) -> Vec<u8> {
            let head = format!(
                "HTTP/1.1 {} Status\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n",
                self.status,
                self.headers,
                self.body.len()
            );
            [head.as_bytes(), &self.body[..self.sent]].concat()
        }
    }

    pub(crate) struct Server {
        base: String,
        seen: Arc<Mutex<Vec<Request>>>,
    }

    impl Server {
        pub(crate) fn start(answer: impl Fn(&Request) -> Reply + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let sink = Arc::clone(&seen);
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut lines = BufReader::new(&stream).lines().map_while(Result::ok);
                    let path = lines
                        .next()
                        .and_then(|line| line.split(' ').nth(1).map(str::to_string));
                    let Some(path) = path else {
                        continue;
                    };
                    let headers = lines
                        .take_while(|line| !line.is_empty())
                        .filter_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            Some((name.to_ascii_lowercase(), value.trim().to_string()))
                        })
                        .collect();
                    let request = Request {
                        index: sink.lock().unwrap().len(),
                        path,
                        headers,
                    };
                    let reply = answer(&request);
                    sink.lock().unwrap().push(request);
                    let _ = (&stream).write_all(&reply.encode());
                }
            });
            Self { base, seen }
        }

        pub(crate) fn url(&self, path: &str) -> String {
            format!("{}{path}", self.base)
        }

        pub(crate) fn requests(&self) -> Vec<Request> {
            self.seen.lock().unwrap().clone()
        }

        pub(crate) fn paths(&self) -> Vec<String> {
            let requests = self.requests();
            requests.into_iter().map(|request| request.path).collect()
        }
    }

    pub(crate) fn seed(paths: &Paths, api_url: &str, assets: &[(&str, &str)]) {
        let assets: Vec<Value> = assets
            .iter()
            .map(|(name, url)| json!({"name": name, "url": url}))
            .collect();
        let release = json!({"tag_name": "v1", "assets": assets});
        super::store(&paths.api_cache_file(), api_url, release).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::io;
    use std::path::PathBuf;
    use std::time::Instant;

    use serde_json::json;
    use tempfile::TempDir;

    use super::testing::{Reply, Request, Server, seed};
    use super::*;
    use crate::log::take_recorded;

    const PYTHON_CACHE: &str =
        r#"{"URL": {"data": {"tag_name": "v0.2.1", "assets": []}, "ts": 1794534530.666341}}"#;

    fn latest_tag(paths: &Paths, api_url: &str) -> Result<String> {
        Ok(github_json::<Release>(paths, api_url, false)?.tag_name)
    }

    fn paths(folder: &TempDir) -> Paths {
        Paths::new(folder.path().join("asset"), folder.path().join("data"))
    }

    fn part(dest: &Path) -> PathBuf {
        dest.with_added_extension("part")
    }

    fn noise(length: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        };
        (0..length).map(|_| next()).collect()
    }

    fn release(tag: &str, assets: Value) -> Reply {
        Reply::new(200, json!({"tag_name": tag, "assets": assets}).to_string())
    }

    fn ranged(request: &Request, content: &[u8]) -> Reply {
        let range = request.header("range").unwrap();
        let (first, last) = range
            .strip_prefix("bytes=")
            .unwrap()
            .split_once('-')
            .unwrap();
        let first: usize = first.parse().unwrap();
        let end = last.parse().map_or(content.len(), |last: usize| last + 1);
        let span = format!("bytes {first}-{}/{}", end - 1, content.len());
        Reply::new(206, &content[first..end]).header("Content-Range", span)
    }

    fn ranges(server: &Server) -> Vec<Option<String>> {
        let requests = server.requests();
        let range = |request: &Request| request.header("range").map(str::to_string);
        requests.iter().map(range).collect()
    }

    fn download(server: &Server, path: &str, dest: &Path) -> (Result<()>, Vec<f64>) {
        let progress = RefCell::new(Vec::new());
        let record = |fraction| progress.borrow_mut().push(fraction);
        let outcome = fetch_to(&server.url(path), dest, Some(&record), None);
        (outcome, progress.into_inner())
    }

    fn backdate(paths: &Paths, api_url: &str, fetched: SystemTime) {
        let mut cache = load(&paths.api_cache_file());
        cache.get_mut(api_url).unwrap().fetched = fetched;
        let text = serde_json::to_string(&cache).unwrap();
        fs::write(paths.api_cache_file(), text).unwrap();
    }

    #[test]
    fn rate_limit_message_names_the_wait() {
        let message = |minutes| RateLimitError { minutes }.to_string();
        assert_eq!(message(0), "GitHub rate limit reached, try again later");
        assert_eq!(
            message(1),
            "GitHub rate limit reached, try again in 1 minute"
        );
        assert_eq!(
            message(2),
            "GitHub rate limit reached, try again in 2 minutes"
        );
        assert_eq!(
            message(60),
            "GitHub rate limit reached, try again in 60 minutes"
        );
    }

    #[test]
    fn rate_limits_are_told_from_other_refusals() {
        let server = Server::start(|request| {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let reset = now.as_secs() + 90;
            let refused = Reply::new(403, []);
            match request.path.as_str() {
                "/429" => Reply::new(429, []),
                "/spent" => refused
                    .header("X-RateLimit-Remaining", 0)
                    .header("X-RateLimit-Reset", reset),
                "/spent-and-retry" => refused
                    .header("X-RateLimit-Remaining", 0)
                    .header("X-RateLimit-Reset", reset - 180)
                    .header("Retry-After", 30),
                "/retry" => refused.header("Retry-After", 121),
                "/left" => refused
                    .header("X-RateLimit-Remaining", 12)
                    .header("X-RateLimit-Reset", reset),
                "/forbidden" => refused,
                _ => Reply::new(404, []),
            }
        });
        let minutes = |path: &str| {
            let error = send(AGENT.get(server.url(path))).unwrap_err();
            let limit = error.downcast_ref::<RateLimitError>();
            limit.map(|limit| limit.minutes)
        };
        assert_eq!(minutes("/429"), Some(0));
        assert_eq!(minutes("/spent"), Some(2));
        assert_eq!(minutes("/spent-and-retry"), Some(1));
        assert_eq!(minutes("/retry"), Some(3));
        assert_eq!(minutes("/left"), None);
        assert_eq!(minutes("/forbidden"), None);
        assert_eq!(minutes("/missing"), None);
    }

    #[test]
    fn rate_limits_and_cancels_pass_and_other_failures_take_the_message() {
        let limit = RateLimitError { minutes: 2 };
        let limited = download_error("ThrowbackLoader download failed", limit.into());
        assert!(limited.is::<RateLimitError>());
        assert_eq!(
            limited.to_string(),
            "GitHub rate limit reached, try again in 2 minutes"
        );
        let cancelled = download_error("Wine download failed", CancelledError.into());
        assert!(cancelled.is::<CancelledError>());
        assert!(take_recorded().is_empty());
        let missing = anyhow!("no release asset matching *.zip");
        let failed = download_error("ThrowbackLoader download failed", missing);
        assert_eq!(failed.to_string(), "ThrowbackLoader download failed");
        assert_eq!(
            take_recorded(),
            ["ThrowbackLoader download failed - no release asset matching *.zip"]
        );
        let unpack = anyhow::Error::from(io::Error::other("disk full")).context("unpack");
        let failed = download_error("DepotDownloader download failed", unpack);
        assert_eq!(failed.to_string(), "DepotDownloader download failed");
        assert_eq!(
            take_recorded(),
            ["DepotDownloader download failed - unpack: disk full"]
        );
    }

    #[test]
    fn answers_are_cached_for_ten_minutes() {
        let server = Server::start(|request| release(&format!("v{}", request.index), json!([])));
        let folder = tempfile::tempdir().unwrap();
        let paths = paths(&folder);
        let url = server.url("/repos/xeralin/ThrowbackLauncher/releases/latest");
        let other = "https://api.github.com/repos/ip7z/7zip/releases/latest";
        seed(&paths, other, &[]);
        let tag = || latest_tag(&paths, &url).unwrap();
        let aged = |age: u64| backdate(&paths, &url, SystemTime::now() - Duration::from_secs(age));
        assert_eq!(tag(), "v0");
        assert_eq!(tag(), "v0");
        let sent = &server.requests()[0];
        assert_eq!(sent.header("user-agent"), Some("Throwback"));
        assert_ne!(sent.header("accept"), Some("application/octet-stream"));
        aged(599);
        assert_eq!(tag(), "v0");
        aged(601);
        assert_eq!(tag(), "v1");
        backdate(&paths, &url, SystemTime::now() + Duration::from_secs(60));
        assert_eq!(tag(), "v2");
        let forced: Value = github_json(&paths, &url, true).unwrap();
        assert_eq!(forced["tag_name"], "v3");
        assert_eq!(tag(), "v3");
        assert_eq!(server.paths().len(), 4);
        assert!(load(&paths.api_cache_file()).contains_key(other));
    }

    #[test]
    fn a_failed_request_does_not_fall_back_to_a_stale_answer() {
        let server = Server::start(|_| Reply::new(500, "down"));
        let folder = tempfile::tempdir().unwrap();
        let paths = paths(&folder);
        let url = server.url("/releases/latest");
        seed(&paths, &url, &[]);
        backdate(&paths, &url, UNIX_EPOCH);
        let before = fs::read(paths.api_cache_file()).unwrap();
        assert!(latest_tag(&paths, &url).is_err());
        assert_eq!(fs::read(paths.api_cache_file()).unwrap(), before);
        assert_eq!(server.paths().len(), 1);
    }

    #[test]
    fn an_answer_is_cached_even_when_the_caller_cannot_read_it() {
        let server = Server::start(|_| release("v1", json!([])));
        let folder = tempfile::tempdir().unwrap();
        let paths = paths(&folder);
        let url = server.url("/releases/latest");
        assert!(github_json::<Vec<Value>>(&paths, &url, false).is_err());
        assert_eq!(latest_tag(&paths, &url).unwrap(), "v1");
        assert_eq!(server.paths().len(), 1);
    }

    #[test]
    fn a_cache_from_the_python_launcher_is_rebuilt() {
        let server = Server::start(|_| release("v0.3.0", json!([])));
        let folder = tempfile::tempdir().unwrap();
        let paths = paths(&folder);
        let url = server.url("/releases/latest");
        fs::create_dir_all(paths.bin_dir()).unwrap();
        fs::write(paths.api_cache_file(), PYTHON_CACHE.replace("URL", &url)).unwrap();
        assert_eq!(latest_tag(&paths, &url).unwrap(), "v0.3.0");
        assert_eq!(latest_tag(&paths, &url).unwrap(), "v0.3.0");
        assert_eq!(server.paths().len(), 1);
    }

    #[test]
    fn invalidate_removes_the_whole_cache() {
        let folder = tempfile::tempdir().unwrap();
        let paths = paths(&folder);
        invalidate_api_cache(&paths).unwrap();
        seed(&paths, "url", &[]);
        invalidate_api_cache(&paths).unwrap();
        assert!(!paths.api_cache_file().exists());
    }

    #[test]
    fn asset_lookup_asks_again_before_giving_up() {
        let server = Server::start(|request| {
            let assets = json!([
                {"name": "tool-linux-arm64.zip", "url": "one"},
                {"name": "tool-linux-x64.zip", "url": "two"},
            ]);
            match request.index {
                0 => release("v2", json!([])),
                _ => release("v2", assets),
            }
        });
        let folder = tempfile::tempdir().unwrap();
        let paths = paths(&folder);
        let url = server.url("/releases/latest");
        let found = |suffix: &str| github_asset(&paths, &url, suffix);
        let asset = |url: &str| ("v2".to_string(), url.to_string());
        assert_eq!(found("linux-x64.zip").unwrap(), asset("two"));
        assert_eq!(server.paths().len(), 2);
        assert_eq!(found(".zip").unwrap(), asset("one"));
        assert_eq!(server.paths().len(), 2);
        assert!(found(".7z").is_err());
        assert_eq!(server.paths().len(), 3);
    }

    #[test]
    fn pictures_come_only_from_github_and_only_up_to_the_limit() {
        let hosted = [
            "https://github.com/user-attachments/assets/1",
            "https://raw.githubusercontent.com/o/r/main/a.png",
            "https://GitHub.com/o/r/raw/HEAD/a.png",
            "https://github.githubassets.com/a.png",
        ];
        assert!(hosted.into_iter().all(github_media));
        let foreign = [
            "http://github.com/a.png",
            "https://example.com/a.png",
            "https://github.com@example.com/a.png",
            "https://example.com/raw.githubusercontent.com/a.png",
            "https://notgithubusercontent.com/a.png",
            "https://github.com.example.com/a.png",
        ];
        assert!(!foreign.into_iter().any(github_media));
        assert!(fetch_picture("https://example.com/a.png").is_err());
        let server = Server::start(|_| Reply::new(200, vec![7; 9]));
        assert_eq!(capped(&server.url("/a.png"), 9).unwrap(), vec![7; 9]);
        assert!(capped(&server.url("/a.png"), 8).is_err());
    }

    #[test]
    fn downloads_replace_the_target_and_report_progress() {
        let content = noise(150_000);
        let body = content.clone();
        let server = Server::start(move |_| Reply::new(200, body.clone()));
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("Runtime.zip");
        fs::write(&dest, "old").unwrap();
        fs::write(part(&dest), "stale").unwrap();
        let (outcome, progress) = download(&server, "/assets/1", &dest);
        outcome.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert!(!part(&dest).exists());
        assert_eq!(progress.first(), Some(&0.0));
        assert_eq!(progress.last(), Some(&1.0));
        assert!(progress.is_sorted());
        let requests = server.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].header("user-agent"), Some("Throwback"));
        assert_eq!(
            requests[0].header("accept"),
            Some("application/octet-stream")
        );
        assert_eq!(requests[0].header("accept-encoding"), None);
        assert_eq!(requests[0].header("range"), None);
    }

    #[test]
    fn downloads_resume_after_a_cut_connection() {
        let content = noise(200_000);
        let body = content.clone();
        let server = Server::start(move |request| match request.index {
            0 => Reply::new(200, body.clone()).cut(70_000),
            _ => ranged(request, &body),
        });
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("Runtime.zip");
        let started = Instant::now();
        let (outcome, progress) = download(&server, "/asset", &dest);
        outcome.unwrap();
        assert!(started.elapsed() >= RETRY_DELAY);
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert_eq!(ranges(&server), [None, Some("bytes=70000-".to_string())]);
        assert!(progress.is_sorted());
        assert_eq!(progress.last(), Some(&1.0));
    }

    #[test]
    fn downloads_start_over_when_the_range_is_ignored() {
        let content = noise(200_000);
        let body = content.clone();
        let server = Server::start(move |request| match request.index {
            0 => Reply::new(200, body.clone()).cut(70_000),
            _ => Reply::new(200, body.clone()),
        });
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("Runtime.zip");
        let (outcome, progress) = download(&server, "/asset", &dest);
        outcome.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert_eq!(ranges(&server), [None, Some("bytes=70000-".to_string())]);
        assert_eq!(progress.iter().filter(|done| **done == 0.0).count(), 2);
        assert_eq!(progress.last(), Some(&1.0));
    }

    #[test]
    fn a_refused_resume_fails_the_download() {
        let body = noise(200_000);
        let server = Server::start(move |request| match request.index {
            0 => Reply::new(200, body.clone()).cut(70_000),
            _ => Reply::new(416, []),
        });
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("Runtime.zip");
        let (outcome, _) = download(&server, "/asset", &dest);
        assert!(outcome.is_err());
        assert_eq!(ranges(&server), [None, Some("bytes=70000-".to_string())]);
        assert!(!dest.exists());
        assert!(!part(&dest).exists());
    }

    #[test]
    fn downloads_give_up_after_three_attempts() {
        let body = noise(200_000);
        let server = Server::start(move |_| Reply::new(200, body.clone()).cut(70_000));
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("Runtime.zip");
        fs::write(&dest, "old").unwrap();
        let started = Instant::now();
        let (outcome, _) = download(&server, "/asset", &dest);
        assert!(outcome.is_err());
        assert!(started.elapsed() >= RETRY_DELAY * 2);
        assert_eq!(server.paths().len(), 3);
        assert_eq!(fs::read(&dest).unwrap(), b"old");
        assert!(!part(&dest).exists());
    }

    #[test]
    fn server_errors_are_retried() {
        let content = noise(1000);
        let body = content.clone();
        let server = Server::start(move |request| match request.index {
            0 => Reply::new(503, "busy"),
            _ => Reply::new(200, body.clone()),
        });
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("7zz");
        let (outcome, progress) = download(&server, "/asset", &dest);
        outcome.unwrap();
        assert_eq!(fs::read(&dest).unwrap(), content);
        assert_eq!(progress.last(), Some(&1.0));
        assert_eq!(ranges(&server), [None, None]);
    }

    #[test]
    fn client_errors_and_rate_limits_are_not_retried() {
        let server = Server::start(|request| match request.path.as_str() {
            "/limited" => Reply::rate_limit(),
            _ => Reply::new(404, []),
        });
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("tl.zip");
        let (gone, _) = download(&server, "/gone", &dest);
        assert!(!gone.unwrap_err().is::<RateLimitError>());
        let (limited, _) = download(&server, "/limited", &dest);
        assert_eq!(
            limited.unwrap_err().to_string(),
            "GitHub rate limit reached, try again in 2 minutes"
        );
        assert_eq!(server.paths(), ["/gone", "/limited"]);
        assert!(!dest.exists());
        assert!(!part(&dest).exists());
    }

    #[test]
    fn cancelled_downloads_stop_without_waiting() {
        let body = noise(200_000);
        let server = Server::start(move |request| match request.path.as_str() {
            "/down" => Reply::new(500, []),
            _ => Reply::new(200, body.clone()),
        });
        let folder = tempfile::tempdir().unwrap();
        let dest = folder.path().join("_wine.tar.xz");
        let started = Instant::now();
        for path in ["/asset", "/down"] {
            let outcome = fetch_to(&server.url(path), &dest, None, Some(&|| true));
            assert!(outcome.unwrap_err().is::<CancelledError>(), "{path}");
        }
        assert!(started.elapsed() < RETRY_DELAY);
        assert_eq!(server.paths(), ["/asset", "/down"]);
        assert!(!dest.exists());
        assert!(!part(&dest).exists());
    }
}

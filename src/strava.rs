//! Strava: upload finished rides.
//!
//! Strava only lets an app connect accounts it has approved, so VeloForge
//! uses your own (free) Strava API application: you create it once at
//! strava.com/settings/api and paste its Client ID and Client Secret.
//! Connecting opens Strava in your browser, which sends you back to
//! VeloForge on this computer with a code; VeloForge trades it for access
//! tokens, renews them when they expire, and uploads rides as FIT files.
//! Requests run on a background thread; results come back over a channel.

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

pub const BASE_URL: &str = "https://www.strava.com";
/// Where you create the API application.
pub const API_SETTINGS: &str = "https://www.strava.com/settings/api";

/// Stored separately from the rest of the state, readable only by you.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: String,
    /// Unix time the access token stops working.
    #[serde(default)]
    pub expires_at: i64,
    /// Who connected, for Settings.
    #[serde(default)]
    pub athlete: String,
}

impl Credentials {
    pub fn has_app(&self) -> bool {
        !self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }

    pub fn is_connected(&self) -> bool {
        self.has_app() && !self.refresh_token.is_empty()
    }

    pub fn load(path: &std::path::Path) -> Credentials {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        crate::icu::save_private(path, &serde_json::to_string_pretty(self)?)
    }
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Client {
    pub fn new(base: &str) -> Client {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .build();
        Client {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_string(),
        }
    }

    /// Strava's page asking you to let VeloForge upload rides.
    pub fn authorize_url(&self, client_id: &str, redirect: &str, state: &str) -> String {
        format!(
            "{}/oauth/authorize?client_id={}&response_type=code&redirect_uri={}&approval_prompt=auto&scope=activity:write&state={}",
            self.base,
            crate::icu::encode_query(client_id.trim()),
            crate::icu::encode_query(redirect),
            crate::icu::encode_query(state),
        )
    }

    fn check(status: u16, body: &str) -> Result<()> {
        match status {
            200..=299 => Ok(()),
            400 if body.contains("invalid") => bail!(
                "Strava did not accept the Client ID or Client Secret. Copy them again from strava.com/settings/api."
            ),
            401 | 403 => bail!(
                "Strava did not accept VeloForge's access. Reconnect in Settings → Strava."
            ),
            429 => bail!("Strava asked us to slow down; try again in a quarter of an hour."),
            _ => bail!(
                "Strava replied {status}: {}",
                body.chars().take(200).collect::<String>()
            ),
        }
    }

    /// Trade an authorization code, or a refresh token, for access tokens.
    fn token(&self, creds: &Credentials, grant: &str, value: &str) -> Result<Credentials> {
        let key = if grant == "authorization_code" {
            "code"
        } else {
            "refresh_token"
        };
        let form = format!(
            "client_id={}&client_secret={}&grant_type={}&{key}={}",
            crate::icu::encode_query(creds.client_id.trim()),
            crate::icu::encode_query(creds.client_secret.trim()),
            grant,
            crate::icu::encode_query(value),
        );
        let mut resp = self
            .agent
            .post(&format!("{}/oauth/token", self.base))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(form.as_bytes())
            .map_err(|e| anyhow!("could not reach Strava: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        Self::check(status, &body)?;
        let v: serde_json::Value =
            serde_json::from_str(&body).context("unexpected reply from Strava")?;
        let mut out = creds.clone();
        out.access_token = v["access_token"]
            .as_str()
            .context("Strava sent no access token")?
            .to_string();
        if let Some(r) = v["refresh_token"].as_str() {
            out.refresh_token = r.to_string();
        }
        out.expires_at = v["expires_at"].as_i64().unwrap_or(0);
        let a = &v["athlete"];
        let name = format!(
            "{} {}",
            a["firstname"].as_str().unwrap_or(""),
            a["lastname"].as_str().unwrap_or("")
        );
        if !name.trim().is_empty() {
            out.athlete = name.trim().to_string();
        }
        Ok(out)
    }

    pub fn exchange(&self, creds: &Credentials, code: &str) -> Result<Credentials> {
        self.token(creds, "authorization_code", code)
    }

    /// The credentials with an access token good for a few more minutes.
    pub fn fresh(&self, creds: &Credentials, now: i64) -> Result<Credentials> {
        if !creds.access_token.is_empty() && creds.expires_at - now > 300 {
            return Ok(creds.clone());
        }
        self.token(creds, "refresh_token", &creds.refresh_token)
    }

    /// Upload a ride file and wait for Strava to turn it into an activity.
    /// Returns the activity id. A ride Strava already has counts as done.
    pub fn upload(
        &self,
        creds: &Credentials,
        file_name: &str,
        data: &[u8],
        name: &str,
        poll_every: Duration,
    ) -> Result<String> {
        let data_type = if file_name.ends_with(".fit") {
            "fit"
        } else {
            "tcx"
        };
        let boundary = format!(
            "veloforge{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let mut body = Vec::with_capacity(data.len() + 800);
        for (field, value) in [
            ("data_type", data_type),
            ("name", name),
            ("trainer", "1"),
            ("external_id", file_name),
        ] {
            body.extend(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend(data);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        let bearer = format!("Bearer {}", creds.access_token);
        let mut resp = self
            .agent
            .post(&format!("{}/api/v3/uploads", self.base))
            .header("Authorization", &bearer)
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send(&body[..])
            .map_err(|e| anyhow!("could not reach Strava: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Self::check(status, &text)?;
        let mut v: serde_json::Value =
            serde_json::from_str(&text).context("unexpected reply from Strava")?;
        let id = v["id"].as_i64().context("Strava sent no upload id")?;
        // Strava processes uploads in the background: ask until it is done.
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            if let Some(a) = v["activity_id"].as_i64() {
                return Ok(a.to_string());
            }
            if let Some(e) = v["error"].as_str().filter(|e| !e.is_empty()) {
                // "… duplicate of <a href='/activities/123'>…" or "… duplicate of activity 123".
                if let Some(rest) = e.split("duplicate of").nth(1) {
                    let digits: String = rest
                        .chars()
                        .skip_while(|c| !c.is_ascii_digit())
                        .take_while(|c| c.is_ascii_digit())
                        .collect();
                    return Ok(digits);
                }
                bail!("Strava could not read the ride: {e}");
            }
            if Instant::now() > deadline {
                bail!("Strava is still processing the ride; it should appear there shortly.");
            }
            std::thread::sleep(poll_every);
            let mut resp = self
                .agent
                .get(&format!("{}/api/v3/uploads/{id}", self.base))
                .header("Authorization", &bearer)
                .call()
                .map_err(|e| anyhow!("could not reach Strava: {e}"))?;
            let status = resp.status().as_u16();
            let text = resp.body_mut().read_to_string().unwrap_or_default();
            Self::check(status, &text)?;
            v = serde_json::from_str(&text).context("unexpected reply from Strava")?;
        }
    }
}

/// The `code` from Strava's redirect, e.g.
/// "GET /strava?state=abc&code=xyz&scope=read,activity:write HTTP/1.1".
pub fn code_from_request(line: &str, state: &str) -> Result<String> {
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    let get = |key: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.to_string())
    };
    if get("error").is_some() {
        bail!("Strava was not allowed to connect (you pressed Cancel).");
    }
    if get("state").as_deref() != Some(state) {
        bail!("Strava's reply did not match this request; please try again.");
    }
    if !get("scope").is_some_and(|s| s.contains("activity:write")) {
        bail!("Please leave \"Upload your activities\" ticked on Strava's page.");
    }
    get("code").context("Strava's reply had no code")
}

/// Wait for the browser to come back from Strava to `listeners`; reply with
/// a page saying it can be closed.
fn wait_for_code(listeners: &[TcpListener], state: &str, timeout: Duration) -> Result<String> {
    for l in listeners {
        l.set_nonblocking(true)?;
    }
    let deadline = Instant::now() + timeout;
    loop {
        for l in listeners {
            if let Ok((stream, _)) = l.accept() {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut line = String::new();
                let _ = BufReader::new(stream.try_clone()?).read_line(&mut line);
                // Browsers also ask for /favicon.ico; only Strava's redirect counts.
                if !line.contains("/strava") {
                    respond(stream, "");
                    continue;
                }
                let result = code_from_request(&line, state);
                respond(
                    stream,
                    match &result {
                        Ok(_) => "VeloForge is connected to Strava. You can close this tab.",
                        Err(_) => "VeloForge could not connect to Strava. Go back to VeloForge to see why.",
                    },
                );
                return result;
            }
        }
        if Instant::now() > deadline {
            bail!("Strava did not answer in time; please try again.");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn respond(mut stream: TcpStream, message: &str) {
    let page = format!(
        "<!doctype html><meta charset=utf-8><title>VeloForge</title><body style=\"font:18px sans-serif;margin:3em;background:#0f1116;color:#e8eaf0\"><p>{message}</p>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
        page.len()
    );
}

pub enum Request {
    /// Open Strava in the browser and wait for permission.
    Connect(Credentials),
    Upload {
        creds: Credentials,
        file: String,
        name: String,
        /// The summary's file, to match the reply to the ride.
        key: String,
    },
}

pub enum Reply {
    Connected(Result<Credentials, String>),
    Uploaded {
        key: String,
        result: Result<String, String>,
        /// Renewed tokens, to keep.
        creds: Option<Credentials>,
    },
}

pub struct Worker {
    tx: Sender<Request>,
    pub rx: Receiver<Reply>,
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context) -> Worker {
        let base = std::env::var("VELOFORGE_STRAVA_URL").unwrap_or_else(|_| BASE_URL.to_string());
        let (tx, req_rx) = channel::<Request>();
        let (reply_tx, rx) = channel();
        std::thread::Builder::new()
            .name("strava".into())
            .spawn(move || {
                let client = Client::new(&base);
                while let Ok(req) = req_rx.recv() {
                    let reply = match req {
                        Request::Connect(creds) => {
                            Reply::Connected(connect(&client, &creds).map_err(|e| format!("{e:#}")))
                        }
                        Request::Upload {
                            creds,
                            file,
                            name,
                            key,
                        } => {
                            let now = chrono::Utc::now().timestamp();
                            match client.fresh(&creds, now) {
                                Err(e) => Reply::Uploaded {
                                    key,
                                    result: Err(format!("{e:#}")),
                                    creds: None,
                                },
                                Ok(fresh) => {
                                    let result = std::fs::read(&file)
                                        .map_err(|e| anyhow!("could not read {file}: {e}"))
                                        .and_then(|data| {
                                            let file_name = std::path::Path::new(&file)
                                                .file_name()
                                                .and_then(|n| n.to_str())
                                                .unwrap_or("ride.fit");
                                            client.upload(
                                                &fresh,
                                                file_name,
                                                &data,
                                                &name,
                                                Duration::from_secs(2),
                                            )
                                        })
                                        .map_err(|e| format!("{e:#}"));
                                    Reply::Uploaded {
                                        key,
                                        result,
                                        creds: (fresh != creds).then_some(fresh),
                                    }
                                }
                            }
                        }
                    };
                    if reply_tx.send(reply).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn Strava worker");
        Worker { tx, rx }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }
}

fn connect(client: &Client, creds: &Credentials) -> Result<Credentials> {
    // Strava allows sending the browser back to this computer ("localhost").
    let v4 = TcpListener::bind("127.0.0.1:0").context("could not listen for Strava's reply")?;
    let port = v4.local_addr()?.port();
    let mut listeners = vec![v4];
    // Some browsers try IPv6 first for "localhost".
    if let Ok(v6) = TcpListener::bind(("::1", port)) {
        listeners.push(v6);
    }
    let mut seed = [0u8; 12];
    let _ = getrandom::fill(&mut seed);
    let state: String = seed.iter().map(|b| format!("{b:02x}")).collect();
    let redirect = format!("http://localhost:{port}/strava");
    crate::platform::open(client.authorize_url(&creds.client_id, &redirect, &state));
    let code = wait_for_code(&listeners, &state, Duration::from_secs(300))?;
    client.exchange(creds, &code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::{Arc, Mutex};

    /// Requests the mock received: (request line, headers, body).
    type RequestLog = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

    /// A tiny fake Strava: records requests, returns canned replies.
    fn mock_server(replies: Vec<(u16, &'static str)>) -> (String, RequestLog) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            for (status, body) in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                reader.read_line(&mut request_line).unwrap();
                let mut headers = String::new();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    headers += &line;
                }
                let mut payload = vec![0; len];
                reader.read_exact(&mut payload).unwrap();
                log2.lock()
                    .unwrap()
                    .push((request_line.trim().to_string(), headers, payload));
                let mut s = stream;
                let _ = write!(s, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            }
        });
        (format!("http://{addr}"), log)
    }

    fn app() -> Credentials {
        Credentials {
            client_id: "12345".into(),
            client_secret: "s3cret".into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_redirect_carries_the_code() {
        let line = "GET /strava?state=abc&code=xyz789&scope=read,activity:write HTTP/1.1";
        assert_eq!(code_from_request(line, "abc").unwrap(), "xyz789");
        assert!(code_from_request(line, "other").is_err());
        let denied = "GET /strava?state=abc&error=access_denied HTTP/1.1";
        assert!(code_from_request(denied, "abc")
            .unwrap_err()
            .to_string()
            .contains("Cancel"));
        let no_upload = "GET /strava?state=abc&code=x&scope=read HTTP/1.1";
        assert!(code_from_request(no_upload, "abc").is_err());
    }

    #[test]
    fn connecting_trades_the_code_for_tokens() {
        let (base, log) = mock_server(vec![(
            200,
            r#"{"token_type":"Bearer","expires_at":1900000000,"access_token":"acc1","refresh_token":"ref1","athlete":{"firstname":"Hasan","lastname":"A"}}"#,
        )]);
        let c = Client::new(&base).exchange(&app(), "xyz").unwrap();
        assert_eq!(c.access_token, "acc1");
        assert_eq!(c.refresh_token, "ref1");
        assert_eq!(c.expires_at, 1_900_000_000);
        assert_eq!(c.athlete, "Hasan A");
        assert!(c.is_connected());
        let log = log.lock().unwrap();
        assert_eq!(log[0].0, "POST /oauth/token HTTP/1.1");
        let body = String::from_utf8_lossy(&log[0].2);
        assert!(body.contains("client_id=12345"), "{body}");
        assert!(body.contains("grant_type=authorization_code"), "{body}");
        assert!(body.contains("code=xyz"), "{body}");
    }

    #[test]
    fn expired_tokens_are_renewed_before_use() {
        let mut c = app();
        c.access_token = "old".into();
        c.refresh_token = "ref1".into();
        c.expires_at = 1000;
        // Still good: no request at all.
        let (base, log) = mock_server(vec![]);
        assert_eq!(Client::new(&base).fresh(&c, 100).unwrap(), c);
        assert!(log.lock().unwrap().is_empty());
        // Nearly expired: renewed.
        let (base, log) = mock_server(vec![(
            200,
            r#"{"access_token":"new","refresh_token":"ref2","expires_at":9000}"#,
        )]);
        let f = Client::new(&base).fresh(&c, 900).unwrap();
        assert_eq!(
            (f.access_token.as_str(), f.refresh_token.as_str()),
            ("new", "ref2")
        );
        let body = String::from_utf8_lossy(&log.lock().unwrap()[0].2).to_string();
        assert!(
            body.contains("grant_type=refresh_token&refresh_token=ref1"),
            "{body}"
        );
    }

    #[test]
    fn uploads_wait_for_the_activity() {
        let (base, log) = mock_server(vec![
            (
                201,
                r#"{"id":77,"status":"Your activity is still being processed.","activity_id":null,"error":null}"#,
            ),
            (
                200,
                r#"{"id":77,"status":"Your activity is ready.","activity_id":555,"error":null}"#,
            ),
        ]);
        let mut c = app();
        c.access_token = "acc1".into();
        let id = Client::new(&base)
            .upload(&c, "ride.fit", b"FITDATA", "Sweet Spot", Duration::ZERO)
            .unwrap();
        assert_eq!(id, "555");
        let log = log.lock().unwrap();
        assert_eq!(log[0].0, "POST /api/v3/uploads HTTP/1.1");
        assert!(log[0]
            .1
            .to_ascii_lowercase()
            .contains("authorization: bearer acc1"));
        let body = String::from_utf8_lossy(&log[0].2);
        assert!(body.contains("name=\"data_type\"\r\n\r\nfit\r\n"), "{body}");
        assert!(body.contains("name=\"trainer\"\r\n\r\n1\r\n"), "{body}");
        assert!(body.contains("FITDATA"));
        assert_eq!(log[1].0, "GET /api/v3/uploads/77 HTTP/1.1");
    }

    #[test]
    fn a_ride_strava_already_has_counts_as_uploaded() {
        let (base, _) = mock_server(vec![(
            201,
            r#"{"id":78,"activity_id":null,"error":"ride.fit duplicate of <a href='/activities/4242' target='_blank'>Sweet Spot</a>"}"#,
        )]);
        let mut c = app();
        c.access_token = "acc1".into();
        let id = Client::new(&base)
            .upload(&c, "ride.fit", b"x", "x", Duration::ZERO)
            .unwrap();
        assert_eq!(id, "4242");
    }

    #[test]
    fn failures_are_explained() {
        let (base, _) = mock_server(vec![(401, r#"{"message":"Authorization Error"}"#)]);
        let mut c = app();
        c.access_token = "bad".into();
        let e = Client::new(&base)
            .upload(&c, "ride.fit", b"x", "x", Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(e.contains("Reconnect"), "{e}");
        let (base, _) = mock_server(vec![(
            201,
            r#"{"id":79,"activity_id":null,"error":"Error parsing file."}"#,
        )]);
        let e = Client::new(&base)
            .upload(&c, "ride.fit", b"x", "x", Duration::ZERO)
            .unwrap_err()
            .to_string();
        assert!(e.contains("could not read"), "{e}");
    }

    #[test]
    fn the_browser_comes_back_with_the_code() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                s,
                "GET /strava?state=st&code=c0de&scope=read,activity:write HTTP/1.1\r\nHost: localhost\r\n\r\n"
            )
            .unwrap();
            let mut page = String::new();
            let _ = s.read_to_string(&mut page);
            page
        });
        let code = wait_for_code(&[l], "st", Duration::from_secs(5)).unwrap();
        assert_eq!(code, "c0de");
        assert!(browser.join().unwrap().contains("You can close this tab"));
    }
}

//! intervals.icu integration: download planned workouts, upload rides.
//! Requests run on a background thread; results come back over a channel.

use anyhow::{anyhow, bail, Context, Result};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;

pub const BASE_URL: &str = "https://intervals.icu";

/// Stored separately from the rest of the state, readable only by the user.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Credentials {
    /// Athlete id such as "i12345"; "0" means the key's own athlete.
    pub athlete: String,
    pub key: String,
}

impl Credentials {
    pub fn is_set(&self) -> bool {
        !self.key.trim().is_empty()
    }

    pub fn load(path: &std::path::Path) -> Credentials {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        use std::io::Write;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        // Readable only by the user. On Windows the file lives in the user's
        // own AppData folder, which other accounts cannot read.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options.open(path)?;
        f.write_all(serde_json::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Athlete {
    pub name: String,
    pub ftp: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlannedEvent {
    pub id: i64,
    pub date: NaiveDate,
    pub name: String,
}

pub fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn encode_query(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub struct Client {
    agent: ureq::Agent,
    base: String,
    creds: Credentials,
}

impl Client {
    pub fn new(creds: Credentials, base: &str) -> Client {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        Client {
            agent: ureq::Agent::new_with_config(config),
            base: base.trim_end_matches('/').to_string(),
            creds,
        }
    }

    fn athlete_path(&self) -> String {
        let id = self.creds.athlete.trim();
        format!(
            "{}/api/v1/athlete/{}",
            self.base,
            if id.is_empty() { "0" } else { id }
        )
    }

    fn auth(&self) -> String {
        format!(
            "Basic {}",
            base64(format!("API_KEY:{}", self.creds.key.trim()).as_bytes())
        )
    }

    fn check(status: u16, body: &str) -> Result<()> {
        match status {
            200..=299 => Ok(()),
            401 | 403 => bail!("intervals.icu did not accept the API key. Copy it again from Settings → Developer Settings on intervals.icu."),
            404 => bail!("intervals.icu could not find that athlete. Check the athlete ID (it looks like i12345), or use 0."),
            429 => bail!("intervals.icu asked us to slow down; try again in a minute."),
            _ => bail!("intervals.icu replied {status}: {}", body.chars().take(200).collect::<String>()),
        }
    }

    fn get(&self, url: &str) -> Result<String> {
        let mut resp = self
            .agent
            .get(url)
            .header("Authorization", &self.auth())
            .call()
            .map_err(|e| anyhow!("could not reach intervals.icu: {e}"))?;
        let status = resp.status().as_u16();
        let body = resp.body_mut().read_to_string().unwrap_or_default();
        Self::check(status, &body)?;
        Ok(body)
    }

    pub fn athlete(&self) -> Result<Athlete> {
        let v: serde_json::Value = serde_json::from_str(&self.get(&self.athlete_path())?)
            .context("unexpected reply from intervals.icu")?;
        let name = v["name"]
            .as_str()
            .or(v["firstname"].as_str())
            .unwrap_or("your account")
            .to_string();
        let ftp = v["sportSettings"].as_array().and_then(|list| {
            list.iter()
                .find(|s| {
                    s["types"]
                        .as_array()
                        .is_some_and(|t| t.iter().any(|x| x == "Ride" || x == "VirtualRide"))
                })
                .and_then(|s| s["ftp"].as_f64())
                .map(|f| f as f32)
        });
        Ok(Athlete { name, ftp })
    }

    pub fn planned_rides(&self, oldest: NaiveDate, newest: NaiveDate) -> Result<Vec<PlannedEvent>> {
        let url = format!(
            "{}/events?oldest={oldest}&newest={newest}",
            self.athlete_path()
        );
        let v: serde_json::Value = serde_json::from_str(&self.get(&url)?)
            .context("unexpected reply from intervals.icu")?;
        let mut out = Vec::new();
        for e in v.as_array().into_iter().flatten() {
            if e["category"].as_str() != Some("WORKOUT") {
                continue;
            }
            if let Some(t) = e["type"].as_str() {
                if t != "Ride" && t != "VirtualRide" {
                    continue;
                }
            }
            let (Some(id), Some(date)) = (e["id"].as_i64(), e["start_date_local"].as_str()) else {
                continue;
            };
            let Ok(date) = NaiveDate::parse_from_str(&date[..10.min(date.len())], "%Y-%m-%d")
            else {
                continue;
            };
            out.push(PlannedEvent {
                id,
                date,
                name: e["name"].as_str().unwrap_or("Workout").to_string(),
            });
        }
        Ok(out)
    }

    pub fn workout_zwo(&self, event: i64) -> Result<String> {
        self.get(&format!(
            "{}/events/{event}/download.zwo",
            self.athlete_path()
        ))
    }

    /// Upload a ride file; returns the new activity id.
    pub fn upload(&self, file_name: &str, data: &[u8], name: &str) -> Result<String> {
        let boundary = format!(
            "veloforge{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let mut body = Vec::with_capacity(data.len() + 300);
        body.extend(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend(data);
        body.extend(format!("\r\n--{boundary}--\r\n").as_bytes());
        let url = format!(
            "{}/activities?name={}",
            self.athlete_path(),
            encode_query(name)
        );
        let mut resp = self
            .agent
            .post(&url)
            .header("Authorization", &self.auth())
            .header(
                "Content-Type",
                &format!("multipart/form-data; boundary={boundary}"),
            )
            .send(&body[..])
            .map_err(|e| anyhow!("could not reach intervals.icu: {e}"))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        Self::check(status, &text)?;
        let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let id = if v.is_array() { &v[0]["id"] } else { &v["id"] };
        Ok(id
            .as_str()
            .map(String::from)
            .or(id.as_i64().map(|n| n.to_string()))
            .unwrap_or_default())
    }
}

pub enum Request {
    Test(Credentials),
    Sync {
        creds: Credentials,
        oldest: NaiveDate,
        newest: NaiveDate,
    },
    Upload {
        creds: Credentials,
        file: String,
        name: String,
    },
}

/// A planned ride with its workout file (or why it could not be fetched).
pub type PlannedRide = (PlannedEvent, Result<String, String>);

pub enum Reply {
    Connected(Result<Athlete, String>),
    Synced {
        oldest: NaiveDate,
        newest: NaiveDate,
        result: Result<Vec<PlannedRide>, String>,
    },
    Uploaded {
        file: String,
        result: Result<String, String>,
    },
}

pub struct Worker {
    tx: Sender<Request>,
    pub rx: Receiver<Reply>,
}

impl Worker {
    pub fn spawn(ctx: eframe::egui::Context) -> Worker {
        Self::spawn_with_base(ctx, BASE_URL.to_string())
    }

    pub fn spawn_with_base(ctx: eframe::egui::Context, base: String) -> Worker {
        let (tx, req_rx) = channel::<Request>();
        let (reply_tx, rx) = channel();
        std::thread::Builder::new()
            .name("intervals.icu".into())
            .spawn(move || {
                while let Ok(req) = req_rx.recv() {
                    let reply = match req {
                        Request::Test(c) => Reply::Connected(
                            Client::new(c, &base).athlete().map_err(|e| e.to_string()),
                        ),
                        Request::Sync {
                            creds,
                            oldest,
                            newest,
                        } => {
                            let client = Client::new(creds, &base);
                            let result = client
                                .planned_rides(oldest, newest)
                                .map_err(|e| e.to_string())
                                .map(|events| {
                                    events
                                        .into_iter()
                                        .map(|e| {
                                            let zwo =
                                                client.workout_zwo(e.id).map_err(|e| e.to_string());
                                            (e, zwo)
                                        })
                                        .collect()
                                });
                            Reply::Synced {
                                oldest,
                                newest,
                                result,
                            }
                        }
                        Request::Upload { creds, file, name } => {
                            let result =
                                std::fs::read(&file)
                                    .map_err(|e| e.to_string())
                                    .and_then(|data| {
                                        let file_name = std::path::Path::new(&file)
                                            .file_name()
                                            .and_then(|n| n.to_str())
                                            .unwrap_or("ride.tcx");
                                        Client::new(creds, &base)
                                            .upload(file_name, &data, &name)
                                            .map_err(|e| e.to_string())
                                    });
                            Reply::Uploaded { file, result }
                        }
                    };
                    if reply_tx.send(reply).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("spawn intervals.icu worker");
        Worker { tx, rx }
    }

    pub fn send(&self, r: Request) {
        let _ = self.tx.send(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"API_KEY:abc123"), "QVBJX0tFWTphYmMxMjM=");
    }

    /// Requests the mock received: (request line, headers, body).
    type RequestLog = Arc<Mutex<Vec<(String, String, Vec<u8>)>>>;

    /// A tiny fake intervals.icu: records requests, returns canned replies.
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

    fn creds() -> Credentials {
        Credentials {
            athlete: "i42".into(),
            key: "abc123".into(),
        }
    }

    #[test]
    fn athlete_and_auth() {
        let (base, log) = mock_server(vec![(
            200,
            r#"{"id":"i42","name":"Hasan","sportSettings":[{"types":["Run"],"ftp":null},{"types":["Ride","VirtualRide"],"ftp":248}]}"#,
        )]);
        let a = Client::new(creds(), &base).athlete().unwrap();
        assert_eq!(
            a,
            Athlete {
                name: "Hasan".into(),
                ftp: Some(248.0)
            }
        );
        let log = log.lock().unwrap();
        assert_eq!(log[0].0, "GET /api/v1/athlete/i42 HTTP/1.1");
        assert!(
            log[0]
                .1
                .contains("authorization: Basic QVBJX0tFWTphYmMxMjM=")
                || log[0]
                    .1
                    .contains("Authorization: Basic QVBJX0tFWTphYmMxMjM=")
        );
    }

    #[test]
    fn rejected_key_is_explained() {
        let (base, _) = mock_server(vec![(401, r#"{"error":"Unauthorized"}"#)]);
        let e = Client::new(creds(), &base)
            .athlete()
            .unwrap_err()
            .to_string();
        assert!(e.contains("API key"), "{e}");
    }

    #[test]
    fn planned_rides_and_workout_download() {
        let (base, log) = mock_server(vec![
            (
                200,
                r#"[{"id":7,"category":"WORKOUT","type":"Ride","name":"VO2 5x3","start_date_local":"2026-11-03T00:00:00"},
                    {"id":8,"category":"WORKOUT","type":"Run","name":"Easy run","start_date_local":"2026-11-04T00:00:00"},
                    {"id":9,"category":"NOTE","name":"Holiday","start_date_local":"2026-11-05T00:00:00"}]"#,
            ),
            (
                200,
                r#"<workout_file><name>VO2 5x3</name><workout><SteadyState Duration="180" Power="1.15"/></workout></workout_file>"#,
            ),
        ]);
        let c = Client::new(creds(), &base);
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let events = c.planned_rides(d("2026-11-01"), d("2026-11-30")).unwrap();
        assert_eq!(
            events,
            vec![PlannedEvent {
                id: 7,
                date: d("2026-11-03"),
                name: "VO2 5x3".into()
            }]
        );
        let zwo = c.workout_zwo(7).unwrap();
        assert!(crate::workout::parse_zwo(&zwo, "icu-7").is_ok());
        let log = log.lock().unwrap();
        assert_eq!(
            log[0].0,
            "GET /api/v1/athlete/i42/events?oldest=2026-11-01&newest=2026-11-30 HTTP/1.1"
        );
        assert_eq!(
            log[1].0,
            "GET /api/v1/athlete/i42/events/7/download.zwo HTTP/1.1"
        );
    }

    #[test]
    fn upload_sends_multipart_file() {
        let (base, log) = mock_server(vec![(201, r#"{"id":"i9876","name":"Sweet Spot"}"#)]);
        let id = Client::new(creds(), &base)
            .upload("ride.tcx", b"<tcx/>", "Sweet Spot 3x10")
            .unwrap();
        assert_eq!(id, "i9876");
        let log = log.lock().unwrap();
        assert_eq!(
            log[0].0,
            "POST /api/v1/athlete/i42/activities?name=Sweet%20Spot%203x10 HTTP/1.1"
        );
        assert!(log[0]
            .1
            .to_ascii_lowercase()
            .contains("content-type: multipart/form-data; boundary="));
        let body = String::from_utf8_lossy(&log[0].2);
        assert!(body.contains("name=\"file\"; filename=\"ride.tcx\""));
        assert!(body.contains("<tcx/>"));
    }

    #[test]
    fn credentials_file_is_private() {
        let path =
            std::env::temp_dir().join(format!("veloforge-creds-{}.json", std::process::id()));
        creds().save(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(Credentials::load(&path), creds());
        let _ = std::fs::remove_file(path);
    }
}

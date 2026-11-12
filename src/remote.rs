//! Phone remote: a small web server on the local network. A phone on the
//! same Wi-Fi opens the page (from a QR code in Settings), sees the live
//! numbers and can start, pause, skip steps and change the intensity.
//!
//! Every request must carry the access code from the QR code, so other
//! devices on the network can't control the ride. Nothing else is exposed.

use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 8642;

/// What the phone asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Start, pause or resume (like Space).
    Toggle,
    /// Harder / easier: intensity, slope, difficulty or heart rate target.
    Up,
    Down,
    /// Next workout step.
    Skip,
    /// 3D view <-> dashboard.
    View,
    /// Finish and save the ride.
    Finish,
}

impl Action {
    fn parse(s: &str) -> Option<Action> {
        Some(match s {
            "toggle" => Action::Toggle,
            "up" => Action::Up,
            "down" => Action::Down,
            "skip" => Action::Skip,
            "view" => Action::View,
            "finish" => Action::Finish,
            _ => return None,
        })
    }
}

struct Shared {
    token: String,
    /// Latest ride state as JSON, updated by the app.
    state: Mutex<String>,
    last_seen: Mutex<Option<Instant>>,
    stop: AtomicBool,
    actions: Mutex<Sender<Action>>,
    wake: Box<dyn Fn() + Send + Sync>,
}

pub struct Remote {
    pub rx: Receiver<Action>,
    pub port: u16,
    shared: Arc<Shared>,
}

impl Remote {
    /// Start serving on all interfaces. `wake` is called after each action
    /// so the app handles it promptly even when idle.
    pub fn start(
        port: u16,
        token: String,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Remote> {
        // A previous server on this port may still be shutting down.
        let mut tries = 0;
        let listener = loop {
            match TcpListener::bind(("0.0.0.0", port)) {
                Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && tries < 8 => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(60));
                }
                r => break r?,
            }
        };
        listener.set_nonblocking(true)?;
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            token,
            state: Mutex::new("{}".into()),
            last_seen: Mutex::new(None),
            stop: AtomicBool::new(false),
            actions: Mutex::new(tx),
            wake: Box::new(wake),
        });
        let s = shared.clone();
        std::thread::Builder::new()
            .name("phone-remote".into())
            .spawn(move || {
                while !s.stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let s = s.clone();
                            std::thread::spawn(move || {
                                let _ = handle(stream, &s);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(40));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })?;
        Ok(Remote { rx, port, shared })
    }

    pub fn set_state(&self, json: String) {
        if let Ok(mut s) = self.shared.state.lock() {
            *s = json;
        }
    }

    /// A phone has polled within the last few seconds.
    pub fn phone_connected(&self) -> bool {
        self.shared
            .last_seen
            .lock()
            .ok()
            .and_then(|t| *t)
            .is_some_and(|t| t.elapsed() < Duration::from_secs(4))
    }

    pub fn url(&self) -> String {
        let host = local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "localhost".into());
        format!("http://{host}:{}/?k={}", self.port, self.shared.token)
    }
}

impl Drop for Remote {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
    }
}

/// This computer's address on the local network (the one a phone on the
/// same Wi-Fi can reach). No packets are sent.
pub fn local_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

/// A new random access code (8 characters, no look-alikes).
pub fn new_token() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // Fall back to the clock; still unguessable enough on a home network.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (t >> (i * 7)) as u8 ^ (i as u8).wrapping_mul(97);
        }
    }
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then_some(v)
    })
}

fn handle(mut stream: TcpStream, s: &Shared) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    // Read the request head (we never need a body).
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut chunk)?;
        if n == 0 || buf.len() > 8192 {
            return Ok(());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let head = String::from_utf8_lossy(&buf);
    let mut parts = head.lines().next().unwrap_or("").split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let authorised = query_param(query, "k") == Some(s.token.as_str());

    let (status, ctype, body): (&str, &str, String) = match (method, path) {
        ("GET", "/") if authorised => ("200 OK", "text/html; charset=utf-8", PAGE.to_string()),
        ("GET", "/") => (
            "403 Forbidden",
            "text/html; charset=utf-8",
            DENIED.to_string(),
        ),
        ("GET", "/api/state") if authorised => {
            if let Ok(mut t) = s.last_seen.lock() {
                *t = Some(Instant::now());
            }
            let state = s.state.lock().map(|j| j.clone()).unwrap_or_default();
            ("200 OK", "application/json", state)
        }
        ("POST", "/api/do") if authorised => {
            match query_param(query, "a").and_then(Action::parse) {
                Some(a) => {
                    if let Ok(tx) = s.actions.lock() {
                        let _ = tx.send(a);
                    }
                    (s.wake)();
                    ("200 OK", "application/json", "{\"ok\":true}".into())
                }
                None => (
                    "400 Bad Request",
                    "application/json",
                    "{\"ok\":false}".into(),
                ),
            }
        }
        (_, "/api/state" | "/api/do") => (
            "403 Forbidden",
            "application/json",
            "{\"error\":\"wrong code\"}".into(),
        ),
        _ => ("404 Not Found", "text/plain", "Not found".into()),
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\
         Referrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

const DENIED: &str = r#"<!doctype html><html><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>VeloForge</title></head>
<body style="background:#101116;color:#e8e8ec;font-family:system-ui,sans-serif;padding:24px">
<h2>VeloForge remote</h2>
<p>This link is missing the access code. In VeloForge, open
<b>Settings &rarr; Phone remote</b> and scan the QR code.</p></body></html>"#;

/// The remote control page (self-contained, no external resources).
const PAGE: &str = r##"<!doctype html>
<html lang="en"><head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<meta name="theme-color" content="#101116">
<meta name="apple-mobile-web-app-capable" content="yes">
<meta name="mobile-web-app-capable" content="yes">
<title>VeloForge</title>
<style>
:root { --bg:#101116; --card:#1c1e26; --line:#2a2d38; --text:#ecedf1; --weak:#9a9db0; --accent:#de702c; --green:#2f9e5e; }
* { box-sizing:border-box; -webkit-tap-highlight-color:transparent; }
html,body { margin:0; background:var(--bg); color:var(--text); font-family:system-ui,-apple-system,"Segoe UI",Roboto,sans-serif; }
body { padding:14px 14px calc(18px + env(safe-area-inset-bottom)); max-width:560px; margin:0 auto; }
header { display:flex; align-items:center; justify-content:space-between; gap:10px; margin-bottom:12px; }
.brand { font-weight:700; letter-spacing:.2px; display:flex; align-items:center; gap:8px; }
.dot { width:10px; height:10px; border-radius:50%; background:var(--accent); box-shadow:0 0 12px var(--accent); }
.chip { font-size:13px; padding:4px 10px; border-radius:999px; background:var(--card); color:var(--weak); border:1px solid var(--line); white-space:nowrap; }
.chip.run { color:#fff; background:var(--green); border-color:transparent; }
.chip.pause { color:#111; background:#f0c45a; border-color:transparent; }
#title { color:var(--weak); font-size:14px; margin:0 0 10px; white-space:nowrap; overflow:hidden; text-overflow:ellipsis; }
.grid { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
.tile { background:var(--card); border-radius:14px; padding:10px 12px 12px; border-left:4px solid var(--line); min-height:84px; }
.tile .k { font-size:11px; letter-spacing:.6px; color:var(--weak); text-transform:uppercase; }
.tile .v { font-size:40px; font-weight:600; line-height:1.1; margin-top:4px; font-variant-numeric:tabular-nums; }
.tile .v small { font-size:14px; color:var(--weak); font-weight:500; margin-left:3px; }
.tile.big .v { font-size:56px; }
.interval { background:var(--card); border-radius:14px; padding:12px 14px; margin-top:10px; display:none; }
.interval .now { font-size:28px; font-weight:600; margin:2px 0; }
.interval .sub { color:var(--weak); font-size:14px; }
.bar { height:6px; background:#ffffff1a; border-radius:3px; margin-top:10px; overflow:hidden; }
.bar > div { height:100%; width:0; background:var(--accent); transition:width .5s; }
.controls { margin-top:14px; display:grid; gap:10px; }
button { font:inherit; color:var(--text); background:var(--card); border:1px solid var(--line); border-radius:14px; padding:16px; font-size:18px; font-weight:600; touch-action:manipulation; }
button:active { transform:scale(.98); filter:brightness(1.2); }
button:disabled { opacity:.35; }
#toggle { background:var(--green); border-color:transparent; font-size:22px; padding:20px; }
#toggle.pause { background:#3a3d4a; }
.adjust { display:grid; grid-template-columns:72px 1fr 72px; gap:10px; align-items:center; }
.adjust .mid { text-align:center; }
.adjust .mid .k { font-size:12px; color:var(--weak); text-transform:uppercase; letter-spacing:.6px; }
.adjust .mid .v { font-size:26px; font-weight:600; }
.adjust button { font-size:30px; padding:10px 0; }
.row { display:grid; grid-template-columns:1fr 1fr; gap:10px; }
#finish { color:#ffb9a0; }
#lost { display:none; background:#5a2a1c; color:#fff; padding:10px 12px; border-radius:12px; margin-bottom:10px; font-size:14px; }
#companion { color:var(--weak); font-size:14px; margin-top:8px; text-align:center; }
</style></head>
<body>
<header><div class="brand"><span class="dot"></span>VeloForge</div><span id="state" class="chip">…</span></header>
<div id="lost">Can't reach VeloForge — is it still running, and is this phone on the same Wi-Fi?</div>
<p id="title"></p>
<div class="grid">
  <div class="tile big" id="t-power"><div class="k">Power (3s)</div><div class="v"><span id="power">—</span><small>W</small></div></div>
  <div class="tile big" id="t-hr" style="border-left-color:#e6505f"><div class="k">Heart rate</div><div class="v"><span id="hr">—</span><small>bpm</small></div></div>
  <div class="tile" id="t-target"><div class="k" id="target-k">Target</div><div class="v"><span id="target">—</span><small id="target-u">W</small></div></div>
  <div class="tile" style="border-left-color:#5aaae6"><div class="k" id="third-k">Cadence</div><div class="v"><span id="third">—</span><small id="third-u">rpm</small></div></div>
  <div class="tile"><div class="k">Time</div><div class="v" id="time">0:00</div></div>
  <div class="tile"><div class="k">Distance</div><div class="v"><span id="dist">0.00</span><small>km</small></div></div>
</div>
<div class="interval" id="interval">
  <div class="sub" id="i-label"></div>
  <div class="now" id="i-now"></div>
  <div class="sub" id="i-next"></div>
  <div class="bar"><div id="i-bar"></div></div>
</div>
<div id="companion"></div>
<div class="controls">
  <button id="toggle" onclick="act('toggle')">Start</button>
  <div class="adjust">
    <button onclick="act('down')" aria-label="Easier">−</button>
    <div class="mid"><div class="k" id="adj-k">Intensity</div><div class="v" id="adj-v">100%</div></div>
    <button onclick="act('up')" aria-label="Harder">+</button>
  </div>
  <div class="row">
    <button id="skip" onclick="act('skip')">Skip step</button>
    <button id="view" onclick="act('view')">Dashboard</button>
  </div>
  <button id="finish" onclick="if (confirm('Finish and save this ride?')) act('finish')">Finish ride</button>
</div>
<script>
const key = new URLSearchParams(location.search).get('k') || '';
const $ = id => document.getElementById(id);
let failures = 0;
function fmt(v, d) { return v === null || v === undefined ? '—' : Number(v).toFixed(d || 0); }
async function act(a) {
  try { await fetch('/api/do?a=' + a + '&k=' + encodeURIComponent(key), { method: 'POST' }); } catch (e) {}
  setTimeout(poll, 120);
}
function render(s) {
  const chip = $('state');
  chip.textContent = s.state === 'running' ? 'Riding' : s.state === 'paused' ? 'Paused' : 'Ready';
  chip.className = 'chip' + (s.state === 'running' ? ' run' : s.state === 'paused' ? ' pause' : '');
  $('title').textContent = s.title || '';
  $('power').textContent = fmt(s.power);
  $('t-power').style.borderLeftColor = s.power_color || '#2a2d38';
  $('hr').textContent = fmt(s.hr);
  if (s.hr_color) $('t-hr').style.borderLeftColor = s.hr_color;
  if (s.target !== null && s.target !== undefined) {
    $('target-k').textContent = 'Target'; $('target').textContent = fmt(s.target); $('target-u').textContent = 'W';
    $('t-target').style.borderLeftColor = s.target_color || '#2a2d38';
  } else {
    $('target-k').textContent = 'Speed'; $('target').textContent = fmt(s.speed, 1); $('target-u').textContent = 'km/h';
    $('t-target').style.borderLeftColor = '#2a2d38';
  }
  if (s.cadence !== null && s.cadence !== undefined) {
    $('third-k').textContent = 'Cadence'; $('third').textContent = fmt(s.cadence); $('third-u').textContent = 'rpm';
  } else {
    $('third-k').textContent = 'Grade'; $('third').textContent = (s.grade >= 0 ? '+' : '') + fmt(s.grade, 1); $('third-u').textContent = '%';
  }
  $('time').textContent = s.time || '0:00';
  $('dist').textContent = fmt(s.distance, 2);
  const iv = s.interval;
  $('interval').style.display = iv ? 'block' : 'none';
  if (iv) {
    $('i-label').textContent = iv.label; $('i-now').textContent = iv.now;
    $('i-next').textContent = iv.next || ''; $('i-bar').style.width = Math.round((iv.progress || 0) * 100) + '%';
    $('i-bar').style.background = iv.color || '#de702c';
  }
  $('companion').textContent = s.companion || '';
  const t = $('toggle');
  t.textContent = s.state === 'running' ? 'Pause' : s.state === 'paused' ? 'Resume' : 'Start';
  t.className = s.state === 'running' ? 'pause' : '';
  t.disabled = s.state === 'ready' && !s.can_start;
  $('adj-k').textContent = s.adjust_label || '';
  $('adj-v').textContent = s.adjust_value || '';
  $('skip').disabled = !s.can_skip;
  $('view').textContent = s.view_3d ? 'Dashboard' : '3D view';
  $('finish').disabled = s.state === 'ready';
}
async function poll() {
  try {
    const r = await fetch('/api/state?k=' + encodeURIComponent(key), { cache: 'no-store' });
    if (!r.ok) throw new Error(r.status);
    render(await r.json());
    failures = 0;
  } catch (e) { failures++; }
  $('lost').style.display = failures >= 3 ? 'block' : 'none';
}
poll();
setInterval(poll, 700);
// Keep the screen on while riding, where supported.
if ('wakeLock' in navigator) {
  const lock = () => navigator.wakeLock.request('screen').catch(() => {});
  lock(); document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'visible') lock(); });
}
</script>
</body></html>
"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn request(port: u16, req: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn serves_page_and_actions_only_with_the_code() {
        let port = 18642;
        let woken = Arc::new(AtomicBool::new(false));
        let w = woken.clone();
        let r = Remote::start(port, "abc23456".into(), move || {
            w.store(true, Ordering::Relaxed)
        })
        .unwrap();
        r.set_state("{\"state\":\"ready\"}".into());

        let denied = request(port, "GET / HTTP/1.1\r\nHost: x\r\n\r\n");
        assert!(denied.starts_with("HTTP/1.1 403"), "{denied}");
        let wrong = request(port, "POST /api/do?a=toggle&k=nope HTTP/1.1\r\n\r\n");
        assert!(wrong.starts_with("HTTP/1.1 403"));
        assert!(r.rx.try_recv().is_err());

        let page = request(port, "GET /?k=abc23456 HTTP/1.1\r\n\r\n");
        assert!(page.starts_with("HTTP/1.1 200") && page.contains("<title>VeloForge</title>"));
        let state = request(port, "GET /api/state?k=abc23456 HTTP/1.1\r\n\r\n");
        assert!(state.ends_with("{\"state\":\"ready\"}"), "{state}");
        assert!(r.phone_connected());

        let ok = request(port, "POST /api/do?k=abc23456&a=skip HTTP/1.1\r\n\r\n");
        assert!(ok.starts_with("HTTP/1.1 200"));
        assert_eq!(r.rx.recv_timeout(Duration::from_secs(1)), Ok(Action::Skip));
        assert!(woken.load(Ordering::Relaxed));
        let bad = request(port, "POST /api/do?k=abc23456&a=explode HTTP/1.1\r\n\r\n");
        assert!(bad.starts_with("HTTP/1.1 400"));
    }

    #[test]
    fn tokens_are_random_and_readable() {
        let a = new_token();
        let b = new_token();
        assert_eq!(a.len(), 8);
        assert_ne!(a, b);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() && c != 'l' && c != '1'));
    }
}

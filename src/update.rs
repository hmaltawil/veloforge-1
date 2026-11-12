//! Tells you when a newer VeloForge has been released. At start-up it asks
//! GitHub for the latest release, once, in the background (turn it off in
//! Settings → Data & about). The request carries no information about you
//! or your rides, only the app's version.

use anyhow::{bail, Context, Result};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// GitHub's description of the newest (non pre-release) release.
pub const LATEST_URL: &str = "https://api.github.com/repos/hmaltawil/veloforge-1/releases/latest";
/// Where people can see every release.
pub const RELEASES_PAGE: &str = "https://github.com/hmaltawil/veloforge-1/releases";

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// Version without the leading "v", for example "1.0.1".
    pub version: String,
    /// The release's page on GitHub, where the downloads are.
    pub url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Checking,
    UpToDate,
    Available(Release),
    Failed(String),
}

/// "v1.2.3", "1.2.3" or "1.2.3-beta" → ((1, 2, 3), is_pre_release).
fn parse_version(s: &str) -> Option<((u64, u64, u64), bool)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match s.split_once(['-', '+']) {
        Some((core, _)) => (core, s.as_bytes()[core.len()] == b'-'),
        None => (s, false),
    };
    let mut parts = core.split('.').map(|p| p.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(((major, minor, patch), pre))
}

/// Whether `candidate` is a later version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some((new, new_pre)), Some((cur, cur_pre))) => {
            // 1.1.0 is newer than 1.1.0-beta.
            new > cur || (new == cur && cur_pre && !new_pre)
        }
        _ => false,
    }
}

/// Ask `url` (GitHub's "latest release") which version is newest.
/// `Ok(None)` means nothing has been released yet.
pub fn fetch_latest(url: &str) -> Result<Option<Release>> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let mut resp = agent
        .get(url)
        // GitHub asks every client to name itself.
        .header(
            "User-Agent",
            &format!("VeloForge/{}", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("could not reach GitHub")?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().read_to_string().unwrap_or_default();
    match status {
        200 => {}
        404 => return Ok(None),
        403 | 429 => bail!("GitHub asked us to try again later"),
        _ => bail!("GitHub replied {status}"),
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).context("unexpected reply from GitHub")?;
    let tag = v["tag_name"].as_str().context("the reply has no version")?;
    let url = v["html_url"]
        .as_str()
        .filter(|u| u.starts_with("https://github.com/"))
        .unwrap_or(RELEASES_PAGE);
    if parse_version(tag).is_none() {
        bail!("GitHub's latest release has an unexpected version: {tag}");
    }
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: url.to_string(),
    }))
}

/// Check in the background; the answer arrives on the returned channel,
/// and `ctx` is asked to repaint so it is shown promptly.
pub fn check(ctx: eframe::egui::Context, url: String) -> Receiver<Status> {
    let (tx, rx) = channel();
    let spawned = std::thread::Builder::new()
        .name("update-check".into())
        .spawn(move || {
            let status = match fetch_latest(&url) {
                Ok(Some(r)) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => {
                    Status::Available(r)
                }
                Ok(_) => Status::UpToDate,
                Err(e) => Status::Failed(format!("{e:#}")),
            };
            let _ = tx.send(status);
            ctx.request_repaint();
        });
    if let Err(e) = spawned {
        let (tx, rx) = channel();
        let _ = tx.send(Status::Failed(e.to_string()));
        return rx;
    }
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    #[test]
    fn versions_compare_like_releases() {
        assert!(is_newer("v1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("v2.0.0", "1.12.3"));
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(is_newer("1.1.0", "1.1.0-beta"));
        assert!(is_newer("v1.1", "1.0.0"));
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.9", "1.0.0"));
        assert!(!is_newer("1.1.0-beta", "1.1.0"));
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(!is_newer("1.0.0.1", "1.0.0"));
    }

    /// A one-request fake GitHub: returns the reply and records the request.
    fn mock(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                head += &line;
            }
            let mut s = stream;
            let _ = write!(
                s,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            head
        });
        (format!("http://{addr}/releases/latest"), handle)
    }

    #[test]
    fn reads_the_latest_release() {
        let (url, server) = mock(
            200,
            r#"{"tag_name":"v1.2.0","html_url":"https://github.com/hmaltawil/veloforge-1/releases/tag/v1.2.0","prerelease":false}"#,
        );
        let r = fetch_latest(&url).unwrap().unwrap();
        assert_eq!(
            r,
            Release {
                version: "1.2.0".into(),
                url: "https://github.com/hmaltawil/veloforge-1/releases/tag/v1.2.0".into()
            }
        );
        let head = server.join().unwrap().to_ascii_lowercase();
        assert!(head.starts_with("get /releases/latest http/1.1"), "{head}");
        assert!(head.contains("user-agent: veloforge/"), "{head}");
    }

    #[test]
    fn only_github_links_are_offered() {
        let (url, _server) = mock(
            200,
            r#"{"tag_name":"v1.2.0","html_url":"https://example.com/elsewhere"}"#,
        );
        assert_eq!(fetch_latest(&url).unwrap().unwrap().url, RELEASES_PAGE);
    }

    #[test]
    fn no_release_yet_is_not_an_error() {
        let (url, _server) = mock(404, r#"{"message":"Not Found"}"#);
        assert_eq!(fetch_latest(&url).unwrap(), None);
    }

    #[test]
    fn failures_are_explained() {
        let (url, _server) = mock(403, r#"{"message":"API rate limit exceeded"}"#);
        let e = fetch_latest(&url).unwrap_err().to_string();
        assert!(e.contains("try again later"), "{e}");
        let (url, _server) = mock(200, "not json");
        assert!(fetch_latest(&url).is_err());
    }
}

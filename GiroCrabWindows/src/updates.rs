//! Update check against the GitHub releases (port of `GiroCrabApp/.../AppUpdates.swift`).
//! Pure logic behind a `Fetcher` seam, so the tests run offline. Windows has no auto-install:
//! a newer release only offers its installer (or release page) in the browser.

use serde::Deserialize;

pub const RELEASES_URL: &str =
    "https://api.github.com/repos/GiroCrab/GiroCrab/releases?per_page=50";
pub const CHANNEL_KEY: &str = "updates.channel";
pub const AUTO_KEY: &str = "updates.auto";
pub const LAST_KEY: &str = "updates.last_check";
const NOTES_MAX: usize = 600;
const PER_PAGE: usize = 50;
/// Upper bound of requests per check (250 releases).
const MAX_PAGES: usize = 5;

/// Release kind, ordered by maturity; a channel accepts every kind `>=` itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Channel {
    Nightly,
    Beta,
    Stable,
}

impl Channel {
    /// Menu order of the settings: Stabil, Beta, Nightly.
    pub fn from_index(i: i32) -> Channel {
        match i {
            1 => Channel::Beta,
            2 => Channel::Nightly,
            _ => Channel::Stable,
        }
    }
    pub fn index(self) -> i32 {
        match self {
            Channel::Stable => 0,
            Channel::Beta => 1,
            Channel::Nightly => 2,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
            Channel::Nightly => "nightly",
        }
    }
}

/// Stored channel, else the channel of the running build (like the Mac app).
pub fn channel_setting(stored: Option<&str>, current: &str) -> Channel {
    match stored {
        Some("stable") => Channel::Stable,
        Some("beta") => Channel::Beta,
        Some("nightly") => Channel::Nightly,
        _ => Version::parse(current).map_or(Channel::Stable, |v| v.kind),
    }
}

/// Off unless switched on: no network access the user did not ask for.
pub fn auto_enabled(stored: Option<&str>) -> bool {
    stored == Some("1")
}

/// `X.Y.Z`, `X.Y.Z-beta.N`, `X.Y.Z-nightly.YYYYMMDD` (optional leading `v`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    core: (u64, u64, u64),
    kind: Channel,
    pre: u64,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (s, None),
        };
        let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
        let core = (it.next()??, it.next()??, it.next()??);
        if it.next().is_some() {
            return None;
        }
        let (kind, pre) = match pre {
            None => (Channel::Stable, 0),
            Some(p) => {
                let (name, n) = p.split_once('.')?;
                let n = n.parse().ok()?;
                match name {
                    "beta" => (Channel::Beta, n),
                    "nightly" => (Channel::Nightly, n),
                    _ => return None,
                }
            }
        };
        Some(Version { core, kind, pre })
    }
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    pub notes: String,
    /// Installer or release page; empty if neither is on an allowed host.
    pub url: String,
}

/// Only `https://api.github.com/…` and `https://github.com/…`, host matched exactly.
pub fn allowed_host(url: &str) -> bool {
    let Ok(url) = crate::sync_flow::safe_https_url(url) else {
        return false;
    };
    let host = url["https://".len()..]
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    matches!(host, "api.github.com" | "github.com")
}

fn download_url(assets: &[RawAsset]) -> Option<String> {
    let pick = |ext: &str| {
        assets.iter().find(|a| {
            a.name.to_ascii_lowercase().ends_with(ext)
                && a.browser_download_url.starts_with("https://github.com/")
                && allowed_host(&a.browser_download_url)
        })
    };
    pick(".msi")
        .or_else(|| pick(".exe"))
        .map(|a| a.browser_download_url.clone())
}

fn notes(body: &str) -> String {
    let body = body.trim();
    match body.char_indices().nth(NOTES_MAX) {
        Some((i, _)) => format!("{} …", &body[..i]),
        None => body.to_string(),
    }
}

/// Newest release on `channel` that is newer than `current`.
pub fn newest(json: &[u8], current: &str, channel: Channel) -> Result<Option<Update>, String> {
    let list: Vec<RawRelease> =
        serde_json::from_slice(json).map_err(|_| "Antwort von GitHub nicht lesbar.".to_string())?;
    let current =
        Version::parse(current).ok_or_else(|| "Eigene Version nicht lesbar.".to_string())?;
    Ok(list
        .into_iter()
        .filter(|r| !r.draft)
        .filter_map(|r| Version::parse(&r.tag_name).map(|v| (v, r)))
        .filter(|(v, _)| v.kind >= channel && *v > current)
        .max_by_key(|(v, _)| *v)
        .map(|(_, r)| Update {
            version: r.tag_name.trim_start_matches(['v', 'V']).to_string(),
            notes: notes(r.body.as_deref().unwrap_or("")),
            url: download_url(&r.assets)
                .or_else(|| Some(r.html_url).filter(|u| allowed_host(u)))
                .unwrap_or_default(),
        }))
}

pub trait Fetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// The system `curl` (Windows 10+, macOS, Linux): no HTTP crate needed.
pub struct CurlFetcher;

impl Fetcher for CurlFetcher {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        if !allowed_host(url) {
            return Err("Adresse nicht erlaubt.".into());
        }
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["--proto", "=https", "--tlsv1.2", "--fail", "--silent"])
            .args(["--show-error", "--max-time", "20", "-A", "GiroCrab-Windows"])
            .args(["-H", "Accept: application/vnd.github+json", url])
            .stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: no console flashing up behind the GUI
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("curl nicht verfügbar: {e}"))?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(format!(
                "Abfrage fehlgeschlagen: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

/// Follows the release pages (newest first) until one holds an update for `channel`, so many
/// newer prereleases cannot push the newest stable release out of the first page.
pub fn check(f: &impl Fetcher, current: &str, channel: Channel) -> Result<Option<Update>, String> {
    for page in 1..=MAX_PAGES {
        let url = if page == 1 {
            RELEASES_URL.to_string()
        } else {
            format!("{RELEASES_URL}&page={page}")
        };
        let json = f.get(&url)?;
        if let Some(u) = newest(&json, current, channel)? {
            return Ok(Some(u));
        }
        let len =
            serde_json::from_slice::<Vec<serde::de::IgnoredAny>>(&json).map_or(0, |l| l.len());
        if len < PER_PAGE {
            break; // last page
        }
    }
    Ok(None)
}

/// Opens the installer / release page; only github.com is handed to the browser.
pub fn open_download(url: &str) -> Result<(), String> {
    if !url.starts_with("https://github.com/") || !allowed_host(url) {
        return Err("Download-Adresse nicht erlaubt.".into());
    }
    crate::sync_flow::open_in_browser(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const JSON: &str = r#"[
      {"tag_name":"v1.0.0","draft":true,"html_url":"https://github.com/GiroCrab/GiroCrab/releases/tag/v1.0.0"},
      {"tag_name":"v0.9.0-nightly.20261121","prerelease":true,"html_url":"https://github.com/GiroCrab/GiroCrab/releases/tag/n"},
      {"tag_name":"v0.8.9-beta.1","prerelease":true,"body":"Beta","html_url":"https://github.com/GiroCrab/GiroCrab/releases/tag/b"},
      {"tag_name":"v0.8.8","body":"Stabil","html_url":"https://github.com/GiroCrab/GiroCrab/releases/tag/v0.8.8","assets":[
        {"name":"GiroCrab-0.8.8.dmg","browser_download_url":"https://github.com/GiroCrab/GiroCrab/releases/download/v0.8.8/a.dmg"},
        {"name":"girocrab-windows-x86_64-setup.exe","browser_download_url":"https://github.com/GiroCrab/GiroCrab/releases/download/v0.8.8/setup.exe"}]},
      {"tag_name":"nightly","prerelease":true,"html_url":"https://github.com/GiroCrab/GiroCrab/releases/tag/nightly"}
    ]"#;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn versions_order_stable_over_beta_over_nightly() {
        assert!(v("0.8.8") > v("0.8.8-beta.2"));
        assert!(v("0.8.8-beta.2") > v("0.8.8-beta.1"));
        assert!(v("0.8.8-beta.1") > v("0.8.8-nightly.20261122"));
        assert!(v("0.8.8-nightly.20261122") > v("0.8.8-nightly.20261121"));
        assert!(v("0.8.10") > v("v0.8.9"));
        assert!(v("0.9.0-nightly.20260101") > v("0.8.99"));
        // nightly.yml tags last release + 1 patch: newer than a stable or beta
        // baseline, older than the next beta/stable of that version.
        assert!(v("0.9.1-nightly.20261125") > v("0.9.0"));
        assert!(v("0.9.1-nightly.20261125") > v("0.9.0-beta.2"));
        assert!(v("0.9.1-beta.1") > v("0.9.1-nightly.20261125"));
        assert!(v("0.9.1") > v("0.9.1-nightly.20261125"));
        for bad in [
            "nightly",
            "0.8",
            "0.8.8.1",
            "0.8.8-rc.1",
            "0.8.8-beta",
            "x.8.8",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn channel_filters_releases() {
        let up = |cur, ch| newest(JSON.as_bytes(), cur, ch).unwrap().map(|u| u.version);
        assert_eq!(up("0.8.7", Channel::Stable).as_deref(), Some("0.8.8"));
        assert_eq!(up("0.8.7", Channel::Beta).as_deref(), Some("0.8.9-beta.1"));
        assert_eq!(
            up("0.8.7", Channel::Nightly).as_deref(),
            Some("0.9.0-nightly.20261121")
        );
        assert_eq!(up("0.8.8", Channel::Stable), None, "draft 1.0.0 ignored");
        assert_eq!(up("0.9.0", Channel::Nightly), None);
        assert!(newest(b"{oops", "0.8.7", Channel::Stable).is_err());
    }

    #[test]
    fn channel_defaults_to_running_build() {
        assert_eq!(channel_setting(None, "0.8.8"), Channel::Stable);
        assert_eq!(channel_setting(None, "0.8.9-beta.1"), Channel::Beta);
        assert_eq!(
            channel_setting(None, "0.9.0-nightly.20261121"),
            Channel::Nightly
        );
        assert_eq!(channel_setting(Some("beta"), "0.8.8"), Channel::Beta);
        assert!(!auto_enabled(None) && auto_enabled(Some("1")));
    }

    #[test]
    fn host_check_is_exact() {
        assert!(allowed_host(
            "https://api.github.com/repos/GiroCrab/GiroCrab/releases"
        ));
        assert!(allowed_host(
            "https://github.com/GiroCrab/GiroCrab/releases/tag/v0.8.8"
        ));
        for bad in [
            "http://github.com/x",
            "https://github.com.evil.example/x",
            "https://evil.example/github.com/",
            "https://user@github.com/x",
            "https://github.com:8443/x",
            "https://github.com/a b",
            "file:///C:/x",
        ] {
            assert!(!allowed_host(bad), "{bad}");
        }
        assert!(CurlFetcher.get("https://evil.example/").is_err());
        assert!(open_download("https://api.github.com/x").is_err());
    }

    #[test]
    fn asset_prefers_msi_then_exe_then_release_page() {
        let a = |n: &str, u: &str| RawAsset {
            name: n.into(),
            browser_download_url: u.into(),
        };
        let gh = "https://github.com/GiroCrab/GiroCrab/releases/download/v1/";
        let list = [
            a("x.dmg", &format!("{gh}x.dmg")),
            a("setup.exe", &format!("{gh}setup.exe")),
            a("GiroCrab.MSI", &format!("{gh}g.msi")),
        ];
        assert_eq!(download_url(&list), Some(format!("{gh}g.msi")));
        assert_eq!(download_url(&list[..2]), Some(format!("{gh}setup.exe")));
        assert_eq!(
            download_url(&[a("s.exe", "https://evil.example/s.exe")]),
            None
        );
        let u = newest(JSON.as_bytes(), "0.8.7", Channel::Stable)
            .unwrap()
            .unwrap();
        assert!(u.url.ends_with("/setup.exe") && u.notes == "Stabil");
        let u = newest(JSON.as_bytes(), "0.8.7", Channel::Beta)
            .unwrap()
            .unwrap();
        assert!(u.url.ends_with("/releases/tag/b"));
    }

    struct Fake(RefCell<Vec<String>>);
    impl Fetcher for Fake {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.0.borrow_mut().push(url.into());
            Ok(JSON.as_bytes().to_vec())
        }
    }

    #[test]
    fn check_asks_the_releases_api_only() {
        let f = Fake(RefCell::default());
        let u = check(&f, "0.8.7", Channel::Stable).unwrap();
        assert_eq!(u.map(|u| u.version).as_deref(), Some("0.8.8"));
        assert_eq!(*f.0.borrow(), vec![RELEASES_URL.to_string()]);
        assert!(allowed_host(RELEASES_URL));
    }

    /// 50 newer nightlies on page 1, the newest stable release only on page 2.
    struct Paged(RefCell<Vec<String>>);
    impl Fetcher for Paged {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.0.borrow_mut().push(url.into());
            let tag = |n: usize| {
                format!(
                    r#"{{"tag_name":"v0.9.0-nightly.{}","html_url":"https://github.com/x"}}"#,
                    20261000 + n
                )
            };
            let page = if url.ends_with("&page=2") {
                r#"[{"tag_name":"v0.8.8","html_url":"https://github.com/x"}]"#.to_string()
            } else {
                format!("[{}]", (0..PER_PAGE).map(tag).collect::<Vec<_>>().join(","))
            };
            Ok(page.into_bytes())
        }
    }

    #[test]
    fn check_follows_pages_for_stable_behind_many_prereleases() {
        let f = Paged(RefCell::default());
        let u = check(&f, "0.8.7", Channel::Stable).unwrap();
        assert_eq!(u.map(|u| u.version).as_deref(), Some("0.8.8"));
        assert_eq!(
            *f.0.borrow(),
            vec![RELEASES_URL.to_string(), format!("{RELEASES_URL}&page=2")]
        );
        // nothing newer anywhere: stops at a short page, never beyond MAX_PAGES
        let f = Paged(RefCell::default());
        assert_eq!(check(&f, "0.8.8", Channel::Stable).unwrap(), None);
        assert_eq!(f.0.borrow().len(), 2);
    }
}

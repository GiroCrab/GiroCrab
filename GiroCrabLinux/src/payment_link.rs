//! `girocrab://transfer?name=…&iban=…&bic=…&amount=…&purpose=…` (PaymentLink.swift,
//! docs/payment-link.md): another app or a web page asks GiroCrab to open a transfer form
//! with these fields. The link only fills the form; nothing is ever sent without the
//! user's approval in the sheet (AppModel.qml › `openLink`).
//!
//! The desktop starts `girocrab %u` for each link. A running instance takes it over a
//! local socket (like the Mac, which hands the link to the open app): [`forward`] in the
//! new process, [`listen`] in the running one. The link is untrusted input and never logged.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

pub const SCHEME: &str = "girocrab";
pub const TRANSFER_HOST: &str = "transfer";
/// Longest link taken at all (Linux only: argv and socket input are bounded before parsing).
pub const MAX_LINK_BYTES: usize = 4096;

#[derive(Debug, PartialEq)]
pub enum Failure {
    /// Not a `girocrab://transfer` link.
    Unsupported,
    /// SEPA transfers are in euro only.
    Currency(String),
    /// Without an IBAN there is no recipient to prefill.
    MissingIban,
}

impl Failure {
    pub fn message(&self) -> String {
        match self {
            Failure::Unsupported => {
                crate::i18n::tr("GiroCrab kann mit diesem Link nichts anfangen.")
            }
            Failure::Currency(code) => crate::i18n::tr(
                "Der Link verlangt eine Zahlung in %1. Überweisungen sind nur in Euro möglich.",
            )
            .replace("%1", code),
            Failure::MissingIban => crate::i18n::tr("Der Link enthält keine IBAN."),
        }
    }
}

/// `ParsedPayment` of a link; the amount in cents (the form formats it, `Fmt.plain`).
#[derive(Debug, PartialEq, serde::Serialize)]
pub struct Payment {
    pub name: Option<String>,
    pub iban: String,
    pub bic: Option<String>,
    pub amount_cents: Option<i64>,
    pub purpose: Option<String>,
}

/// Whether `url` is meant for this parser (other `girocrab://` links may follow later).
pub fn handles(url: &str) -> bool {
    url.split_once(':')
        .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case(SCHEME))
}

/// Percent-decoding as `URLComponents.queryItems` does it (`+` stays `+`). None: a broken
/// escape or no UTF-8 (Swift's `URL(string:)` refuses such a link).
fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b.get(i + 1..i + 3)?;
            if !hex.iter().all(u8::is_ascii_hexdigit) {
                return None;
            }
            out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The payment of a transfer link. Texts are trimmed and single-line, the name capped at
/// 70 and the purpose at 140 characters (SEPA); the amount accepts `12.50` and `12,50`.
pub fn payment(url: &str) -> Result<Payment, Failure> {
    // An URL is printable ASCII (RFC 3986); anything else, or an oversized link, is refused.
    if url.len() > MAX_LINK_BYTES || !url.bytes().all(|c| c.is_ascii_graphic()) {
        return Err(Failure::Unsupported);
    }
    let url = url.split_once('#').map_or(url, |(u, _)| u);
    let rest = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case(SCHEME) => rest,
        _ => return Err(Failure::Unsupported),
    };
    let Some((location, query)) = rest.split_once('?') else {
        return Err(Failure::Unsupported);
    };
    let host = location.split_once('/').map_or(location, |(h, _)| h);
    if !host.eq_ignore_ascii_case(TRANSFER_HOST) {
        return Err(Failure::Unsupported);
    }
    let mut items = Vec::new();
    for item in query.split('&') {
        let (name, value) = item.split_once('=').unwrap_or((item, ""));
        let (Some(name), Some(value)) = (decode(name), decode(value)) else {
            return Err(Failure::Unsupported);
        };
        items.push((name.to_lowercase(), value));
    }
    let value = |key: &str| -> Option<String> {
        let raw = items.iter().find(|(n, _)| n == key).map_or("", |(_, v)| v);
        // Line breaks (and, on Linux, every other control character) become spaces.
        let line: String = raw
            .chars()
            .map(|c| {
                if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                    ' '
                } else {
                    c
                }
            })
            .collect();
        let line = line.trim();
        (!line.is_empty()).then(|| line.to_string())
    };
    let compact = |s: String| -> String {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_uppercase()
    };
    if let Some(currency) = value("currency")
        && currency.to_uppercase() != "EUR"
    {
        return Err(Failure::Currency(currency.to_uppercase()));
    }
    let iban = value("iban").map(compact).unwrap_or_default();
    if iban.is_empty() {
        return Err(Failure::MissingIban);
    }
    Ok(Payment {
        name: value("name").map(|s| s.chars().take(70).collect()),
        iban,
        bic: value("bic").map(compact),
        amount_cents: value("amount")
            .and_then(girocrab_core::payment_input::parse_amount_loose)
            .filter(|&c| c > 0),
        purpose: value("purpose").map(|s| s.chars().take(140).collect()),
    })
}

// MARK: hand-over to the running instance

/// Private to the user: `$XDG_RUNTIME_DIR` (0700), in the Flatpak the app's own part of it.
fn socket_path() -> Option<PathBuf> {
    let run = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if !run.is_absolute() {
        return None;
    }
    Some(match std::env::var("FLATPAK_ID") {
        Ok(id) if !id.is_empty() && !id.contains('/') => run.join("app").join(id).join("link.sock"),
        _ => run.join("girocrab/link.sock"),
    })
}

/// The socket this instance listens on (removed again by [`stop`]).
static LISTENING: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Hands `url` to a running instance. False: none runs (this process opens the link itself).
pub fn forward(url: &str) -> bool {
    if url.len() > MAX_LINK_BYTES {
        return false;
    }
    let Some(path) = socket_path() else {
        return false;
    };
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    stream.write_all(url.as_bytes()).is_ok() && stream.shutdown(std::net::Shutdown::Write).is_ok()
}

/// Takes links of later starts (`forward`) and hands each one to `on_link`, on a thread
/// of its own. Does nothing when another instance already listens or there is no runtime dir.
pub fn listen(mut on_link: impl FnMut(String) + Send + 'static) {
    use std::os::unix::fs::PermissionsExt;
    let mut listening = LISTENING.lock().unwrap_or_else(|e| e.into_inner());
    if listening.is_some() {
        return;
    }
    let Some(path) = socket_path() else { return };
    if let Some(dir) = path.parent()
        && !dir.exists()
    {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        // Another instance answers there: it takes the links. Else a crash left the file.
        Err(_) if UnixStream::connect(&path).is_ok() => return,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(_) => return,
            }
        }
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    *listening = Some(path);
    std::thread::spawn(move || {
        let mut failures: u32 = 0;
        for stream in listener.incoming() {
            // A persistent error (EMFILE …) would spin: wait longer after each one in a row.
            let Ok(stream) = stream else {
                failures = failures.saturating_add(1);
                std::thread::sleep(accept_backoff(failures));
                continue;
            };
            failures = 0;
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = Vec::new();
            if stream
                .take(MAX_LINK_BYTES as u64 + 1)
                .read_to_end(&mut buf)
                .is_err()
                || buf.len() > MAX_LINK_BYTES
            {
                continue;
            }
            if let Ok(url) = String::from_utf8(buf)
                && handles(&url)
            {
                on_link(url);
            }
        }
    });
}

/// Pause after `failures` accept errors in a row: 50 ms, doubling, at most 5 s.
fn accept_backoff(failures: u32) -> Duration {
    Duration::from_millis(
        50u64
            .saturating_mul(1 << failures.saturating_sub(1).min(7))
            .min(5000),
    )
}

/// Quitting: the socket file goes (a file left by a crash is replaced by the next `listen`).
pub fn stop() {
    if let Some(path) = LISTENING.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // PaymentLinkTests.swift

    #[test]
    fn parses_all_fields() {
        let p = payment("girocrab://transfer?name=Stadtwerke%20M%C3%BCnchen&iban=de89%203704%200044%200532%200130%2000&bic=cobadeffxxx&amount=85,5&purpose=Abschlag%0AStrom").unwrap();
        assert_eq!(p.name.as_deref(), Some("Stadtwerke München"));
        assert_eq!(p.iban, "DE89370400440532013000");
        assert_eq!(p.bic.as_deref(), Some("COBADEFFXXX"));
        assert_eq!(p.amount_cents, Some(8550));
        assert_eq!(p.purpose.as_deref(), Some("Abschlag Strom"), "single line");
    }

    #[test]
    fn dot_amount_and_limits() {
        let long = "x".repeat(200);
        let p = payment(&format!(
            "girocrab://transfer?iban=DE89370400440532013000&amount=12.50&name={long}&purpose={long}"
        ))
        .unwrap();
        assert_eq!(p.amount_cents, Some(1250));
        assert_eq!(p.name.unwrap().chars().count(), 70);
        assert_eq!(p.purpose.unwrap().chars().count(), 140);
    }

    #[test]
    fn optional_fields_and_bad_amounts_stay_empty() {
        let p =
            payment("girocrab://transfer?iban=DE89370400440532013000&amount=-5&name=%20&foo=bar")
                .unwrap();
        assert_eq!(p.amount_cents, None);
        assert_eq!(p.name, None);
        assert_eq!(p.purpose, None);
        let p = payment("girocrab://transfer?iban=DE89370400440532013000&amount=abc").unwrap();
        assert_eq!(p.amount_cents, None);
    }

    #[test]
    fn refuses_what_is_no_transfer() {
        assert_eq!(
            payment("girocrab://transfer?name=x"),
            Err(Failure::MissingIban)
        );
        assert_eq!(
            payment("girocrab://transfer?iban=DE89370400440532013000&currency=usd"),
            Err(Failure::Currency("USD".into()))
        );
        assert_eq!(
            payment("girocrab://other?iban=DE89370400440532013000"),
            Err(Failure::Unsupported)
        );
        assert_eq!(
            payment("https://transfer?iban=DE89370400440532013000"),
            Err(Failure::Unsupported)
        );
        assert!(payment("GIROCRAB://Transfer?IBAN=DE89370400440532013000&currency=EUR").is_ok());
    }

    // Linux: argv and the socket hand over raw text

    #[test]
    fn refuses_malformed_or_oversized_links() {
        let ok = "girocrab://transfer?iban=DE89370400440532013000";
        assert!(payment(ok).is_ok());
        for bad in [
            "girocrab:transfer?iban=DE89370400440532013000",
            "girocrab:///transfer?iban=DE89370400440532013000",
            "girocrab://transfer",
            "girocrab://transfer?iban=DE89%ZZ",
            "girocrab://transfer?iban=DE89%+1",
            "girocrab://transfer?iban=%FF%FE",
            "girocrab://transfer?iban=DE89 3704",
            "girocrab://transfer?iban=DE89\n3704",
            "girocrab://transfer?iban=DE89370400440532013000&name=München",
        ] {
            assert_eq!(payment(bad), Err(Failure::Unsupported), "{bad:?}");
        }
        let long = format!("{ok}&purpose={}", "x".repeat(MAX_LINK_BYTES));
        assert_eq!(payment(&long), Err(Failure::Unsupported));
    }

    #[test]
    fn control_characters_become_spaces_and_plus_stays() {
        let p = payment(
            "girocrab://transfer?iban=DE89370400440532013000&name=A%09B%E2%80%A8C&purpose=1+2#frag",
        )
        .unwrap();
        assert_eq!(p.name.as_deref(), Some("A B C"));
        assert_eq!(p.purpose.as_deref(), Some("1+2"));
    }

    #[test]
    fn accept_errors_back_off_up_to_five_seconds() {
        assert_eq!(accept_backoff(1), Duration::from_millis(50));
        assert_eq!(accept_backoff(2), Duration::from_millis(100));
        assert_eq!(accept_backoff(7), Duration::from_millis(3200));
        assert_eq!(accept_backoff(8), Duration::from_secs(5));
        assert_eq!(accept_backoff(u32::MAX), Duration::from_secs(5));
    }

    #[test]
    fn handles_only_the_own_scheme() {
        assert!(handles("GiroCrab://transfer"));
        assert!(!handles("https://girocrab.de"));
        assert!(!handles("--demo-seed"));
    }
}

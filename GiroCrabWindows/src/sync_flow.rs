//! Bank sync flow of the Windows app, independent of Slint so it can be unit-tested.
//!
//! `sync_connection` asks for the PIN (FinTS, comdirect, Trade Republic) only when the connector
//! needs one, hands it to the core and keeps no copy; `drive` then runs the multi-step TAN
//! dialog (code, photoTAN, decoupled push, PSD2 redirect) against a `Stepper`.
//! PIN and TAN live only in memory (`Zeroizing`), are never logged and never written to disk.

use std::time::{Duration, Instant};

use girocrab_core::connectors::SyncSummary;
use girocrab_core::{Challenge, ChallengeKind, ConnectorKind, SyncStep};
use zeroize::Zeroizing;

/// Upper bound for one bank dialog (push approval, TAN entry).
const DIALOG_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// What the app has to ask the user before a connector can sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    /// PIN / password first, then the bank's TAN challenges.
    Secret,
    /// No secret up front (PSD2 aggregation): the bank page in the browser authorises.
    Browser,
    /// No online access: the user picks an export file that the core imports (Finpension, #975).
    FileImport,
    /// API username, password and signature (classic PayPal NVP API, #945). They live in the
    /// vault's settings, which the core reads itself; `ensure_api_signature` asks when one is missing.
    ApiSignature,
    /// Needs an API key or similar that the Windows app cannot collect yet.
    Unsupported,
}

pub fn auth_for(kind: ConnectorKind) -> Auth {
    match kind {
        ConnectorKind::VolksbankKasselGoettingen
        | ConnectorKind::FinTsGeneric
        | ConnectorKind::Direkt1822
        | ConnectorKind::Comdirect
        | ConnectorKind::TradeRepublic => Auth::Secret,
        ConnectorKind::AmericanExpress
        | ConnectorKind::AmazonVisa
        | ConnectorKind::EnableBanking => Auth::Browser,
        ConnectorKind::Finpension => Auth::FileImport,
        ConnectorKind::PayPal => Auth::ApiSignature,
        _ => Auth::Unsupported,
    }
}

/// The user interface as the flow sees it. `None` / `false` = the user cancelled.
pub trait Ui {
    fn ask_secret(&self, bank: &str) -> Option<Zeroizing<String>>;
    /// TAN / code (also photoTAN, with `challenge.image`, and the PSD2 redirect result).
    fn ask_code(&self, challenge: &Challenge) -> Option<Zeroizing<String>>;
    /// Shows the push approval prompt and waits `interval`; `false` = cancelled.
    fn wait_push(&self, challenge: &Challenge, interval: Duration) -> bool;
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// The three PayPal API signature fields, entered masked. Only the PayPal sync asks; the
    /// default (= cancelled) serves front ends that never do, e.g. the transfer flow.
    fn ask_api_signature(&self, _bank: &str) -> Option<ApiSignature> {
        None
    }
}

/// Setting keys of the PayPal API signature, as the core catalog (`KNOWN_SETTINGS`) lists them.
pub const API_SIGNATURE_KEYS: [&str; 3] = [
    "paypal.api_username",
    "paypal.api_password",
    "paypal.api_signature",
];
const API_SIGNATURE_LABELS: [&str; 3] = ["API-Benutzername", "API-Passwort", "Signatur"];
const PAYPAL_REST_KEYS: [&str; 2] = ["paypal.client_id", "paypal.client_secret"];

/// PayPal API credentials. No `Debug`: the values must never reach a log or an error text.
pub struct ApiSignature {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
    pub signature: Zeroizing<String>,
}

impl ApiSignature {
    fn values(&self) -> [&Zeroizing<String>; 3] {
        [&self.username, &self.password, &self.signature]
    }

    /// Rejects an empty field. The message names the field, never a value.
    pub fn validate(&self) -> Result<(), String> {
        for (value, label) in self.values().into_iter().zip(API_SIGNATURE_LABELS) {
            if value.trim().is_empty() {
                return Err(format!("PayPal: Bitte das Feld „{label}“ ausfüllen."));
            }
        }
        Ok(())
    }
}

/// Where the API signature is kept (production: the vault's encrypted settings).
pub trait SignatureStore {
    /// Keys of the settings that hold a value.
    fn stored_keys(&self) -> Result<Vec<String>, String>;
    fn store(&self, key: &str, value: &str) -> Result<(), String>;
}

/// Like the Mac app's `missingSettings`: once one signature field is set, all three are needed;
/// with none set, a complete REST app (client ID + secret) syncs without a prompt.
pub fn needs_api_signature(stored: &[String]) -> bool {
    let has = |key: &str| stored.iter().any(|s| s == key);
    let set = API_SIGNATURE_KEYS.into_iter().filter(|k| has(k)).count();
    set < API_SIGNATURE_KEYS.len() && (set > 0 || !PAYPAL_REST_KEYS.into_iter().all(has))
}

/// Makes sure the API signature is stored before a PayPal sync: asks once when a field is
/// missing, rejects empty fields (an empty value would delete the stored one), then saves all three.
pub fn ensure_api_signature<U: Ui, T: SignatureStore + ?Sized>(
    bank: &str,
    ui: &U,
    store: &T,
) -> Result<(), String> {
    if !needs_api_signature(&store.stored_keys()?) {
        return Ok(());
    }
    let sig = ui
        .ask_api_signature(bank)
        .ok_or_else(|| CANCELLED.to_string())?;
    sig.validate()?;
    for (key, value) in API_SIGNATURE_KEYS.into_iter().zip(sig.values()) {
        store.store(key, value.trim())?;
    }
    Ok(())
}

/// The running bank session (`SyncSession` in production, a fake in tests).
pub trait Stepper {
    fn start(&self) -> Result<SyncStep, String>;
    fn submit_code(&self, code: String) -> Result<SyncStep, String>;
    fn poll(&self) -> Result<SyncStep, String>;
    fn cancel(&self);
}

pub const CANCELLED: &str = "Abruf abgebrochen.";

/// Shown instead of a core message that might quote the file.
pub const INVALID_EXPORT: &str = "Die Datei ist kein gültiger Finpension-Export (CSV oder JSON).";

/// Second sentence after "<Bank> hinzugefügt.": file-import banks are fed with an export file.
pub fn added_hint(file_import: bool) -> &'static str {
    if file_import {
        "Mit „Jetzt abrufen“ wählen Sie die Exportdatei (CSV oder JSON) und importieren sie."
    } else {
        "Mit „Jetzt abrufen“ starten Sie den ersten Abruf."
    }
}

/// Reads an export file. The message never names the path or quotes the content.
pub fn read_export(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("Die Datei konnte nicht gelesen werden: {e}"))?;
    if bytes.is_empty() {
        return Err("Die Datei ist leer.".into());
    }
    Ok(bytes)
}

/// Core import error → user text. Keeps only the last `:`-separated part of the first line (the
/// reason after the core's category prefixes, e.g. "Ungültige Eingabe: Finpension: …") and drops it
/// entirely if it looks like file content (quotes, separators, JSON, no letters at all, e.g. a bare
/// number) or is overlong.
pub fn import_error_text(core: &str) -> String {
    let head = core
        .lines()
        .next()
        .unwrap_or_default()
        .rsplit(':')
        .next()
        .unwrap_or_default()
        .trim();
    if head.is_empty()
        || head.chars().count() > 120
        || head.contains(['"', '\'', ';', '{', '['])
        || !head.chars().any(char::is_alphabetic)
        || head.split_whitespace().any(looks_like_isin)
    {
        return INVALID_EXPORT.into();
    }
    format!("Finpension-Import: {head}")
}

/// Fallback guard: a word shaped like an ISIN (2 letters, 9 letters/digits, 1 check digit) is file
/// content and never shown, whatever message the core builds.
fn looks_like_isin(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let b = w.as_bytes();
    b.len() == 12
        && b[..2].iter().all(u8::is_ascii_uppercase)
        && b[2..11]
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && b[11].is_ascii_digit()
}

/// Asks for the secret if needed, opens the session with it and drives it to the end.
pub fn sync_connection<S: Stepper, U: Ui>(
    kind: ConnectorKind,
    bank: &str,
    ui: &U,
    begin: impl FnOnce(String) -> Result<S, String>,
) -> Result<SyncSummary, String> {
    let secret = match auth_for(kind) {
        Auth::Secret => match ui.ask_secret(bank) {
            Some(s) if !s.trim().is_empty() => s,
            Some(_) => return Err("Bitte die PIN eingeben.".into()),
            None => return Err(CANCELLED.into()),
        },
        // PayPal's API signature is read by the core from the settings (`ensure_api_signature`).
        Auth::Browser | Auth::ApiSignature => Zeroizing::new(String::new()),
        Auth::FileImport => {
            return Err(format!(
                "{bank}: Dieser Zugang wird per Exportdatei importiert."
            ));
        }
        Auth::Unsupported => {
            return Err(format!(
                "{bank}: Dieser Zugang wird von der Windows-App noch nicht unterstützt."
            ));
        }
    };
    // Move the PIN out: the core wipes its copy when the session ends, ours is left empty.
    let mut secret = secret;
    let session = begin(std::mem::take(&mut *secret))?;
    drive(&session, ui)
}

pub fn drive<S: Stepper, U: Ui>(session: &S, ui: &U) -> Result<SyncSummary, String> {
    let deadline = Instant::now() + DIALOG_TIMEOUT;
    let mut step = session.start()?;
    loop {
        if Instant::now() > deadline {
            session.cancel();
            return Err("Zeitüberschreitung: Der Abruf wurde abgebrochen.".into());
        }
        step = match step {
            SyncStep::Finished { summary } => return Ok(summary),
            SyncStep::StillWaiting { challenge } => wait_and_poll(session, ui, &challenge)?,
            SyncStep::NeedsChallenge { challenge } => match challenge.kind {
                ChallengeKind::DecoupledPush | ChallengeKind::Progress => {
                    wait_and_poll(session, ui, &challenge)?
                }
                ChallengeKind::Code | ChallengeKind::ImageCode => submit(session, ui, &challenge)?,
                ChallengeKind::Redirect => {
                    let url = challenge.redirect_url.as_deref().unwrap_or_default();
                    if let Err(e) = safe_https_url(url).and_then(|u| ui.open_url(u)) {
                        session.cancel();
                        return Err(format!("Die Bank-Seite konnte nicht geöffnet werden: {e}"));
                    }
                    submit(session, ui, &challenge)?
                }
            },
        };
    }
}

fn wait_and_poll<S: Stepper, U: Ui>(
    session: &S,
    ui: &U,
    challenge: &Challenge,
) -> Result<SyncStep, String> {
    let interval = Duration::from_secs(challenge.poll_interval_seconds.clamp(1, 30) as u64);
    if !ui.wait_push(challenge, interval) {
        session.cancel();
        return Err(CANCELLED.into());
    }
    session.poll()
}

fn submit<S: Stepper, U: Ui>(
    session: &S,
    ui: &U,
    challenge: &Challenge,
) -> Result<SyncStep, String> {
    match ui.ask_code(challenge) {
        Some(mut code) if !code.trim().is_empty() => {
            session.submit_code(std::mem::take(&mut *code).trim().to_string())
        }
        _ => {
            session.cancel();
            Err(CANCELLED.into())
        }
    }
}

/// Only plain https URLs are handed to the system: no other scheme, no control characters,
/// whitespace or quotes.
pub fn safe_https_url(url: &str) -> Result<&str, String> {
    let url = url.trim();
    let ok = url.len() > "https://".len()
        && url.len() <= 4096
        && url.starts_with("https://")
        && !url
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c == '"' || c == '\\');
    if ok {
        Ok(url)
    } else {
        Err("ungültige Adresse (nur https erlaubt)".into())
    }
}

/// Opens `url` in the user's default browser.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    let url = safe_https_url(url)?;
    #[cfg(windows)]
    // `rundll32 url.dll,FileProtocolHandler` is what `start` does, but without a cmd.exe
    // command line, where `&` and `%VAR%` in a bank URL would be interpreted.
    let mut cmd = {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    fn challenge(kind: ChallengeKind, url: Option<&str>) -> Challenge {
        Challenge {
            kind,
            title: "TAN".into(),
            message: "Bitte freigeben".into(),
            method_name: "pushTAN".into(),
            redirect_url: url.map(String::from),
            image: None,
            expires_in_seconds: None,
            poll_interval_seconds: 2,
        }
    }

    fn summary() -> SyncSummary {
        SyncSummary {
            accounts_updated: 1,
            transactions_imported: 3,
            transactions_categorized: 0,
            transactions_skipped_archived: 0,
            new_accounts: vec![],
            revoked_accounts: vec![],
            postbox_documents: None,
            warnings: vec![],
            hints: vec![],
            documents_imported: 0,
            security_transactions: 0,
            holdings: 0,
            history_incomplete: false,
            history_remaining: 0,
            history_total: 0,
            documents_off: false,
            approval_skipped: false,
        }
    }

    #[derive(Default)]
    struct FakeSession {
        steps: RefCell<VecDeque<Result<SyncStep, String>>>,
        log: RefCell<Vec<String>>,
    }
    impl FakeSession {
        fn with(steps: Vec<Result<SyncStep, String>>) -> Self {
            Self {
                steps: RefCell::new(steps.into()),
                log: RefCell::default(),
            }
        }
        fn next(&self, what: String) -> Result<SyncStep, String> {
            self.log.borrow_mut().push(what);
            self.steps
                .borrow_mut()
                .pop_front()
                .expect("unexpected call")
        }
    }
    impl Stepper for FakeSession {
        fn start(&self) -> Result<SyncStep, String> {
            self.next("start".into())
        }
        fn submit_code(&self, code: String) -> Result<SyncStep, String> {
            self.next(format!("code:{code}"))
        }
        fn poll(&self) -> Result<SyncStep, String> {
            self.next("poll".into())
        }
        fn cancel(&self) {
            self.log.borrow_mut().push("cancel".into());
        }
    }

    #[derive(Default)]
    struct FakeUi {
        pin: Option<&'static str>,
        codes: RefCell<VecDeque<Option<&'static str>>>,
        push_ok: bool,
        opened: RefCell<Vec<String>>,
        asked_pin: RefCell<u32>,
        signature: Option<[&'static str; 3]>,
        asked_signature: RefCell<u32>,
    }
    impl Ui for FakeUi {
        fn ask_secret(&self, _bank: &str) -> Option<Zeroizing<String>> {
            *self.asked_pin.borrow_mut() += 1;
            self.pin.map(|p| Zeroizing::new(p.to_string()))
        }
        fn ask_code(&self, _c: &Challenge) -> Option<Zeroizing<String>> {
            self.codes
                .borrow_mut()
                .pop_front()
                .flatten()
                .map(|c| Zeroizing::new(c.to_string()))
        }
        fn wait_push(&self, _c: &Challenge, _i: Duration) -> bool {
            self.push_ok
        }
        fn open_url(&self, url: &str) -> Result<(), String> {
            self.opened.borrow_mut().push(url.into());
            Ok(())
        }
        fn ask_api_signature(&self, _bank: &str) -> Option<ApiSignature> {
            *self.asked_signature.borrow_mut() += 1;
            self.signature.map(|[u, p, s]| ApiSignature {
                username: Zeroizing::new(u.into()),
                password: Zeroizing::new(p.into()),
                signature: Zeroizing::new(s.into()),
            })
        }
    }

    #[derive(Default)]
    struct FakeStore {
        values: RefCell<Vec<(String, String)>>,
    }
    impl SignatureStore for FakeStore {
        fn stored_keys(&self) -> Result<Vec<String>, String> {
            Ok(self
                .values
                .borrow()
                .iter()
                .map(|(k, _)| k.clone())
                .collect())
        }
        fn store(&self, key: &str, value: &str) -> Result<(), String> {
            self.values.borrow_mut().push((key.into(), value.into()));
            Ok(())
        }
    }

    fn need(kind: ChallengeKind, url: Option<&str>) -> Result<SyncStep, String> {
        Ok(SyncStep::NeedsChallenge {
            challenge: challenge(kind, url),
        })
    }
    fn done() -> Result<SyncStep, String> {
        Ok(SyncStep::Finished { summary: summary() })
    }

    #[test]
    fn fints_pin_then_code_tan_finishes() {
        let ui = FakeUi {
            pin: Some("12345"),
            codes: RefCell::new([Some(" 654321 ")].into()),
            ..Default::default()
        };
        let session = FakeSession::with(vec![need(ChallengeKind::Code, None), done()]);
        let mut got_pin = String::new();
        let out = sync_connection(ConnectorKind::FinTsGeneric, "Testbank", &ui, |pin| {
            got_pin = pin;
            Ok(session)
        });
        assert_eq!(out.unwrap().transactions_imported, 3);
        assert_eq!(got_pin, "12345");
    }

    #[test]
    fn tan_is_trimmed_and_submitted_once() {
        let ui = FakeUi {
            codes: RefCell::new([Some(" 654321 ")].into()),
            ..Default::default()
        };
        let session = FakeSession::with(vec![need(ChallengeKind::ImageCode, None), done()]);
        drive(&session, &ui).unwrap();
        assert_eq!(*session.log.borrow(), ["start", "code:654321"]);
    }

    #[test]
    fn cancelled_pin_never_opens_a_session() {
        let ui = FakeUi {
            pin: None,
            ..Default::default()
        };
        let out = sync_connection::<FakeSession, _>(ConnectorKind::Direkt1822, "Bank", &ui, |_| {
            panic!("session must not start without a PIN")
        });
        assert_eq!(out.unwrap_err(), CANCELLED);
    }

    #[test]
    fn empty_pin_is_rejected_before_the_bank_is_contacted() {
        let ui = FakeUi {
            pin: Some("  "),
            ..Default::default()
        };
        let out = sync_connection::<FakeSession, _>(ConnectorKind::Comdirect, "Bank", &ui, |_| {
            panic!("no session for an empty PIN")
        });
        assert!(out.is_err());
    }

    #[test]
    fn cancelled_tan_cancels_the_session() {
        let ui = FakeUi {
            codes: RefCell::new([None].into()),
            ..Default::default()
        };
        let session = FakeSession::with(vec![need(ChallengeKind::Code, None)]);
        assert_eq!(drive(&session, &ui).unwrap_err(), CANCELLED);
        assert_eq!(*session.log.borrow(), ["start", "cancel"]);
    }

    #[test]
    fn decoupled_push_polls_until_approved() {
        let ui = FakeUi {
            push_ok: true,
            ..Default::default()
        };
        let waiting = || {
            Ok(SyncStep::StillWaiting {
                challenge: challenge(ChallengeKind::DecoupledPush, None),
            })
        };
        let session = FakeSession::with(vec![
            need(ChallengeKind::DecoupledPush, None),
            waiting(),
            done(),
        ]);
        drive(&session, &ui).unwrap();
        assert_eq!(*session.log.borrow(), ["start", "poll", "poll"]);
    }

    #[test]
    fn cancelling_the_push_prompt_stops_polling() {
        let ui = FakeUi {
            push_ok: false,
            ..Default::default()
        };
        let session = FakeSession::with(vec![need(ChallengeKind::DecoupledPush, None)]);
        assert_eq!(drive(&session, &ui).unwrap_err(), CANCELLED);
        assert_eq!(*session.log.borrow(), ["start", "cancel"]);
    }

    #[test]
    fn psd2_redirect_opens_browser_then_submits_pasted_url() {
        let url = "https://bank.example/auth?client=1&state=abc";
        let pasted = "https://girocrab.de/callback?code=XYZ&state=abc";
        let ui = FakeUi {
            codes: RefCell::new([Some(pasted)].into()),
            ..Default::default()
        };
        let session = FakeSession::with(vec![need(ChallengeKind::Redirect, Some(url)), done()]);
        let out = sync_connection(ConnectorKind::EnableBanking, "N26", &ui, |pin| {
            assert!(pin.is_empty(), "PSD2 asks no PIN");
            Ok(session)
        });
        assert!(out.is_ok());
        assert_eq!(*ui.opened.borrow(), [url]);
        assert_eq!(*ui.asked_pin.borrow(), 0);
    }

    #[test]
    fn psd2_redirect_rejects_non_https_urls() {
        let ui = FakeUi::default();
        let session = FakeSession::with(vec![need(
            ChallengeKind::Redirect,
            Some("file:///C:/Windows/calc.exe"),
        )]);
        assert!(drive(&session, &ui).is_err());
        assert!(ui.opened.borrow().is_empty());
        assert_eq!(*session.log.borrow(), ["start", "cancel"]);
    }

    #[test]
    fn url_check() {
        assert!(safe_https_url("https://a.example/x?y=1&z=%20").is_ok());
        for bad in [
            "http://a.example",
            "https://",
            "https://a b",
            "https://a\"&calc",
            "javascript:1",
            "",
        ] {
            assert!(safe_https_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn connector_auth_modes() {
        assert_eq!(auth_for(ConnectorKind::FinTsGeneric), Auth::Secret);
        assert_eq!(auth_for(ConnectorKind::EnableBanking), Auth::Browser);
        assert_eq!(auth_for(ConnectorKind::Finpension), Auth::FileImport);
        assert_eq!(auth_for(ConnectorKind::PayPal), Auth::ApiSignature);
    }

    #[test]
    fn empty_api_signature_field_is_rejected_before_anything_is_stored() {
        let ui = FakeUi {
            signature: Some(["nutzer-123", "  ", "sig-geheim-456"]),
            ..Default::default()
        };
        let store = FakeStore::default();
        let err = ensure_api_signature("PayPal", &ui, &store).unwrap_err();
        assert!(err.contains("API-Passwort"), "{err}");
        assert!(!err.contains("nutzer-123") && !err.contains("sig-geheim-456"));
        assert!(store.values.borrow().is_empty());
    }

    #[test]
    fn cancelled_api_signature_stores_nothing() {
        let ui = FakeUi::default();
        let store = FakeStore::default();
        assert_eq!(
            ensure_api_signature("PayPal", &ui, &store).unwrap_err(),
            CANCELLED
        );
        assert!(store.values.borrow().is_empty());
    }

    #[test]
    fn api_signature_is_stored_once_then_paypal_syncs_without_a_secret() {
        let ui = FakeUi {
            signature: Some([" nutzer ", "pw", "sig"]),
            ..Default::default()
        };
        let store = FakeStore::default();
        ensure_api_signature("PayPal", &ui, &store).unwrap();
        ensure_api_signature("PayPal", &ui, &store).unwrap();
        assert_eq!(*ui.asked_signature.borrow(), 1);
        let stored = store.values.borrow().clone();
        assert_eq!(stored[0], ("paypal.api_username".into(), "nutzer".into()));
        assert_eq!(stored.len(), 3);

        let session = FakeSession::with(vec![done()]);
        let out = sync_connection(ConnectorKind::PayPal, "PayPal", &ui, |secret| {
            assert!(secret.is_empty(), "the core reads the signature itself");
            Ok(session)
        });
        assert_eq!(out.unwrap().transactions_imported, 3);
        assert_eq!(*ui.asked_pin.borrow(), 0);
    }

    #[test]
    fn signature_prompt_follows_the_mac_rules() {
        let keys = |k: &[&str]| k.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(needs_api_signature(&[]));
        assert!(!needs_api_signature(&keys(&PAYPAL_REST_KEYS)));
        assert!(!needs_api_signature(&keys(&API_SIGNATURE_KEYS)));
        // One signature field set: signature mode, the other two are still missing.
        assert!(needs_api_signature(&keys(&[
            "paypal.client_id",
            "paypal.client_secret",
            "paypal.api_signature",
        ])));
    }

    #[test]
    fn added_hint_points_file_import_to_the_export() {
        assert!(added_hint(true).contains("Exportdatei"));
        assert!(added_hint(false).contains("ersten Abruf"));
    }

    #[test]
    fn missing_export_file_is_a_read_error_without_path() {
        let path =
            std::env::temp_dir().join(format!("girocrab-975-missing-{}.csv", std::process::id()));
        let err = read_export(&path).unwrap_err();
        assert!(
            err.starts_with("Die Datei konnte nicht gelesen werden"),
            "{err}"
        );
        assert!(!err.contains("girocrab-975"), "{err}");
    }

    #[test]
    fn empty_export_file_is_rejected() {
        let path =
            std::env::temp_dir().join(format!("girocrab-975-empty-{}.json", std::process::id()));
        std::fs::write(&path, b"").unwrap();
        let result = read_export(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(result.unwrap_err(), "Die Datei ist leer.");
    }

    #[test]
    fn core_errors_never_carry_file_content() {
        assert_eq!(
            import_error_text("Keine Finpension-Verbindung."),
            "Finpension-Import: Keine Finpension-Verbindung."
        );
        assert_eq!(
            import_error_text("Ungültiger Export: Zeile 2 \"4711;Kauf\""),
            INVALID_EXPORT
        );
        assert_eq!(
            import_error_text(
                "Ungültige Eingabe: Finpension: Keine Positionen oder Buchungen erkannt."
            ),
            "Finpension-Import: Keine Positionen oder Buchungen erkannt."
        );
        // A bare value from the file after the last colon is never shown.
        assert_eq!(
            import_error_text("Ungültiger Export: Zeile 3: 4711"),
            INVALID_EXPORT
        );
        assert_eq!(
            import_error_text("2026-01-02;Kauf;\"Fonds\""),
            INVALID_EXPORT
        );
        assert_eq!(import_error_text("{\"iban\"} kaputt"), INVALID_EXPORT);
        assert_eq!(import_error_text(""), INVALID_EXPORT);
    }

    #[test]
    fn import_errors_never_carry_an_isin() {
        // The fixed core texts keep their reason.
        assert_eq!(
            import_error_text("Ungültige Eingabe: Finpension: missing name in a holding"),
            "Finpension-Import: missing name in a holding"
        );
        assert_eq!(
            import_error_text("Ungültige Eingabe: Finpension: negative figures in a holding"),
            "Finpension-Import: negative figures in a holding"
        );
        // Fallback: even a message that quotes an ISIN is dropped.
        for core in [
            "Ungültige Eingabe: Finpension: missing name for DE0007164600",
            "Ungültige Eingabe: Finpension: negative figures for US0378331005.",
        ] {
            assert_eq!(import_error_text(core), INVALID_EXPORT, "{core}");
        }
    }

    #[test]
    fn fixed_value_errors_are_shown_without_file_content() {
        // The core's fixed reasons for bad values reach the user unchanged.
        for reason in [
            "unsupported currency",
            "invalid number",
            "invalid date",
            "invalid ISIN in a holding",
        ] {
            assert_eq!(
                import_error_text(&format!("Ungültige Eingabe: Finpension: {reason}")),
                format!("Finpension-Import: {reason}")
            );
        }
        // The old form with the raw value: nothing after the quote's first line may slip through
        // as file content once a value carries a colon and a line break.
        let old = "Ungültige Eingabe: Finpension: unsupported currency 'x:geheim\nrest'";
        let shown = import_error_text(old);
        assert!(!shown.contains("rest") && !shown.contains('\n'), "{shown}");
    }
}

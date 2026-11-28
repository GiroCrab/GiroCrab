use std::fmt;

use thiserror::Error;

use crate::i18n::{tr, trf};

/// Every error that can cross the FFI boundary.
#[derive(Debug, Error, uniffi::Error)]
pub enum CoreError {
    Database {
        message: String,
    },
    WrongKey,
    NotFound {
        what: String,
    },
    InvalidInput {
        message: String,
    },
    MissingSetting {
        key: String,
    },
    Network {
        message: String,
    },
    Bank {
        code: String,
        message: String,
    },
    Authentication {
        message: String,
    },
    Protocol {
        message: String,
    },
    /// Trade Republic: AWS WAF blocks direct logins – the app must open the
    /// in-app browser login (WKWebView) and hand over the session cookies.
    BrowserLoginRequired {
        message: String,
    },
    SessionClosed,
    /// This app version may no longer contact the bank (signed banking
    /// policy, see `banking_gate`). `message` is already localised by the app.
    BankingBlocked {
        message: String,
    },
    /// The vault was written by a newer GiroCrab (schema `vault_version`,
    /// this build knows up to `app_version`). It is left untouched.
    SchemaTooNew {
        vault_version: i64,
        app_version: i64,
    },
}

impl CoreError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        CoreError::InvalidInput {
            message: message.into(),
        }
    }

    pub(crate) fn protocol(message: impl Into<String>) -> Self {
        CoreError::Protocol {
            message: message.into(),
        }
    }
}

impl From<rusqlite::Error> for CoreError {
    fn from(e: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(err, _) = &e
            && err.code == rusqlite::ErrorCode::NotADatabase
        {
            return CoreError::WrongKey;
        }
        CoreError::Database {
            message: e.to_string(),
        }
    }
}

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        // The URL of a failed request can carry session paths or query
        // strings (FNZ): it never ends up in the error text, only the cause.
        // The whole cause chain: "error sending request" alone does not tell a
        // missing server (DNS) from a refused connection or a TLS failure.
        CoreError::Network {
            message: error_chain(&e.without_url()),
        }
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::protocol(format!("JSON: {e}"))
    }
}

impl From<regex::Error> for CoreError {
    fn from(e: regex::Error) -> Self {
        CoreError::invalid(trf("Ungültiger regulärer Ausdruck: {0}", &[&e]))
    }
}

/// Localised (German/English) message, see `crate::i18n`.
impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            CoreError::Database { message } => trf("Datenbankfehler: {0}", &[message]),
            CoreError::WrongKey => tr(
                "Der Tresor konnte nicht entsperrt werden (falscher Schlüssel oder beschädigte Datei)",
            )
            .to_string(),
            CoreError::NotFound { what } => trf("Nicht gefunden: {0}", &[what]),
            CoreError::InvalidInput { message } => trf("Ungültige Eingabe: {0}", &[message]),
            CoreError::MissingSetting { key } => trf(
                "Einstellung fehlt: {0}. Bitte in den Einstellungen eintragen.",
                &[key],
            ),
            CoreError::Network { message } => trf("Netzwerkfehler: {0}", &[message]),
            CoreError::Bank { code, message } => trf(
                "Die Bank hat die Anfrage abgelehnt: {0} {1}",
                &[code, message],
            ),
            CoreError::Authentication { message } => {
                trf("Anmeldung fehlgeschlagen: {0}", &[message])
            }
            CoreError::Protocol { message } => trf("Protokollfehler: {0}", &[message]),
            CoreError::BrowserLoginRequired { message } => {
                trf("Browser-Anmeldung nötig: {0}", &[message])
            }
            CoreError::SessionClosed => tr("Dieser Abruf ist nicht mehr aktiv").to_string(),
            CoreError::BankingBlocked { message } => message.clone(),
            CoreError::SchemaTooNew { .. } => tr(
                "Dieser Tresor wurde mit einer neueren GiroCrab-Version gespeichert. Bitte die App aktualisieren.",
            )
            .to_string(),
        };
        f.write_str(&text)
    }
}

pub type CoreResult<T> = Result<T, CoreError>;

/// `error` plus its whole `source()` chain, joined with ": " (URLs removed).
pub(crate) fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(e) = cur {
        let text = strip_url(&e.to_string());
        if !text.is_empty() && parts.last() != Some(&text) {
            parts.push(text);
        }
        cur = e.source();
    }
    parts.join(": ")
}

/// Drops `(url: …)`-style fragments (they can carry tokens or session paths).
/// Cuts at the URL scheme only, so a cause such as "http2 error" stays.
pub(crate) fn strip_url(message: &str) -> String {
    let start = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| message.find(scheme))
        .min();
    match start {
        Some(i) => message[..i].trim_end_matches([' ', '(', ':']).to_string(),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_chain_lists_all_causes() {
        #[derive(Debug)]
        struct Outer(std::io::Error);
        impl std::fmt::Display for Outer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "error sending request")
            }
        }
        impl std::error::Error for Outer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let e = Outer(std::io::Error::new(
            std::io::ErrorKind::ConnectionReset,
            "connection reset by peer",
        ));
        assert_eq!(
            error_chain(&e),
            "error sending request: connection reset by peer"
        );
    }

    #[test]
    fn strip_url_drops_the_url_but_keeps_http_in_the_cause() {
        assert_eq!(
            strip_url("error sending request for url (https://bank.example/s/4711?fnz=1)"),
            "error sending request for url"
        );
        assert_eq!(strip_url("redirect to http://a.example/x"), "redirect to");
        assert_eq!(strip_url("http2 error"), "http2 error");
    }

    /// A failed request names its cause (#2020: a FinTS server that no longer
    /// exists showed only "error sending request"), never the URL's path or query.
    #[tokio::test]
    async fn network_error_names_the_cause_without_the_url() {
        // nothing listens on this port
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        // no_proxy: a proxy from the environment would answer instead of the port
        let e = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{port}/session/4711?fnz=secret"))
            .send()
            .await
            .unwrap_err();
        let CoreError::Network { message } = CoreError::from(e) else {
            panic!("not a network error");
        };
        assert!(message.starts_with("error sending request: "), "{message}");
        assert!(message.len() > "error sending request: ".len(), "{message}");
        for leak in ["session", "4711", "secret", "http"] {
            assert!(!message.contains(leak), "{leak} in {message}");
        }
    }
}

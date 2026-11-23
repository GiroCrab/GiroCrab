//! Offline accounts (#679, #81): validation of the "Offline-Konto anlegen" form without Slint.
//! Mac reference: `OfflineAccountSheet` / `OfflineAccountOptions` in `OfflineAccountsUI.swift`.
//! `offline_ui.rs` only moves the values between the window and the vault.

use crate::assets_loans::optional_cents;
use crate::manage::validate_name;

/// Validated form, ready for `Vault::create_offline_account`.
#[derive(Debug, PartialEq, Eq)]
pub struct OfflineForm {
    pub name: String,
    pub currency: String,
    pub opening_cents: i64,
}

/// ISO 4217 code: three letters, upper-cased; empty = EUR (the core's default too).
pub fn normalize_currency(text: &str) -> Result<String, String> {
    let code = text.trim().to_ascii_uppercase();
    if code.is_empty() {
        return Ok("EUR".into());
    }
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase()) {
        Ok(code)
    } else {
        Err("Währung: Bitte einen dreistelligen Code wie EUR oder CHF eingeben.".into())
    }
}

/// Name required (the core would silently fall back to "Bargeld"), currency as ISO code,
/// opening balance optional (empty = 0), a typo is an error rather than 0.
pub fn validate(name: &str, currency: &str, opening: &str) -> Result<OfflineForm, String> {
    Ok(OfflineForm {
        name: validate_name("das Konto", name)?,
        currency: normalize_currency(currency)?,
        opening_cents: optional_cents(opening, "Anfangssaldo")?.unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_overlong_name_is_rejected() {
        assert!(validate("", "EUR", "").unwrap_err().contains("Namen"));
        assert!(validate("   ", "EUR", "").unwrap_err().contains("Namen"));
        assert!(
            validate(&"x".repeat(81), "EUR", "")
                .unwrap_err()
                .contains("80")
        );
        assert_eq!(validate("  Kasse ", "", "").unwrap().name, "Kasse");
    }

    #[test]
    fn currency_is_an_iso_code_and_defaults_to_eur() {
        assert_eq!(normalize_currency(""), Ok("EUR".into()));
        assert_eq!(normalize_currency(" chf "), Ok("CHF".into()));
        assert!(normalize_currency("EU").is_err());
        assert!(normalize_currency("EURO").is_err());
        assert!(normalize_currency("E1R").is_err());
        assert!(normalize_currency("ÄUR").is_err());
    }

    #[test]
    fn opening_balance_formats() {
        let cents = |t: &str| validate("Kasse", "EUR", t).map(|f| f.opening_cents);
        assert_eq!(cents(""), Ok(0));
        assert_eq!(cents("12,50"), Ok(1_250));
        assert_eq!(cents("1.234,56"), Ok(123_456));
        assert_eq!(cents("1.234.567,00"), Ok(123_456_700));
        assert!(cents("abc").unwrap_err().contains("Anfangssaldo"));
        assert!(cents("12,5x").is_err());
    }
}

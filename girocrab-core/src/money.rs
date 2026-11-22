//! Integer money helpers. Amounts are i64 cents; parsing goes through
//! [`Decimal`], never through floats.
//!
//! "1.500" is 1,50 € in a JSON API and 1.500 € in a text field, so there are
//! two kinds of parsers:
//!
//! * Machine formats (JSON, XML, FinTS, NVP) fix the decimal separator in
//!   their spec and never group digits: [`decimal`] with [`cents`] or
//!   [`cents_rounded`], and the `json_number_to_cents*` helpers.
//!   [`loose_decimal`] also accepts the odd API that sends display text.
//! * Text written for people (input fields, CSV, web pages, PDFs) has to
//!   guess the separator: [`parse_cents`] for amounts and
//!   [`parse_cents_rounded`] for prices.

use std::borrow::Cow;

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};

use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};

/// [`parse_cents`] for the FFI.
pub fn parse_amount_to_cents(text: String) -> Result<i64, CoreError> {
    parse_cents(&text)
}

/// `numerator / denominator` of `total_cents`, rounded toward zero, so the
/// cent that does not divide stays with the rest of a split (#1663): half of
/// 87,43 € is 43,71 €, the rest 43,72 €. `None` for a zero denominator or a
/// share beyond `i64`.
pub fn share_cents(total_cents: i64, numerator: i64, denominator: i64) -> Option<i64> {
    if denominator == 0 {
        return None;
    }
    i64::try_from(i128::from(total_cents) * i128::from(numerator) / i128::from(denominator)).ok()
}

/// A part typed into the split sheet (#1663): an amount read like
/// [`parse_cents`] ("43,71") or a percentage of `total_cents` ("50 %",
/// "33,3%"), rounded toward zero like [`share_cents`]. A percentage may carry
/// any number of decimals; its lone separator is always the decimal one.
pub fn parse_split_part(text: &str, total_cents: i64) -> CoreResult<i64> {
    let Some(percent) = text.trim().strip_suffix('%') else {
        return parse_cents(text);
    };
    let invalid = || CoreError::InvalidInput {
        message: trf("Kein gültiger Anteil: „{0}“", &[&text]),
    };
    let p = text_decimal(percent, Lone::Decimal).ok_or_else(invalid)?;
    Decimal::from(total_cents)
        .checked_mul(p)
        .and_then(|d| d.checked_div(Decimal::ONE_HUNDRED))
        .and_then(|d| d.trunc().to_i64())
        .ok_or_else(invalid)
}

/// Parses an amount written for people ("1.234,56 €", "-12.5", "(5,00)",
/// "12,30-", "EUR 1'234.50") into cents. More than two decimals are an error
/// unless they are zeros.
///
/// Spaces, apostrophes (`'`, `’`), "€" and "EUR" are ignored; one leading or
/// trailing sign (`-` also as U+2212 or U+2013) or parentheses mark a
/// negative amount. Separators:
/// * both `.` and `,` present: the last one is the decimal separator
/// * one kind present several times: grouping only ("1.234.567")
/// * one kind present once: grouping if exactly three digits follow and
///   1-3 digits without a leading zero precede it ("1.500", "12,345"),
///   otherwise the decimal separator ("12,5", "0,500", "1500.00")
///
/// Grouped digits must come in groups of three.
pub fn parse_cents(text: &str) -> CoreResult<i64> {
    parse_text(text, false)
}

/// Like [`parse_cents`], but for prices and depot values: a lone separator is
/// always the decimal separator, since prices often carry three decimals
/// ("55,550"), and extra decimals round half away from zero instead of being
/// rejected ("412.3871" -> 41239, "1.234,5678" -> 123457). Bookings stay strict.
pub fn parse_cents_rounded(text: &str) -> CoreResult<i64> {
    parse_text(text, true)
}

fn parse_text(text: &str, round: bool) -> CoreResult<i64> {
    let invalid = |message| CoreError::InvalidInput { message };
    if text.trim().is_empty() {
        return Err(invalid(tr("Kein Betrag angegeben").into()));
    }
    let lone = if round { Lone::Decimal } else { Lone::Guess };
    let d = text_decimal(text, lone)
        .ok_or_else(|| invalid(trf("Kein gültiger Betrag: „{0}“", &[&text])))?;
    if !round && d.normalize().scale() > 2 {
        return Err(invalid(trf("Zu viele Nachkommastellen in „{0}“", &[&text])));
    }
    if round { cents_rounded(d) } else { cents(d) }
        .ok_or_else(|| invalid(trf("Betrag zu groß: „{0}“", &[&text])))
}

/// A machine-format number with the decimal separator `sep`:
/// `[+-]digits[sep digits]`, no grouping ("1234,5" with `,`, "-0.01" with `.`).
pub fn decimal(text: &str, sep: char) -> Option<Decimal> {
    let (neg, body) = split_sign(text.trim());
    let (int, frac) = body.split_once(sep).unwrap_or((body, ""));
    from_digits(neg, int, frac)
}

/// A JSON amount: a number in its shortest textual form (so float noise such
/// as 8398.279999999999 rounds as written) or exponent form (4e-7), or a
/// string read by [`loose_decimal`].
fn json_decimal(v: &serde_json::Value) -> Option<Decimal> {
    match v {
        serde_json::Value::Number(n) => {
            let s = n.to_string();
            decimal(&s, '.').or_else(|| Decimal::from_scientific(&s).ok())
        }
        serde_json::Value::String(s) => loose_decimal(s),
        _ => None,
    }
}

/// A machine decimal with `.` ("119.000" is 119), else display text with a
/// lone separator read as the decimal separator ("1.234,56", "12,5").
pub fn loose_decimal(text: &str) -> Option<Decimal> {
    decimal(text, '.').or_else(|| text_decimal(text, Lone::Decimal))
}

/// Exact cents: `None` for non-zero digits after the second decimal or an
/// amount beyond `i64`.
pub fn cents(d: Decimal) -> Option<i64> {
    let c = d.checked_mul(Decimal::ONE_HUNDRED)?;
    if c.fract().is_zero() {
        c.to_i64()
    } else {
        None
    }
}

/// Cents rounded half away from zero; `None` beyond `i64`.
pub fn cents_rounded(d: Decimal) -> Option<i64> {
    d.checked_mul(Decimal::ONE_HUNDRED)?
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
}

/// [`json_decimal`] in exact cents.
pub fn json_number_to_cents(v: &serde_json::Value) -> CoreResult<i64> {
    json_decimal(v)
        .and_then(cents)
        .ok_or_else(|| not_an_amount(v))
}

/// [`json_decimal`] in rounded cents.
pub fn json_number_to_cents_rounded(v: &serde_json::Value) -> CoreResult<i64> {
    json_decimal(v)
        .and_then(cents_rounded)
        .ok_or_else(|| not_an_amount(v))
}

fn not_an_amount(v: &serde_json::Value) -> CoreError {
    CoreError::Protocol {
        message: format!("expected amount, got {v}"),
    }
}

/// What a separator that occurs once, as the only kind, means.
#[derive(Clone, Copy, PartialEq)]
enum Lone {
    /// Grouping before exactly three digits after 1-3 digits without a
    /// leading zero ("1.500"), else decimal (see [`parse_cents`]).
    Guess,
    /// Always decimal: prices and API text ("55,550"; "1.500" is 1,50).
    Decimal,
}

/// The amount in text for people; see [`parse_cents`] for the rules.
fn text_decimal(text: &str, lone: Lone) -> Option<Decimal> {
    let compact: String = text
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '\'' | '’' | '€'))
        .map(|c| {
            if matches!(c, '\u{2212}' | '\u{2013}') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let (paren, t) = match compact.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(t) => (true, t),
        None => (false, compact.as_str()),
    };
    // EUR may stand outside the sign ("EUR -5", "5 EUR-") or inside ("-5 EUR").
    let t = strip_eur(t);
    let (neg, t) = match t.strip_suffix('-') {
        Some(t) => (true, t),
        None => split_sign(t.strip_suffix('+').unwrap_or(t)),
    };
    if paren && neg {
        return None;
    }
    let t = strip_eur(t);
    let (int, frac) = match decimal_separator(t, lone) {
        Some(p) => (&t[..p], &t[p + 1..]),
        None => (t, ""),
    };
    from_digits(paren || neg, &ungroup(int)?, frac)
}

/// Position of the decimal separator (see [`parse_cents`]), `None` when every
/// separator groups digits.
fn decimal_separator(t: &str, lone: Lone) -> Option<usize> {
    let last = t.rfind(['.', ','])?;
    let sep = t.as_bytes()[last];
    if t.bytes().any(|b| matches!(b, b'.' | b',') && b != sep) {
        return Some(last);
    }
    if t.bytes().filter(|&b| b == sep).count() > 1 {
        return None;
    }
    let (int, frac) = (&t[..last], &t[last + 1..]);
    let grouping = lone == Lone::Guess
        && frac.len() == 3
        && (1..=3).contains(&int.len())
        && !int.starts_with('0');
    (!grouping).then_some(last)
}

/// The digits of an integer part with optional grouping ("1.234.567"): one
/// kind of separator, a first group of 1-3 digits, then groups of three.
fn ungroup(int: &str) -> Option<Cow<'_, str>> {
    let Some(sep) = int.chars().find(|&c| c == '.' || c == ',') else {
        return Some(Cow::Borrowed(int));
    };
    let mut groups = int.split(sep);
    let first_ok = groups.next().is_some_and(|g| (1..=3).contains(&g.len()));
    (first_ok && groups.all(|g| g.len() == 3)).then(|| Cow::Owned(int.replace(sep, "")))
}

fn strip_eur(t: &str) -> &str {
    let is_eur = |s: Option<&str>| s.is_some_and(|s| s.eq_ignore_ascii_case("EUR"));
    if is_eur(t.get(..3)) {
        &t[3..]
    } else if is_eur(t.get(t.len().saturating_sub(3)..)) {
        &t[..t.len() - 3]
    } else {
        t
    }
}

fn split_sign(t: &str) -> (bool, &str) {
    match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    }
}

/// `int.frac` from ASCII digits; either part may be empty, not both.
fn from_digits(neg: bool, int: &str, frac: &str) -> Option<Decimal> {
    if int.is_empty() && frac.is_empty() {
        return None;
    }
    let mantissa = int.bytes().chain(frac.bytes()).try_fold(0i128, |m, b| {
        let digit = char::from(b).to_digit(10)?;
        m.checked_mul(10)?.checked_add(digit.into())
    })?;
    let scale = u32::try_from(frac.len()).ok()?;
    let d = Decimal::try_from_i128_with_scale(mantissa, scale).ok()?;
    Some(if neg { -d } else { d })
}

/// German-style formatting used in logs/tests: "-1.234,56 EUR".
pub fn format_cents(cents: i64, currency: String) -> String {
    let neg = cents < 0;
    let abs = cents.unsigned_abs();
    let euros = abs / 100;
    let rest = abs % 100;
    let digits = euros.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    format!(
        "{}{},{:02} {}",
        if neg { "-" } else { "" },
        grouped,
        rest,
        currency
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(s: &str) -> Option<i64> {
        parse_cents(s).ok()
    }

    #[test]
    fn text_amounts() {
        for (input, cents) in [
            ("1234,56", 123_456),
            ("1.234,56", 123_456),
            ("1,234.56", 123_456),
            ("1,234,567.89", 123_456_789),
            ("-12.345.678,9", -1_234_567_890),
            ("-12.5", -1250),
            ("+0,07", 7),
            ("12", 1200),
            ("12,", 1200),
            (",5", 50),
            ("-0.01", -1),
            ("1500.0000", 150_000),
            ("1 234,56", 123_456),
            ("1'234.50", 123_450),
            ("1\u{a0}234,56\u{a0}€", 123_456),
            ("€ 5", 500),
            ("EUR 1.234,56", 123_456),
            ("1.234,56 eur", 123_456),
            ("- 0,73 €", -73),
            ("−12,30 €", -1230),
            ("12,30-", -1230),
            ("12,30+", 1230),
            ("(5,00)", -500),
            ("(5,00 €)", -500),
            ("EUR -5", -500),
            ("5 EUR-", -500),
        ] {
            assert_eq!(text(input), Some(cents), "{input:?}");
        }
    }

    #[test]
    fn one_separator_before_three_digits_groups() {
        assert_eq!(text("1.500"), Some(150_000));
        assert_eq!(text("1.000"), Some(100_000));
        assert_eq!(text("12,345"), Some(1_234_500));
        assert_eq!(text("-1.234"), Some(-123_400));
        assert_eq!(text("-1.234.567"), Some(-123_456_700));
        // A leading zero or more digits before it keep it a decimal separator.
        assert_eq!(text("0,500"), Some(50));
        assert_eq!(text("1500.000"), Some(150_000));
    }

    #[test]
    fn malformed_text_is_rejected() {
        for bad in [
            "",
            " ",
            "abc",
            "-",
            "EUR",
            "€",
            ",",
            "1.00,00",
            ".000,00",
            "1.0000,00",
            "1..000,00",
            "1.2,3.4",
            "--5",
            "-5-",
            "+5-",
            "-(5)",
            "(-5)",
            "5 S",
            "12abc",
            "1,5.0,0",
        ] {
            assert_eq!(text(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn text_errors_name_the_reason() {
        let msg = |s| parse_cents(s).unwrap_err().to_string();
        assert!(msg(" ").contains("Kein Betrag"), "{}", msg(" "));
        assert!(msg("1,2x").contains("Kein gültiger Betrag"));
        assert!(msg("1,2345").contains("Nachkommastellen"));
        assert!(msg("92233720368547758,08").contains("zu groß"));
    }

    #[test]
    fn rounded_text() {
        let r = |s| parse_cents_rounded(s).unwrap();
        assert_eq!(r("412.3871"), 41239);
        assert_eq!(r("1.234,5678"), 123_457);
        assert_eq!(r("12,3456"), 1235);
        assert_eq!(r("-0,005"), -1);
        assert_eq!(r("0.999"), 100);
        // Prices: a lone separator is the decimal separator.
        assert_eq!(r("55,550 "), 5555);
        assert_eq!(r("1.500"), 150);
        assert_eq!(r("1.234.567"), 123_456_700);
    }

    #[test]
    fn machine_decimals_never_group() {
        let c = |s, sep| decimal(s, sep).and_then(cents);
        assert_eq!(c("1.500", '.'), Some(150));
        assert_eq!(c("1500,000", ','), Some(150_000));
        assert_eq!(c("243,", ','), Some(24_300));
        assert_eq!(c("71.01", '.'), Some(7101));
        assert_eq!(c(" -0.31 ", '.'), Some(-31));
        assert_eq!(c("+7", '.'), Some(700));
        assert_eq!(c("1.005", '.'), None);
        for bad in [
            "", "-", ".", "1,00", "1.0.0", "1 000.00", "1_000", "1e3", "abc",
        ] {
            assert_eq!(decimal(bad, '.'), None, "{bad:?}");
        }
        assert_eq!(decimal("11,672", ',').and_then(cents_rounded), Some(1167));
    }

    #[test]
    fn json_amounts() {
        let exact = |v| json_number_to_cents(&v).ok();
        let r = |v| json_number_to_cents_rounded(&v).unwrap();
        assert_eq!(exact(json!(12.5)), Some(1250));
        assert_eq!(exact(json!("1.500")), Some(150));
        assert_eq!(exact(json!("1.234,56")), Some(123_456));
        assert_eq!(exact(json!("1,500")), Some(150));
        assert_eq!(exact(json!("1,234.56")), Some(123_456));
        assert_eq!(exact(json!("412.3871")), None);
        assert_eq!(exact(json!(null)), None);
        assert_eq!(r(json!("412.3871")), 41239);
        assert_eq!(r(json!("412.3849")), 41238);
        assert_eq!(r(json!("-0.005")), -1);
        assert_eq!(r(json!("-12.344")), -1234);
        assert_eq!(r(json!(61.3412)), 6134);
        assert_eq!(r(json!("1.234,5678")), 123_457);
        assert_eq!(r(json!(4e-7)), 0);
        assert_eq!(r(json!(2.5e16)), 2_500_000_000_000_000_000);
        assert_eq!(r(json!("12")), 1200);
        // Float noise and binary halves round as printed, not as stored.
        assert_eq!(r(json!(-4.040000000000001)), -404);
        assert_eq!(r(json!(8398.279999999999)), 839_828);
        assert_eq!(r(json!(40.745)), 4075);
        assert!(json_number_to_cents_rounded(&json!(1e300)).is_err());
        assert!(json_number_to_cents_rounded(&json!(null)).is_err());
    }

    #[test]
    fn beyond_i64_is_an_error_not_saturation() {
        assert_eq!(text("-92233720368547758,08"), Some(i64::MIN));
        assert_eq!(text("99999999999999999999999999999999999999999"), None);
    }

    #[test]
    fn formats() {
        assert_eq!(format_cents(-123456, "EUR".into()), "-1.234,56 EUR");
        assert_eq!(format_cents(5, "EUR".into()), "0,05 EUR");
    }

    /// #1663: the cent that does not divide stays with the rest, in either direction.
    #[test]
    fn shares_round_toward_zero() {
        assert_eq!(share_cents(8_743, 1, 2), Some(4_371));
        assert_eq!(share_cents(-8_743, 1, 2), Some(-4_371));
        assert_eq!(share_cents(10_000, 1, 3), Some(3_333));
        assert_eq!(share_cents(10_000, 2, 3), Some(6_666));
        assert_eq!(share_cents(1, 1, 4), Some(0));
        assert_eq!(share_cents(i64::MAX, 3, 2), None);
        assert_eq!(share_cents(100, 1, 0), None);
    }

    #[test]
    fn split_parts_read_amounts_and_percentages() {
        let part = |s: &str| parse_split_part(s, 8_743).ok();
        for (input, cents) in [
            ("43,71", Some(4_371)),
            ("1.500", Some(150_000)),
            ("50 %", Some(4_371)),
            ("50%", Some(4_371)),
            (" 50 % ", Some(4_371)),
            ("33,3 %", Some(2_911)),
            ("33.333%", Some(2_914)),
            ("100 %", Some(8_743)),
            ("150 %", Some(13_114)),
            ("−10 %", Some(-874)),
            ("0 %", Some(0)),
            ("%", None),
            ("abc %", None),
            ("50 %%", None),
            ("", None),
        ] {
            assert_eq!(part(input), cents, "{input:?}");
        }
        assert_eq!(parse_split_part("50 %", -8_743).ok(), Some(-4_371));
    }
}

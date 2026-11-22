//! Transfer dialog helpers for payment text from the clipboard and the GiroCode (EPC QR) view (#663).
//! Plain functions on top of `girocrab_core::payment_input`; no vault, no network, nothing is stored
//! or logged (the texts are personal data).

use girocrab_core::payment_input::{
    GiroCodeMatrix, PaymentDetails, girocode_matrix, girocode_payload, parse_payment_text,
};

/// Pixels per QR module; the core's quiet zone is kept as it is.
pub const MODULE_PX: u32 = 6;

/// Pixels per QR module for the copied/saved PNG (same as `pixelsPerModule` on the Mac).
pub const EXPORT_MODULE_PX: u32 = 10;

/// The text fields of the transfer dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fields {
    pub name: String,
    pub iban: String,
    pub amount: String,
    pub purpose: String,
}

/// Cents as the dialog's amount text ("12,50").
fn amount_text(cents: i64) -> String {
    format!("{},{:02}", cents / 100, cents % 100)
}

/// Fills only the EMPTY fields of `current` from `found`; filled fields are never overwritten.
/// Returns the merged fields and how many were filled.
pub fn merge_empty(current: &Fields, found: &PaymentDetails) -> (Fields, usize) {
    let mut out = current.clone();
    let mut filled = 0;
    let mut fill = |field: &mut String, value: Option<String>| {
        if field.trim().is_empty()
            && let Some(v) = value.filter(|v| !v.trim().is_empty())
        {
            *field = v;
            filled += 1;
        }
    };
    fill(&mut out.name, found.name.clone());
    fill(&mut out.iban, found.iban.clone());
    fill(
        &mut out.amount,
        found.amount_cents.filter(|c| *c > 0).map(amount_text),
    );
    fill(&mut out.purpose, found.purpose.clone());
    (out, filled)
}

/// Parses clipboard text into the empty fields; `Err` = status text when nothing was recognised
/// or everything recognised is already filled in.
pub fn paste_into(current: &Fields, clipboard: &str) -> Result<Fields, String> {
    let found = parse_payment_text(clipboard.to_string());
    if found == PaymentDetails::default() {
        return Err("In der Zwischenablage wurden keine Zahlungsdaten erkannt.".into());
    }
    let (merged, filled) = merge_empty(current, &found);
    if filled == 0 {
        return Err(
            "Die erkannten Zahlungsdaten stehen schon in den Feldern; nur leere Felder werden gefüllt."
                .into(),
        );
    }
    Ok(merged)
}

/// RGBA pixels (white background, dark modules black) of the matrix, each module
/// `scale` x `scale` pixels. Returns (width, height, bytes).
pub fn matrix_to_rgba(m: &GiroCodeMatrix, scale: u32) -> (u32, u32, Vec<u8>) {
    let n = m.width as usize;
    let s = scale.max(1) as usize;
    let side = n * s;
    let mut px = vec![255u8; side * side * 4];
    for (i, dark) in m.modules.iter().enumerate().take(n * n) {
        if !*dark {
            continue;
        }
        let (row, col) = (i / n, i % n);
        for y in row * s..(row + 1) * s {
            for x in col * s..(col + 1) * s {
                let o = (y * side + x) * 4;
                px[o..o + 3].fill(0);
            }
        }
    }
    (side as u32, side as u32, px)
}

/// The EPC payload for the form, or the status text saying what is missing. The amount is
/// required by the EPC payload.
pub fn girocode_payload_for(f: &Fields) -> Result<String, String> {
    if f.name.trim().is_empty() {
        return Err("Für den GiroCode fehlt der Empfängername.".into());
    }
    if !girocrab_core::connectors::validate_iban(f.iban.clone()) {
        return Err("Für den GiroCode fehlt eine gültige Empfänger-IBAN.".into());
    }
    let cents = girocrab_core::parse_amount_to_cents(f.amount.clone())
        .ok()
        .filter(|c| *c > 0)
        .ok_or_else(|| "Für den GiroCode fehlt ein gültiger Betrag (z. B. 12,50).".to_string())?;
    girocode_payload(
        f.name.clone(),
        f.iban.clone(),
        String::new(),
        cents,
        f.purpose.clone(),
    )
    .ok_or_else(|| "Aus diesen Angaben lässt sich kein GiroCode erzeugen.".to_string())
}

fn matrix_for(payload: String) -> Result<GiroCodeMatrix, String> {
    girocode_matrix(payload).ok_or_else(|| "Der GiroCode ist zu groß für einen QR-Code.".into())
}

/// The QR matrix for the form (see [`girocode_payload_for`]).
pub fn girocode_for(f: &Fields) -> Result<GiroCodeMatrix, String> {
    matrix_for(girocode_payload_for(f)?)
}

/// The QR image for the form (see [`girocode_for`]).
pub fn girocode_image(f: &Fields) -> Result<slint::Image, String> {
    let m = girocode_for(f)?;
    let (w, h, px) = matrix_to_rgba(&m, MODULE_PX);
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(&px, w, h);
    Ok(slint::Image::from_rgba8(buf))
}

fn encode_png(w: u32, h: u32, px: Vec<u8>) -> Result<Vec<u8>, String> {
    let failed = || "Das GiroCode-Bild konnte nicht erzeugt werden.".to_string();
    let img = image::RgbaImage::from_raw(w, h, px).ok_or_else(failed)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|_| failed())?;
    Ok(out.into_inner())
}

/// PNG bytes of the GiroCode for `payload`, each module `scale` x `scale` pixels (fixed size,
/// independent of the screen). Pure: nothing is written or logged.
pub fn girocode_png_bytes(payload: &str, scale: u32) -> Result<Vec<u8>, String> {
    let (w, h, px) = matrix_to_rgba(&matrix_for(payload.to_string())?, scale);
    encode_png(w, h, px)
}

/// Device-independent bitmap (`CF_DIB`: BITMAPINFOHEADER + 32-bit BGRA rows, bottom-up) for
/// programs that do not read PNG from the clipboard.
pub fn dib_bytes(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let row = (w as usize * 4).max(1);
    let mut out = Vec::with_capacity(40 + rgba.len());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes()); // positive height = rows bottom-up
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(w * h * 4).to_le_bytes());
    out.extend_from_slice(&[0; 16]); // resolution, palette
    for line in rgba.chunks_exact(row).rev() {
        for p in line.as_chunks::<4>().0 {
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out
}

/// PNG and DIB of the form's GiroCode for the clipboard, in export resolution.
pub fn girocode_clipboard_images(f: &Fields) -> Result<(Vec<u8>, Vec<u8>), String> {
    let (w, h, px) = matrix_to_rgba(&girocode_for(f)?, EXPORT_MODULE_PX);
    let dib = dib_bytes(w, h, &px);
    Ok((encode_png(w, h, px)?, dib))
}

/// Suggested file name `GiroCode-<Empfänger>.png`, without characters Windows rejects.
pub fn girocode_png_file_name(recipient: &str) -> String {
    // Slashes first: `safe_file_name` would otherwise drop everything before them.
    let name = crate::lists::safe_file_name(&recipient.replace(['/', '\\'], "_"), "");
    if name.is_empty() {
        "GiroCode.png".into()
    } else {
        format!("GiroCode-{name}.png")
    }
}

/// Status text shown instead of the GiroCode in privacy mode or while the vault is locked
/// (`privacy` = `daily::effective_privacy`). Neutral: it names no recipient, IBAN or amount.
pub const GIROCODE_BLOCKED: &str =
    "Im Privatmodus und bei gesperrtem Tresor lässt sich kein GiroCode anzeigen oder exportieren.";

/// Gate for showing, copying and saving the GiroCode. Callers check it BEFORE reading any field,
/// so nothing personal is touched while privacy applies.
pub fn girocode_allowed(privacy: bool) -> Result<(), String> {
    if privacy {
        Err(GIROCODE_BLOCKED.into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(name: &str, iban: &str, amount: &str, purpose: &str) -> Fields {
        Fields {
            name: name.into(),
            iban: iban.into(),
            amount: amount.into(),
            purpose: purpose.into(),
        }
    }

    fn found() -> PaymentDetails {
        PaymentDetails {
            name: Some("Erika Muster".into()),
            iban: Some("DE89370400440532013000".into()),
            bic: None,
            amount_cents: Some(1205),
            purpose: Some("Rechnung 42".into()),
        }
    }

    #[test]
    fn merge_fills_only_empty_fields() {
        let (m, n) = merge_empty(&fields("Max", "", "  ", ""), &found());
        assert_eq!(
            m,
            fields("Max", "DE89370400440532013000", "12,05", "Rechnung 42")
        );
        assert_eq!(n, 3);
    }

    #[test]
    fn merge_never_overwrites() {
        let cur = fields("A", "B", "1", "C");
        let (m, n) = merge_empty(&cur, &found());
        assert_eq!(m, cur);
        assert_eq!(n, 0);
    }

    #[test]
    fn merge_skips_missing_values() {
        let only_iban = PaymentDetails {
            iban: Some("DE89370400440532013000".into()),
            ..Default::default()
        };
        let (m, n) = merge_empty(&Fields::default(), &only_iban);
        assert_eq!(m, fields("", "DE89370400440532013000", "", ""));
        assert_eq!(n, 1);
    }

    #[test]
    fn paste_reports_nothing_recognised() {
        assert!(
            paste_into(&Fields::default(), "  \n ")
                .unwrap_err()
                .contains("keine Zahlungsdaten")
        );
    }

    #[test]
    fn paste_parses_text() {
        let f = paste_into(
            &fields("Erika", "", "", ""),
            "IBAN: DE89 3704 0044 0532 0130 00\nBetrag: 12,50 EUR",
        )
        .unwrap();
        assert_eq!(f.name, "Erika");
        assert_eq!(f.iban.replace(' ', ""), "DE89370400440532013000");
        assert_eq!(f.amount, "12,50");
    }

    #[test]
    fn matrix_is_scaled_to_integer_pixels() {
        // 2x2: dark, light / light, dark
        let m = GiroCodeMatrix {
            width: 2,
            modules: vec![true, false, false, true],
        };
        let (w, h, px) = matrix_to_rgba(&m, 3);
        assert_eq!((w, h, px.len()), (6, 6, 6 * 6 * 4));
        let at = |x: usize, y: usize| px[(y * 6 + x) * 4..(y * 6 + x) * 4 + 4].to_vec();
        assert_eq!(at(0, 0), [0, 0, 0, 255]);
        assert_eq!(at(2, 2), [0, 0, 0, 255]);
        assert_eq!(at(3, 0), [255, 255, 255, 255]);
        assert_eq!(at(0, 3), [255, 255, 255, 255]);
        assert_eq!(at(5, 5), [0, 0, 0, 255]);
    }

    #[test]
    fn girocode_needs_name_iban_amount() {
        let ok = fields(
            "Erika Muster",
            "DE89 3704 0044 0532 0130 00",
            "12,50",
            "Rechnung",
        );
        let m = girocode_for(&ok).unwrap();
        assert_eq!(m.modules.len(), (m.width * m.width) as usize);
        assert!(girocode_for(&fields("", &ok.iban, "1", "")).is_err());
        assert!(girocode_for(&fields("A", "DE00", "1", "")).is_err());
        assert!(girocode_for(&fields("A", &ok.iban, "", "")).is_err());
        assert!(girocode_for(&fields("A", &ok.iban, "0", "")).is_err());
    }

    fn ok_fields() -> Fields {
        fields(
            "Erika Muster",
            "DE89 3704 0044 0532 0130 00",
            "12,50",
            "Rechnung",
        )
    }

    #[test]
    fn png_has_signature_and_fixed_export_size() {
        let payload = girocode_payload_for(&ok_fields()).unwrap();
        let png = girocode_png_bytes(&payload, EXPORT_MODULE_PX).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        let (w, h, _) = matrix_to_rgba(&girocode_matrix(payload).unwrap(), EXPORT_MODULE_PX);
        let be = |o: usize| u32::from_be_bytes(png[o..o + 4].try_into().unwrap());
        assert_eq!((be(16), be(20)), (w, h));
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (w, h));
    }

    #[test]
    fn png_rejects_oversized_payload() {
        assert!(girocode_png_bytes(&"x".repeat(5000), EXPORT_MODULE_PX).is_err());
    }

    #[test]
    fn file_name_is_cleaned_from_recipient() {
        assert_eq!(
            girocode_png_file_name("Erika Muster"),
            "GiroCode-Erika Muster.png"
        );
        let n = girocode_png_file_name("A/B\\C <x>: \"y\"|?*\n.");
        assert!(
            n.starts_with("GiroCode-A_B_C") && n.ends_with(".png"),
            "{n}"
        );
        assert!(
            !n.contains(|c: char| c.is_control() || "<>:\"|?*/\\".contains(c)),
            "{n}"
        );
        assert_eq!(girocode_png_file_name("  ..  "), "GiroCode.png");
    }

    #[test]
    fn dib_is_bottom_up_bgra() {
        // 2x2, only the top-left module dark.
        let m = GiroCodeMatrix {
            width: 2,
            modules: vec![true, false, false, false],
        };
        let (w, h, px) = matrix_to_rgba(&m, 1);
        let dib = dib_bytes(w, h, &px);
        assert_eq!(dib.len(), 40 + 2 * 2 * 4);
        assert_eq!(&dib[..4], &40u32.to_le_bytes());
        assert_eq!(&dib[14..16], &32u16.to_le_bytes());
        // First stored row is the bottom row (light); the dark pixel opens the second.
        assert_eq!(&dib[40..44], &[255, 255, 255, 255]);
        assert_eq!(&dib[48..52], &[0, 0, 0, 255]);
        assert!(girocode_clipboard_images(&Fields::default()).is_err());
    }

    #[test]
    fn girocode_blocked_in_privacy_mode_and_when_locked() {
        use crate::daily::effective_privacy;
        // (toggle, unlocked): privacy on, locked, both.
        for (toggle, unlocked) in [(true, true), (false, false), (true, false)] {
            assert_eq!(
                girocode_allowed(effective_privacy(toggle, unlocked)),
                Err(GIROCODE_BLOCKED.to_string()),
                "toggle={toggle} unlocked={unlocked}"
            );
        }
    }

    #[test]
    fn girocode_allowed_when_unlocked_and_privacy_off() {
        assert_eq!(
            girocode_allowed(crate::daily::effective_privacy(false, true)),
            Ok(())
        );
    }

    #[test]
    fn blocked_status_is_neutral() {
        let msg = girocode_allowed(true).unwrap_err();
        assert!(!msg.is_empty());
        // No amount, IBAN or other digits can end up in the status line.
        assert!(!msg.contains(|c: char| c.is_ascii_digit()), "{msg}");
        assert!(!msg.contains(crate::daily::MASK));
    }

    #[test]
    fn blocked_status_differs_from_missing_field_texts() {
        // The user must be able to tell "blocked" from "something is missing".
        let missing = girocode_payload_for(&Fields::default()).unwrap_err();
        assert_ne!(girocode_allowed(true).unwrap_err(), missing);
    }
}

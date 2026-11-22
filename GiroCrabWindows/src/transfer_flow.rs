//! SEPA transfer dialog flow: drives a `TransferSession` through the same PIN / TAN / push / redirect
//! prompts as the bank sync (`sync_flow::Ui`) plus the recipient name check, independent of Slint.
//! Nothing is retried automatically: an unclear outcome is reported and never resent.

use std::time::{Duration, Instant};

use girocrab_core::connectors::PayeeCheck;
use girocrab_core::{Account, AccountKind, Challenge, ChallengeKind, TransferStep};

use crate::daily::TransferInput;
use crate::sync_flow::{CANCELLED, Ui, safe_https_url};

/// Shown while a savings account is the sender (wording of the Mac sheet).
pub const SAVINGS_HINT: &str =
    "Vom Tagesgeldkonto sind nur Umbuchungen auf das Referenzkonto möglich.";
/// Shown when a savings account has no reference account; sending stays blocked.
pub const SAVINGS_NO_REFERENCE: &str =
    "Für dieses Tagesgeldkonto gibt es kein Referenzkonto (eigenes Girokonto mit IBAN).";
/// Longest recipient name a SEPA transfer accepts (same limit as `daily::validate_transfer`).
const RECIPIENT_NAME_MAX: usize = 70;

/// The only recipient a transfer from a savings account may go to (#582).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    pub name: String,
    pub iban: String,
}

/// What the transfer dialog allows as recipient for one sender.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RecipientRule {
    /// Current account etc.: free input.
    #[default]
    Free,
    /// Savings account: fixed to its reference account, fields locked.
    Fixed(Recipient),
    /// Savings account without a reference account: nothing may be sent.
    Unavailable,
}

fn is_savings(a: &Account) -> bool {
    a.kind == AccountKind::Savings
}

fn usable_iban(a: &Account) -> Option<String> {
    let iban: String = a.iban.split_whitespace().collect();
    (!iban.is_empty()).then(|| iban.to_uppercase())
}

/// Reference account of a savings account, derived like the Mac's `internalTransferTargets(from:)`:
/// an unarchived current account with an IBAN, same bank connection first. `None` for any other
/// sender type or when no such account exists.
pub fn savings_counterpart(account: &Account, accounts: &[Account]) -> Option<Recipient> {
    if !is_savings(account) {
        return None;
    }
    let giro = |a: &&Account| {
        a.id != account.id
            && a.kind == AccountKind::Checking
            && a.archived_unix.is_none()
            && usable_iban(a).is_some()
    };
    let same_bank =
        |a: &&Account| account.connection_id.is_some() && a.connection_id == account.connection_id;
    let target = accounts
        .iter()
        .filter(giro)
        .find(same_bank)
        .or_else(|| accounts.iter().find(giro))?;
    // The account holder is what the bank's recipient check compares; fall back to the label.
    // A label may be longer than a SEPA name allows: the locked field must still pass the form check.
    let source = match target.owner.trim() {
        "" => target.name.trim(),
        owner => owner,
    };
    let name: String = source.chars().take(RECIPIENT_NAME_MAX).collect();
    Some(Recipient {
        name: name.trim_end().to_string(),
        iban: usable_iban(target)?,
    })
}

pub fn recipient_rule(account: &Account, accounts: &[Account]) -> RecipientRule {
    match savings_counterpart(account, accounts) {
        Some(r) => RecipientRule::Fixed(r),
        None if is_savings(account) => RecipientRule::Unavailable,
        None => RecipientRule::Free,
    }
}

/// Enforces the rule on a recipient name and IBAN (transfer form and GiroCode export): the typed
/// text stays for a free sender, a fixed recipient replaces it, an unavailable one is refused.
pub fn apply_recipient_fields(
    name: &mut String,
    iban: &mut String,
    rule: &RecipientRule,
) -> Result<(), String> {
    match rule {
        RecipientRule::Free => Ok(()),
        RecipientRule::Fixed(r) => {
            *name = r.name.clone();
            *iban = r.iban.clone();
            Ok(())
        }
        RecipientRule::Unavailable => Err(SAVINGS_NO_REFERENCE.into()),
    }
}

/// Enforces the rule on the submitted form: the field text never wins over a fixed recipient.
pub fn apply_recipient_rule(input: &mut TransferInput, rule: &RecipientRule) -> Result<(), String> {
    apply_recipient_fields(&mut input.name, &mut input.iban, rule)
}

const DIALOG_TIMEOUT: Duration = Duration::from_secs(10 * 60);

pub trait TransferUi: Ui {
    /// The recipient name check did not confirm the payee: `true` = send anyway.
    fn confirm_payee(&self, check: &PayeeCheck) -> bool;
}

pub trait TransferStepper {
    fn start(&self) -> Result<TransferStep, String>;
    fn submit_code(&self, code: String) -> Result<TransferStep, String>;
    fn poll(&self) -> Result<TransferStep, String>;
    fn confirm_payee(&self) -> Result<TransferStep, String>;
    fn cancel(&self);
}

pub fn payee_text(c: &PayeeCheck) -> String {
    use girocrab_core::connectors::PayeeMatch::*;
    let mut s = match c.result {
        Match => "Der Name stimmt mit dem Kontoinhaber überein.".to_string(),
        CloseMatch => format!(
            "Der Name weicht leicht ab. Kontoinhaber laut Empfängerbank: {}.",
            c.bank_name.as_deref().unwrap_or("unbekannt")
        ),
        NoMatch => "Der Name stimmt NICHT mit dem Kontoinhaber überein.".to_string(),
        NotPossible => format!(
            "Die Namensprüfung war nicht möglich{}.",
            c.reason
                .as_deref()
                .map(|r| format!(" ({r})"))
                .unwrap_or_default()
        ),
    };
    if let Some(n) = c.notice.as_deref().filter(|n| !n.is_empty()) {
        s.push('\n');
        s.push_str(n);
    }
    s
}

/// Runs the transfer to its end; `Ok` = text for the status line.
pub fn drive_transfer<S: TransferStepper, U: TransferUi>(
    session: &S,
    ui: &U,
) -> Result<String, String> {
    let deadline = Instant::now() + DIALOG_TIMEOUT;
    let mut step = session.start()?;
    loop {
        if Instant::now() > deadline {
            session.cancel();
            return Err("Zeitüberschreitung: Die Überweisung wurde abgebrochen.".into());
        }
        step = match step {
            TransferStep::Finished { outcome } => {
                let mut msg = format!(
                    "Überweisung gesendet ({}). Referenz: {}",
                    if outcome.instant { "Echtzeit" } else { "SEPA" },
                    outcome.end_to_end_id
                );
                if let Some(e) = outcome.record_error {
                    msg += &format!(
                        "\nSie wurde nicht gespeichert ({e}) und erscheint mit dem nächsten Abruf. NICHT erneut senden."
                    );
                }
                return Ok(msg);
            }
            TransferStep::InstantDeclined { reason } => {
                return Err(format!("Nicht ausgeführt: {reason}"));
            }
            TransferStep::OutcomeUnknown { reason, .. } => {
                return Err(format!(
                    "Ergebnis unklar: {reason} Bitte das Konto prüfen und NICHT erneut senden."
                ));
            }
            TransferStep::NeedsPayeeConfirmation { check } => {
                if ui.confirm_payee(&check) {
                    session.confirm_payee()?
                } else {
                    session.cancel();
                    return Err(CANCELLED_TRANSFER.into());
                }
            }
            TransferStep::StillWaiting { challenge } => wait_and_poll(session, ui, &challenge)?,
            TransferStep::NeedsChallenge { challenge } => match challenge.kind {
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

pub const CANCELLED_TRANSFER: &str = "Überweisung abgebrochen.";

fn wait_and_poll<S: TransferStepper, U: TransferUi>(
    session: &S,
    ui: &U,
    c: &Challenge,
) -> Result<TransferStep, String> {
    if !ui.wait_push(
        c,
        Duration::from_secs(c.poll_interval_seconds.clamp(1, 30) as u64),
    ) {
        session.cancel();
        return Err(CANCELLED_TRANSFER.into());
    }
    session.poll()
}

fn submit<S: TransferStepper, U: TransferUi>(
    session: &S,
    ui: &U,
    c: &Challenge,
) -> Result<TransferStep, String> {
    match ui.ask_code(c) {
        Some(mut code) if !code.trim().is_empty() => {
            session.submit_code(std::mem::take(&mut *code).trim().to_string())
        }
        _ => {
            session.cancel();
            Err(CANCELLED.replace("Abruf", "Überweisung"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use girocrab_core::connectors::{PayeeMatch, TransferOutcome};
    use std::cell::RefCell;
    use zeroize::Zeroizing;

    fn acc(id: i64, kind: AccountKind, conn: Option<i64>, iban: &str) -> Account {
        let mut a = crate::nav::testing::account(id, &format!("Konto {id}"), "Bank", kind, 0);
        a.connection_id = conn;
        a.iban = iban.to_string();
        a
    }

    #[test]
    fn savings_prefers_checking_of_same_connection() {
        let savings = acc(1, AccountKind::Savings, Some(7), "DE01");
        let all = vec![
            savings.clone(),
            acc(2, AccountKind::Checking, Some(9), "DE02 0000"),
            acc(3, AccountKind::Checking, Some(7), "de03 0000"),
        ];
        let r = savings_counterpart(&savings, &all).unwrap();
        assert_eq!(r.iban, "DE030000");
        assert_eq!(r.name, "Konto 3");
    }

    #[test]
    fn savings_uses_owner_as_recipient_name_when_set() {
        let savings = acc(1, AccountKind::Savings, Some(7), "DE01");
        let mut giro = acc(2, AccountKind::Checking, Some(7), "DE02");
        giro.owner = " Erika Muster ".into();
        let r = savings_counterpart(&savings, &[savings.clone(), giro]).unwrap();
        assert_eq!(r.name, "Erika Muster");
    }

    #[test]
    fn fixed_recipient_name_stays_within_the_transfer_limit() {
        let savings = acc(1, AccountKind::Savings, Some(7), "DE01");

        // Empty owner, label of exactly 70 characters: kept as is.
        let mut giro = acc(2, AccountKind::Checking, Some(7), "DE02");
        giro.owner = "  ".into();
        giro.name = "ä".repeat(70);
        let r = savings_counterpart(&savings, &[savings.clone(), giro.clone()]).unwrap();
        assert_eq!(r.name, "ä".repeat(70));

        // Empty owner, label of 71 characters: shortened to 70 characters (not bytes).
        giro.name = "ä".repeat(71);
        let r = savings_counterpart(&savings, &[savings.clone(), giro.clone()]).unwrap();
        assert_eq!(r.name.chars().count(), 70);
        assert_eq!(r.name, "ä".repeat(70));

        // A cut right after a space leaves no trailing blank.
        giro.name = format!("{} Rest", "a".repeat(69));
        let r = savings_counterpart(&savings, &[savings.clone(), giro.clone()]).unwrap();
        assert_eq!(r.name, "a".repeat(69));

        // A long owner is shortened as well.
        giro.owner = "b".repeat(80);
        let r = savings_counterpart(&savings, &[savings.clone(), giro]).unwrap();
        assert_eq!(r.name, "b".repeat(70));
    }

    #[test]
    fn savings_falls_back_to_other_checking_and_skips_archived_or_without_iban() {
        let savings = acc(1, AccountKind::Savings, Some(7), "DE01");
        let mut archived = acc(2, AccountKind::Checking, Some(7), "DE02");
        archived.archived_unix = Some(1);
        let all = vec![
            savings.clone(),
            archived,
            acc(3, AccountKind::Checking, Some(7), ""),
            acc(4, AccountKind::Checking, Some(9), "DE04"),
        ];
        assert_eq!(savings_counterpart(&savings, &all).unwrap().iban, "DE04");
    }

    #[test]
    fn savings_without_reference_blocks_sending() {
        let savings = acc(1, AccountKind::Savings, Some(7), "DE01");
        let all = vec![
            savings.clone(),
            acc(2, AccountKind::Savings, Some(7), "DE02"),
        ];
        assert_eq!(savings_counterpart(&savings, &all), None);
        let rule = recipient_rule(&savings, &all);
        assert_eq!(rule, RecipientRule::Unavailable);
        let mut input = TransferInput::default();
        assert_eq!(
            apply_recipient_rule(&mut input, &rule),
            Err(SAVINGS_NO_REFERENCE.to_string())
        );
    }

    #[test]
    fn checking_stays_free_and_fixed_rule_overrides_typed_fields() {
        let giro = acc(1, AccountKind::Checking, Some(7), "DE01");
        let savings = acc(2, AccountKind::Savings, Some(7), "DE02");
        let all = vec![giro.clone(), savings.clone()];
        assert_eq!(recipient_rule(&giro, &all), RecipientRule::Free);

        let mut input = TransferInput {
            name: "Fremd".into(),
            iban: "DE99".into(),
            ..Default::default()
        };
        apply_recipient_rule(&mut input, &recipient_rule(&savings, &all)).unwrap();
        assert_eq!(
            (input.name.as_str(), input.iban.as_str()),
            ("Konto 1", "DE01")
        );
    }

    #[test]
    fn recipient_fields_follow_the_rule_for_girocode_export() {
        let giro = acc(1, AccountKind::Checking, Some(7), "DE01");
        let savings = acc(2, AccountKind::Savings, Some(7), "DE02");
        let all = vec![giro.clone(), savings.clone()];

        // Free: the typed recipient stays.
        let (mut name, mut iban) = (String::from("Fremd"), String::from("DE99"));
        apply_recipient_fields(&mut name, &mut iban, &RecipientRule::Free).unwrap();
        assert_eq!((name.as_str(), iban.as_str()), ("Fremd", "DE99"));

        // Fixed: the reference account replaces typed and empty fields alike.
        let fixed = recipient_rule(&savings, &all);
        apply_recipient_fields(&mut name, &mut iban, &fixed).unwrap();
        assert_eq!((name.as_str(), iban.as_str()), ("Konto 1", "DE01"));
        let (mut name, mut iban) = (String::new(), String::new());
        apply_recipient_fields(&mut name, &mut iban, &fixed).unwrap();
        assert_eq!((name.as_str(), iban.as_str()), ("Konto 1", "DE01"));

        // Unavailable: no export, and the fields are left alone.
        let (mut name, mut iban) = (String::from("Fremd"), String::from("DE99"));
        assert_eq!(
            apply_recipient_fields(&mut name, &mut iban, &RecipientRule::Unavailable),
            Err(SAVINGS_NO_REFERENCE.to_string())
        );
        assert_eq!((name.as_str(), iban.as_str()), ("Fremd", "DE99"));
    }

    struct Fake {
        steps: RefCell<Vec<TransferStep>>,
        calls: RefCell<Vec<String>>,
    }
    impl Fake {
        fn new(mut steps: Vec<TransferStep>) -> Self {
            steps.reverse();
            Fake {
                steps: RefCell::new(steps),
                calls: RefCell::new(vec![]),
            }
        }
        fn next(&self, what: &str) -> Result<TransferStep, String> {
            self.calls.borrow_mut().push(what.into());
            Ok(self.steps.borrow_mut().pop().expect("no more steps"))
        }
    }
    impl TransferStepper for Fake {
        fn start(&self) -> Result<TransferStep, String> {
            self.next("start")
        }
        fn submit_code(&self, code: String) -> Result<TransferStep, String> {
            self.next(&format!("code:{code}"))
        }
        fn poll(&self) -> Result<TransferStep, String> {
            self.next("poll")
        }
        fn confirm_payee(&self) -> Result<TransferStep, String> {
            self.next("confirm")
        }
        fn cancel(&self) {
            self.calls.borrow_mut().push("cancel".into());
        }
    }

    struct FakeUi {
        code: Option<&'static str>,
        payee_ok: bool,
    }
    impl Ui for FakeUi {
        fn ask_secret(&self, _: &str) -> Option<Zeroizing<String>> {
            None
        }
        fn ask_code(&self, _: &Challenge) -> Option<Zeroizing<String>> {
            self.code.map(|c| Zeroizing::new(c.to_string()))
        }
        fn wait_push(&self, _: &Challenge, _: Duration) -> bool {
            true
        }
        fn open_url(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }
    impl TransferUi for FakeUi {
        fn confirm_payee(&self, _: &PayeeCheck) -> bool {
            self.payee_ok
        }
    }

    fn challenge(kind: ChallengeKind) -> Challenge {
        Challenge {
            kind,
            title: String::new(),
            message: String::new(),
            method_name: String::new(),
            redirect_url: None,
            image: None,
            expires_in_seconds: None,
            poll_interval_seconds: 1,
        }
    }

    fn done() -> TransferStep {
        TransferStep::Finished {
            outcome: TransferOutcome {
                instant: false,
                end_to_end_id: "GC123".into(),
                record_error: None,
            },
        }
    }

    fn check() -> PayeeCheck {
        PayeeCheck {
            result: PayeeMatch::NoMatch,
            entered_name: "A".into(),
            iban: "DE00".into(),
            bank_name: None,
            reason: None,
            notice: Some("Hinweis der Bank".into()),
        }
    }

    #[test]
    fn tan_is_submitted_trimmed_then_finishes() {
        let s = Fake::new(vec![
            TransferStep::NeedsChallenge {
                challenge: challenge(ChallengeKind::Code),
            },
            done(),
        ]);
        let ui = FakeUi {
            code: Some(" 123456 "),
            payee_ok: false,
        };
        let msg = drive_transfer(&s, &ui).unwrap();
        assert!(msg.contains("GC123"));
        assert_eq!(*s.calls.borrow(), vec!["start", "code:123456"]);
    }

    #[test]
    fn declined_payee_check_cancels_without_sending() {
        let s = Fake::new(vec![TransferStep::NeedsPayeeConfirmation {
            check: check(),
        }]);
        let ui = FakeUi {
            code: None,
            payee_ok: false,
        };
        assert_eq!(drive_transfer(&s, &ui).unwrap_err(), CANCELLED_TRANSFER);
        assert_eq!(*s.calls.borrow(), vec!["start", "cancel"]);
        assert!(payee_text(&check()).contains("NICHT"));
        assert!(payee_text(&check()).contains("Hinweis der Bank"));
    }

    #[test]
    fn confirmed_payee_continues() {
        let s = Fake::new(vec![
            TransferStep::NeedsPayeeConfirmation { check: check() },
            done(),
        ]);
        let ui = FakeUi {
            code: None,
            payee_ok: true,
        };
        assert!(drive_transfer(&s, &ui).is_ok());
        assert_eq!(*s.calls.borrow(), vec!["start", "confirm"]);
    }

    #[test]
    fn cancelled_tan_cancels_and_unknown_outcome_is_never_retried() {
        let s = Fake::new(vec![TransferStep::NeedsChallenge {
            challenge: challenge(ChallengeKind::Code),
        }]);
        let ui = FakeUi {
            code: None,
            payee_ok: false,
        };
        assert!(drive_transfer(&s, &ui).unwrap_err().contains("abgebrochen"));
        assert_eq!(s.calls.borrow().last().unwrap(), "cancel");

        let s = Fake::new(vec![TransferStep::OutcomeUnknown {
            reason: "Verbindung verloren.".into(),
            end_to_end_id: "X".into(),
        }]);
        let e = drive_transfer(&s, &ui).unwrap_err();
        assert!(e.contains("NICHT erneut senden"));
        assert_eq!(*s.calls.borrow(), vec!["start"]);
    }
}

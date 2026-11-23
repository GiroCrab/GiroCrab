use super::*;

fn tr_cash_account() -> RemoteAccount {
    RemoteAccount {
        external_ref: "tr:cash".into(),
        name: "Verrechnungskonto".into(),
        owner: String::new(),
        iban: String::new(),
        account_number: String::new(),
        currency: "EUR".into(),
        kind: AccountKind::Checking,
        balance_cents: 0,
        balance_date: "2025-12-10".into(),
    }
}

fn tr_booking(id: &str, date: &str, cents: i64, counterparty: &str) -> RemoteTransaction {
    RemoteTransaction {
        account_ref: "tr:cash".into(),
        external_id: id.into(),
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents: cents,
        currency: "EUR".into(),
        counterparty: counterparty.into(),
        ..Default::default()
    }
}

/// A PDF whose text carries what the matching reads: event word, amount and a date.
fn tr_pdf(event: &str, subject: &str, amount: &str, date: &str) -> Vec<u8> {
    crate::documents::simple_pdf(&[
        format!("WERTPAPIERABRECHNUNG {event} {subject}"),
        format!("{amount} EUR"),
        date.to_string(),
    ])
}

#[test]
fn an_unstructured_trade_document_with_several_amounts_stays_unlinked() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-tr-ambiguous-amounts");
    let connection = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let pdf = crate::documents::simple_pdf(&[
        format!("WERTPAPIERABRECHNUNG KAUF {subject}"),
        "Kurs 100,00 EUR".into(),
        "Gesamt 500,00 EUR".into(),
        "04.12.2025".into(),
    ]);
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        connection,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("different-buy", "2025-12-04", -100_00, subject)],
            documents: vec![tr_document("ambiguous", subject, pdf, "2025-12-04")],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
}

fn tr_document(
    source: &str,
    subject: &str,
    bytes: Vec<u8>,
    date: &str,
) -> crate::documents::RemoteDocument {
    crate::documents::RemoteDocument {
        account_ref: "tr:cash".into(),
        booking_external_id: None,
        title: format!("Abrechnung \u{2013} {subject}"),
        doc_type: "abrechnung".into(),
        mime_type: "application/pdf".into(),
        file_name: format!("{source}.pdf"),
        bytes,
        source_url: format!("https://example.invalid/{source}"),
        date: date.into(),
    }
}

fn open_vault(tag: &str) -> (crate::test_support::TempDir, std::sync::Arc<Vault>) {
    let dir = crate::test_support::TempDir::new(tag);
    let v = Vault::open(
        dir.join("v.db").to_string_lossy().to_string(),
        vec![8u8; 32],
    )
    .unwrap();
    (dir, v)
}

fn booking_id_of(v: &Vault, external_id: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT id FROM transactions WHERE external_id = ?1",
            [external_id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn a_late_pdf_is_linked_to_an_older_stored_booking_and_stays_put() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-late-pdf");
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    // the booking is stored first, in a fetch without documents
    let first = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![tr_booking("b1", "2025-12-04", -25_00, subject)],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, first).unwrap();
    // the PDF comes later, alone, under another id than the booking
    let pdf = tr_pdf("KAUF", subject, "25,00", "02.12.2025");
    let depot = RemoteAccount {
        external_ref: "tr:depot".into(),
        name: "Depot".into(),
        kind: AccountKind::Brokerage,
        ..tr_cash_account()
    };
    let later = || {
        let document = crate::documents::RemoteDocument {
            account_ref: depot.external_ref.clone(),
            ..tr_document("d1", subject, pdf.clone(), "2025-12-04")
        };
        FetchResult {
            accounts: vec![tr_cash_account(), depot.clone()],
            documents: vec![document],
            ..Default::default()
        }
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, later()).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "b1")));
    let account = Some(account_id_of(&v, "tr:depot"));
    assert_eq!(docs[0].account_id, account);
    // the same delivery again: no second document, the link and the account do not change
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, later()).unwrap();
    let again = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(again.len(), 1);
    assert_eq!(
        (again[0].booking_id, again[0].account_id),
        (docs[0].booking_id, account)
    );
}

#[test]
fn a_purchase_pdf_is_not_linked_to_the_credit_of_an_equal_sale() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-pdf-direction");
    let conn = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "login".into())
        .unwrap();
    let data = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![
            // the sale's credit is on the very day of the PDF, the purchase's debit two days later
            tr_booking("sale", "2025-12-02", 25_00, subject),
            tr_booking("buy", "2025-12-04", -25_00, subject),
        ],
        documents: vec![tr_document(
            "d2",
            subject,
            tr_pdf("KAUF", subject, "25,00", "02.12.2025"),
            "2025-12-02",
        )],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, conn, data).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "buy")));
}

#[test]
fn a_trade_republic_security_name_does_not_determine_the_booking_direction() {
    let subject = "Dividenden ETF";
    let (_dir, v) = open_vault("girocrab-tr-event-not-name");
    let kind = ConnectorKind::TradeRepublic;
    let conn = v
        .add_bank_connection(kind, String::new(), "login".into())
        .unwrap();
    let data = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![
            tr_booking("credit", "2025-12-04", 25_00, subject),
            tr_booking("purchase", "2025-12-04", -25_00, subject),
        ],
        documents: vec![tr_document(
            "receipt",
            subject,
            tr_pdf("KAUF", subject, "25,00", "04.12.2025"),
            "2025-12-04",
        )],
        ..Default::default()
    };
    store_fetch(&v, kind, conn, data).unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(booking_id_of(&v, "purchase"))
    );
}

#[test]
fn trade_republic_requires_the_complete_security_name_after_the_separator() {
    let (_dir, v) = open_vault("girocrab-tr-full-security-name");
    let kind = ConnectorKind::TradeRepublic;
    let conn = v
        .add_bank_connection(kind, String::new(), "login".into())
        .unwrap();
    let subject = "ACME – Global Equity";
    let data = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![
            tr_booking("wrong", "2025-12-04", -25_00, "ACME Short Bond"),
            tr_booking("correct", "2025-12-04", -25_00, subject),
        ],
        documents: vec![tr_document(
            "receipt",
            subject,
            tr_pdf("KAUF", subject, "25,00", "04.12.2025"),
            "2025-12-04",
        )],
        ..Default::default()
    };
    store_fetch(&v, kind, conn, data).unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(booking_id_of(&v, "correct"))
    );
}

#[test]
fn a_manual_link_and_another_login_are_left_alone() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-pdf-isolation");
    let a = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "a".into())
        .unwrap();
    let b = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "b".into())
        .unwrap();
    // login b owns the only matching booking
    let other = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![tr_booking("only-b", "2025-12-04", -25_00, subject)],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, b, other).unwrap();
    // login a gets the PDF: nothing in its own accounts matches, so it stays unlinked
    let pdf = tr_pdf("KAUF", subject, "25,00", "02.12.2025");
    let mine = FetchResult {
        accounts: vec![tr_cash_account()],
        documents: vec![tr_document("d3", subject, pdf, "2025-12-04")],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, a, mine).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(
        docs[0].booking_id, None,
        "a booking of another login is no candidate"
    );
    // the user links it by hand to a booking of login a; a later fetch keeps that choice
    let own = FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![
            tr_booking("mine-1", "2025-12-04", -25_00, subject),
            tr_booking("mine-2", "2025-12-20", -99_00, "Etwas anderes"),
        ],
        ..Default::default()
    };
    store_fetch(&v, ConnectorKind::TradeRepublic, a, own).unwrap();
    let manual = booking_id_of(&v, "mine-2");
    v.link_document(docs[0].id, Some(manual)).unwrap();
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        a,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("mine-1", "2025-12-04", -25_00, subject)],
            ..Default::default()
        },
    )
    .unwrap();
    let kept = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(
        kept[0].booking_id,
        Some(manual),
        "a manual link is never replaced"
    );
}

#[test]
fn a_repeat_fetch_leaves_a_manually_unlinked_document_open() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-manual-unlink");
    let kind = ConnectorKind::TradeRepublic;
    let conn = v
        .add_bank_connection(kind, String::new(), "login".into())
        .unwrap();
    let fetch = || FetchResult {
        accounts: vec![tr_cash_account()],
        transactions: vec![tr_booking("original", "2025-12-04", -25_00, subject)],
        documents: vec![tr_document(
            "receipt",
            subject,
            tr_pdf("KAUF", subject, "25,00", "04.12.2025"),
            "2025-12-04",
        )],
        ..Default::default()
    };
    store_fetch(&v, kind, conn, fetch()).unwrap();
    let doc = v.list_all_documents(Default::default()).unwrap().remove(0);
    assert_eq!(doc.booking_id, Some(booking_id_of(&v, "original")));

    v.link_document(doc.id, None).unwrap();
    store_fetch(&v, kind, conn, fetch()).unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );

    // The old best match still takes part in ranking and blocks the new neighbour.
    let mut next = fetch();
    next.transactions
        .push(tr_booking("nearby", "2025-12-05", -25_00, subject));
    store_fetch(&v, kind, conn, next).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].booking_id, None);
    assert_eq!(docs[0].account_id, doc.account_id);
}

#[test]
fn a_closer_booking_does_not_relink_a_manually_unlinked_document() {
    let subject = "JPM USD EM Bond USD (Dist)";
    let (_dir, v) = open_vault("girocrab-manual-unlink-closer");
    let kind = ConnectorKind::TradeRepublic;
    let conn = v
        .add_bank_connection(kind, String::new(), "login".into())
        .unwrap();
    let pdf = tr_pdf("KAUF", subject, "25,00", "04.12.2025");
    store_fetch(
        &v,
        kind,
        conn,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("dec6", "2025-12-06", -25_00, subject)],
            documents: vec![tr_document("receipt", subject, pdf.clone(), "2025-12-04")],
            ..Default::default()
        },
    )
    .unwrap();
    let doc = v.list_all_documents(Default::default()).unwrap().remove(0);
    assert_eq!(doc.booking_id, Some(booking_id_of(&v, "dec6")));

    // User manually detaches the PDF; a later sync brings an exact-date Dec 4 booking.
    v.link_document(doc.id, None).unwrap();
    store_fetch(
        &v,
        kind,
        conn,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![
                tr_booking("dec6", "2025-12-06", -25_00, subject),
                tr_booking("dec4", "2025-12-04", -25_00, subject),
            ],
            documents: vec![tr_document("receipt", subject, pdf, "2025-12-04")],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );

    // Explicit manual re-linking still works and clears the unlink marker.
    let dec4 = booking_id_of(&v, "dec4");
    v.link_document(doc.id, Some(dec4)).unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(dec4)
    );
}

// ---- comdirect: trade documents belong to the depot and document a booking on the Girokonto ----

const CD_GIRO: &str = "DE02200411550123456700";
const CD_GIRO2: &str = "DE75200411550765432100";
const CD_DEPOT: &str = "123456700";
const CD_ISIN: &str = "IE00B3RBWM25";

fn cd_account(external_ref: &str, kind: AccountKind) -> RemoteAccount {
    RemoteAccount {
        external_ref: external_ref.into(),
        name: external_ref.into(),
        owner: String::new(),
        iban: if kind == AccountKind::Checking {
            external_ref.into()
        } else {
            String::new()
        },
        // the depot is named by its number in the documents
        account_number: if kind == AccountKind::Brokerage {
            external_ref.into()
        } else {
            String::new()
        },
        currency: "EUR".into(),
        kind,
        balance_cents: 0,
        balance_date: "2025-12-10".into(),
    }
}

fn cd_accounts(two_girokonten: bool) -> Vec<RemoteAccount> {
    let mut all = vec![
        cd_account(CD_GIRO, AccountKind::Checking),
        cd_account(CD_DEPOT, AccountKind::Brokerage),
    ];
    if two_girokonten {
        all.push(cd_account(CD_GIRO2, AccountKind::Checking));
    }
    all
}

fn cd_booking(
    account: &str,
    id: &str,
    date: &str,
    cents: i64,
    currency: &str,
    purpose: &str,
) -> RemoteTransaction {
    RemoteTransaction {
        account_ref: account.into(),
        external_id: id.into(),
        booking_date: date.into(),
        value_date: date.into(),
        amount_cents: cents,
        currency: currency.into(),
        counterparty: "comdirect".into(),
        // what a trade booking names: the security by its ISIN
        purpose: format!("{purpose} {CD_ISIN}"),
        ..Default::default()
    }
}

/// A trade confirmation as text: event word, ISIN, amount with currency and a date.
fn cd_pdf(event: &str, amount: &str, date: &str, currency: &str) -> Vec<u8> {
    crate::documents::simple_pdf(&[
        format!("GESCHAEFTSABRECHNUNG {event}"),
        format!("Wertpapier ISIN {CD_ISIN}"),
        format!("Ausmachender Betrag {amount} {currency}"),
        format!("Geschaeftstag {date}"),
    ])
}

/// Like `cd_pdf`, with the settlement account the confirmation names.
fn cd_pdf_for(iban: &str, event: &str, amount: &str, date: &str) -> Vec<u8> {
    crate::documents::simple_pdf(&[
        format!("GESCHAEFTSABRECHNUNG {event}"),
        format!("Verrechnungskonto IBAN {iban}"),
        format!("Wertpapier ISIN {CD_ISIN}"),
        format!("Ausmachender Betrag {amount} EUR"),
        format!("Geschaeftstag {date}"),
    ])
}

fn cd_document(
    source: &str,
    account_ref: &str,
    title: &str,
    doc_type: &str,
    bytes: Vec<u8>,
    date: &str,
) -> crate::documents::RemoteDocument {
    crate::documents::RemoteDocument {
        account_ref: account_ref.into(),
        booking_external_id: None,
        title: title.into(),
        doc_type: doc_type.into(),
        mime_type: "application/pdf".into(),
        file_name: format!("{source}.pdf"),
        bytes,
        source_url: format!("comdirect:postbox:{source}"),
        date: date.into(),
    }
}

fn cd_purchase_title() -> &'static str {
    "Wertpapierabrechnung Kauf 12,5 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025"
}

fn account_id_of(v: &Vault, external_ref: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT id FROM accounts WHERE external_ref = ?1",
            [external_ref],
            |r| r.get(0),
        )
        .unwrap()
}

/// A comdirect connection: the REST kind, or a FinTS login (kind `FinTsGeneric`, key with a comdirect BLZ).
fn cd_connection(v: &Vault, fints: bool, login: &str) -> (ConnectorKind, i64) {
    if fints {
        let (id, _) = Vault::add_bank_connection_keyed(
            &v.db().unwrap(),
            ConnectorKind::FinTsGeneric,
            "fints:20041155",
            Some("20041155"),
            String::new(),
            login.into(),
        )
        .unwrap();
        (ConnectorKind::FinTsGeneric, id)
    } else {
        let id = v
            .add_bank_connection(ConnectorKind::Comdirect, String::new(), login.into())
            .unwrap();
        (ConnectorKind::Comdirect, id)
    }
}

fn cd_fetch(
    two_girokonten: bool,
    transactions: Vec<RemoteTransaction>,
    documents: Vec<crate::documents::RemoteDocument>,
) -> FetchResult {
    FetchResult {
        accounts: cd_accounts(two_girokonten),
        transactions,
        documents,
        ..Default::default()
    }
}

#[test]
fn comdirect_uses_pdf_dates_even_when_the_postbox_date_is_missing_or_far_away() {
    for (postbox_date, receipt_first) in [
        ("", false),
        ("", true),
        ("2025-12-20", false),
        ("2025-12-20", true),
    ] {
        let (_dir, v) = open_vault("girocrab-cd-undated-receipt");
        let (kind, conn) = cd_connection(&v, false, "login");
        let booking = cd_booking(
            CD_GIRO,
            "trade",
            "2025-12-04",
            -100_00,
            "EUR",
            "Wertpapierkauf",
        );
        let receipt = cd_document(
            "receipt",
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
            postbox_date,
        );
        let with_booking = cd_fetch(false, vec![booking], vec![]);
        let with_receipt = cd_fetch(false, vec![], vec![receipt]);
        let batches = if receipt_first {
            [with_receipt, with_booking]
        } else {
            [with_booking, with_receipt]
        };
        for batch in batches {
            store_fetch(&v, kind, conn, batch).unwrap();
        }
        let docs = v.list_all_documents(Default::default()).unwrap();
        assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "trade")));
        assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));
    }
}

#[test]
fn a_stored_booking_can_tie_on_a_pdf_date_outside_the_postbox_period() {
    let (_dir, v) = open_vault("girocrab-cd-pdf-date-tie");
    let (kind, conn) = cd_connection(&v, false, "login");
    let booking = |id, date| cd_booking(CD_GIRO, id, date, -100_00, "EUR", "Wertpapierkauf");
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(false, vec![booking("old", "2025-11-19")], vec![]),
    )
    .unwrap();
    let receipt = cd_document(
        "receipt",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        cd_pdf(
            "Wertpapierkauf",
            "100,00",
            "19.11.2025 und 10.12.2025",
            "EUR",
        ),
        "2025-12-10",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(false, vec![booking("new", "2025-12-10")], vec![receipt]),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
}

#[test]
fn a_comdirect_receipt_cannot_link_to_an_archived_settlement_account() {
    let (_dir, v) = open_vault("girocrab-cd-archived-settlement");
    let (kind, conn) = cd_connection(&v, false, "login");
    let booking =
        |account, id| cd_booking(account, id, "2025-12-04", -100_00, "EUR", "Wertpapierkauf");
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(true, vec![booking(CD_GIRO, "archived")], vec![]),
    )
    .unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE accounts SET archived_unix = 1 WHERE external_ref = ?1",
            [CD_GIRO],
        )
        .unwrap();

    let receipt = |source, account| {
        cd_document(
            source,
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            cd_pdf_for(account, "Wertpapierkauf", "100,00", "04.12.2025"),
            "2025-12-04",
        )
    };
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![booking(CD_GIRO2, "active")],
            vec![receipt("archived", CD_GIRO), receipt("active", CD_GIRO2)],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 2);
    let link = |suffix: &str| {
        docs.iter()
            .find(|doc| doc.source_url.ends_with(suffix))
            .unwrap()
            .booking_id
    };
    assert_eq!(link(":archived"), None);
    assert_eq!(link(":active"), Some(booking_id_of(&v, "active")));
}

#[test]
fn a_comdirect_receipt_uses_the_settlement_currency_when_the_price_has_the_same_amount() {
    let (_dir, v) = open_vault("girocrab-cd-settlement-currency");
    let (kind, conn) = cd_connection(&v, false, "login");
    let booking = |id, currency| {
        cd_booking(
            CD_GIRO,
            id,
            "2025-12-04",
            -100_00,
            currency,
            "Wertpapierkauf",
        )
    };
    let receipt = cd_document(
        "receipt",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        crate::documents::simple_pdf(&[
            format!("Wertpapierkauf {CD_ISIN}"),
            "Ausmachender Betrag 100,00 EUR Kurswert 100,00 USD".into(),
            "Geschaeftstag 04.12.2025".into(),
        ]),
        "2025-12-04",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(false, vec![booking("usd", "USD")], vec![receipt]),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(false, vec![booking("eur", "EUR")], vec![]),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(booking_id_of(&v, "eur"))
    );
}

#[test]
fn comdirect_does_not_use_the_sole_account_when_a_different_account_is_named() {
    for account_line in [
        "Verrechnungskonto Nr. 765432100",
        "Verrechnungskonto IBAN DE75 2004 1155 0765 4321 00",
    ] {
        let (_dir, v) = open_vault("girocrab-cd-unknown-account");
        let (kind, conn) = cd_connection(&v, false, "login");
        let receipt = cd_document(
            "receipt",
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            crate::documents::simple_pdf(&[
                format!("Wertpapierkauf {CD_ISIN}"),
                account_line.into(),
                "Ausmachender Betrag 100,00 EUR".into(),
                "Geschaeftstag 04.12.2025".into(),
            ]),
            "2025-12-04",
        );
        let booking = cd_booking(
            CD_GIRO,
            "other",
            "2025-12-04",
            -100_00,
            "EUR",
            "Wertpapierkauf",
        );
        store_fetch(
            &v,
            kind,
            conn,
            cd_fetch(false, vec![booking], vec![receipt]),
        )
        .unwrap();
        let docs = v.list_all_documents(Default::default()).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].booking_id, None, "{account_line}");
    }
}

#[test]
fn comdirect_matches_a_stored_receipt_when_it_gets_its_first_account() {
    let (_dir, v) = open_vault("girocrab-cd-late-account");
    let (kind, conn) = cd_connection(&v, false, "login");
    let booking = cd_booking(
        CD_GIRO,
        "trade",
        "2025-12-04",
        -100_00,
        "EUR",
        "Wertpapierkauf",
    );
    let mut receipt = cd_document(
        "receipt",
        "",
        cd_purchase_title(),
        "abrechnung",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-04",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(false, vec![booking], vec![receipt.clone()]),
    )
    .unwrap();
    let doc = v.list_all_documents(Default::default()).unwrap().remove(0);
    assert_eq!((doc.account_id, doc.booking_id), (None, None));

    // The later PostBox delivery identifies the depot without re-downloading the PDF.
    receipt.account_ref = CD_DEPOT.into();
    receipt.bytes.clear();
    receipt.date.clear();
    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![receipt])).unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].id, doc.id);
    assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "trade")));
}

#[test]
fn comdirect_matches_a_stored_receipt_to_an_adopted_moneymoney_booking() {
    let (_dir, v) = open_vault("girocrab-cd-adopted-booking");
    let (kind, conn) = cd_connection(&v, false, "login");
    let receipt = cd_document(
        "receipt",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-04",
    );
    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![receipt])).unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );

    let booking = cd_booking(
        CD_GIRO,
        "trade",
        "2025-12-04",
        -100_00,
        "EUR",
        "Wertpapierkauf",
    );
    let csv = format!(
        "Datum;Name;Verwendungszweck;Betrag\n04.12.2025;{};{};-100,00\n",
        booking.counterparty, booking.purpose
    );
    let giro = account_id_of(&v, CD_GIRO);
    let imported = v
        .import_moneymoney_data(csv.into_bytes(), "test.csv".into(), Some(giro), true, true)
        .unwrap();
    assert_eq!(imported.imported, 1);
    let imported_id: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT id FROM transactions WHERE account_id = ?1 AND external_id LIKE 'mm-%'",
            [giro],
            |row| row.get(0),
        )
        .unwrap();

    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![])).unwrap();
    assert_eq!(booking_id_of(&v, "trade"), imported_id);
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(imported_id)
    );
}

#[test]
fn comdirect_links_the_receipt_only_after_the_booking_posts() {
    for (fints, prefix) in [
        (false, "pending-hold-comdirect-"),
        (true, "pending-hold-fints-"),
    ] {
        let (_dir, v) = open_vault("girocrab-cd-hold");
        let (kind, conn) = cd_connection(&v, fints, "login");
        let hold_id = format!("{prefix}trade");
        let booking = |id| cd_booking(CD_GIRO, id, "2025-12-04", -100_00, "EUR", "Wertpapierkauf");
        let receipt = cd_document(
            "receipt",
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
            "2025-12-04",
        );
        let mut pending = cd_fetch(false, vec![booking(&hold_id)], vec![receipt]);
        pending.hold_prefix = Some(prefix.into());
        store_fetch(&v, kind, conn, pending).unwrap();
        assert_eq!(
            v.list_all_documents(Default::default()).unwrap()[0].booking_id,
            None
        );

        let mut posted = cd_fetch(false, vec![booking("posted")], vec![]);
        posted.hold_prefix = Some(prefix.into());
        store_fetch(&v, kind, conn, posted).unwrap();
        let hold_deleted: bool = v
            .db()
            .unwrap()
            .query_row(
                "SELECT deleted_ms IS NOT NULL FROM transactions WHERE external_id = ?1",
                [&hold_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(hold_deleted);
        let docs = v.list_all_documents(Default::default()).unwrap();
        assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "posted")));
        assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));
    }
}

#[test]
fn a_comdirect_trade_document_stays_with_the_depot_and_documents_the_girokonto_booking_in_either_order()
 {
    for fints in [false, true] {
        // 0: both in one fetch, 1: the booking first, 2: the PDF first
        for order in 0..3 {
            let (_dir, v) = open_vault("girocrab-cd-order");
            let (kind, conn) = cd_connection(&v, fints, "login");
            let booking = cd_booking(
                CD_GIRO,
                "b1",
                "2025-12-04",
                -801_50,
                "EUR",
                "Wertpapierkauf",
            );
            let doc = cd_document(
                "d1",
                CD_DEPOT,
                cd_purchase_title(),
                "abrechnung",
                cd_pdf("Wertpapierkauf", "801,50", "04.12.2025", "EUR"),
                "2025-12-05",
            );
            match order {
                0 => {
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![doc])).unwrap();
                }
                1 => {
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![])).unwrap();
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![doc])).unwrap();
                }
                _ => {
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![doc])).unwrap();
                    store_fetch(&v, kind, conn, cd_fetch(false, vec![booking], vec![])).unwrap();
                }
            }
            let docs = v.list_all_documents(Default::default()).unwrap();
            assert_eq!(docs.len(), 1, "fints={fints} order={order}");
            // the document keeps the depot, the booking is the one on the Girokonto
            assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));
            assert_eq!(
                docs[0].booking_id,
                Some(booking_id_of(&v, "b1")),
                "fints={fints} order={order}"
            );
            // fetching it again changes nothing and creates no second document
            let again_doc = cd_document(
                "d1",
                CD_DEPOT,
                cd_purchase_title(),
                "abrechnung",
                cd_pdf("Wertpapierkauf", "801,50", "04.12.2025", "EUR"),
                "2025-12-05",
            );
            store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![again_doc])).unwrap();
            let after = v.list_all_documents(Default::default()).unwrap();
            assert_eq!(after.len(), 1);
            assert_eq!(
                (after[0].booking_id, after[0].account_id),
                (docs[0].booking_id, docs[0].account_id)
            );
        }
    }
}

#[test]
fn comdirect_content_matching_leaves_duplicate_pdfs_and_nearby_bookings_alone() {
    let (_dir, v) = open_vault("girocrab-cd-duplicate-pdf");
    let (kind, conn) = cd_connection(&v, false, "login");
    let document = |key| {
        cd_document(
            key,
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
            "2025-12-04",
        )
    };
    let booking = |id, date| cd_booking(CD_GIRO, id, date, -100_00, "EUR", "Wertpapierkauf");
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![booking("original", "2025-12-04")],
            vec![document("first"), document("duplicate")],
        ),
    )
    .unwrap();
    let stored_docs = || v.list_all_documents(Default::default()).unwrap();
    let count_links = |docs: &[crate::documents::DocumentInfo]| {
        docs.iter().filter(|d| d.booking_id.is_some()).count()
    };
    let docs = stored_docs();
    assert_eq!(docs.len(), 2);
    for source in ["comdirect:postbox:first", "comdirect:postbox:duplicate"] {
        assert!(docs.iter().any(|doc| doc.source_url == source));
    }
    assert_eq!(
        count_links(&docs),
        1,
        "the first link blocks a second one in the same pass"
    );

    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![booking("next", "2025-12-06")],
            vec![document("duplicate-later")],
        ),
    )
    .unwrap();
    let docs = stored_docs();
    assert_eq!(docs.len(), 3);
    assert_eq!(
        docs.iter()
            .find(|doc| doc.source_url == "comdirect:postbox:duplicate-later")
            .unwrap()
            .booking_id,
        None
    );
    assert_eq!(
        count_links(&docs),
        1,
        "an occupied best match blocks the fallback"
    );
}

#[test]
fn comdirect_leaves_ambiguous_wrong_way_and_wrong_currency_candidates_alone() {
    let (_dir, v) = open_vault("girocrab-cd-ambiguous");
    let (kind, conn) = cd_connection(&v, false, "login");
    let pdf = cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR");
    let doc = |source: &str| {
        cd_document(
            source,
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            pdf.clone(),
            "2025-12-05",
        )
    };
    // the same debit on two Girokonten: nothing tells them apart
    let twin = |account: &str, id: &str| {
        cd_booking(account, id, "2025-12-04", -100_00, "EUR", "Wertpapierkauf")
    };
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![twin(CD_GIRO, "t1"), twin(CD_GIRO2, "t2")],
            vec![doc("d1")],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs[0].booking_id, None, "ambiguous: left unlinked");
    assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));

    // a credit of the same amount is no candidate for a purchase, a USD booking none for a EUR document
    let (_dir2, v2) = open_vault("girocrab-cd-direction");
    let (kind2, conn2) = cd_connection(&v2, false, "login");
    store_fetch(
        &v2,
        kind2,
        conn2,
        cd_fetch(
            false,
            vec![
                cd_booking(
                    CD_GIRO,
                    "credit",
                    "2025-12-04",
                    100_00,
                    "EUR",
                    "Wertpapierverkauf",
                ),
                cd_booking(
                    CD_GIRO,
                    "usd",
                    "2025-12-04",
                    -100_00,
                    "USD",
                    "Wertpapierkauf",
                ),
            ],
            vec![doc("d2")],
        ),
    )
    .unwrap();
    assert_eq!(
        v2.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
    // a sale confirmation does take the credit
    store_fetch(
        &v2,
        kind2,
        conn2,
        cd_fetch(
            false,
            vec![],
            vec![cd_document(
                "d3",
                CD_DEPOT,
                "Wertpapierabrechnung Verkauf 12,5 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025",
                "abrechnung",
                cd_pdf("Wertpapierverkauf", "100,00", "04.12.2025", "EUR"),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    let sale = v2
        .list_all_documents(Default::default())
        .unwrap()
        .into_iter()
        .find(|d| d.title.contains("Verkauf"))
        .unwrap();
    assert_eq!(sale.booking_id, Some(booking_id_of(&v2, "credit")));
}

#[test]
fn comdirect_period_documents_and_other_logins_get_no_booking_and_a_manual_link_stays() {
    let (_dir, v) = open_vault("girocrab-cd-period");
    let (kind, a) = cd_connection(&v, false, "a");
    let (_, b) = cd_connection(&v, false, "b");
    let amount = "100,00";
    // login b owns a matching booking (on an account of its own)
    store_fetch(
        &v,
        kind,
        b,
        FetchResult {
            accounts: vec![cd_account(CD_GIRO2, AccountKind::Checking)],
            transactions: vec![cd_booking(
                CD_GIRO2,
                "only-b",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            ..Default::default()
        },
    )
    .unwrap();
    let pdf = cd_pdf("Wertpapierkauf", amount, "04.12.2025", "EUR");
    let finanzreport = cd_document(
        "report",
        CD_DEPOT,
        "Finanzreport Nr. 12 per 31.12.2025",
        "kontoauszug",
        pdf.clone(),
        "2025-12-05",
    );
    let tax = cd_document(
        "tax",
        CD_DEPOT,
        "Steuermitteilung WKN A1JX52 vom 05.12.2025",
        "steuer",
        pdf.clone(),
        "2025-12-05",
    );
    let trade = cd_document(
        "trade",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        pdf,
        "2025-12-05",
    );
    store_fetch(
        &v,
        kind,
        a,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "mine-1",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![finanzreport, tax, trade],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 3);
    for d in &docs {
        if d.title.starts_with("Finanzreport") || d.title.starts_with("Steuermitteilung") {
            assert_eq!(d.booking_id, None, "{}: a period document", d.title);
        } else {
            // its own login's booking, not the one of login b
            assert_eq!(d.booking_id, Some(booking_id_of(&v, "mine-1")));
        }
    }
    // a subsequent fetch keeps a manual link to a different booking
    store_fetch(
        &v,
        kind,
        a,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "other",
                "2025-12-20",
                -5_00,
                "EUR",
                "Etwas anderes",
            )],
            vec![],
        ),
    )
    .unwrap();
    let trade_id = docs.iter().find(|d| d.title.contains("Kauf")).unwrap().id;
    let manual = booking_id_of(&v, "other");
    v.link_document(trade_id, Some(manual)).unwrap();
    store_fetch(
        &v,
        kind,
        a,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "mine-1",
                "2025-12-04",
                -100_00,
                "EUR",
                "VANGUARD TEST FONDS",
            )],
            vec![],
        ),
    )
    .unwrap();
    let kept = v
        .list_all_documents(Default::default())
        .unwrap()
        .into_iter()
        .find(|d| d.id == trade_id)
        .unwrap();
    assert_eq!(kept.booking_id, Some(manual));
}

#[test]
fn a_comdirect_document_goes_to_the_girokonto_it_names_or_stays_open() {
    let (_dir, v) = open_vault("girocrab-cd-accounts");
    let (kind, conn) = cd_connection(&v, false, "login");
    let twin = |account: &str, id: &str, date: &str| {
        cd_booking(account, id, date, -100_00, "EUR", "Wertpapierkauf")
    };
    // the same purchase on both Girokonten; the wrong one is nearer to the date in the PDF
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![
                twin(CD_GIRO, "wrong", "2025-12-04"),
                twin(CD_GIRO2, "right", "2025-12-08"),
            ],
            vec![cd_document(
                "named",
                CD_DEPOT,
                cd_purchase_title(),
                "abrechnung",
                cd_pdf_for(CD_GIRO2, "Wertpapierkauf", "100,00", "04.12.2025"),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs[0].booking_id, Some(booking_id_of(&v, "right")));
    assert_eq!(docs[0].account_id, Some(account_id_of(&v, CD_DEPOT)));

    // a second confirmation names no account: with two Girokonten that is no evidence, it stays open
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![twin(CD_GIRO, "only-here", "2025-12-04")],
            vec![cd_document(
                "unnamed",
                CD_DEPOT,
                "Wertpapierabrechnung Kauf 3 St. WKN A1JX52 (VANGUARD TEST FONDS) vom 04.12.2025",
                "abrechnung",
                cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
                "2025-12-05",
            )],
        ),
    )
    .unwrap();
    let open = v
        .list_all_documents(Default::default())
        .unwrap()
        .into_iter()
        .find(|d| d.title.contains("3 St."))
        .unwrap();
    assert_eq!(open.booking_id, None);
}

#[test]
fn a_tied_isin_match_blocks_a_weaker_name_match() {
    let (_dir, v) = open_vault("girocrab-cd-tied-isin");
    let (kind, conn) = cd_connection(&v, false, "login");
    let name_only = RemoteTransaction {
        purpose: "VANGUARD TEST FONDS".into(),
        ..cd_booking(CD_GIRO, "name-only", "2025-12-04", -100_00, "EUR", "")
    };
    let isin_only = |id| cd_booking(CD_GIRO, id, "2025-12-04", -100_00, "EUR", "Wertpapierkauf");
    let document = cd_document(
        "d1",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-04",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![isin_only("isin1"), isin_only("isin2"), name_only],
            vec![document],
        ),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
}

#[test]
fn a_missing_isin_match_can_fall_back_to_wkn_or_name() {
    for purpose in ["WKN A1JX52", "VANGUARD TEST FONDS"] {
        let (_dir, v) = open_vault("girocrab-cd-identifier-fallback");
        let (kind, conn) = cd_connection(&v, false, "login");
        let booking = RemoteTransaction {
            purpose: purpose.into(),
            ..cd_booking(CD_GIRO, "trade", "2025-12-04", -100_00, "EUR", "")
        };
        let document = cd_document(
            "d1",
            CD_DEPOT,
            cd_purchase_title(),
            "abrechnung",
            cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
            "2025-12-04",
        );
        store_fetch(
            &v,
            kind,
            conn,
            cd_fetch(false, vec![booking], vec![document]),
        )
        .unwrap();
        assert_eq!(
            v.list_all_documents(Default::default()).unwrap()[0].booking_id,
            Some(booking_id_of(&v, "trade")),
            "{purpose}"
        );
    }
}

#[test]
fn two_named_girokonten_remain_ambiguous_even_with_a_closer_booking() {
    let (_dir, v) = open_vault("girocrab-cd-two-named-accounts");
    let (kind, conn) = cd_connection(&v, false, "login");
    let pdf = cd_pdf_for(
        &format!("{CD_GIRO} {CD_GIRO2}"),
        "Wertpapierkauf",
        "100,00",
        "04.12.2025",
    );
    let document = cd_document(
        "d1",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        pdf,
        "2025-12-04",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            true,
            vec![
                cd_booking(
                    CD_GIRO,
                    "closer",
                    "2025-12-04",
                    -100_00,
                    "EUR",
                    "Wertpapierkauf",
                ),
                cd_booking(
                    CD_GIRO2,
                    "farther",
                    "2025-12-05",
                    -100_00,
                    "EUR",
                    "Wertpapierkauf",
                ),
            ],
            vec![document],
        ),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
}

#[test]
fn comdirect_information_and_cost_documents_never_take_a_booking() {
    let (_dir, v) = open_vault("girocrab-cd-info");
    let (kind, conn) = cd_connection(&v, false, "login");
    // a cost information names the ISIN and an amount, a date: the same traits as a trade confirmation
    let cost = cd_document(
        "cost",
        CD_DEPOT,
        "Kosteninformation VANGUARD TEST FONDS WKN A1JX52 vom 04.12.2025",
        "postbox",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-05",
    );
    // an unclassified document with a trade's words, and a dividend notice without a direction word
    let notice = cd_document(
        "notice",
        CD_DEPOT,
        "Mitteilung WKN A1JX52 (VANGUARD TEST FONDS)",
        "postbox",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-05",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "b",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![cost, notice],
        ),
    )
    .unwrap();
    let docs = v.list_all_documents(Default::default()).unwrap();
    assert_eq!(docs.len(), 2);
    for d in docs {
        assert_eq!(d.booking_id, None, "{}", d.title);
        assert_eq!(d.account_id, Some(account_id_of(&v, CD_DEPOT)));
    }
}

#[test]
fn comdirect_links_only_the_settled_amount_when_a_pre_tax_amount_also_appears() {
    let (_dir, v) = open_vault("girocrab-cd-settled");
    let (kind, conn) = cd_connection(&v, false, "login");
    // the confirmation settles 500,00; a 100,00 pre-tax booking of the same security exists, the 500,00 one does not
    let pdf = crate::documents::simple_pdf(&[
        "GESCHAEFTSABRECHNUNG Wertpapierkauf".to_string(),
        format!("Wertpapier ISIN {CD_ISIN}"),
        "Kurs 100,00 EUR".to_string(),
        "Provision 1,50 EUR".to_string(),
        "Zu Ihren Lasten vor Steuern 100,00 EUR".to_string(),
        "Ausmachender Betrag 500,00 EUR".to_string(),
        "Geschaeftstag 04.12.2025".to_string(),
    ]);
    let doc = cd_document(
        "settled",
        CD_DEPOT,
        cd_purchase_title(),
        "abrechnung",
        pdf.clone(),
        "2025-12-05",
    );
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "pre-tax",
                "2025-12-04",
                -100_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![doc],
        ),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        None
    );
    // the booking of the settled amount arrives later: now it is linked
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![cd_booking(
                CD_GIRO,
                "settled",
                "2025-12-04",
                -500_00,
                "EUR",
                "Wertpapierkauf",
            )],
            vec![],
        ),
    )
    .unwrap();
    assert_eq!(
        v.list_all_documents(Default::default()).unwrap()[0].booking_id,
        Some(booking_id_of(&v, "settled"))
    );
}

mod resort;

#[test]
fn later_fetches_do_not_reparse_unmatchable_or_distant_open_pdfs() {
    let subject = "Dividenden ETF";
    let (_dir, v) = open_vault("girocrab-pdf-match-cache");
    let kind = ConnectorKind::TradeRepublic;
    let conn = v
        .add_bank_connection(kind, String::new(), "login".into())
        .unwrap();
    let amb = crate::documents::simple_pdf(&[
        "WERTPAPIERABRECHNUNG".into(),
        "Dividenden".into(),
        "ETF".into(),
        "KAUF".into(),
        "Kurs 10,00 EUR".into(),
        "Gesamt 25,00 EUR".into(),
        "04.12.2025".into(),
    ]);
    let val = crate::documents::simple_pdf(&[
        "WERTPAPIERABRECHNUNG".into(),
        "Dividenden".into(),
        "ETF".into(),
        "KAUF".into(),
        "25,00 EUR".into(),
        "04.12.2025".into(),
    ]);
    store_fetch(
        &v,
        kind,
        conn,
        FetchResult {
            accounts: vec![tr_cash_account()],
            documents: vec![
                tr_document("amb", subject, amb, "2025-12-04"),
                tr_document("val", subject, val.clone(), "2025-12-04"),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    v.db()
        .unwrap()
        .execute_batch("UPDATE booking_documents SET pdf_bytes = zeroblob(length(pdf_bytes));")
        .unwrap();
    store_fetch(
        &v,
        kind,
        conn,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("far", "2025-12-20", -25_00, subject)],
            ..Default::default()
        },
    )
    .unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE booking_documents SET pdf_bytes = ?1 WHERE file_name = 'val.pdf'",
            [val],
        )
        .unwrap();
    store_fetch(
        &v,
        kind,
        conn,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![
                tr_booking("cr", "2025-12-04", 25_00, subject),
                tr_booking("buy", "2025-12-04", -25_00, subject),
            ],
            ..Default::default()
        },
    )
    .unwrap();
    let linked: Option<i64> = v
        .db()
        .unwrap()
        .query_row(
            "SELECT booking_id FROM booking_documents WHERE file_name = 'val.pdf'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(linked, Some(booking_id_of(&v, "buy")));
}

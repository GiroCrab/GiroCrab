use super::*;

#[test]
fn resort_refiles_comdirect_receipts_and_matches_open_ones_without_replacing_links() {
    let (_dir, v) = open_vault("girocrab-resort-comdirect");
    let (kind, conn) = cd_connection(&v, false, "login");
    let booking = |id, cents| cd_booking(CD_GIRO, id, "2025-12-04", cents, "EUR", "Wertpapierkauf");
    store_fetch(
        &v,
        kind,
        conn,
        cd_fetch(
            false,
            vec![booking("old", -200_00), booking("open", -100_00)],
            vec![],
        ),
    )
    .unwrap();
    let existing = booking_id_of(&v, "old");
    let giro = account_id_of(&v, CD_GIRO);
    let depot = account_id_of(&v, CD_DEPOT);
    let store = |source, account, booking, amount| {
        crate::documents::insert_doc(
            &v.db().unwrap(),
            booking,
            account,
            cd_purchase_title(),
            "postbox",
            "application/pdf",
            "receipt.pdf",
            &cd_pdf("Wertpapierkauf", amount, "04.12.2025", "EUR"),
            source,
            "2025-12-04",
        )
        .unwrap()
        .unwrap()
    };
    let linked = store(
        "comdirect:postbox:linked",
        Some(giro),
        Some(existing),
        "100,00",
    );
    let orphan = store("comdirect:postbox:orphan", None, None, "100,00");
    let local = store("", Some(giro), None, "100,00");

    let result = v.resort_portal_documents(conn).unwrap();
    assert_eq!((result.resorted, result.linked), (2, 1));
    let docs = v.list_all_documents(Default::default()).unwrap();
    let row = |id| {
        let doc = docs.iter().find(|doc| doc.id == id).unwrap();
        (doc.account_id, doc.booking_id, doc.doc_type.as_str())
    };
    assert_eq!(row(linked), (Some(depot), Some(existing), "abrechnung"));
    assert_eq!(
        row(orphan),
        (Some(depot), Some(booking_id_of(&v, "open")), "abrechnung")
    );
    assert_eq!(row(local), (Some(giro), None, "postbox"));

    let again = v.resort_portal_documents(conn).unwrap();
    assert_eq!((again.resorted, again.linked), (0, 0));
}

#[test]
fn resort_leaves_orphan_postbox_documents_with_ambiguous_origin_open() {
    let (_dir, v) = open_vault("girocrab-resort-comdirect-orphan");
    let (_, prior) = cd_connection(&v, true, "prior");
    let (kind, conn) = cd_connection(&v, false, "current");
    let receipt = cd_document(
        "orphan",
        "",
        cd_purchase_title(),
        "postbox",
        cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
        "2025-12-04",
    );
    store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![receipt])).unwrap();
    for deleted in [false, true] {
        if deleted {
            v.db()
                .unwrap()
                .execute(
                    "UPDATE bank_connections SET deleted_ms = 1 WHERE id = ?1",
                    [prior],
                )
                .unwrap();
        }
        let result = v.resort_portal_documents(conn).unwrap();
        assert_eq!((result.resorted, result.linked), (0, 0));
        let doc = v.list_all_documents(Default::default()).unwrap().remove(0);
        assert_eq!(
            (doc.account_id, doc.booking_id, doc.doc_type.as_str()),
            (None, None, "postbox")
        );
    }
}

#[test]
fn resort_skips_archived_and_cross_connection_documents() {
    let (_dir, v) = open_vault("girocrab-resort-scope");
    let (kind, conn) = cd_connection(&v, false, "login");
    let booking = cd_booking(
        CD_GIRO,
        "own",
        "2025-12-04",
        -100_00,
        "EUR",
        "Wertpapierkauf",
    );
    store_fetch(&v, kind, conn, cd_fetch(true, vec![booking], vec![])).unwrap();
    let foreign = v
        .add_bank_connection(ConnectorKind::TradeRepublic, String::new(), "other".into())
        .unwrap();
    store_fetch(
        &v,
        ConnectorKind::TradeRepublic,
        foreign,
        FetchResult {
            accounts: vec![tr_cash_account()],
            transactions: vec![tr_booking("foreign", "2025-12-04", -100_00, "ETF")],
            ..Default::default()
        },
    )
    .unwrap();
    let giro = account_id_of(&v, CD_GIRO);
    let archived = account_id_of(&v, CD_GIRO2);
    let foreign_account = account_id_of(&v, "tr:cash");
    let own_booking = booking_id_of(&v, "own");
    let foreign_booking = booking_id_of(&v, "foreign");
    let db = v.db().unwrap();
    db.execute(
        "UPDATE accounts SET archived_unix = 1 WHERE id = ?1",
        [archived],
    )
    .unwrap();
    let receipts = [
        ("archived", archived, None),
        ("foreign-booking", giro, Some(foreign_booking)),
        ("foreign-account", foreign_account, Some(own_booking)),
    ];
    for (source, account, booking) in receipts {
        crate::documents::insert_doc(
            &db,
            booking,
            Some(account),
            cd_purchase_title(),
            "postbox",
            "application/pdf",
            "receipt.pdf",
            &cd_pdf("Wertpapierkauf", "100,00", "04.12.2025", "EUR"),
            &format!("comdirect:postbox:{source}"),
            "2025-12-04",
        )
        .unwrap()
        .unwrap();
    }
    drop(db);

    let result = v.resort_portal_documents(conn).unwrap();
    assert_eq!((result.resorted, result.linked), (0, 0));
    let docs = v.list_all_documents(Default::default()).unwrap();
    for (source, account, booking) in receipts {
        let doc = docs
            .iter()
            .find(|doc| doc.source_url == format!("comdirect:postbox:{source}"))
            .unwrap();
        assert_eq!(
            (doc.account_id, doc.booking_id, doc.doc_type.as_str()),
            (Some(account), booking, "postbox")
        );
    }
}

#[test]
fn resort_retypes_portal_statements_without_moving_them() {
    for kind in [ConnectorKind::Fnz, ConnectorKind::Quirion] {
        let (_dir, v) = open_vault("girocrab-resort-statement");
        let conn = v
            .add_bank_connection(kind, String::new(), "login".into())
            .unwrap();
        let receipt = cd_document(
            "statement",
            CD_GIRO,
            "Kontoauszug Nr. 5",
            "postbox",
            crate::documents::simple_pdf(&["Kontoauszug".to_owned()]),
            "2025-12-04",
        );
        let receipt = crate::documents::RemoteDocument {
            source_url: format!("{}:{conn}:statement", kind.as_str()),
            ..receipt
        };
        store_fetch(&v, kind, conn, cd_fetch(false, vec![], vec![receipt])).unwrap();
        let result = v.resort_portal_documents(conn).unwrap();
        assert_eq!((result.resorted, result.linked), (1, 0));
        let doc = v.list_all_documents(Default::default()).unwrap().remove(0);
        assert_eq!(doc.doc_type, "kontoauszug");
        assert_eq!(doc.account_id, Some(account_id_of(&v, CD_GIRO)));
    }
}

#[test]
fn resort_retypes_accountless_portal_statements_only_for_their_source_login() {
    for kind in [ConnectorKind::Fnz, ConnectorKind::Quirion] {
        let (_dir, v) = open_vault("girocrab-resort-accountless-portal");
        let conn = v
            .add_bank_connection(kind, String::new(), "first".into())
            .unwrap();
        let other = v
            .add_bank_connection(kind, String::new(), "second".into())
            .unwrap();
        let mut ids = Vec::new();
        {
            let db = v.db().unwrap();
            for id in [conn, other] {
                let source = format!("{}:{id}:statement", kind.as_str());
                ids.push(
                    crate::documents::insert_doc(
                        &db,
                        None,
                        None,
                        "Kontoauszug Nr. 5",
                        "postbox",
                        "application/pdf",
                        "statement.pdf",
                        b"%PDF",
                        &source,
                        "2025-12-04",
                    )
                    .unwrap()
                    .unwrap(),
                );
            }
        }
        let result = v.resort_portal_documents(conn).unwrap();
        assert_eq!((result.resorted, result.linked), (1, 0));
        let docs = v.list_all_documents(Default::default()).unwrap();
        for (id, doc_type) in ids.into_iter().zip(["kontoauszug", "postbox"]) {
            let doc = docs.iter().find(|doc| doc.id == id).unwrap();
            assert_eq!(
                (doc.account_id, doc.booking_id, doc.doc_type.as_str()),
                (None, None, doc_type)
            );
        }
    }
}

#[test]
fn resort_retypes_statements_after_duplicate_portal_logins_are_merged() {
    for kind in [ConnectorKind::Fnz, ConnectorKind::Quirion] {
        let (_dir, v) = open_vault("girocrab-resort-merged-portal");
        let key = kind.as_str();
        let keeper = v
            .add_bank_connection(kind, String::new(), "login".into())
            .unwrap();
        let duplicate = {
            let db = v.db().unwrap();
            db.execute(
                "INSERT INTO bank_connections(connector, display_name, login_name) VALUES (?1, 'Duplicate', 'login')",
                [key],
            ).unwrap();
            db.last_insert_rowid()
        };
        let source = format!("{key}:{duplicate}:statement");
        let receipt = crate::documents::RemoteDocument {
            source_url: source.clone(),
            ..cd_document(
                "statement",
                CD_GIRO,
                "Kontoauszug Nr. 5",
                "postbox",
                crate::documents::simple_pdf(&["Kontoauszug".to_owned()]),
                "2025-12-04",
            )
        };
        store_fetch(&v, kind, duplicate, cd_fetch(false, vec![], vec![receipt])).unwrap();
        let account = account_id_of(&v, CD_GIRO);

        assert_eq!(v.deduplicate_bank_connections().unwrap(), 1);
        assert_eq!(v.connection_status(keeper).unwrap().account_count, 2);
        assert_eq!(v.resort_portal_documents(keeper).unwrap().resorted, 1);
        let doc = v.list_all_documents(Default::default()).unwrap().remove(0);
        assert_eq!(doc.source_url, source);
        assert_eq!(doc.doc_type, "kontoauszug");
        assert_eq!(doc.account_id, Some(account));
    }
}

#[test]
fn resort_files_a_trade_republic_dividend_at_the_depot_using_the_stored_title() {
    let (_dir, v) = open_vault("girocrab-resort-tr-dividend");
    let kind = ConnectorKind::TradeRepublic;
    let conn = v
        .add_bank_connection(kind, String::new(), "login".into())
        .unwrap();
    let subject = "Global Zins ETF";
    let depot = RemoteAccount {
        external_ref: "tr:depot".into(),
        name: "Depot".into(),
        kind: AccountKind::Brokerage,
        ..tr_cash_account()
    };
    let data = FetchResult {
        accounts: vec![tr_cash_account(), depot],
        transactions: vec![tr_booking("dividend", "2025-12-04", 17_28, subject)],
        ..Default::default()
    };
    store_fetch(&v, kind, conn, data).unwrap();
    let cash = account_id_of(&v, "tr:cash");
    let store = |title, source, doc_type, pdf: &[u8]| {
        crate::documents::insert_doc(
            &v.db().unwrap(),
            None,
            Some(cash),
            title,
            doc_type,
            "application/pdf",
            "receipt.pdf",
            pdf,
            source,
            "2025-12-04",
        )
        .unwrap()
        .unwrap()
    };
    let pdf = tr_pdf("DIVIDENDE", subject, "17,28", "04.12.2025");
    let dividend = store(
        "Dividendenbeleg – Global Zins ETF",
        "https://example.invalid/dividend",
        "beleg",
        &pdf,
    );
    let manual = store("Dividendenbeleg – Global Zins ETF", "", "beleg", &pdf);
    let generic = store(
        "Dokumente – Global Zins ETF",
        "https://example.invalid/generic",
        "beleg",
        b"%PDF",
    );

    let result = v.resort_portal_documents(conn).unwrap();
    assert_eq!((result.resorted, result.linked), (1, 1));
    let docs = v.list_all_documents(Default::default()).unwrap();
    let row = |id| {
        let doc = docs.iter().find(|doc| doc.id == id).unwrap();
        (doc.account_id, doc.booking_id, doc.doc_type.as_str())
    };
    assert_eq!(
        row(dividend),
        (
            Some(account_id_of(&v, "tr:depot")),
            Some(booking_id_of(&v, "dividend")),
            "abrechnung"
        )
    );
    for id in [manual, generic] {
        assert_eq!(row(id), (Some(cash), None, "beleg"));
    }
    let again = v.resort_portal_documents(conn).unwrap();
    assert_eq!((again.resorted, again.linked), (0, 0));
}

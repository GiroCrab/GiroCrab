use std::sync::Arc;

use rusqlite::params;

use super::*;
use crate::models::BookingFilter;
use crate::sync_merge::assert_converged;
use crate::test_support::TempDir;

const KEY: [u8; 32] = [7u8; 32];
const URL: &str = "https://archiv.example/documents/123";

fn open(dir: &TempDir, file: &str) -> Arc<Vault> {
    Vault::open(dir.join(file).to_string_lossy().into(), KEY.to_vec()).unwrap()
}

/// One account `A1` with the bookings named in `exts` (same keys on every
/// vault, so a merge matches them by external id); `T<n>` is booked on day
/// `n` whatever its position. Returns the account id.
fn seed(v: &Vault, exts: &[&str]) -> i64 {
    let c = v.db().unwrap();
    c.execute(
        "INSERT OR IGNORE INTO account_groups(name) VALUES ('G')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT OR IGNORE INTO accounts(group_id, external_ref, name) \
         VALUES ((SELECT id FROM account_groups WHERE name='G'), 'A1', 'Konto')",
        [],
    )
    .unwrap();
    for ext in exts {
        let day: i64 = ext.trim_start_matches('T').parse().unwrap();
        c.execute(
            "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty) \
             VALUES ((SELECT id FROM accounts WHERE external_ref = 'A1'), ?1, ?2, ?3, 'Laden')",
            params![ext, format!("2026-01-{day:02}"), -100 * day],
        )
        .unwrap();
    }
    c.query_row(
        "SELECT id FROM accounts WHERE external_ref = 'A1'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

fn id_of(v: &Vault, ext: &str) -> i64 {
    v.db()
        .unwrap()
        .query_row(
            "SELECT id FROM transactions WHERE external_id = ?1",
            [ext],
            |r| r.get(0),
        )
        .unwrap()
}

fn exec(v: &Vault, sql: &str) {
    v.db().unwrap().execute_batch(sql).unwrap();
}

fn text(v: &Vault, sql: &str) -> String {
    v.db().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn n(v: &Vault, sql: &str) -> i64 {
    v.db().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

fn merge(into: &Vault, from: &Vault) -> crate::vault::MergeStats {
    into.merge_other_vault(from.file_path(), KEY.to_vec())
        .unwrap()
}

fn both_ways(a: &Vault, b: &Vault) {
    merge(a, b);
    merge(b, a);
    assert_converged(a, b);
}

/// External id of the booking each live reference hangs on ("-" = none), sorted.
fn linked(v: &Vault) -> Vec<String> {
    let db = v.db().unwrap();
    let mut st = db
        .prepare(
            "SELECT IFNULL(t.external_id, '-') FROM document_links l
               LEFT JOIN transactions t ON t.id = l.booking_id
              WHERE l.deleted_ms IS NULL ORDER BY 1",
        )
        .unwrap();
    let rows = st
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    rows
}

fn document_count(v: &Vault, ext: &str) -> u32 {
    let id = id_of(v, ext);
    v.list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|b| b.id == id)
        .unwrap()
        .document_count
}

fn is_invalid(r: Result<impl Sized, CoreError>) -> bool {
    matches!(r, Err(CoreError::InvalidInput { .. }))
}

fn is_not_found(r: Result<impl Sized, CoreError>) -> bool {
    matches!(r, Err(CoreError::NotFound { .. }))
}

// ------------------------------------------------------------------ input

#[test]
fn only_plain_http_links_are_accepted() {
    for ok in [
        "https://archiv.example/documents/123",
        "http://192.168.1.20:8000/documents/7/details",
        "  https://archiv.example/share/abc  ",
    ] {
        assert!(parse_link(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "   ",
        "archiv.example/documents/123",
        "ftp://archiv.example/x.pdf",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "data:text/html,x",
        "https://",
        "https://user:geheim@archiv.example/x",
        "https://user@archiv.example/x",
    ] {
        assert!(is_invalid(parse_link(bad)), "{bad}");
    }
    let long = format!("https://archiv.example/{}", "a".repeat(MAX_URL));
    assert!(is_invalid(parse_link(&long)));
}

#[test]
fn links_are_stored_normalised() {
    let url = parse_link(" HTTPS://Archiv.Example/Beleg 1?q=ä ").unwrap();
    assert_eq!(url.as_str(), "https://archiv.example/Beleg%201?q=%C3%A4");
}

#[test]
fn error_messages_never_repeat_the_link() {
    let Err(CoreError::InvalidInput { message }) = parse_link("ftp://geheim.example/akte") else {
        panic!("accepted");
    };
    assert!(!message.contains("geheim"), "{message}");
}

#[test]
fn a_missing_title_comes_from_the_url() {
    let title = |s: &str| title_from_url(&parse_link(s).unwrap());
    assert_eq!(
        title("https://www.archiv.example/documents/123/?page=2#top"),
        "archiv.example/documents/123"
    );
    assert_eq!(title("https://archiv.example"), "archiv.example");
    let long = title(&format!("https://archiv.example/{}", "a".repeat(200)));
    assert_eq!(long.chars().count(), MAX_DERIVED_TITLE);
    assert!(long.ends_with('…'));
    assert_eq!(clean_title("  Rechnung \n 2026-01  "), "Rechnung 2026-01");
    assert_eq!(clean_title(&"ü".repeat(300)).chars().count(), MAX_TITLE);
}

#[test]
fn reference_ids_are_negative_row_ids() {
    assert_eq!(doc_id_of(5), -5);
    assert_eq!(row_id_of(-5), Some(5));
    assert_eq!(row_id_of(5), None);
    assert_eq!(row_id_of(0), None);
    assert_eq!(row_id_of(i64::MIN), None);
}

// ------------------------------------------------------------------ vault API

#[test]
fn a_reference_shows_up_like_a_stored_document() {
    let dir = TempDir::new("links-list");
    let v = open(&dir, "v.db");
    let acc = seed(&v, &["T1", "T2"]);
    let t1 = id_of(&v, "T1");
    let id = v
        .attach_document_link(t1, URL.into(), String::new())
        .unwrap();
    assert!(id < 0);

    let docs = v.list_documents_for_booking(t1).unwrap();
    assert_eq!(docs.len(), 1);
    let d = &docs[0];
    assert_eq!(d.id, id);
    assert_eq!(d.link_url, URL);
    assert_eq!(d.title, "archiv.example/documents/123");
    assert_eq!(d.doc_date, "2026-01-01");
    assert_eq!((d.booking_id, d.account_id), (Some(t1), Some(acc)));
    assert_eq!(d.account_name, "Konto");
    assert!(
        d.booking_label.contains("2026-01-01"),
        "{}",
        d.booking_label
    );
    assert_eq!(
        (d.size_bytes, d.source_url.as_str(), d.doc_type.as_str()),
        (0, "", "beleg")
    );
    assert!(v
        .list_documents_for_booking(id_of(&v, "T2"))
        .unwrap()
        .is_empty());
    assert_eq!(document_count(&v, "T1"), 1);
    assert_eq!(document_count(&v, "T2"), 0);

    let all = |f: DocumentFilter| v.list_all_documents(f).unwrap().len();
    assert_eq!(all(DocumentFilter::default()), 1);
    let search = |s: &str| {
        all(DocumentFilter {
            search: Some(s.into()),
            ..Default::default()
        })
    };
    assert_eq!(search("documents/123"), 1); // title and URL
    assert_eq!(search("laden"), 1); // counterparty
    assert_eq!(search("nirgends"), 0);
    let of_type = |t: &str| {
        all(DocumentFilter {
            doc_type: Some(t.into()),
            ..Default::default()
        })
    };
    assert_eq!((of_type("beleg"), of_type("kontoauszug")), (1, 0));
    let of_account = |a: i64| {
        all(DocumentFilter {
            account_id: Some(a),
            ..Default::default()
        })
    };
    assert_eq!((of_account(acc), of_account(acc + 1)), (1, 0));
    assert_eq!(
        all(DocumentFilter {
            unlinked_only: true,
            ..Default::default()
        }),
        0
    );
}

#[test]
fn a_given_title_is_kept() {
    let dir = TempDir::new("links-title");
    let v = open(&dir, "v.db");
    seed(&v, &["T1"]);
    let t1 = id_of(&v, "T1");
    v.attach_document_link(t1, URL.into(), "  Rechnung  Januar ".into())
        .unwrap();
    assert_eq!(
        v.list_documents_for_booking(t1).unwrap()[0].title,
        "Rechnung Januar"
    );
}

#[test]
fn references_sort_with_stored_files_by_date() {
    let dir = TempDir::new("links-sort");
    let v = open(&dir, "v.db");
    seed(&v, &["T1", "T2", "T3"]);
    let pdf = crate::documents::simple_pdf(&["Rechnung".into()]);
    v.attach_document_to_booking(id_of(&v, "T1"), "alt.pdf".into(), pdf.clone())
        .unwrap();
    v.attach_document_link(id_of(&v, "T2"), URL.into(), "Mitte".into())
        .unwrap();
    v.attach_document_to_booking(id_of(&v, "T3"), "neu.pdf".into(), pdf)
        .unwrap();
    let titles: Vec<String> = v
        .list_all_documents(DocumentFilter::default())
        .unwrap()
        .into_iter()
        .map(|d| d.title)
        .collect();
    assert_eq!(titles, ["neu", "Mitte", "alt"]);
    assert_eq!(document_count(&v, "T2"), 1);
}

#[test]
fn one_link_on_several_bookings_but_once_per_booking() {
    let dir = TempDir::new("links-twice");
    let v = open(&dir, "v.db");
    seed(&v, &["T1", "T2"]);
    let (t1, t2) = (id_of(&v, "T1"), id_of(&v, "T2"));
    // an invoice paid in two instalments
    v.attach_document_link(t1, URL.into(), String::new())
        .unwrap();
    v.attach_document_link(t2, URL.into(), String::new())
        .unwrap();
    assert_eq!((document_count(&v, "T1"), document_count(&v, "T2")), (1, 1));
    assert!(is_invalid(v.attach_document_link(
        t1,
        URL.into(),
        "nochmal".into()
    )));
    // moving the second one over would double it as well
    let on = |booking: i64| v.list_documents_for_booking(booking).unwrap()[0].id;
    assert!(is_invalid(v.link_document(on(t2), Some(t1))));
    assert_eq!(linked(&v), ["T1", "T2"]);
    // onto its own booking: nothing to refuse
    v.link_document(on(t1), Some(t1)).unwrap();
    assert!(is_not_found(v.attach_document_link(
        t1 + 100,
        URL.into(),
        String::new()
    )));
    assert!(is_invalid(v.attach_document_link(
        t1,
        "ftp://archiv.example/x".into(),
        String::new()
    )));
    assert_eq!(n(&v, "SELECT count(*) FROM document_links"), 2);
}

#[test]
fn relink_unlink_and_delete() {
    let dir = TempDir::new("links-edit");
    let v = open(&dir, "v.db");
    let acc = seed(&v, &["T1", "T2", "T3"]);
    let (t1, t2, t3) = (id_of(&v, "T1"), id_of(&v, "T2"), id_of(&v, "T3"));
    let id = v
        .attach_document_link(t1, URL.into(), "Rechnung".into())
        .unwrap();
    let stamp = || n(&v, "SELECT updated_ms FROM document_links");
    let mark = || text(&v, "SELECT rev FROM document_links");
    let (created, first_mark) = (stamp(), mark());
    assert!(!first_mark.is_empty());

    v.link_document(id, Some(t2)).unwrap();
    assert_eq!(linked(&v), ["T2"]);
    assert_eq!((document_count(&v, "T1"), document_count(&v, "T2")), (0, 1));
    // every edit is stamped and marked anew
    assert!(stamp() > created);
    assert_ne!(mark(), first_mark);

    // unlinked: still a document of its account
    v.link_document(id, None).unwrap();
    assert_eq!(linked(&v), ["-"]);
    let unlinked = v
        .list_all_documents(DocumentFilter {
            unlinked_only: true,
            account_id: Some(acc),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(unlinked.len(), 1);
    assert_eq!(
        (unlinked[0].booking_id, unlinked[0].account_id),
        (None, Some(acc))
    );
    assert!(unlinked[0].booking_label.is_empty());

    // only live bookings take a reference
    assert!(is_not_found(v.link_document(id, Some(t3 + 100))));
    crate::sync_merge::soft_delete_booking(&v.db().unwrap(), t3).unwrap();
    assert!(is_not_found(v.link_document(id, Some(t3))));
    assert!(is_not_found(v.attach_document_link(
        t3,
        URL.into(),
        String::new()
    )));
    assert_eq!(linked(&v), ["-"]);

    v.link_document(id, Some(t1)).unwrap();
    v.delete_document(id).unwrap();
    assert!(v
        .list_all_documents(DocumentFilter::default())
        .unwrap()
        .is_empty());
    assert_eq!(document_count(&v, "T1"), 0);
    // the tombstone says nothing about what it pointed at
    assert_eq!(
        text(
            &v,
            "SELECT url || '|' || title || '|' || doc_date || '|' || IFNULL(booking_id, '') || '|'
                    || IFNULL(account_id, '') || '|' || (deleted_ms IS NOT NULL)
               FROM document_links"
        ),
        "|||||1"
    );
    assert!(is_not_found(v.link_document(id, Some(t1))));
    // deleting again, or something that never existed, is quiet like for files
    v.delete_document(id).unwrap();
    v.delete_document(-9999).unwrap();
    v.delete_document(i64::MIN).unwrap();
}

#[test]
fn a_reference_moves_to_another_account_like_a_stored_file() {
    let dir = TempDir::new("links-account");
    let v = open(&dir, "v.db");
    let acc = seed(&v, &["T1"]);
    exec(
        &v,
        "INSERT INTO accounts(group_id, external_ref, name)
         VALUES ((SELECT group_id FROM accounts LIMIT 1), 'A2', 'Zweites')",
    );
    let other = n(&v, "SELECT id FROM accounts WHERE external_ref = 'A2'");
    let id = v
        .attach_document_link(id_of(&v, "T1"), URL.into(), "Rechnung".into())
        .unwrap();
    let stamp = n(&v, "SELECT updated_ms FROM document_links");

    // its own account: it stays on its booking
    v.set_document_account(id, Some(acc)).unwrap();
    assert_eq!(linked(&v), ["T1"]);
    // another account: off the booking, and stamped as an edit
    v.set_document_account(id, Some(other)).unwrap();
    assert_eq!(linked(&v), ["-"]);
    assert_eq!(n(&v, "SELECT account_id FROM document_links"), other);
    assert!(n(&v, "SELECT updated_ms FROM document_links") > stamp);
    // no account at all
    v.set_document_account(id, None).unwrap();
    assert_eq!(
        n(
            &v,
            "SELECT count(*) FROM document_links WHERE account_id IS NULL"
        ),
        1
    );
    assert!(is_not_found(v.set_document_account(-999, None)));
}

#[test]
fn a_deleted_booking_leaves_its_reference_unlinked() {
    let dir = TempDir::new("links-gone");
    let v = open(&dir, "v.db");
    seed(&v, &["T1"]);
    let t1 = id_of(&v, "T1");
    v.attach_document_link(t1, URL.into(), String::new())
        .unwrap();
    crate::sync_merge::soft_delete_booking(&v.db().unwrap(), t1).unwrap();
    assert!(v.list_documents_for_booking(t1).unwrap().is_empty());
    let docs = v
        .list_all_documents(DocumentFilter {
            unlinked_only: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].booking_id, None);
}

#[test]
fn a_reference_has_no_file_and_is_never_exported() {
    let dir = TempDir::new("links-export");
    let v = open(&dir, "v.db");
    seed(&v, &["T1"]);
    let t1 = id_of(&v, "T1");
    let id = v
        .attach_document_link(t1, URL.into(), "Rechnung".into())
        .unwrap();
    assert!(is_invalid(v.get_document_bytes(id)));
    let out = dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let out_dir = out.to_string_lossy().to_string();
    assert!(is_invalid(v.export_document(id, out_dir.clone())));
    assert_eq!(v.export_all_documents(out_dir.clone()).unwrap(), 0);
    assert_eq!(v.mirror_new_documents(out_dir.clone()).unwrap(), 0);

    // next to a stored file only the file is written
    v.attach_document_to_booking(
        t1,
        "bon.pdf".into(),
        crate::documents::simple_pdf(&["Kassenbon".into()]),
    )
    .unwrap();
    assert_eq!(v.export_all_documents(out_dir.clone()).unwrap(), 1);
    assert_eq!(v.mirror_new_documents(out_dir).unwrap(), 1);
    assert_eq!(document_count(&v, "T1"), 2);
}

/// The count sits in the booking list's hot path: one index lookup per row.
#[test]
fn the_count_of_a_booking_comes_from_the_partial_index() {
    let dir = TempDir::new("links-plan");
    let v = open(&dir, "v.db");
    seed(&v, &["T1"]);
    let db = v.db().unwrap();
    let plan: Vec<String> = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN SELECT t.id, {COUNT_FOR_BOOKING_SQL} FROM transactions t"
        ))
        .unwrap()
        .query_map([], |r| r.get::<_, String>(3))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    let plan = plan.join(" | ");
    assert!(plan.contains("document_links_booking"), "{plan}");
    assert!(!plan.contains("SCAN l"), "{plan}");
}

// ------------------------------------------------------------------ merge

#[test]
fn merge_brings_references_over_and_changes_nothing_twice() {
    let dir = TempDir::new("links-merge");
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    // the same bookings with other row ids on each side
    seed(&a, &["T1", "T2", "T3"]);
    seed(&b, &["T3", "T2", "T1"]);
    assert_ne!(id_of(&a, "T1"), id_of(&b, "T1"));
    b.attach_document_link(id_of(&b, "T1"), URL.into(), "Rechnung".into())
        .unwrap();
    b.attach_document_link(id_of(&b, "T3"), URL.into(), "Rechnung".into())
        .unwrap();

    // the bookings themselves are unchanged: only the references come over
    let st = merge(&a, &b);
    assert_eq!((st.bookings_added, st.bookings_updated), (0, 0));
    assert_eq!(linked(&a), ["T1", "T3"]);
    let doc = &a.list_documents_for_booking(id_of(&a, "T1")).unwrap()[0];
    assert_eq!(
        (doc.title.as_str(), doc.link_url.as_str()),
        ("Rechnung", URL)
    );
    assert_eq!(doc.doc_date, "2026-01-01");
    assert_eq!(doc.account_name, "Konto");
    assert_eq!(document_count(&a, "T1"), 1);

    assert_eq!(merge(&a, &b).changes, 0);
    both_ways(&a, &b);
    assert_eq!(merge(&b, &a).changes, 0);
}

#[test]
fn newest_edit_wins_in_both_merge_orders() {
    for a_first in [true, false] {
        let dir = TempDir::new("links-lww");
        let a = open(&dir, "a.db");
        let b = open(&dir, "b.db");
        seed(&a, &["T1", "T2", "T3"]);
        seed(&b, &["T1", "T2", "T3"]);
        let id = a
            .attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
            .unwrap();
        both_ways(&a, &b);
        // offline: a moves it to T2, b later to T3
        a.link_document(id, Some(id_of(&a, "T2"))).unwrap();
        let theirs = b.list_all_documents(DocumentFilter::default()).unwrap()[0].id;
        b.link_document(theirs, Some(id_of(&b, "T3"))).unwrap();
        exec(&a, "UPDATE document_links SET updated_ms = 2000");
        exec(&b, "UPDATE document_links SET updated_ms = 3000");
        if a_first {
            merge(&a, &b);
            merge(&b, &a);
        } else {
            merge(&b, &a);
            merge(&a, &b);
        }
        assert_converged(&a, &b);
        assert_eq!(
            (linked(&a), linked(&b)),
            (vec!["T3".to_string()], vec!["T3".to_string()])
        );
    }
}

/// Two edits in the same millisecond: the edit mark `rev` decides, the same
/// way on both devices. Nothing device-specific (row ids, account
/// references) takes part.
#[test]
fn equal_stamps_are_decided_by_the_edit_mark_in_both_merge_orders() {
    for (rev_a, rev_b, winner) in [("r1", "r2", "T3"), ("r2", "r1", "T2")] {
        for a_first in [true, false] {
            let dir = TempDir::new("links-tie");
            let a = open(&dir, "a.db");
            let b = open(&dir, "b.db");
            // other row ids on each side
            seed(&a, &["T1", "T2", "T3"]);
            seed(&b, &["T3", "T2", "T1"]);
            let id = a
                .attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
                .unwrap();
            both_ways(&a, &b);
            a.link_document(id, Some(id_of(&a, "T2"))).unwrap();
            let theirs = b.list_all_documents(DocumentFilter::default()).unwrap()[0].id;
            b.link_document(theirs, Some(id_of(&b, "T3"))).unwrap();
            exec(
                &a,
                &format!("UPDATE document_links SET updated_ms = 5000, rev = '{rev_a}'"),
            );
            exec(
                &b,
                &format!("UPDATE document_links SET updated_ms = 5000, rev = '{rev_b}'"),
            );
            if a_first {
                merge(&a, &b);
                merge(&b, &a);
            } else {
                merge(&b, &a);
                merge(&a, &b);
            }
            assert_converged(&a, &b);
            assert_eq!(linked(&a), [winner], "{rev_a}/{rev_b}, a first: {a_first}");
            assert_eq!(linked(&b), [winner]);
        }
    }
}

/// The same edit on both sides, but on one of them the booking has changed
/// its identity since (a repair there moved the reference along, unstamped,
/// and deleted the old row). The side that still points at a live booking
/// wins, in both merge orders.
#[test]
fn the_same_edit_on_both_sides_follows_the_live_booking() {
    for a_first in [true, false] {
        let dir = TempDir::new("links-live");
        let a = open(&dir, "a.db");
        let b = open(&dir, "b.db");
        seed(&a, &["T1", "T2"]);
        seed(&b, &["T1", "T2"]);
        a.attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
            .unwrap();
        both_ways(&a, &b);
        // on a, T2 takes over from T1
        exec(
            &a,
            &format!("UPDATE document_links SET booking_id = {}", id_of(&a, "T2")),
        );
        // the id first: the vault lock is not re-entrant
        let old = id_of(&a, "T1");
        crate::sync_merge::soft_delete_booking(&a.db().unwrap(), old).unwrap();
        if a_first {
            merge(&a, &b);
            merge(&b, &a);
        } else {
            merge(&b, &a);
            merge(&a, &b);
        }
        assert_converged(&a, &b);
        assert_eq!(linked(&a), ["T2"], "a first: {a_first}");
        assert_eq!(linked(&b), ["T2"], "a first: {a_first}");
        assert_eq!(document_count(&b, "T2"), 1);
    }
}

#[test]
fn a_deletion_stays_against_a_later_edit_and_clears_the_content() {
    for a_first in [true, false] {
        let dir = TempDir::new("links-del");
        let a = open(&dir, "a.db");
        let b = open(&dir, "b.db");
        seed(&a, &["T1", "T2"]);
        seed(&b, &["T1", "T2"]);
        let id = a
            .attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
            .unwrap();
        both_ways(&a, &b);
        // a deletes; b, still offline, edits afterwards
        a.delete_document(id).unwrap();
        let theirs = b.list_all_documents(DocumentFilter::default()).unwrap()[0].id;
        b.link_document(theirs, Some(id_of(&b, "T2"))).unwrap();
        exec(
            &b,
            "UPDATE document_links SET updated_ms = (SELECT max(updated_ms) FROM document_links) + 100000",
        );
        if a_first {
            merge(&a, &b);
            merge(&b, &a);
        } else {
            merge(&b, &a);
            merge(&a, &b);
        }
        assert_converged(&a, &b);
        for v in [&a, &b] {
            assert!(v
                .list_all_documents(DocumentFilter::default())
                .unwrap()
                .is_empty());
            assert_eq!(
                text(
                    v,
                    "SELECT url || title || doc_date || IFNULL(booking_id, '') || IFNULL(account_id, '')
                       FROM document_links"
                ),
                ""
            );
            assert_eq!(document_count(v, "T2"), 0);
        }
        // and again: nothing comes back, nothing changes
        assert_eq!(merge(&a, &b).changes, 0);
    }
}

/// Bookings the two sides know under different ids are paired by their
/// fingerprint and one side renames its row. A reference must follow the
/// pairing itself: a lookup by the other side's external id would miss.
#[test]
fn a_reference_follows_a_booking_paired_by_fingerprint() {
    let fp = crate::importers::moneymoney_import::fingerprint("2026-02-01", -4200, "", "Laden", "");
    let setup = |tag: &str| {
        let dir = TempDir::new(tag);
        let a = open(&dir, "a.db");
        let b = open(&dir, "b.db");
        seed(&a, &[]);
        seed(&b, &[]);
        // a imported it from a file, b fetched it from the bank
        a.db()
            .unwrap()
            .execute(
                "INSERT INTO transactions(account_id, external_id, import_fp, booking_date, amount_cents, counterparty)
                 VALUES ((SELECT id FROM accounts), 'mm-1', ?1, '2026-02-01', -4200, 'Laden')",
                [&fp],
            )
            .unwrap();
        exec(
            &b,
            "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty)
             VALUES ((SELECT id FROM accounts), 'bank-1', '2026-02-01', -4200, 'Laden')",
        );
        (dir, a, b)
    };

    // reference on the bank's copy, merged into the imported one
    let (_dir, a, b) = setup("links-fp-1");
    b.attach_document_link(id_of(&b, "bank-1"), URL.into(), String::new())
        .unwrap();
    merge(&a, &b);
    assert_eq!(n(&a, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(linked(&a), ["bank-1"]);
    both_ways(&a, &b);

    // reference on the imported copy, merged into the bank's one
    let (_dir, a, b) = setup("links-fp-2");
    a.attach_document_link(id_of(&a, "mm-1"), URL.into(), String::new())
        .unwrap();
    merge(&b, &a);
    assert_eq!(n(&b, "SELECT count(*) FROM transactions"), 1);
    assert_eq!(linked(&b), ["bank-1"]);
    both_ways(&a, &b);
}

/// Two identical bookings share one fingerprint; the merge pairs them in
/// external-id order. Each reference stays on the booking its own was paired
/// with, not on the first one that carries the fingerprint.
#[test]
fn references_on_identical_bookings_keep_their_own_booking() {
    let fp = crate::importers::moneymoney_import::fingerprint("2026-03-01", -350, "", "Café", "");
    let dir = TempDir::new("links-same-fp");
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    seed(&a, &[]);
    seed(&b, &[]);
    // two equal coffees on one day: imported on a, fetched on b
    for ext in ["mm-1", "mm-2"] {
        a.db()
            .unwrap()
            .execute(
                "INSERT INTO transactions(account_id, external_id, import_fp, booking_date, amount_cents, counterparty)
                 VALUES ((SELECT id FROM accounts), ?1, ?2, '2026-03-01', -350, 'Café')",
                params![ext, fp],
            )
            .unwrap();
    }
    exec(
        &b,
        "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty)
         VALUES ((SELECT id FROM accounts), 'bank-1', '2026-03-01', -350, 'Café'),
                ((SELECT id FROM accounts), 'bank-2', '2026-03-01', -350, 'Café')",
    );
    b.attach_document_link(id_of(&b, "bank-2"), URL.into(), "Bon 2".into())
        .unwrap();
    merge(&a, &b);
    assert_eq!(n(&a, "SELECT count(*) FROM transactions"), 2);
    assert_eq!(linked(&a), ["bank-2"]);
    both_ways(&a, &b);
    assert_eq!(linked(&b), ["bank-2"]);
}

#[test]
fn a_side_without_references_keeps_those_of_the_other() {
    let dir = TempDir::new("links-none");
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    seed(&a, &["T1"]);
    seed(&b, &["T1"]);
    a.attach_document_link(id_of(&a, "T1"), URL.into(), String::new())
        .unwrap();
    // b has no reference at all: a keeps its own
    both_ways(&a, &b);
    assert_eq!(
        (linked(&a), linked(&b)),
        (vec!["T1".to_string()], vec!["T1".to_string()])
    );
}

/// Two devices link the same address to the same booking before they meet:
/// afterwards it is one reference, the same one on both, in whichever order
/// they merge. A reference to something else on that booking stays.
#[test]
fn the_same_reference_made_on_two_devices_is_one() {
    for first_a in [true, false] {
        let dir = TempDir::new(if first_a {
            "links-double-a"
        } else {
            "links-double-b"
        });
        let a = open(&dir, "a.db");
        let b = open(&dir, "b.db");
        seed(&a, &["T1"]);
        seed(&b, &["T1"]);
        a.attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
            .unwrap();
        both_ways(&a, &b);
        assert_eq!(document_count(&b, "T1"), 1);
        // offline: each links the same second address
        for v in [&a, &b] {
            v.attach_document_link(
                id_of(v, "T1"),
                "https://shop.example/bon/7".into(),
                "Bon".into(),
            )
            .unwrap();
        }
        if first_a {
            merge(&a, &b);
            merge(&b, &a);
            merge(&a, &b);
        } else {
            merge(&b, &a);
            merge(&a, &b);
            merge(&b, &a);
        }
        let state = |v: &Vault| {
            text(
                v,
                "SELECT group_concat(line, ' | ') FROM (
                    SELECT uuid || ':' || (url = '') || ':' || (deleted_ms IS NOT NULL) || ':'
                           || updated_ms || ':' || rev AS line
                      FROM document_links ORDER BY uuid)",
            )
        };
        assert_eq!(state(&a), state(&b));
        for v in [&a, &b] {
            assert_eq!(document_count(v, "T1"), 2, "each address once");
            assert_eq!(
                n(
                    v,
                    "SELECT count(*) FROM document_links WHERE deleted_ms IS NOT NULL"
                ),
                1
            );
            assert_eq!(
                n(
                    v,
                    "SELECT count(*) FROM document_links WHERE deleted_ms IS NOT NULL
                       AND (booking_id IS NOT NULL OR url != '')"
                ),
                0
            );
        }
        // nothing changes any more
        both_ways(&a, &b);
        assert_eq!(document_count(&a, "T1"), 2);
        assert_eq!(state(&a), state(&b));
    }
}

/// The same address linked on two devices to different bookings and taken
/// off them again: two receipts without a booking, each with its own title.
/// They are not one reference, in either merge order.
#[test]
fn unlinked_references_to_the_same_address_stay_apart() {
    let dir = TempDir::new("links-double-unlinked");
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    seed(&a, &["T1", "T2"]);
    seed(&b, &["T1", "T2"]);
    let on_a = a
        .attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
        .unwrap();
    let on_b = b
        .attach_document_link(id_of(&b, "T2"), URL.into(), "Gutschrift".into())
        .unwrap();
    a.link_document(on_a, None).unwrap();
    b.link_document(on_b, None).unwrap();
    both_ways(&a, &b);
    both_ways(&a, &b);
    for v in [&a, &b] {
        assert_eq!(
            text(
                v,
                "SELECT group_concat(title, ',') FROM
                    (SELECT title FROM document_links WHERE deleted_ms IS NULL ORDER BY title)"
            ),
            "Gutschrift,Rechnung"
        );
    }
}

/// A merge says how many references it brought: the app reloads its
/// document views only then.
#[test]
fn a_merge_reports_the_references_it_changed() {
    let dir = TempDir::new("links-merge-count");
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    seed(&a, &["T1"]);
    seed(&b, &["T1"]);
    a.attach_document_link(id_of(&a, "T1"), URL.into(), "Rechnung".into())
        .unwrap();
    assert_eq!(merge(&b, &a).documents_updated, 1);
    assert_eq!(merge(&b, &a).documents_updated, 0);
    assert_eq!(merge(&a, &b).documents_updated, 0);
}

#[test]
fn bookings_without_a_document_can_be_listed_and_counted() {
    let dir = TempDir::new("links-missing");
    let v = open(&dir, "v.db");
    seed(&v, &["T1", "T2", "T3", "T4"]);
    v.attach_document_link(id_of(&v, "T1"), URL.into(), String::new())
        .unwrap();
    v.attach_document_to_booking(
        id_of(&v, "T2"),
        "bon.pdf".into(),
        crate::documents::simple_pdf(&["Kassenbon".into()]),
    )
    .unwrap();
    // a removed reference no longer counts
    let gone = v
        .attach_document_link(id_of(&v, "T3"), URL.into(), String::new())
        .unwrap();
    v.delete_document(gone).unwrap();

    let missing = BookingFilter {
        without_document: true,
        ..Default::default()
    };
    let mut ids: Vec<i64> = v
        .list_transactions(missing.clone())
        .unwrap()
        .transactions
        .iter()
        .map(|b| b.id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, [id_of(&v, "T3"), id_of(&v, "T4")]);
    assert_eq!(v.count_transactions(missing).unwrap(), 2);
    assert_eq!(v.count_transactions(BookingFilter::default()).unwrap(), 4);
}

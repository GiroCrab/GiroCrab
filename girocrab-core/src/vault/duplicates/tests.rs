use std::sync::Arc;

use rusqlite::params;

use super::super::Vault;

fn vault() -> (crate::test_support::TempDir, Arc<Vault>) {
    let dir = crate::test_support::TempDir::new("dup");
    let path = dir.join("v.db");
    let v = Vault::open(path.to_string_lossy().to_string(), vec![6u8; 32]).unwrap();
    (dir, v)
}

fn account(v: &Vault) -> i64 {
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO account_groups(name) SELECT 'G' WHERE NOT EXISTS (SELECT 1 FROM account_groups)",
        [],
    )
    .unwrap();
    db.execute(
        "INSERT INTO accounts(group_id, external_ref, name)
         VALUES ((SELECT MIN(id) FROM account_groups), 'acc-1', 'Girokonto')",
        [],
    )
    .unwrap();
    db.last_insert_rowid()
}

fn row(v: &Vault, acc: i64, ext: &str, date: &str, cents: i64, name: &str, purpose: &str) -> i64 {
    let db = v.db().unwrap();
    db.execute(
        // imported rows carry their import key, as the MoneyMoney import stores it
        "INSERT INTO transactions(account_id, external_id, booking_date, amount_cents, counterparty, purpose, import_fp)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, CASE WHEN ?2 LIKE 'mm-%' THEN 'fp-' || ?2 ELSE '' END)",
        params![acc, ext, date, cents, name, purpose],
    )
    .unwrap();
    db.last_insert_rowid()
}

fn live(v: &Vault, acc: i64) -> Vec<(String, String)> {
    let db = v.db().unwrap();
    let mut st = db
        .prepare("SELECT external_id, note FROM transactions WHERE account_id = ?1 AND deleted_ms IS NULL ORDER BY id")
        .unwrap();
    st.query_map([acc], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|x| x.unwrap())
        .collect()
}

#[test]
fn lists_bank_and_imported_rows_with_the_same_amount_close_by() {
    let (_d, v) = vault();
    let acc = account(&v);
    // the account fee as FinTS and as MoneyMoney (old key-file text) describe it
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-05-29",
        -990,
        "Kontoführung",
        "Kontoführung Konto vom 01.05.2026 bis 31.05.2026",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-05-29",
        -990,
        "Akto:12345678",
        "ABZR: 01.05.2026-31.05.2026",
    );
    // not a pair: other amount, too far apart, both imported, a reservation
    row(&v, acc, "fints-2", "2026-05-29", -991, "X", "");
    row(&v, acc, "mm-2", "2026-05-20", -990, "Y", "");
    row(&v, acc, "mm-3", "2026-06-10", -500, "Z", "");
    row(&v, acc, "mm-4", "2026-06-10", -500, "Z", "");
    row(
        &v,
        acc,
        "pending-hold-fints-x",
        "2026-05-29",
        -990,
        "Kontoführung",
        "",
    );
    let pairs = v.possible_duplicates().unwrap();
    assert_eq!(pairs.len(), 1, "{pairs:?}");
    assert_eq!(pairs[0].bank.booking_id, bank);
    assert_eq!(pairs[0].imported.booking_id, mm);
    assert_eq!(pairs[0].amount_cents, -990);
    assert_eq!(pairs[0].account_name, "Girokonto");
}

#[test]
fn merge_keeps_the_imported_row_under_the_bank_id() {
    let (_d, v) = vault();
    let acc = account(&v);
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-01-12",
        -200,
        "UBER *TRIP",
        "UBER *TRIP 00:30:00",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-01-11",
        -200,
        "UBER TRIP",
        "UBER TRIP 00:30:00 EREF: X",
    );
    // a note on the bank row the imported one lacks carries over
    v.update_transaction_note(bank, "Fahrt zum Bahnhof".into())
        .unwrap();
    v.merge_possible_duplicate(bank, mm, true).unwrap();
    assert_eq!(
        live(&v, acc),
        vec![("fints-1".into(), "Fahrt zum Bahnhof".into())]
    );
    // the imported row is the one that lives on, the bank row keeps its old id as tombstone
    let db = v.db().unwrap();
    let ext: String = db
        .query_row(
            "SELECT external_id FROM transactions WHERE id = ?1",
            [mm],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ext, "fints-1");
    let (tomb, deleted): (String, Option<i64>) = db
        .query_row(
            "SELECT external_id, deleted_ms FROM transactions WHERE id = ?1",
            [bank],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(tomb, "mm-1");
    assert!(deleted.is_some());
    drop(db);
    assert!(v.possible_duplicates().unwrap().is_empty());
    // a second merge of the same pair is refused
    assert!(v.merge_possible_duplicate(bank, mm, true).is_err());
}

#[test]
fn merge_takes_the_bank_texts_and_lets_the_rules_categorise() {
    let (_d, v) = vault();
    let acc = account(&v);
    let fee = v
        .create_category("Kontogebühren".into(), None, String::new(), String::new())
        .unwrap();
    v.db()
        .unwrap()
        .execute(
            "INSERT INTO rules(name, category_id, pattern) VALUES ('Gebühr', ?1, 'Kontoführung')",
            [fee],
        )
        .unwrap();
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-05-29",
        -990,
        "Kontoführung",
        "Kontoführung Konto vom 01.05.2026 bis 31.05.2026",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-05-29",
        -990,
        "Akto:12345678",
        "ABZR: 01.05.2026-31.05.2026SPES: 9,9 S",
    );
    v.merge_possible_duplicate(bank, mm, true).unwrap();
    let (name, purpose, cat): (String, String, Option<i64>) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT counterparty, purpose, category_id FROM transactions WHERE id = ?1",
            [mm],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(name, "Kontoführung");
    assert!(purpose.starts_with("Kontoführung Konto"));
    assert_eq!(cat, Some(fee));
}

#[test]
fn keeping_the_imported_entry_keeps_its_texts_and_category() {
    let (_d, v) = vault();
    let acc = account(&v);
    let mine = v
        .create_category("Fahrrad".into(), None, String::new(), String::new())
        .unwrap();
    let other = v
        .create_category("Sonstiges".into(), None, String::new(), String::new())
        .unwrap();
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-03-23",
        1200,
        "Hannah Müller",
        "Fahrrad X3 Hannah Müller End-to-End-Ref.: NOTPROVIDED",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-03-23",
        1200,
        "Hannah Muller",
        "Fahrrad X3",
    );
    v.set_transaction_category(bank, Some(other)).unwrap();
    v.set_transaction_category(mm, Some(mine)).unwrap();
    v.update_transaction_note(bank, "von der Bank".into())
        .unwrap();
    v.merge_possible_duplicate(bank, mm, false).unwrap();
    let (ext, name, purpose, cat, note): (String, String, String, Option<i64>, String) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT external_id, counterparty, purpose, category_id, note FROM transactions WHERE id = ?1",
            [mm],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    // under the bank's id, MoneyMoney's texts and category; the bank's note fills the gap
    assert_eq!(ext, "fints-1");
    assert_eq!(
        (name.as_str(), purpose.as_str()),
        ("Hannah Muller", "Fahrrad X3")
    );
    assert_eq!(cat, Some(mine));
    assert_eq!(note, "von der Bank");
}

#[test]
fn a_category_by_hand_beats_a_rule_and_notes_are_joined() {
    let (_d, v) = vault();
    let acc = account(&v);
    let by_hand = v
        .create_category("Fahrrad".into(), None, String::new(), String::new())
        .unwrap();
    let by_rule = v
        .create_category("Sonstiges".into(), None, String::new(), String::new())
        .unwrap();
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-03-23",
        1200,
        "Hannah Müller",
        "Fahrrad X3 End-to-End-Ref.: NOTPROVIDED",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-03-23",
        1200,
        "Hannah Muller",
        "Fahrrad X3",
    );
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET category_id = ?2, category_source = 'rule' WHERE id = ?1",
            params![bank, by_rule],
        )
        .unwrap();
    v.set_transaction_category(mm, Some(by_hand)).unwrap();
    v.update_transaction_note(bank, "Anzahlung".into()).unwrap();
    v.update_transaction_note(mm, "Kunde Müller".into())
        .unwrap();
    // the bank's texts are kept, but the category by hand wins
    v.merge_possible_duplicate(bank, mm, true).unwrap();
    let (name, cat, src, note): (String, Option<i64>, String, String) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT counterparty, category_id, category_source, note FROM transactions WHERE id = ?1",
            [mm],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(name, "Hannah Müller");
    assert_eq!((cat, src.as_str()), (Some(by_hand), "manual"));
    assert_eq!(note, "Anzahlung · Kunde Müller");
}

#[test]
fn merging_one_pair_resolves_the_competing_ones() {
    // two Uber rides of 2 € on neighbouring days: four candidate pairs
    let (_d, v) = vault();
    let acc = account(&v);
    let b1 = row(&v, acc, "fints-1", "2026-01-12", -200, "UBER", "00:30:00");
    let b2 = row(&v, acc, "fints-2", "2026-01-13", -200, "UBER", "18:31:00");
    let m1 = row(&v, acc, "mm-1", "2026-01-12", -200, "UBER", "00:30:00");
    let m2 = row(&v, acc, "mm-2", "2026-01-13", -200, "UBER", "18:31:00");
    assert_eq!(v.possible_duplicates().unwrap().len(), 4);
    v.merge_possible_duplicate(b1, m1, true).unwrap();
    let left = v.possible_duplicates().unwrap();
    assert_eq!(left.len(), 1, "{left:?}");
    assert_eq!(
        (left[0].bank.booking_id, left[0].imported.booking_id),
        (b2, m2)
    );
}

#[test]
fn a_row_already_taken_over_by_the_bank_is_no_candidate() {
    let (_d, v) = vault();
    let acc = account(&v);
    // imported, then adopted by a fetch: bank id, but still the import key
    let adopted = row(&v, acc, "mm-1", "2026-02-02", -1500, "Miete", "Februar");
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET external_id = 'fints-9' WHERE id = ?1",
            [adopted],
        )
        .unwrap();
    row(
        &v,
        acc,
        "mm-2",
        "2026-02-03",
        -1500,
        "Miete",
        "Februar Nachzahlung",
    );
    assert!(v.possible_duplicates().unwrap().is_empty());
}

#[test]
fn tags_of_the_bank_row_carry_over() {
    let (_d, v) = vault();
    let acc = account(&v);
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-01-22",
        -200,
        "UBER *TRIP, AMSTERDAM NL",
        "UBER *TRIP 18:31:00",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-01-22",
        -200,
        "Uber Trip,/Amsterdam//NL",
        "UBER TRIP 18:31:00 EREF: X",
    );
    let tag = v.create_tag("doppel".into(), String::new()).unwrap().id;
    v.set_booking_tags(bank, vec![tag]).unwrap();
    // a tagged bank row is offered too, with its tags
    let pairs = v.possible_duplicates().unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].bank.tags, vec!["doppel".to_string()]);
    assert!(pairs[0].imported.tags.is_empty());
    v.merge_possible_duplicate(bank, mm, false).unwrap();
    let tags: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM transaction_tags WHERE transaction_id = ?1 AND tag_id = ?2",
            params![mm, tag],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tags, 1);
}

#[test]
fn dismissed_pairs_stay_hidden_and_bank_rows_with_extras_are_left_out() {
    let (_d, v) = vault();
    let acc = account(&v);
    let bank = row(
        &v,
        acc,
        "fints-1",
        "2026-03-23",
        1200,
        "Hannah Müller",
        "Fahrrad",
    );
    let mm = row(
        &v,
        acc,
        "mm-1",
        "2026-03-23",
        1200,
        "Hannah Muller",
        "Fahrrad",
    );
    v.dismiss_possible_duplicate(bank, mm).unwrap();
    assert!(v.possible_duplicates().unwrap().is_empty());
    // a bank row with a split would lose it on merge: not offered
    let b2 = row(&v, acc, "fints-2", "2026-04-01", 500, "A", "");
    row(&v, acc, "mm-2", "2026-04-01", 500, "A", "");
    v.db()
        .unwrap()
        .execute(
            "INSERT INTO transaction_splits(transaction_id, amount_cents) VALUES (?1, 500)",
            [b2],
        )
        .unwrap();
    assert!(v.possible_duplicates().unwrap().is_empty());
}

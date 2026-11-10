use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rusqlite::params;

use crate::vault::Vault;

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gc-groups-{tag}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open(dir: &std::path::Path, file: &str) -> Arc<Vault> {
    Vault::open(dir.join(file).to_string_lossy().into(), vec![7u8; 32]).unwrap()
}

/// Same bank login + accounts in a vault: (Giro, Depot, Visa) with balances.
fn seed(v: &Vault) -> Vec<i64> {
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO bank_connections(connector, display_name, login_name) VALUES ('fints:12345678', 'Bank', 'me')",
        [],
    )
    .unwrap();
    let con = db.last_insert_rowid();
    db.execute("INSERT INTO account_groups(name) VALUES ('G')", [])
        .unwrap();
    let g = db.last_insert_rowid();
    let mut ids = vec![];
    for (ext, name, kind, bal) in [
        ("A1", "Giro", "checking", 100_00),
        ("A2", "Depot", "brokerage", 5_000_00),
        ("A3", "Visa", "credit_card", -50_00),
    ] {
        db.execute(
            "INSERT INTO accounts(connection_id, group_id, external_ref, name, kind, balance_cents) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![con, g, ext, name, kind, bal],
        )
        .unwrap();
        ids.push(db.last_insert_rowid());
    }
    ids
}

fn tick() {
    std::thread::sleep(Duration::from_millis(3));
}

fn names(v: &Vault) -> Vec<(String, Vec<String>)> {
    v.list_user_groups()
        .unwrap()
        .into_iter()
        .map(|g| (g.name, g.accounts.into_iter().map(|a| a.name).collect()))
        .collect()
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

#[test]
fn create_rename_delete() {
    let dir = temp_dir("crud");
    let v = open(&dir, "a.db");
    let ids = seed(&v);

    // nothing grouped yet: only "Ohne Gruppe", sorted by kind (Giro, Visa, Depot)
    let gs = v.list_user_groups().unwrap();
    assert_eq!(gs.len(), 1);
    assert!(gs[0].is_ungrouped && gs[0].uuid.is_empty());
    assert_eq!(names(&v)[0].1, s(&["Giro", "Visa", "Depot"]));

    let privat = v
        .create_user_group(" Privat ".into(), "person".into())
        .unwrap();
    assert!(
        v.create_user_group("privat".into(), "person".into())
            .is_err(),
        "names are unique, ignoring case"
    );
    assert!(v.create_user_group("  ".into(), "person".into()).is_err());
    let firma = v
        .create_user_group("Firma".into(), "briefcase".into())
        .unwrap();

    // empty user groups are listed (drop targets)
    assert_eq!(
        names(&v),
        vec![
            ("Privat".into(), vec![]),
            ("Firma".into(), vec![]),
            ("Ohne Gruppe".into(), s(&["Giro", "Visa", "Depot"]))
        ]
    );

    v.move_account_to_group(ids[0], Some(privat.clone()), None)
        .unwrap();
    v.move_account_to_group(ids[1], Some(privat.clone()), Some(0))
        .unwrap();
    v.move_account_to_group(ids[2], Some(firma.clone()), None)
        .unwrap();
    let gs = v.list_user_groups().unwrap();
    assert_eq!(
        gs.len(),
        2,
        "no 'Ohne Gruppe' when every account is grouped"
    );
    assert_eq!(
        names(&v)[0].1,
        s(&["Depot", "Giro"]),
        "index 0 puts Depot first"
    );
    assert_eq!(gs[0].balance_cents, 5_100_00);
    assert_eq!(gs[0].sf_symbol, "person");

    v.update_user_group(firma.clone(), "Geschäft".into(), "building.2".into(), true)
        .unwrap();
    assert!(v
        .update_user_group(firma.clone(), "PRIVAT".into(), "x".into(), true)
        .is_err());
    assert!(v
        .update_user_group("nope".into(), "X".into(), "x".into(), true)
        .is_err());

    v.reorder_user_groups(vec![firma.clone(), privat.clone()])
        .unwrap();
    v.reorder_accounts_in_group(Some(privat.clone()), vec![ids[0], ids[1], ids[2]])
        .unwrap();
    assert_eq!(
        names(&v),
        vec![
            ("Geschäft".into(), s(&["Visa"])),
            ("Privat".into(), s(&["Giro", "Depot"]))
        ]
    );

    // deleting keeps the accounts, they go to "Ohne Gruppe"
    v.delete_user_group(privat.clone()).unwrap();
    assert!(
        v.move_account_to_group(ids[0], Some(privat), None).is_err(),
        "deleted group"
    );
    assert_eq!(
        names(&v),
        vec![
            ("Geschäft".into(), s(&["Visa"])),
            ("Ohne Gruppe".into(), s(&["Giro", "Depot"]))
        ]
    );
    assert_eq!(v.list_accounts().unwrap().len(), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn excluded_group_leaves_the_total() {
    let dir = temp_dir("total");
    let v = open(&dir, "a.db");
    let ids = seed(&v);
    let firma = v
        .create_user_group("Firma".into(), "briefcase".into())
        .unwrap();
    v.move_account_to_group(ids[1], Some(firma.clone()), None)
        .unwrap();
    let total = |v: &Vault| {
        v.balance_history(vec![], 1)
            .unwrap()
            .last()
            .unwrap()
            .balance_cents
    };
    assert_eq!(total(&v), 5_050_00);

    v.update_user_group(firma.clone(), "Firma".into(), "briefcase".into(), false)
        .unwrap();
    let accs = v.list_accounts().unwrap();
    assert!(!accs.iter().find(|a| a.id == ids[1]).unwrap().in_total);
    assert!(accs.iter().find(|a| a.id == ids[0]).unwrap().in_total);
    assert_eq!(total(&v), 50_00, "Depot excluded from the overall total");
    // explicitly selected accounts are still summed
    let one = v.balance_history(vec![ids[1]], 1).unwrap();
    assert_eq!(one.last().unwrap().balance_cents, 5_000_00);
    assert!(!v.list_user_groups().unwrap()[0].include_in_total);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn new_bank_account_joins_its_siblings_group() {
    let dir = temp_dir("place");
    let v = open(&dir, "a.db");
    let ids = seed(&v);
    let mama = v.create_user_group("Mama".into(), "person".into()).unwrap();
    v.move_account_to_group(ids[0], Some(mama.clone()), None)
        .unwrap();
    v.move_account_to_group(ids[1], Some(mama.clone()), None)
        .unwrap();
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO accounts(connection_id, group_id, external_ref, name) SELECT connection_id, group_id, 'A4', 'Neu' FROM accounts WHERE id = ?1",
        [ids[0]],
    )
    .unwrap();
    let new = db.last_insert_rowid();
    super::place_new_account(&db, new).unwrap();
    let (uuid, ms): (Option<String>, i64) = db
        .query_row(
            "SELECT user_group_uuid, placement_ms FROM accounts WHERE id = ?1",
            [new],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(uuid.as_deref(), Some(mama.as_str()));
    assert_eq!(
        ms, 0,
        "automatic placement never beats a user choice on sync"
    );

    // other login without grouped accounts → stays ungrouped
    db.execute("INSERT INTO accounts(group_id, external_ref, name) SELECT group_id, 'X', 'Bar' FROM accounts LIMIT 1", []).unwrap();
    let other = db.last_insert_rowid();
    super::place_new_account(&db, other).unwrap();
    let uuid: Option<String> = db
        .query_row(
            "SELECT user_group_uuid FROM accounts WHERE id = ?1",
            [other],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(uuid, None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn merge_newest_edit_wins_and_deletions_stick() {
    let dir = temp_dir("merge");
    let key = vec![7u8; 32];
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    let ia = seed(&a);
    let ib = seed(&b);

    // A creates a group and moves Giro into it → B gets both
    let privat = a
        .create_user_group("Privat".into(), "person".into())
        .unwrap();
    a.move_account_to_group(ia[0], Some(privat.clone()), None)
        .unwrap();
    tick();
    let st = b.merge_other_vault(a.file_path(), key.clone()).unwrap();
    assert!(st.groups_updated >= 2);
    assert_eq!(names(&b)[0], ("Privat".into(), s(&["Giro"])));

    // concurrent edits: B renames the group, A moves Visa in and renames an account
    b.update_user_group(privat.clone(), "Familie".into(), "person.2".into(), true)
        .unwrap();
    tick();
    a.move_account_to_group(ia[2], Some(privat.clone()), None)
        .unwrap();
    a.rename_account(ia[1], "Mein Depot".into()).unwrap();
    tick();
    b.merge_other_vault(a.file_path(), key.clone()).unwrap();
    a.merge_other_vault(b.file_path(), key.clone()).unwrap();
    for v in [&a, &b] {
        assert_eq!(
            names(v),
            vec![
                ("Familie".into(), s(&["Giro", "Visa"])),
                ("Ohne Gruppe".into(), s(&["Mein Depot"]))
            ]
        );
    }
    // an older rename doesn't overwrite a newer one
    let depot_b = ib[1];
    b.rename_account(depot_b, "Depot B".into()).unwrap();
    tick();
    a.rename_account(ia[1], "Depot A".into()).unwrap();
    b.merge_other_vault(a.file_path(), key.clone()).unwrap();
    a.merge_other_vault(b.file_path(), key.clone()).unwrap();
    for v in [&a, &b] {
        assert!(v
            .list_accounts()
            .unwrap()
            .iter()
            .any(|x| x.name == "Depot A"));
    }

    // A deletes the group; B (which still has it) must not bring it back
    tick();
    a.delete_user_group(privat.clone()).unwrap();
    a.merge_other_vault(b.file_path(), key.clone()).unwrap();
    assert!(a.list_user_groups().unwrap().iter().all(|g| g.is_ungrouped));
    b.merge_other_vault(a.file_path(), key.clone()).unwrap();
    for v in [&a, &b] {
        let gs = v.list_user_groups().unwrap();
        assert_eq!(gs.len(), 1);
        assert!(gs[0].is_ungrouped);
        assert_eq!(gs[0].accounts.len(), 3);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn merge_brings_new_accounts_with_their_group() {
    let dir = temp_dir("merge-new");
    let key = vec![7u8; 32];
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    let ia = seed(&a);
    let g = a
        .create_user_group("Kinder".into(), "figure.and.child.holdinghands".into())
        .unwrap();
    a.move_account_to_group(ia[2], Some(g.clone()), None)
        .unwrap();
    let st = b.merge_other_vault(a.file_path(), key).unwrap();
    assert_eq!(st.accounts_added, 3);
    assert_eq!(names(&b)[0], ("Kinder".into(), s(&["Visa"])));
    let _ = std::fs::remove_dir_all(dir);
}

fn colors(v: &Vault) -> Vec<(String, String)> {
    let mut c: Vec<_> = v
        .list_accounts()
        .unwrap()
        .into_iter()
        .map(|a| (a.name, a.color_hex))
        .collect();
    c.sort();
    c
}

#[test]
fn account_colors_default_stable_and_sync() {
    let dir = temp_dir("colors");
    let key = vec![7u8; 32];
    let a = open(&dir, "a.db");
    let b = open(&dir, "b.db");
    let bar = |v: &Vault| {
        let db = v.db().unwrap();
        db.execute(
            "INSERT OR IGNORE INTO account_groups(name) VALUES ('H')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO accounts(group_id, external_ref, name) \
             VALUES ((SELECT id FROM account_groups WHERE name = 'H'), 'unrelated', 'Bar')",
            [],
        )
        .unwrap();
    };
    // the same accounts, created in a different order → different local ids
    let ia = seed(&a);
    bar(&a);
    bar(&b);
    seed(&b);

    // same default colours on both devices, distinct, nothing stored
    let ca = colors(&a);
    assert_eq!(ca, colors(&b));
    let distinct: std::collections::HashSet<_> = ca.iter().map(|(_, c)| c.clone()).collect();
    assert_eq!(
        distinct.len(),
        ca.len(),
        "no two accounts share a default colour"
    );
    assert!(
        ca.iter()
            .all(|(_, c)| super::ACCOUNT_COLOR_PALETTE[..super::DISTINCT].contains(&c.as_str())),
        "a few accounts only get the clearly distinct hues"
    );
    assert!(ca
        .iter()
        .all(|(_, c)| super::ACCOUNT_COLOR_PALETTE.contains(&c.as_str())));
    let stored: i64 = a
        .db()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM accounts WHERE color_hex IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stored, 0);

    // a user choice syncs both ways; the newer one wins
    assert!(a.set_account_color(ia[0], Some("blue".into())).is_err());
    assert!(a.set_account_color(999, Some("#123456".into())).is_err());
    a.set_account_color(ia[0], Some("#abcdef".into())).unwrap();
    tick();
    b.merge_other_vault(a.file_path(), key.clone()).unwrap();
    let giro = |v: &Vault| {
        v.list_accounts()
            .unwrap()
            .into_iter()
            .find(|x| x.name == "Giro")
            .unwrap()
            .color_hex
    };
    assert_eq!(giro(&b), "#ABCDEF");
    let gb = b
        .list_accounts()
        .unwrap()
        .into_iter()
        .find(|x| x.name == "Giro")
        .unwrap()
        .id;
    b.set_account_color(gb, None).unwrap();
    tick();
    a.merge_other_vault(b.file_path(), key.clone()).unwrap();
    assert_eq!(
        giro(&a),
        ca.iter().find(|(n, _)| n == "Giro").unwrap().1,
        "reset to default syncs too"
    );
    assert_eq!(super::account_color_palette().len(), 12);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn default_colors_distinct_up_to_twelve_and_respect_choices() {
    let dir = temp_dir("colors12");
    let v = open(&dir, "a.db");
    let ids = seed(&v);
    let add = |n: usize| {
        let db = v.db().unwrap();
        for i in 0..n {
            db.execute(
                "INSERT INTO accounts(group_id, external_ref, name) VALUES (1, ?1, ?1)",
                [format!("X{i}-{}", uuid::Uuid::new_v4())],
            )
            .unwrap();
        }
    };
    let counts = |v: &Vault| {
        let mut m: HashMap<String, usize> = HashMap::new();
        for a in v.list_accounts().unwrap() {
            *m.entry(a.color_hex).or_default() += 1;
        }
        m
    };
    // a chosen colour is reserved: no default account gets it
    v.set_account_color(ids[0], Some(super::ACCOUNT_COLOR_PALETTE[3].into()))
        .unwrap();
    add(9); // 12 accounts
    let c = counts(&v);
    assert_eq!(c.len(), 12, "12 accounts → 12 different colours");
    assert_eq!(c[super::ACCOUNT_COLOR_PALETTE[3]], 1);
    add(1); // 13th account: colours repeat as evenly as possible
    let c = counts(&v);
    assert_eq!(c.len(), 12);
    assert_eq!(c.values().max(), Some(&2));
    let _ = std::fs::remove_dir_all(dir);
}

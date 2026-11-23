//! Export / import of rules and tags between two vaults (#1057).

use std::sync::Arc;

use rusqlite::params;

use super::*;
use crate::test_support::TempDir;

const KEY: [u8; 32] = [5u8; 32];

fn open(dir: &TempDir, file: &str) -> Arc<Vault> {
    Vault::open(dir.join(file).to_string_lossy().into(), KEY.to_vec()).unwrap()
}

fn account(v: &Vault, name: &str, iban: &str, number: &str) -> i64 {
    let c = v.db().unwrap();
    c.execute(
        "INSERT OR IGNORE INTO account_groups(name) VALUES ('G')",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO accounts(group_id, external_ref, name, iban, account_number) \
         VALUES ((SELECT id FROM account_groups WHERE name='G'), ?1, ?1, ?2, ?3)",
        params![name, iban, number],
    )
    .unwrap();
    c.last_insert_rowid()
}

fn category(v: &Vault, name: &str, parent: Option<i64>) -> i64 {
    v.create_category(name.into(), parent, "tag".into(), "#336699".into())
        .unwrap()
}

fn rule(name: &str, category_id: i64, accounts: Vec<i64>, tags: Vec<i64>) -> Rule {
    Rule {
        id: 0,
        name: name.into(),
        category_id,
        field: RuleField::Counterparty,
        match_kind: MatchKind::Keyword,
        pattern: format!("{name}|{}", name.to_uppercase()),
        case_sensitive: false,
        min_amount_cents: Some(-50_000),
        max_amount_cents: None,
        account_ids: accounts,
        priority: 0,
        enabled: true,
        match_count: 0,
        tag_ids: tags,
    }
}

/// Fictional IBANs (valid shape, invented numbers).
const IBAN_GIRO: &str = "DE89 3704 0044 0532 0130 00";
const IBAN_CARD: &str = "DE02120300000000202051";

/// The source vault: a category tree, two accounts, two tags and three rules.
fn source(dir: &TempDir) -> Arc<Vault> {
    let a = open(dir, "a.db");
    let giro = account(&a, "Girokonto", IBAN_GIRO, "");
    let card = account(&a, "Kreditkarte", IBAN_CARD, "4711");
    let food = category(&a, "Lebensmittel", None);
    let market = category(&a, "Supermarkt", Some(food));
    let fun = category(&a, "Freizeit", None);
    let urlaub = a.create_tag("Urlaub".into(), "#FF9500".into()).unwrap();
    let familie = a.create_tag("Familie".into(), String::new()).unwrap();
    a.save_rule(rule("rewe", market, vec![giro], vec![familie.id]))
        .unwrap();
    a.save_rule(rule(
        "kino",
        fun,
        vec![giro, card],
        vec![urlaub.id, familie.id],
    ))
    .unwrap();
    // tags only, all accounts
    a.save_rule(rule("hotel", 0, vec![], vec![urlaub.id]))
        .unwrap();
    a
}

#[test]
fn round_trip_into_a_fresh_vault_matches_by_path_iban_and_tag_name() {
    let dir = TempDir::new("rx-roundtrip");
    let a = source(&dir);
    let json = a.export_rules(vec![]).unwrap();
    assert!(json.contains("\"path\""), "{json}");
    assert!(json.contains(IBAN_GIRO), "{json}");

    // the target: other ids (extra rows first), other account order and names,
    // one tag already there in other spelling
    let b = open(&dir, "b.db");
    category(&b, "Wohnen", None);
    let card = account(&b, "Visa", "DE02 1203 0000 0000 2020 51", "");
    let giro = account(&b, "Giro (neu)", "de89370400440532013000", "");
    let food = category(&b, "Lebensmittel", None);
    let market = category(&b, "Supermarkt", Some(food));
    let fun = category(&b, "freizeit", None); // case differs
    b.create_tag("familie".into(), String::new()).unwrap();

    let report = b.import_rules_and_tags(json.clone()).unwrap();
    assert_eq!(report.rules_imported, 3, "{report:?}");
    assert_eq!(report.rules_skipped, 0, "{report:?}");
    assert_eq!(
        (report.tags_created, report.tags_existing),
        (1, 1),
        "{report:?}"
    );
    // the details name them; an existing tag as this vault spells it
    assert_eq!(report.tags_created_names, ["Urlaub"]);
    assert_eq!(report.tags_existing_names, ["familie"]);
    assert_eq!(report.rules_imported_lines, ["rewe", "kino", "hotel"]);
    assert!(report.rules_skipped_lines.is_empty());

    let tags = b.list_tags().unwrap();
    assert_eq!(tags.len(), 2);
    let urlaub = tags.iter().find(|t| t.name == "Urlaub").unwrap();
    assert_eq!(urlaub.color_hex, "#FF9500");
    let familie = tags.iter().find(|t| t.name == "familie").unwrap();

    let rules = b.list_rules().unwrap();
    let rewe = rules.iter().find(|r| r.name == "rewe").unwrap();
    assert_eq!(rewe.category_id, market);
    assert_eq!(rewe.account_ids, vec![giro]);
    assert_eq!(rewe.tag_ids, vec![familie.id]);
    assert_eq!(rewe.min_amount_cents, Some(-50_000));
    let kino = rules.iter().find(|r| r.name == "kino").unwrap();
    assert_eq!(kino.category_id, fun);
    let mut accounts = kino.account_ids.clone();
    accounts.sort_unstable();
    let mut expected = vec![giro, card];
    expected.sort_unstable();
    assert_eq!(accounts, expected);
    let hotel = rules.iter().find(|r| r.name == "hotel").unwrap();
    assert_eq!((hotel.category_id, hotel.account_ids.is_empty()), (0, true));
    assert_eq!(hotel.tag_ids, vec![urlaub.id]);
    // the file's order is kept
    let order: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(order, ["rewe", "kino", "hotel"]);

    // importing the same file again adds nothing
    let again = b.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (again.rules_imported, again.rules_skipped),
        (0, 3),
        "{again:?}"
    );
    assert_eq!(again.tags_created, 0);
    assert!(again.rules_imported_lines.is_empty());
    assert_eq!(again.rules_skipped_lines.len(), 3, "{again:?}");
    assert_eq!(again.tags_existing_names.len(), 2, "{again:?}");
    assert_eq!(b.list_rules().unwrap().len(), 3);
}

#[test]
fn missing_category_or_accounts_skip_the_rule_and_say_so() {
    let dir = TempDir::new("rx-missing");
    let a = source(&dir);
    let json = a.export_rules(vec![]).unwrap();

    // no "Freizeit", no Kreditkarte, Girokonto found by account name only
    let b = open(&dir, "b.db");
    let giro = account(&b, "Girokonto", "", "");
    let food = category(&b, "Lebensmittel", None);
    category(&b, "Supermarkt", Some(food));

    let report = b.import_rules_and_tags(json).unwrap();
    // rewe imported, kino skipped (category), hotel imported (tags only)
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (2, 1),
        "{report:?}"
    );
    assert_eq!(report.rules_imported_lines, ["rewe", "hotel"], "{report:?}");
    assert_eq!(report.rules_skipped_lines.len(), 1, "{report:?}");
    assert!(
        report.rules_skipped_lines[0].contains("kino")
            && report.rules_skipped_lines[0].contains("Freizeit"),
        "{report:?}"
    );
    let rewe = b
        .list_rules()
        .unwrap()
        .into_iter()
        .find(|r| r.name == "rewe")
        .unwrap();
    assert_eq!(rewe.account_ids, vec![giro]);
}

#[test]
fn a_rule_never_widens_to_all_accounts() {
    let dir = TempDir::new("rx-accounts");
    let a = open(&dir, "a.db");
    let card = account(&a, "Kreditkarte", IBAN_CARD, "");
    let giro = account(&a, "Girokonto", IBAN_GIRO, "");
    let cat = category(&a, "Freizeit", None);
    a.save_rule(rule("nur-karte", cat, vec![card], vec![]))
        .unwrap();
    a.save_rule(rule("beide", cat, vec![card, giro], vec![]))
        .unwrap();
    let json = a.export_rules(vec![]).unwrap();

    let b = open(&dir, "b.db");
    let giro_b = account(&b, "Giro", IBAN_GIRO, "");
    category(&b, "Freizeit", None);
    let report = b.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (1, 1),
        "{report:?}"
    );
    let rules = b.list_rules().unwrap();
    assert!(rules.iter().all(|r| r.name != "nur-karte"));
    let beide = rules.iter().find(|r| r.name == "beide").unwrap();
    assert_eq!(beide.account_ids, vec![giro_b]);
    // imported, but naming the account it left out; the other one skipped
    assert_eq!(report.rules_imported_lines.len(), 1, "{report:?}");
    assert!(
        report.rules_imported_lines[0].contains("beide")
            && report.rules_imported_lines[0].contains("Kreditkarte"),
        "{report:?}"
    );
    assert!(
        report.rules_skipped_lines[0].contains("nur-karte"),
        "{report:?}"
    );
}

#[test]
fn starter_category_is_found_by_its_role_after_a_rename() {
    let dir = TempDir::new("rx-role");
    let a = open(&dir, "a.db");
    let cat = category(&a, "Supermarkt", None);
    a.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role = 'Supermarkt' WHERE id = ?1",
            [cat],
        )
        .unwrap();
    a.save_rule(rule("aldi", cat, vec![], vec![])).unwrap();
    let json = a.export_rules(vec![]).unwrap();

    let b = open(&dir, "b.db");
    let renamed = category(&b, "Groceries", None);
    b.db()
        .unwrap()
        .execute(
            "UPDATE categories SET builtin_role = 'Supermarkt' WHERE id = ?1",
            [renamed],
        )
        .unwrap();
    let report = b.import_rules_and_tags(json).unwrap();
    assert_eq!(report.rules_imported, 1, "{report:?}");
    assert_eq!(b.list_rules().unwrap()[0].category_id, renamed);
}

#[test]
fn single_rule_and_tag_exports() {
    let dir = TempDir::new("rx-single");
    let a = source(&dir);
    let kino = a
        .list_rules()
        .unwrap()
        .into_iter()
        .find(|r| r.name == "kino")
        .unwrap();
    let json = a.export_rules(vec![kino.id]).unwrap();
    let file: ExchangeFile = serde_json::from_str(&json).unwrap();
    assert_eq!(file.rules.len(), 1);
    // only the tags the exported rule adds
    let mut names: Vec<_> = file.tags.iter().map(|t| t.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Familie", "Urlaub"]);

    let urlaub = a
        .list_tags()
        .unwrap()
        .into_iter()
        .find(|t| t.name == "Urlaub")
        .unwrap();
    let tags_json = a.export_tags(vec![urlaub.id]).unwrap();
    let b = open(&dir, "b.db");
    let report = b.import_rules_and_tags(tags_json).unwrap();
    assert_eq!((report.tags_created, report.rules_imported), (1, 0));
    assert_eq!(b.list_tags().unwrap()[0].color_hex, "#FF9500");
}

#[test]
fn foreign_or_newer_files_are_refused() {
    let dir = TempDir::new("rx-refuse");
    let v = open(&dir, "v.db");
    assert!(v.import_rules_and_tags("not json".into()).is_err());
    assert!(
        v.import_rules_and_tags(r#"{"format":"other","version":1}"#.into())
            .is_err()
    );
    let newer = format!(r#"{{"format":"{FORMAT}","version":{}}}"#, VERSION + 1);
    assert!(v.import_rules_and_tags(newer).is_err());
    assert!(v.list_rules().unwrap().is_empty());
}

#[test]
fn preview_lists_everything_with_what_the_import_would_do() {
    let dir = TempDir::new("rx-preview");
    let a = source(&dir);
    let json = a.export_rules(vec![]).unwrap();

    // no "Freizeit"; "Familie" already there
    let b = open(&dir, "b.db");
    account(&b, "Girokonto", IBAN_GIRO, "");
    let food = category(&b, "Lebensmittel", None);
    category(&b, "Supermarkt", Some(food));
    b.create_tag("Familie".into(), String::new()).unwrap();

    let preview = b.preview_rules_and_tags(json).unwrap();
    let names: Vec<_> = preview.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["rewe", "kino", "hotel"]);
    assert_eq!(preview.rules[0].category_path, "Lebensmittel › Supermarkt");
    assert_eq!(preview.rules[0].problem, None);
    assert!(
        preview.rules[1]
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("Freizeit")),
        "{preview:?}"
    );
    assert_eq!(preview.rules[2].category_path, "");
    let tags: Vec<_> = preview
        .tags
        .iter()
        .map(|t| (t.name.as_str(), t.exists))
        .collect();
    assert_eq!(tags, [("Urlaub", false), ("Familie", true)]);
    // the preview changes nothing
    assert!(b.list_rules().unwrap().is_empty());
    assert_eq!(b.list_tags().unwrap().len(), 1);
}

#[test]
fn only_the_chosen_rules_and_tags_are_imported() {
    let dir = TempDir::new("rx-selected");
    let a = source(&dir);
    let json = a.export_rules(vec![]).unwrap();

    let b = open(&dir, "b.db");
    account(&b, "Girokonto", IBAN_GIRO, "");
    let food = category(&b, "Lebensmittel", None);
    category(&b, "Supermarkt", Some(food));

    // "rewe" only, no tag ticked: the tag "rewe" adds comes along all the same
    let report = b
        .import_rules_and_tags_selected(json.clone(), vec![0], vec![])
        .unwrap();
    assert_eq!(report.rules_imported_lines, ["rewe"], "{report:?}");
    assert_eq!(report.tags_created_names, ["Familie"], "{report:?}");
    let rules = b.list_rules().unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].tag_ids.len(), 1);

    // a tag only, no rule
    let report = b
        .import_rules_and_tags_selected(json, vec![], vec!["urlaub".into()])
        .unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (0, 0),
        "{report:?}"
    );
    assert_eq!(report.tags_created_names, ["Urlaub"], "{report:?}");
    assert_eq!(b.list_tags().unwrap().len(), 2);
    assert_eq!(b.list_rules().unwrap().len(), 1);
}

#[test]
fn a_skipped_rule_brings_no_tag_along() {
    let dir = TempDir::new("rx-skip-tags");
    let a = source(&dir);
    let json = a.export_rules(vec![]).unwrap();

    // no "Freizeit": "kino" is skipped, its tag "Urlaub" (only "kino" and "hotel" use it) stays out
    let b = open(&dir, "b.db");
    account(&b, "Girokonto", IBAN_GIRO, "");
    let report = b
        .import_rules_and_tags_selected(json, vec![1], vec![])
        .unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (0, 1),
        "{report:?}"
    );
    assert!(report.tags_created_names.is_empty(), "{report:?}");
    assert!(b.list_tags().unwrap().is_empty());
}

/// The export of `a` as JSON, changed by `edit`.
fn edited(a: &Vault, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut v: serde_json::Value = serde_json::from_str(&a.export_rules(vec![]).unwrap()).unwrap();
    edit(&mut v);
    v.to_string()
}

#[test]
fn unknown_field_is_skipped_not_widened() {
    let dir = TempDir::new("rx-field");
    let a = open(&dir, "a.db");
    let cat = category(&a, "Freizeit", None);
    a.save_rule(rule("kino", cat, vec![], vec![])).unwrap();
    let json = edited(&a, |v| v["rules"][0]["field"] = "counterprty".into());

    let b = open(&dir, "b.db");
    category(&b, "Freizeit", None);
    let preview = b.preview_rules_and_tags(json.clone()).unwrap();
    assert!(preview.rules[0].problem.is_some(), "{preview:?}");
    let report = b.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (0, 1),
        "{report:?}"
    );
    assert!(b.list_rules().unwrap().is_empty());
}

#[test]
fn rules_differing_in_accounts_amounts_or_case_are_no_duplicates() {
    let dir = TempDir::new("rx-dupes");
    let a = open(&dir, "a.db");
    let giro = account(&a, "Girokonto", IBAN_GIRO, "");
    let card = account(&a, "Kreditkarte", IBAN_CARD, "");
    let cat = category(&a, "Freizeit", None);
    a.save_rule(rule("kino", cat, vec![giro], vec![])).unwrap();
    a.save_rule(rule("kino", cat, vec![card], vec![])).unwrap();
    a.save_rule(Rule {
        min_amount_cents: None,
        ..rule("kino", cat, vec![giro], vec![])
    })
    .unwrap();
    a.save_rule(Rule {
        case_sensitive: true,
        ..rule("kino", cat, vec![giro], vec![])
    })
    .unwrap();
    let json = a.export_rules(vec![]).unwrap();

    // one file into an empty vault: four different rules, none a duplicate
    let b = open(&dir, "b.db");
    account(&b, "Girokonto", IBAN_GIRO, "");
    account(&b, "Kreditkarte", IBAN_CARD, "");
    category(&b, "Freizeit", None);
    let report = b.import_rules_and_tags(json.clone()).unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (4, 0),
        "{report:?}"
    );
    // the same file again: all four are present now
    let again = b.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (again.rules_imported, again.rules_skipped),
        (0, 4),
        "{again:?}"
    );
}

#[test]
fn rules_save_rule_refuses_are_skipped_in_preview_and_import() {
    let dir = TempDir::new("rx-invalid");
    let a = source(&dir);
    // "rewe" gets a broken regex, "hotel" (tags only) an empty pattern
    let json = edited(&a, |v| {
        v["rules"][0]["match_kind"] = "regex".into();
        v["rules"][0]["pattern"] = "(unclosed".into();
        v["rules"][2]["pattern"] = "  ".into();
    });

    let b = open(&dir, "b.db");
    account(&b, "Girokonto", IBAN_GIRO, "");
    let food = category(&b, "Lebensmittel", None);
    category(&b, "Supermarkt", Some(food));
    let preview = b.preview_rules_and_tags(json.clone()).unwrap();
    assert!(preview.rules[0].problem.is_some(), "{preview:?}");
    assert!(preview.rules[2].problem.is_some(), "{preview:?}");
    // the reason without the generic „Ungültige Eingabe:“ prefix
    assert!(
        !preview.rules[2]
            .problem
            .as_deref()
            .unwrap()
            .contains("Ungültige"),
        "{preview:?}"
    );

    // only these two chosen: neither is imported, so their tags stay out
    let report = b
        .import_rules_and_tags_selected(json, vec![0, 2], vec![])
        .unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (0, 2),
        "{report:?}"
    );
    assert!(b.list_tags().unwrap().is_empty(), "{report:?}");
}

#[test]
fn a_tag_found_by_uuid_wins_over_one_with_the_same_name() {
    let dir = TempDir::new("rx-uuid");
    let a = open(&dir, "a.db");
    let cat = category(&a, "Arbeit", None);
    let work = a.create_tag("Work".into(), String::new()).unwrap();
    a.save_rule(rule("buero", cat, vec![], vec![work.id]))
        .unwrap();
    let json = a.export_rules(vec![]).unwrap();
    let uuid: String = a
        .db()
        .unwrap()
        .query_row("SELECT uuid FROM tags WHERE id = ?1", [work.id], |r| {
            r.get(0)
        })
        .unwrap();

    // the target: another tag named "Work", and the exported one renamed to "Office"
    let b = open(&dir, "b.db");
    category(&b, "Arbeit", None);
    let other = b.create_tag("Work".into(), String::new()).unwrap();
    let office = b.create_tag("Office".into(), String::new()).unwrap();
    // uuids are derived from the name: give the same-named tag another one first
    b.db()
        .unwrap()
        .execute(
            "UPDATE tags SET uuid = 'other-work' WHERE id = ?1",
            [other.id],
        )
        .unwrap();
    b.db()
        .unwrap()
        .execute(
            "UPDATE tags SET uuid = ?1 WHERE id = ?2",
            params![uuid, office.id],
        )
        .unwrap();
    let report = b.import_rules_and_tags(json).unwrap();
    assert_eq!(report.tags_existing_names, ["Office"], "{report:?}");
    let buero = b
        .list_rules()
        .unwrap()
        .into_iter()
        .find(|r| r.name == "buero")
        .unwrap();
    assert_eq!(buero.tag_ids, vec![office.id]);
    assert_ne!(buero.tag_ids, vec![other.id]);
}

#[test]
fn an_invalid_tag_name_is_reported_and_the_import_goes_on() {
    let dir = TempDir::new("rx-tagname");
    let a = source(&dir);
    let long = "x".repeat(70);
    let json = edited(&a, |v| {
        v["tags"].as_array_mut().unwrap().insert(
            0,
            serde_json::json!({"uuid": "", "name": long, "color_hex": ""}),
        );
    });

    let b = open(&dir, "b.db");
    account(&b, "Girokonto", IBAN_GIRO, "");
    let food = category(&b, "Lebensmittel", None);
    category(&b, "Supermarkt", Some(food));
    let preview = b.preview_rules_and_tags(json.clone()).unwrap();
    assert!(preview.tags[0].problem.is_some(), "{preview:?}");
    assert!(
        preview.tags[1..].iter().all(|t| t.problem.is_none()),
        "{preview:?}"
    );

    let report = b.import_rules_and_tags(json).unwrap();
    assert_eq!(report.tags_skipped_lines.len(), 1, "{report:?}");
    // the tags after it and the rules still came in
    assert_eq!(report.tags_created, 2, "{report:?}");
    assert!(report.rules_imported >= 2, "{report:?}");
}

#[test]
fn rules_differing_only_in_their_tags_are_no_duplicates() {
    let dir = TempDir::new("rx-dupe-tags");
    let a = open(&dir, "a.db");
    let urlaub = a.create_tag("Urlaub".into(), String::new()).unwrap();
    let familie = a.create_tag("Familie".into(), String::new()).unwrap();
    // tag-only rules: same everything but the tag
    a.save_rule(rule("hotel", 0, vec![], vec![urlaub.id]))
        .unwrap();
    a.save_rule(rule("hotel", 0, vec![], vec![familie.id]))
        .unwrap();
    let json = a.export_rules(vec![]).unwrap();

    let b = open(&dir, "b.db");
    let report = b.import_rules_and_tags(json.clone()).unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (2, 0),
        "{report:?}"
    );
    // an exact re-import still adds nothing
    let preview = b.preview_rules_and_tags(json.clone()).unwrap();
    assert!(
        preview.rules.iter().all(|r| r.problem.is_some()),
        "{preview:?}"
    );
    let again = b.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (again.rules_imported, again.rules_skipped),
        (0, 2),
        "{again:?}"
    );
}

#[test]
fn an_iban_shared_by_several_accounts_is_narrowed_by_number_and_name() {
    let dir = TempDir::new("rx-iban-shared");
    let a = open(&dir, "a.db");
    let sub = account(&a, "Unterkonto", IBAN_GIRO, "0002");
    let cat = category(&a, "Freizeit", None);
    a.save_rule(rule("kino", cat, vec![sub], vec![])).unwrap();
    let json = a.export_rules(vec![]).unwrap();

    // the IBAN alone would pick the first account with it
    let b = open(&dir, "b.db");
    account(&b, "Hauptkonto", IBAN_GIRO, "0001");
    let wanted = account(&b, "Unterkonto", IBAN_GIRO, "0002");
    category(&b, "Freizeit", None);
    let report = b.import_rules_and_tags(json.clone()).unwrap();
    assert_eq!(report.rules_imported, 1, "{report:?}");
    assert_eq!(b.list_rules().unwrap()[0].account_ids, vec![wanted]);

    // number and name tell nothing apart: no account is guessed
    let c = open(&dir, "c.db");
    account(&c, "Hauptkonto", IBAN_GIRO, "0001");
    account(&c, "Tagesgeld", IBAN_GIRO, "0003");
    category(&c, "Freizeit", None);
    let report = c.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (report.rules_imported, report.rules_skipped),
        (0, 1),
        "{report:?}"
    );
}

#[test]
fn a_renamed_tag_found_by_uuid_still_makes_a_duplicate() {
    let dir = TempDir::new("rx-dupe-renamed");
    let a = open(&dir, "a.db");
    let urlaub = a.create_tag("Urlaub".into(), String::new()).unwrap();
    a.save_rule(rule("hotel", 0, vec![], vec![urlaub.id]))
        .unwrap();
    let json = a.export_rules(vec![]).unwrap();

    let b = open(&dir, "b.db");
    let first = b.import_rules_and_tags(json.clone()).unwrap();
    assert_eq!(first.rules_imported, 1, "{first:?}");
    // renamed here; the file still says „Urlaub“, the uuid is the same
    let tag = b.list_tags().unwrap()[0].clone();
    b.rename_tag(tag.id, "Reisen".into()).unwrap();

    let preview = b.preview_rules_and_tags(json.clone()).unwrap();
    assert!(preview.rules[0].problem.is_some(), "{preview:?}");
    let again = b.import_rules_and_tags(json).unwrap();
    assert_eq!(
        (
            again.rules_imported,
            again.rules_skipped,
            again.tags_created
        ),
        (0, 1, 0),
        "{again:?}"
    );
    assert_eq!(b.list_rules().unwrap().len(), 1);
}

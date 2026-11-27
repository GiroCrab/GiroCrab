use super::*;

fn raw(id: i64, parent: Option<i64>, name: &str, uuid: &str) -> RawCategoryRow {
    (
        id,
        parent,
        name.into(),
        "tag".into(),
        String::new(),
        0,
        None,
        Some(uuid.into()),
    )
}

/// Same-named siblings receive deterministic display suffixes ordered by UUID,
/// reserving literal labels like `"Sparen (2)"`, while `edit_name` stays clean
/// and rows under distinct parents never collide.
#[test]
fn disambiguate_category_rows_assigns_uuid_ordered_suffixes_and_clean_edit_names() {
    let rows = disambiguate_category_rows(vec![
        raw(20, None, "Sparen", "uuid-b"),
        raw(10, None, "sparen", "uuid-a"),
        raw(30, None, "Sparen (2)", "uuid-c"),
        raw(40, Some(10), "Sparen", "uuid-d"),
    ]);
    let by_id = |id: i64| rows.iter().find(|r| r.0 == id).unwrap();

    // Lower UUID keeps the plain name even when its local ID differs; higher UUID
    // skips "(2)" because a literal "Sparen (2)" sibling already reserves it.
    assert_eq!(by_id(10).2, "sparen");
    assert_eq!(by_id(10).7, "sparen");
    assert_eq!(by_id(20).2, "Sparen (3)");
    assert_eq!(by_id(20).7, "Sparen");
    assert_eq!(by_id(30).2, "Sparen (2)");
    assert_eq!(by_id(30).7, "Sparen (2)");
    // Different parent does not collide with top-level siblings.
    assert_eq!(by_id(40).2, "Sparen");
    assert_eq!(by_id(40).7, "Sparen");
}

#[test]
fn list_categories_exposes_edit_name_for_live_categories() {
    let dir = crate::test_support::TempDir::new("names-edit-name");
    let v = Vault::open(dir.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap();
    let id = v
        .create_category("Sparen".into(), None, "tag".into(), "".into())
        .unwrap();
    let cat = v
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.id == id)
        .unwrap();
    assert_eq!(cat.name, "Sparen");
    assert_eq!(cat.edit_name, "Sparen");
}

/// Same-named siblings stay separate rows: the display label carries a suffix,
/// `edit_name` and the stored name stay what the user typed, and saving the edit
/// name again neither bakes a suffix into the stored name nor re-stamps it.
#[test]
fn same_named_siblings_get_display_suffixes_and_clean_edits() {
    let dir = crate::test_support::TempDir::new("names-siblings");
    let v = Vault::open(dir.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap();
    let a = v
        .create_category("Sparen".into(), None, "tag".into(), "".into())
        .unwrap();
    let b = v
        .create_category("sparen".into(), None, "tag".into(), "".into())
        .unwrap();
    // a literal label of the derived shape stays distinguishable from both
    let literal = v
        .create_category("Sparen (2)".into(), None, "tag".into(), "".into())
        .unwrap();
    let cats = v.list_categories(None, None).unwrap();
    let by_id = |id: i64| cats.iter().find(|c| c.id == id).unwrap();
    assert_eq!(by_id(literal).name, "Sparen (2)");
    assert_eq!(by_id(literal).edit_name, "Sparen (2)");
    // Every device must label the same rows alike, whatever their local ids: the lower
    // UUID keeps the plain name.
    let uuid = |id: i64| -> String {
        v.db()
            .unwrap()
            .query_row("SELECT uuid FROM categories WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap()
    };
    let (plain, suffixed) = if uuid(a) < uuid(b) { (a, b) } else { (b, a) };
    assert_eq!(by_id(plain).name, by_id(plain).edit_name);
    assert_eq!(
        by_id(suffixed).name,
        format!("{} (3)", by_id(suffixed).edit_name)
    );
    assert_eq!(by_id(a).edit_name, "Sparen");
    assert_eq!(by_id(b).edit_name, "sparen");

    // the vault mutex is not reentrant: drop the connection before update_category
    let ms_before: i64 = v
        .db()
        .unwrap()
        .query_row(
            "SELECT name_ms FROM categories WHERE id = ?1",
            [suffixed],
            |r| r.get(0),
        )
        .unwrap();
    v.update_category(
        suffixed,
        by_id(suffixed).edit_name.clone(),
        None,
        "tag".into(),
        "".into(),
    )
    .unwrap();
    let (stored, ms_after): (String, i64) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT name, name_ms FROM categories WHERE id = ?1",
            [suffixed],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, by_id(suffixed).edit_name);
    assert_eq!(ms_after, ms_before);
}

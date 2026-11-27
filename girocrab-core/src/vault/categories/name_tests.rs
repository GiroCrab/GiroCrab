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

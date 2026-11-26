use super::*;

fn vault(dir: &crate::test_support::TempDir) -> std::sync::Arc<Vault> {
    Vault::open(dir.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap()
}

/// Same-named siblings stay separate rows: the display label carries a suffix,
/// `edit_name` and the stored name stay what the user typed, and saving the edit
/// name again neither bakes a suffix into the stored name nor re-stamps it.
#[test]
fn same_named_siblings_get_display_suffixes_and_clean_edits() {
    let dir = crate::test_support::TempDir::new("names-siblings");
    let v = vault(&dir);
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

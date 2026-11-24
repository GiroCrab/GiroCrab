use super::*;
use crate::i18n::{Lang, with_language};

fn vault(dir: &crate::test_support::TempDir) -> std::sync::Arc<Vault> {
    Vault::open(dir.join("v.db").to_string_lossy().into(), vec![7; 32]).unwrap()
}

#[test]
fn cached_booking_names_follow_language_without_a_write() {
    let dir = crate::test_support::TempDir::new("roles-language-cache");
    let v = vault(&dir);
    v.seed_demo_data().unwrap();
    let read = || {
        v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
    };
    let german = with_language(Lang::De, read)
        .into_iter()
        .find(|t| t.category_name == "Supermarkt")
        .unwrap();
    let english = with_language(Lang::En, read)
        .into_iter()
        .find(|t| t.id == german.id)
        .unwrap();
    assert_eq!(english.category_name, "Supermarket");
    assert_eq!(english.category_path, "Food › Supermarket");
    assert_eq!(english.flow, german.flow);
    let german_again = with_language(Lang::De, read)
        .into_iter()
        .find(|t| t.id == german.id)
        .unwrap();
    assert_eq!(german_again.category_path, german.category_path);
}

#[test]
fn custom_names_paths_budgets_and_imports_stay_literal() {
    let dir = crate::test_support::TempDir::new("roles-custom");
    let v = vault(&dir);
    v.seed_starter_categories().unwrap();
    let cats = v.list_categories(None, None).unwrap();
    let leisure = cats.iter().find(|c| c.name == "Freizeit").unwrap().id;
    let custom = v
        .create_category("Freizeit".into(), Some(leisure), "tag".into(), "".into())
        .unwrap();
    v.set_category_budget(custom, Some(10000)).unwrap();
    with_language(Lang::En, || {
        let cats = v.list_categories(None, None).unwrap();
        let c = cats.iter().find(|c| c.id == custom).unwrap();
        assert_eq!(c.name, "Freizeit");
        assert_eq!(c.path, "Leisure › Freizeit");
        let budgets = v.list_category_budget_progress(2026, 11).unwrap();
        let b = budgets.iter().find(|b| b.category_id == custom).unwrap();
        assert_eq!(b.path, c.path);
        // Saving a user name matching a translated default is a real rename.
        v.update_category(
            custom,
            "Leisure".into(),
            Some(leisure),
            "tag".into(),
            "".into(),
        )
        .unwrap();
    });
    assert_eq!(
        v.list_categories(None, None)
            .unwrap()
            .iter()
            .find(|c| c.id == custom)
            .unwrap()
            .name,
        "Leisure"
    );
    // Reusing a starter row for an import transfers ownership of its name.
    let db = v.db().unwrap();
    let mut cache = HashMap::new();
    let mut created = 0;
    let imported = crate::importers::moneymoney_import::find_or_create_category(
        &db,
        &["Freizeit".into()],
        &mut cache,
        &mut created,
    )
    .unwrap();
    assert_eq!(imported, leisure);
    drop(db);
    with_language(Lang::En, || {
        assert_eq!(
            v.list_categories(None, None)
                .unwrap()
                .iter()
                .find(|c| c.id == leisure)
                .unwrap()
                .name,
            "Freizeit"
        );
    });
}

/// An import that reuses a built-in top-level category takes over its name
/// (`builtin_role` is cleared) but keeps the role it started with: its bookings stay
/// investments. Before #931 clearing `builtin_role` also dropped the classification; the
/// ADR lists this as a deliberate exception.
#[test]
fn import_reusing_a_builtin_root_keeps_its_seeded_role() {
    let dir = crate::test_support::TempDir::new("roles-import-reuse");
    let v = vault(&dir);
    v.seed_starter_categories().unwrap();
    let saving = v
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.name == "Sparen & Investieren")
        .unwrap();
    assert_eq!(saving.role, Some(CategoryRole::Investment));
    let db = v.db().unwrap();
    let imported = crate::importers::moneymoney_import::find_or_create_category(
        &db,
        &["Sparen & Investieren".into()],
        &mut HashMap::new(),
        &mut 0,
    )
    .unwrap();
    assert_eq!(imported, saving.id);
    let (builtin, role_ms): (Option<String>, i64) = db
        .query_row(
            "SELECT builtin_role, role_ms FROM categories WHERE id = ?1",
            [saving.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    drop(db);
    assert_eq!((builtin, role_ms), (None, 0));
    let after = v
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.id == saving.id)
        .unwrap();
    assert_eq!(after.role, Some(CategoryRole::Investment));
    assert_eq!(after.effective_role, CategoryRole::Investment);
}

#[test]
fn renamed_builtin_keeps_role_and_custom_name_does_not_classify() {
    let dir = crate::test_support::TempDir::new("roles-flow");
    let v = vault(&dir);
    v.seed_demo_data().unwrap();
    let category = v
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.name == "Sparen & Investieren")
        .unwrap();
    v.update_category(
        category.id,
        "Mein Sparen".into(),
        None,
        category.sf_symbol,
        category.color_hex,
    )
    .unwrap();
    let custom = v
        .create_category("Sparen & Investieren".into(), None, "tag".into(), "".into())
        .unwrap();
    let booking = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.amount_cents < 0 && t.flow == crate::merchants::FlowKind::Consumption)
        .unwrap();
    // Neutralise independent merchant/text signals; isolate category identity.
    v.db().unwrap().execute("UPDATE transactions SET counterparty='Test',purpose='',booking_text='',counterparty_iban='' WHERE id=?1", [booking.id]).unwrap();
    v.set_transaction_category(booking.id, Some(category.id))
        .unwrap();
    let read = || {
        v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
            .into_iter()
            .find(|t| t.id == booking.id)
            .unwrap()
    };
    assert_eq!(read().flow, crate::merchants::FlowKind::Investment);
    assert_eq!(read().category_name, "Mein Sparen");
    v.set_transaction_category(booking.id, Some(custom))
        .unwrap();
    with_language(Lang::En, || {
        let t = read();
        assert_eq!(t.flow, crate::merchants::FlowKind::Consumption);
        assert_eq!(t.category_name, "Sparen & Investieren");
    });
}

#[test]
fn merging_preserves_roles_and_custom_provenance() {
    let a_dir = crate::test_support::TempDir::new("roles-merge-a");
    let b_dir = crate::test_support::TempDir::new("roles-merge-b");
    let a = vault(&a_dir);
    let b = vault(&b_dir);
    a.seed_starter_categories().unwrap();
    b.create_category("Freizeit".into(), None, "tag".into(), "".into())
        .unwrap();
    crate::importers::moneymoney_import::find_or_create_category(
        &b.db().unwrap(),
        &["Umbuchungen".into()],
        &mut HashMap::new(),
        &mut 0,
    )
    .unwrap();
    for v in [&a, &b] {
        let db = v.db().unwrap();
        let id = Vault::ensure_group(&db, "Kreditkarten", "creditcard", 2).unwrap();
        if std::sync::Arc::ptr_eq(v, &a) {
            db.execute(
                "UPDATE account_groups SET name='Meine Karten' WHERE id=?1",
                [id],
            )
            .unwrap();
        }
        db.execute(
            "INSERT INTO account_groups(name) VALUES ('Kreditkarten')",
            [],
        )
        .unwrap();
    }
    a.merge_other_vault(b_dir.join("v.db").to_string_lossy().into(), vec![7; 32])
        .unwrap();
    with_language(Lang::En, || {
        let cats = a.list_categories(None, None).unwrap();
        assert!(cats.iter().any(|c| c.name == "Freizeit"));
        assert!(!cats.iter().any(|c| c.name == "Leisure"));
        assert!(cats.iter().any(|c| c.name == "Income"));
        assert!(cats.iter().any(|c| c.name == "Umbuchungen"));
    });
    let db = a.db().unwrap();
    assert_eq!(
        db.query_row(
            "SELECT builtin_role FROM categories WHERE name='Umbuchungen' AND deleted_ms IS NULL",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "import:transfers"
    );
    let groups: Vec<(String, Option<String>)> = db
        .prepare("SELECT name,builtin_role FROM account_groups ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        groups,
        vec![
            ("Meine Karten".into(), Some("Kreditkarten".into())),
            ("Kreditkarten".into(), None)
        ]
    );
}

#[test]
fn builtin_groups_do_not_capture_custom_names() {
    let dir = crate::test_support::TempDir::new("roles-groups");
    let v = vault(&dir);
    let db = v.db().unwrap();
    db.execute(
        "INSERT INTO account_groups(name) VALUES ('Darlehen & Kredite')",
        [],
    )
    .unwrap();
    let custom = db.last_insert_rowid();
    let builtin = Vault::ensure_group(&db, "Darlehen & Kredite", "house", 4).unwrap();
    assert_ne!(custom, builtin);
    db.execute(
        "UPDATE account_groups SET name='Meine Kredite' WHERE id=?1",
        [builtin],
    )
    .unwrap();
    assert_eq!(
        Vault::ensure_group(&db, "Darlehen & Kredite", "house", 4).unwrap(),
        builtin
    );
}

#[test]
fn category_role_inheritance_and_override() {
    let dir = crate::test_support::TempDir::new("roles-inherit");
    let v = vault(&dir);
    let root = v
        .create_category("Finanzen".into(), None, "tag".into(), "".into())
        .unwrap();
    let child = v
        .create_category("ETF".into(), Some(root), "tag".into(), "".into())
        .unwrap();
    let grand = v
        .create_category("Kaution".into(), Some(child), "tag".into(), "".into())
        .unwrap();
    let find = |id: i64| {
        v.list_categories(None, None)
            .unwrap()
            .into_iter()
            .find(|c| c.id == id)
            .unwrap()
    };
    // Default at root: Consumption, inherited down the tree.
    assert_eq!(find(root).role, None);
    assert_eq!(find(root).effective_role, CategoryRole::Consumption);
    assert_eq!(find(grand).effective_role, CategoryRole::Consumption);

    // Setting Investment on root propagates to child and grandchild.
    v.set_category_role(root, Some(CategoryRole::Investment))
        .unwrap();
    assert_eq!(find(root).role, Some(CategoryRole::Investment));
    assert_eq!(find(child).role, None);
    assert_eq!(find(child).effective_role, CategoryRole::Investment);
    assert_eq!(find(grand).effective_role, CategoryRole::Investment);

    // Overriding grandchild to Neutral leaves child as Investment.
    v.set_category_role(grand, Some(CategoryRole::Neutral))
        .unwrap();
    assert_eq!(find(child).effective_role, CategoryRole::Investment);
    assert_eq!(find(grand).role, Some(CategoryRole::Neutral));
    assert_eq!(find(grand).effective_role, CategoryRole::Neutral);

    // Clearing grandchild returns it to inheriting Investment.
    v.set_category_role(grand, None).unwrap();
    assert_eq!(find(grand).role, None);
    assert_eq!(find(grand).effective_role, CategoryRole::Investment);

    // Non-existent category returns NotFound.
    assert!(matches!(
        v.set_category_role(999_999, Some(CategoryRole::Neutral)),
        Err(CoreError::NotFound { .. })
    ));
}

#[test]
fn custom_category_role_drives_booking_flow_and_kpis() {
    let dir = crate::test_support::TempDir::new("roles-custom-flow");
    let v = vault(&dir);
    v.seed_demo_data().unwrap();
    let outlay = v
        .create_category("Ausgelegt".into(), None, "tag".into(), "".into())
        .unwrap();
    let partner = v
        .create_category("Partner".into(), Some(outlay), "tag".into(), "".into())
        .unwrap();
    let booking = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.amount_cents < 0 && t.flow == crate::merchants::FlowKind::Consumption)
        .unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Test', purpose='', booking_text='', counterparty_iban='' WHERE id=?1",
            [booking.id],
        )
        .unwrap();
    v.set_transaction_category(booking.id, Some(partner))
        .unwrap();
    let flow_of = || {
        v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
            .into_iter()
            .find(|t| t.id == booking.id)
            .unwrap()
            .flow
    };
    let consumption_before = v
        .selection_flow(vec![], None, None)
        .unwrap()
        .consumption_cents;
    assert_eq!(flow_of(), crate::merchants::FlowKind::Consumption);

    // Setting Neutral on parent makes child bookings neutral (Transfer) and removes them from consumption.
    v.set_category_role(outlay, Some(CategoryRole::Neutral))
        .unwrap();
    assert_eq!(flow_of(), crate::merchants::FlowKind::Transfer);
    let consumption_after = v
        .selection_flow(vec![], None, None)
        .unwrap()
        .consumption_cents;
    assert_eq!(
        consumption_before - consumption_after,
        -booking.amount_cents
    );

    // Switching to Investment / Loan classifies accordingly.
    v.set_category_role(partner, Some(CategoryRole::Investment))
        .unwrap();
    assert_eq!(flow_of(), crate::merchants::FlowKind::Investment);
    v.set_category_role(partner, Some(CategoryRole::Loan))
        .unwrap();
    assert_eq!(flow_of(), crate::merchants::FlowKind::Loan);
}

#[test]
fn category_role_sync_merge_prefers_newest_edit() {
    for reverse in [false, true] {
        let a_dir = crate::test_support::TempDir::new("roles-sync-a");
        let b_dir = crate::test_support::TempDir::new("roles-sync-b");
        let a = vault(&a_dir);
        let b = vault(&b_dir);
        a.seed_starter_categories().unwrap();
        b.seed_starter_categories().unwrap();

        let cat_id = |v: &Vault, name: &str| {
            v.list_categories(None, None)
                .unwrap()
                .into_iter()
                .find(|c| c.name == name)
                .unwrap()
                .id
        };
        let a_umb = cat_id(&a, "Umbuchungen");
        let b_umb = cat_id(&b, "Umbuchungen");
        // Vault A keeps the stamp-0 backfill (`Neutral`); Vault B explicitly changes it.
        b.set_category_role(b_umb, Some(CategoryRole::Consumption))
            .unwrap();
        // Both edit `Freizeit` at different times: Vault A is newer.
        let a_frei = cat_id(&a, "Freizeit");
        let b_frei = cat_id(&b, "Freizeit");
        b.set_category_role(b_frei, Some(CategoryRole::Investment))
            .unwrap();
        b.db()
            .unwrap()
            .execute(
                "UPDATE categories SET role_ms = 100 WHERE id = ?1",
                [b_frei],
            )
            .unwrap();
        a.set_category_role(a_frei, Some(CategoryRole::Neutral))
            .unwrap();
        a.db()
            .unwrap()
            .execute(
                "UPDATE categories SET role_ms = 200 WHERE id = ?1",
                [a_frei],
            )
            .unwrap();

        let (dst, src_path) = if reverse {
            (&b, a_dir.join("v.db"))
        } else {
            (&a, b_dir.join("v.db"))
        };
        dst.merge_other_vault(src_path.to_string_lossy().into(), vec![7; 32])
            .unwrap();
        let cats = dst.list_categories(None, None).unwrap();
        let find = |name: &str| cats.iter().find(|c| c.name == name).unwrap();
        // The explicit `Consumption` itself survives, not just its effect at the root.
        assert_eq!(find("Umbuchungen").role, Some(CategoryRole::Consumption));
        assert_eq!(
            find("Umbuchungen").effective_role,
            CategoryRole::Consumption
        );
        assert_eq!(find("Freizeit").role, Some(CategoryRole::Neutral));
        assert_eq!(find("Freizeit").effective_role, CategoryRole::Neutral);
        let _ = a_umb;
    }
}

#[test]
fn role_inheritance_matches_list_and_booking_at_depth_32() {
    let dir = crate::test_support::TempDir::new("roles-depth-32");
    let v = vault(&dir);
    v.seed_demo_data().unwrap();
    let root = v
        .create_category("Tiefe-0".into(), None, "tag".into(), "".into())
        .unwrap();
    v.set_category_role(root, Some(CategoryRole::Neutral))
        .unwrap();
    let mut parent = root;
    for depth in 1..=32 {
        parent = v
            .create_category(
                format!("Tiefe-{depth}"),
                Some(parent),
                "tag".into(),
                "".into(),
            )
            .unwrap();
    }
    let leaf = parent;
    let listed = v
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.id == leaf)
        .unwrap();
    assert_eq!(listed.depth, 32);
    assert_eq!(listed.effective_role, CategoryRole::Neutral);

    let booking = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.amount_cents < 0 && t.flow == crate::merchants::FlowKind::Consumption)
        .unwrap();
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Test', purpose='', booking_text='', counterparty_iban='' WHERE id=?1",
            [booking.id],
        )
        .unwrap();
    v.set_transaction_category(booking.id, Some(leaf)).unwrap();
    let updated = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.id == booking.id)
        .unwrap();
    assert_eq!(updated.flow, crate::merchants::FlowKind::Transfer);
}

/// Behavior-preservation regression test (#931 step 1a): on both a freshly
/// seeded demo vault and an upgraded v41 fixture vault, `categories.role`
/// produces the exact same category classification, per-booking `FlowKind`,
/// `BookingPage` / `SelectionFlow` KPIs, and monthly cashflow as the legacy
/// `builtin_role`-based logic.
#[test]
fn category_roles_preserve_legacy_builtin_classification_and_kpis() {
    use crate::merchants::{FlowKind, MerchantKind, normalise};

    /// v41 `is_investment_category_path`, applied to the top-level `builtin_role`.
    fn is_inv_path(path: &str) -> bool {
        let p = path.to_lowercase();
        [
            "investment",
            "invest",
            "wertpapier",
            "aktie",
            "etf",
            "depot",
            "sparen",
            "vorsorge",
        ]
        .iter()
        .any(|n| p.contains(n))
    }

    /// Self-contained copy of the pre-#931 `classify_by_kind` logic, without
    /// calling into `CategoryRole` or the new `merchants::classify_by_kind`.
    fn legacy_classify(
        amount_cents: i64,
        counterparty: &str,
        purpose: &str,
        booking_text: &str,
        merchant_kind: MerchantKind,
        top_category: &str,
        account_kind: AccountKind,
    ) -> FlowKind {
        let contains_any = |text: &str, needles: &[&str]| needles.iter().any(|n| text.contains(n));
        let t = normalise(&format!("{counterparty} {purpose} {booking_text}"));
        if contains_any(
            &t,
            &["saveback", "round up", "roundup", "round-up", "aufrunden"],
        ) {
            return FlowKind::Perk;
        }
        if top_category == "Sparen & Investieren"
            || matches!(
                merchant_kind,
                MerchantKind::Etf | MerchantKind::Crypto | MerchantKind::Stock
            )
            || contains_any(
                &t,
                &[
                    "sparplan",
                    "wertpapierkauf",
                    "kauf order",
                    "wertp.abrechn",
                    "wertp abrechn",
                    "wertpapierabrechnung",
                    "wp-abrechnung",
                    "wp abrechnung",
                ],
            )
            || is_inv_path(top_category)
        {
            return FlowKind::Investment;
        }
        if top_category == "Kredite & Darlehen"
            || account_kind == AccountKind::Loan
            || contains_any(&t, &["darlehen", "tilgung", "kreditrate"])
        {
            return FlowKind::Loan;
        }
        if matches!(top_category, "Umbuchungen" | "import:transfers")
            || contains_any(
                &t,
                &[
                    "umbuchung",
                    "eigenes konto",
                    "eigene konten",
                    "kreditkartenabrechnung",
                    "zahlung erhalten",
                    "einzahlung",
                    "bankeinzug",
                    "übertrag",
                ],
            )
        {
            return FlowKind::Transfer;
        }
        if amount_cents > 0 {
            FlowKind::Income
        } else {
            FlowKind::Consumption
        }
    }

    fn legacy_category_flow(db: &Connection) -> HashMap<i64, (String, bool)> {
        let mut st = db
            .prepare("SELECT id, parent_id, builtin_role FROM categories WHERE deleted_ms IS NULL")
            .unwrap();
        let rows: HashMap<i64, (Option<i64>, Option<String>)> = st
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut out = HashMap::new();
        for &id in rows.keys() {
            let mut top_role = String::new();
            let mut is_transfer = false;
            let mut cur = Some(id);
            // v41's bound (`category_flows`): 32 categories
            for _ in 0..32 {
                let Some((parent, role)) = cur.and_then(|c| rows.get(&c)) else {
                    break;
                };
                top_role = role.clone().unwrap_or_default();
                is_transfer |= matches!(role.as_deref(), Some("Umbuchungen" | "import:transfers"));
                cur = *parent;
            }
            out.insert(id, (top_role, is_transfer));
        }
        out
    }

    fn verify_vault(label: &str, v: &Vault) {
        let db = v.db().unwrap();
        let legacy_cats = legacy_category_flow(&db);
        let new_roles = Vault::category_roles(&db).unwrap();
        assert_eq!(
            legacy_cats.len(),
            new_roles.len(),
            "{label}: category count"
        );
        for (id, (top_role, is_transfer)) in &legacy_cats {
            let role = new_roles[id];
            // The one category effect v41 applied: a transfer ancestor beats the top-level
            // role (`BookingFinisher::finish`), investment beats loan (`classify_by_kind`).
            let legacy = if *is_transfer {
                Some(CategoryRole::Neutral)
            } else if top_role == "Sparen & Investieren" || is_inv_path(top_role) {
                Some(CategoryRole::Investment)
            } else if top_role == "Kredite & Darlehen" {
                Some(CategoryRole::Loan)
            } else {
                None
            };
            let new = matches!(
                role,
                CategoryRole::Investment | CategoryRole::Loan | CategoryRole::Neutral
            )
            .then_some(role);
            assert_eq!(
                legacy, new,
                "{label}: cat {id} effect mismatch (top={top_role:?}, transfer={is_transfer}, role={role:?})"
            );
        }

        let merchants = v.merchant_rules(&db).unwrap();
        let own_ibans = Vault::own_ibans(&db).unwrap();
        let pair_transfers = Vault::payment_service_transfer_ids(&db).unwrap();
        let loan_links = crate::loans::link_map(&db).unwrap();
        drop(db);

        let page = v.list_transactions(BookingFilter::default()).unwrap();
        assert!(!page.transactions.is_empty(), "{label}: expected bookings");

        let mut expected_flow = SelectionFlow::default();
        let (mut inc, mut exp, mut cons, mut inv, mut perk) = (0i64, 0i64, 0i64, 0i64, 0i64);
        for b in &page.transactions {
            let (top_role, transfer_cat) = b
                .category_id
                .and_then(|c| legacy_cats.get(&c))
                .map(|(r, t)| (r.as_str(), *t))
                .unwrap_or(("", false));
            let bundled = merchants.bundled(&b.counterparty, &b.purpose);
            let mut legacy = legacy_classify(
                b.amount_cents,
                &b.counterparty,
                &b.purpose,
                &b.booking_text,
                bundled.kind,
                top_role,
                b.account_kind,
            );
            if b.is_salary && legacy == FlowKind::Transfer {
                legacy = FlowKind::Income;
            }
            if legacy == FlowKind::Transfer && b.category_id.is_some() && !transfer_cat {
                legacy = if b.amount_cents > 0 {
                    FlowKind::Income
                } else {
                    FlowKind::Consumption
                };
            }
            let own_iban = {
                let iban = crate::iban::normalize_iban(&b.counterparty_iban);
                !iban.is_empty() && own_ibans.contains(&iban)
            };
            let is_own = transfer_cat
                || (b.category_id.is_none()
                    && !matches!(legacy, FlowKind::Investment | FlowKind::Perk)
                    && (own_iban || pair_transfers.contains(&b.id)));
            if is_own {
                legacy = FlowKind::Transfer;
            }
            if loan_links.contains_key(&b.id) {
                legacy = FlowKind::Loan;
            }
            assert_eq!(
                b.flow, legacy,
                "{label}: booking {} ({}) flow drifted",
                b.id, b.counterparty
            );

            if crate::planning::counts_in_sums(0, &b.currency) {
                match kpi_side(legacy, b.amount_cents) {
                    KpiSide::Income => inc += b.amount_cents,
                    KpiSide::Expense => exp += b.amount_cents,
                    KpiSide::Neutral => {}
                }
                match legacy {
                    FlowKind::Consumption => cons += b.amount_cents,
                    FlowKind::Investment => inv += b.amount_cents,
                    FlowKind::Perk => perk += b.amount_cents,
                    _ => {}
                }
                expected_flow.add(legacy, b.amount_cents);
            }
        }
        assert_eq!(page.income_cents, inc, "{label}: income_cents");
        assert_eq!(page.expense_cents, exp, "{label}: expense_cents");
        assert_eq!(page.consumption_cents, cons, "{label}: consumption_cents");
        assert_eq!(page.investment_cents, inv, "{label}: investment_cents");
        assert_eq!(page.perk_cents, perk, "{label}: perk_cents");
        let flow_tuple =
            |f: &SelectionFlow| (f.consumption_cents, f.invested_cents, f.income_cents);
        assert_eq!(
            flow_tuple(&page.flow),
            flow_tuple(&expected_flow),
            "{label}: page.flow"
        );
        assert_eq!(
            flow_tuple(&v.selection_flow(vec![], None, None).unwrap()),
            flow_tuple(&expected_flow),
            "{label}: selection_flow"
        );
        // Monthly cashflow anchored to the newest booking date (so fixed
        // fixtures never age out into an all-zero rolling window).
        let newest = page
            .transactions
            .iter()
            .map(|b| b.booking_date.as_str())
            .max()
            .unwrap();
        let anchor_year: i32 = newest[..4].parse().unwrap();
        let anchor_month: u32 = newest[5..7].parse().unwrap();
        let cf = crate::planning::cashflow_for(
            v,
            PeriodGranularity::Month,
            12,
            &[],
            anchor_year,
            anchor_month,
            false,
        )
        .unwrap();
        assert!(
            cf.iter()
                .any(|p| p.income_cents != 0 || p.expense_cents != 0),
            "{label}: anchored cashflow window must contain non-zero activity"
        );
        let mut by_month: HashMap<String, (i64, i64)> = HashMap::new();
        for b in &page.transactions {
            if !crate::planning::counts_in_sums(0, &b.currency) {
                continue;
            }
            if let Some(m) = b.booking_date.get(..7) {
                let e = by_month.entry(m.to_string()).or_default();
                match kpi_side(b.flow, b.amount_cents) {
                    KpiSide::Income => e.0 += b.amount_cents,
                    KpiSide::Expense => e.1 -= b.amount_cents,
                    KpiSide::Neutral => {}
                }
            }
        }
        for p in cf {
            let (exp_in, exp_out) = by_month.get(&p.key).copied().unwrap_or((0, 0));
            assert_eq!(
                (p.income_cents, p.expense_cents),
                (exp_in, exp_out),
                "{label}: cashflow month {}",
                p.key
            );
        }
    }

    // 1. Fresh demo vault (seeded directly at v42)
    let demo_dir = crate::test_support::TempDir::new("roles-regression-demo");
    let demo_vault = vault(&demo_dir);
    demo_vault.seed_demo_data().unwrap();
    verify_vault("fresh-demo", &demo_vault);

    // 2. Real v41 fixture vault migrated to v42
    let mig_dir = crate::test_support::TempDir::new("roles-regression-v41");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v41.girocrab");
    let dst = mig_dir.join("vault-v41-copy.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    let migrated = Vault::open(dst.to_string_lossy().into(), vec![0x47; 32]).unwrap();
    verify_vault("migrated-v41", &migrated);

    // 3. Control: a transfer category below `Sparen & Investieren` (top level) with a booking.
    // v41 made it a transfer although the top-level role is an investment one.
    let nested_dir = crate::test_support::TempDir::new("roles-regression-v41-nested");
    let dst = nested_dir.join("vault-v41-nested.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    let (nested_cat_id, nested_tx) = {
        let conn = rusqlite::Connection::open(&dst).unwrap();
        conn.pragma_update(None, "key", format!("x'{}'", "47".repeat(32)))
            .unwrap();
        let nested: i64 = conn
            .query_row(
                "INSERT INTO categories(parent_id, name, sf_symbol, color_hex, sort_order, builtin_role)
                 SELECT id, 'Depotüberträge', 'arrow.left.arrow.right', '#445566', 99, 'import:transfers'
                   FROM categories WHERE builtin_role = 'Sparen & Investieren'
                 RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE transactions SET category_id = ?1
              WHERE id = (SELECT MIN(id) FROM transactions WHERE deleted_ms IS NULL)",
            [nested],
        )
        .unwrap();
        (
            nested,
            conn.query_row(
                "SELECT MIN(id) FROM transactions WHERE deleted_ms IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap(),
        )
    };
    let nested = Vault::open(dst.to_string_lossy().into(), vec![0x47; 32]).unwrap();
    let nested_cat = nested
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.name == "Depotüberträge")
        .unwrap();
    assert_eq!(nested_cat.id, nested_cat_id);
    assert_eq!(nested_cat.effective_role, CategoryRole::Neutral);
    // The upgrade keeps the booking in the transfer category, so the comparison below
    // really covers it (v41 made it a transfer, unless it is linked to a loan).
    let moved = nested
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.id == nested_tx)
        .unwrap();
    assert_eq!(moved.category_id, Some(nested_cat_id));
    verify_vault("migrated-v41-nested-transfer", &nested);
}

/// Intentional #931 role-inheritance semantics when reorganizing built-in
/// categories:
/// 1. Moving `Sparen & Investieren` under a custom root keeps its explicit
///    `Investment` role (instead of silently falling back to `Consumption`).
/// 2. Promoting an inheriting child (`ETF-Sparpläne`, `role = None`) to the
///    root follows pure tree inheritance (`Consumption`) without a hidden
///    name-based heuristic, until the user sets an explicit role.
/// 3. Existing v41 vaults where `ETF-Sparpläne` was *already* promoted to the
///    root before upgrading keep `Investment` via schema step 42's backfill.
#[test]
fn moved_builtin_category_follows_tree_role_inheritance() {
    use crate::merchants::FlowKind;

    let dir = crate::test_support::TempDir::new("roles-reorganized-builtins");
    let v = vault(&dir);
    v.seed_demo_data().unwrap();

    let cats = v.list_categories(None, None).unwrap();
    let invest_id = cats
        .iter()
        .find(|c| c.name == "Sparen & Investieren")
        .unwrap()
        .id;
    let etf_id = cats.iter().find(|c| c.name == "ETF-Sparpläne").unwrap().id;
    let custom_root = v
        .create_category("Finanzen".into(), None, "folder".into(), "#445566".into())
        .unwrap();

    // Pick a plain debit without investment/transfer keywords.
    let booking_id = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.amount_cents < 0)
        .unwrap()
        .id;
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Max Mustermann', purpose='Monatsbeitrag', booking_text='Überweisung', counterparty_iban='' WHERE id=?1",
            [booking_id],
        )
        .unwrap();

    let flow_of = |v: &Vault, id: i64| -> FlowKind {
        v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
            .into_iter()
            .find(|t| t.id == id)
            .unwrap()
            .flow
    };

    // 1. Move `Sparen & Investieren` under custom root `Finanzen`:
    //    both the moved parent and its inheriting child keep `Investment`.
    v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET parent_id = ?1 WHERE id = ?2",
            rusqlite::params![custom_root, invest_id],
        )
        .unwrap();
    v.set_transaction_category(booking_id, Some(invest_id))
        .unwrap();
    assert_eq!(flow_of(&v, booking_id), FlowKind::Investment);
    v.set_transaction_category(booking_id, Some(etf_id))
        .unwrap();
    assert_eq!(flow_of(&v, booking_id), FlowKind::Investment);

    // 2. Promote inheriting `ETF-Sparpläne` (`role = None`) to root:
    //    pure tree inheritance yields `Consumption` (no hidden name heuristic),
    //    and setting an explicit role flips it back to `Investment`.
    v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET parent_id = NULL WHERE id = ?1",
            [etf_id],
        )
        .unwrap();
    assert_eq!(flow_of(&v, booking_id), FlowKind::Consumption);
    v.set_category_role(etf_id, Some(CategoryRole::Investment))
        .unwrap();
    assert_eq!(flow_of(&v, booking_id), FlowKind::Investment);

    // 3. Pre-v42 vault where `ETF-Sparpläne` was already promoted to root:
    //    migrating v41 -> v42 backfills `role = 'investment'`, keeping `Investment`.
    let mig_dir = crate::test_support::TempDir::new("roles-v41-promoted-etf");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v41.girocrab");
    let dst = mig_dir.join("vault-v41-promoted.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    {
        // Mutate the v41 copy before opening (so before step 42 runs).
        let conn = rusqlite::Connection::open(&dst).unwrap();
        let hex_key = "47".repeat(32);
        conn.pragma_update(None, "key", format!("x'{hex_key}'"))
            .unwrap();
        conn.execute(
            "UPDATE categories SET parent_id = NULL WHERE builtin_role = 'ETF-Sparpläne'",
            [],
        )
        .unwrap();
    }
    let upgraded = Vault::open(dst.to_string_lossy().into(), vec![0x47; 32]).unwrap();
    let promoted_etf = upgraded
        .list_categories(None, None)
        .unwrap()
        .into_iter()
        .find(|c| c.name == "ETF-Sparpläne")
        .unwrap();
    assert_eq!(promoted_etf.role, Some(CategoryRole::Investment));
    assert_eq!(promoted_etf.effective_role, CategoryRole::Investment);
    let tx_id = upgraded
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.amount_cents < 0)
        .unwrap()
        .id;
    upgraded
        .db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Max Mustermann', purpose='Monatsbeitrag', booking_text='Überweisung', counterparty_iban='' WHERE id=?1",
            [tx_id],
        )
        .unwrap();
    upgraded
        .set_transaction_category(tx_id, Some(promoted_etf.id))
        .unwrap();
    assert_eq!(flow_of(&upgraded, tx_id), FlowKind::Investment);
}

/// Rearranged v41 trees preserve legacy classification on upgrade to v42:
/// - `Gehalt` moved under `Umbuchungen` keeps `Transfer` for bookings assigned
///   to it (matching v41's "a transfer category still wins" rule in
///   `BookingFinisher::finish`); later explicit `set_category_role` still
///   overrides via nearest-role inheritance.
/// - `Sparen & Investieren` already moved under a custom root in v41 stays
///   `Consumption` on upgrade (matching legacy root-only `top_role`).
#[test]
fn v41_rearranged_builtins_preserve_legacy_flow_on_upgrade() {
    use crate::merchants::FlowKind;

    let mig_dir = crate::test_support::TempDir::new("roles-v41-rearranged");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v41.girocrab");
    let dst = mig_dir.join("vault-v41-rearranged.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    {
        let conn = rusqlite::Connection::open(&dst).unwrap();
        let hex_key = "47".repeat(32);
        conn.pragma_update(None, "key", format!("x'{hex_key}'"))
            .unwrap();
        conn.execute(
            "INSERT INTO categories(parent_id, name, sf_symbol, color_hex, sort_order)
             VALUES (NULL, 'Eigene Wurzel', 'folder', '#445566', 99)",
            [],
        )
        .unwrap();
        let custom_root = conn.last_insert_rowid();
        let umb_id: i64 = conn
            .query_row(
                "SELECT id FROM categories WHERE builtin_role = 'Umbuchungen'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "UPDATE categories SET parent_id = ?1 WHERE builtin_role = 'Gehalt'",
            [umb_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE categories SET parent_id = ?1 WHERE builtin_role = 'Sparen & Investieren'",
            [custom_root],
        )
        .unwrap();
    }

    let v = Vault::open(dst.to_string_lossy().into(), vec![0x47; 32]).unwrap();
    let cats = v.list_categories(None, None).unwrap();
    let gehalt = cats.iter().find(|c| c.name == "Gehalt").unwrap();
    let sparen = cats
        .iter()
        .find(|c| c.name == "Sparen & Investieren")
        .unwrap();
    assert_eq!(gehalt.role, None);
    assert_eq!(gehalt.effective_role, CategoryRole::Neutral);
    assert_eq!(sparen.role, None);
    assert_eq!(sparen.effective_role, CategoryRole::Consumption);

    let txs = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions;
    let plain_debit = txs.iter().find(|t| t.amount_cents < 0).unwrap().id;
    let salary_tx = txs.iter().find(|t| t.is_salary).unwrap().id;
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Max Mustermann', purpose='Test', booking_text='Überweisung', counterparty_iban='', salary_flag=0 WHERE id=?1",
            [plain_debit],
        )
        .unwrap();

    let flow_of = |id: i64| {
        v.list_transactions(BookingFilter::default())
            .unwrap()
            .transactions
            .into_iter()
            .find(|t| t.id == id)
            .unwrap()
            .flow
    };

    // In v41, a transfer ancestor always wins in `BookingFinisher::finish`
    // (`is_own_transfer(..., transfer_category)`): both plain and salary-flagged
    // bookings in `Umbuchungen › Gehalt` stay `Transfer`.
    v.set_transaction_category(plain_debit, Some(gehalt.id))
        .unwrap();
    v.set_transaction_category(salary_tx, Some(gehalt.id))
        .unwrap();
    assert_eq!(flow_of(plain_debit), FlowKind::Transfer);
    assert_eq!(flow_of(salary_tx), FlowKind::Transfer);

    // Later explicit role edit on `Gehalt` overrides inherited `Neutral`.
    v.set_category_role(gehalt.id, Some(CategoryRole::Consumption))
        .unwrap();
    assert_eq!(flow_of(plain_debit), FlowKind::Consumption);
    assert_eq!(flow_of(salary_tx), FlowKind::Income);

    // `Sparen & Investieren` already under custom root in v41 stays `Consumption`.
    v.set_transaction_category(plain_debit, Some(sparen.id))
        .unwrap();
    assert_eq!(flow_of(plain_debit), FlowKind::Consumption);
}

/// `Gehalt` two levels below `Umbuchungen` (through a live folder) gets no role of its own on
/// the upgrade and inherits `Neutral`, so its bookings stay `Transfer` as in v41. A deleted
/// folder in between cuts the ancestry instead (`category_roles_backfill_from_builtin_role`).
#[test]
fn v41_gehalt_below_live_folder_under_transfers_inherits_neutral() {
    use crate::merchants::FlowKind;

    let mig_dir = crate::test_support::TempDir::new("roles-v41-gehalt-folder");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v41.girocrab");
    let dst = mig_dir.join("vault-v41-gehalt-folder.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    {
        let conn = rusqlite::Connection::open(&dst).unwrap();
        let hex_key = "47".repeat(32);
        conn.pragma_update(None, "key", format!("x'{hex_key}'"))
            .unwrap();
        let umb_id: i64 = conn
            .query_row(
                "SELECT id FROM categories WHERE builtin_role = 'Umbuchungen'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO categories(parent_id, name, sf_symbol, color_hex, sort_order)
             VALUES (?1, 'Zwischenordner', 'folder', '#445566', 99)",
            [umb_id],
        )
        .unwrap();
        let folder = conn.last_insert_rowid();
        conn.execute(
            "UPDATE categories SET parent_id = ?1 WHERE builtin_role = 'Gehalt'",
            [folder],
        )
        .unwrap();
    }

    let v = Vault::open(dst.to_string_lossy().into(), vec![0x47; 32]).unwrap();
    let cats = v.list_categories(None, None).unwrap();
    let gehalt = cats.iter().find(|c| c.name == "Gehalt").unwrap();
    assert_eq!(gehalt.role, None);
    assert_eq!(gehalt.effective_role, CategoryRole::Neutral);

    let txs = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions;
    let debit = txs.iter().find(|t| t.amount_cents < 0).unwrap().id;
    v.db()
        .unwrap()
        .execute(
            "UPDATE transactions SET counterparty='Max Mustermann', purpose='Test', booking_text='Überweisung', counterparty_iban='', salary_flag=0 WHERE id=?1",
            [debit],
        )
        .unwrap();
    v.set_transaction_category(debit, Some(gehalt.id)).unwrap();
    let flow = v
        .list_transactions(BookingFilter::default())
        .unwrap()
        .transactions
        .into_iter()
        .find(|t| t.id == debit)
        .unwrap()
        .flow;
    assert_eq!(flow, FlowKind::Transfer);
}

/// Device A merges duplicate Y into X (`merged_into = X`) before seeing device B's
/// newer role edit on Y: syncing must forward Y's newer role to survivor X, while
/// ordinary deleted categories (`merged_into = NULL`) never alter live categories.
#[test]
fn merged_duplicate_tombstone_forwards_newer_role_to_survivor() {
    let a_dir = crate::test_support::TempDir::new("roles-tombstone-a");
    let b_dir = crate::test_support::TempDir::new("roles-tombstone-b");
    let a = vault(&a_dir);
    let b = vault(&b_dir);

    let ax = a
        .create_category("Kaution".into(), None, "tag".into(), "".into())
        .unwrap();
    let ay = a
        .create_category("Kaution Alt".into(), None, "tag".into(), "".into())
        .unwrap();
    let a_other = a
        .create_category("Sonstiges".into(), None, "tag".into(), "".into())
        .unwrap();
    // Give X an older role (Consumption @ 100) and mark Y merged into X on device A.
    {
        let db = a.db().unwrap();
        db.execute(
            "UPDATE categories SET uuid = 'uuid-x', role = 'consumption', role_ms = 100 WHERE id = ?1",
            [ax],
        )
        .unwrap();
        db.execute(
            "UPDATE categories SET uuid = 'uuid-y', deleted_ms = 200, merged_into = 'uuid-x', role = NULL, role_ms = 0 WHERE id = ?1",
            [ay],
        )
        .unwrap();
        db.execute(
            "UPDATE categories SET uuid = 'uuid-other', role = 'consumption', role_ms = 100 WHERE id = ?1",
            [a_other],
        )
        .unwrap();
    }

    // Device B still has Y live and gives Y a newer role (`Neutral` @ 500), plus
    // an ordinary deleted category `uuid-del` with an even newer role (`Loan` @ 900).
    // It has the name and parent of the live `Sonstiges`, so a merge that matched by name
    // would show up there.
    let bx = b
        .create_category("Kaution".into(), None, "tag".into(), "".into())
        .unwrap();
    let by = b
        .create_category("Kaution Alt".into(), None, "tag".into(), "".into())
        .unwrap();
    let b_del = b
        .create_category("Sonstiges".into(), None, "tag".into(), "".into())
        .unwrap();
    {
        let db = b.db().unwrap();
        db.execute(
            "UPDATE categories SET uuid = 'uuid-x', role = 'consumption', role_ms = 100 WHERE id = ?1",
            [bx],
        )
        .unwrap();
        db.execute(
            "UPDATE categories SET uuid = 'uuid-y', role = 'neutral', role_ms = 500 WHERE id = ?1",
            [by],
        )
        .unwrap();
        db.execute(
            "UPDATE categories SET uuid = 'uuid-del', deleted_ms = 600, merged_into = NULL, role = 'loan', role_ms = 900 WHERE id = ?1",
            [b_del],
        )
        .unwrap();
    }

    a.merge_other_vault(b_dir.join("v.db").to_string_lossy().into(), vec![7; 32])
        .unwrap();
    let cats = a.list_categories(None, None).unwrap();
    let x = cats.iter().find(|c| c.name == "Kaution").unwrap();
    let other = cats.iter().find(|c| c.name == "Sonstiges").unwrap();
    assert_eq!(x.role, Some(CategoryRole::Neutral));
    assert_eq!(x.effective_role, CategoryRole::Neutral);
    assert_eq!(other.role, Some(CategoryRole::Consumption));
}

/// Clearing or editing a role stamps `role_ms` strictly above any stored ahead-of-clock
/// peer timestamp so the local change wins the next iCloud merge.
#[test]
fn set_category_role_monotonic_stamp_beats_ahead_peer() {
    let dir = crate::test_support::TempDir::new("roles-monotonic-ms");
    let v = vault(&dir);
    let id = v
        .create_category("Ausgelegt".into(), None, "tag".into(), "".into())
        .unwrap();
    let ahead = crate::dates::now_ms() + 60_000;
    v.db()
        .unwrap()
        .execute(
            "UPDATE categories SET role = 'neutral', role_ms = ?2 WHERE id = ?1",
            rusqlite::params![id, ahead],
        )
        .unwrap();

    v.set_category_role(id, None).unwrap();
    let (role, role_ms): (Option<String>, i64) = v
        .db()
        .unwrap()
        .query_row(
            "SELECT role, role_ms FROM categories WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(role, None);
    assert!(role_ms > ahead, "expected role_ms ({role_ms}) > {ahead}");
}

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
    assert!(
        v.set_category_role(999_999, Some(CategoryRole::Neutral))
            .is_err()
    );
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
        let role_of = |name: &str| cats.iter().find(|c| c.name == name).unwrap().effective_role;
        assert_eq!(role_of("Umbuchungen"), CategoryRole::Consumption);
        assert_eq!(role_of("Freizeit"), CategoryRole::Neutral);
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
/// seeded demo vault and an upgraded v38 fixture vault, `categories.role`
/// produces the exact same category classification, per-booking `FlowKind`,
/// `BookingPage` / `SelectionFlow` KPIs, and monthly cashflow as the legacy
/// `builtin_role`-based logic.
#[test]
fn category_roles_preserve_legacy_builtin_classification_and_kpis() {
    use crate::merchants::{FlowKind, classify};

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
            for _ in 0..=32 {
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
            assert_eq!(
                *is_transfer,
                role == CategoryRole::Neutral,
                "{label}: cat {id} transfer mismatch (top={top_role:?}, role={role:?})"
            );
            assert_eq!(
                top_role == "Sparen & Investieren",
                role == CategoryRole::Investment,
                "{label}: cat {id} investment mismatch (top={top_role:?}, role={role:?})"
            );
            assert_eq!(
                top_role == "Kredite & Darlehen",
                role == CategoryRole::Loan,
                "{label}: cat {id} loan mismatch (top={top_role:?}, role={role:?})"
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
            let mut info = merchants.resolve(&b.counterparty, &b.purpose);
            info.kind = bundled.kind;
            // Call `merchants::classify` directly so the legacy investment-path
            // heuristic (`is_investment_category_path`) is preserved in the oracle.
            let mut legacy = classify(
                b.amount_cents,
                &b.counterparty,
                &b.purpose,
                &b.booking_text,
                &info,
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

    // 1. Fresh demo vault (seeded directly at v39)
    let demo_dir = crate::test_support::TempDir::new("roles-regression-demo");
    let demo_vault = vault(&demo_dir);
    demo_vault.seed_demo_data().unwrap();
    verify_vault("fresh-demo", &demo_vault);

    // 2. Real v38 fixture vault migrated to v39
    let mig_dir = crate::test_support::TempDir::new("roles-regression-v38");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v38.girocrab");
    let dst = mig_dir.join("vault-v38-copy.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    let migrated = Vault::open(dst.to_string_lossy().into(), vec![0x47; 32]).unwrap();
    verify_vault("migrated-v38", &migrated);
}

/// Intentional #931 role-inheritance semantics when reorganizing built-in
/// categories:
/// 1. Moving `Sparen & Investieren` under a custom root keeps its explicit
///    `Investment` role (instead of silently falling back to `Consumption`).
/// 2. Promoting an inheriting child (`ETF-Sparpläne`, `role = None`) to the
///    root follows pure tree inheritance (`Consumption`) without a hidden
///    name-based heuristic, until the user sets an explicit role.
/// 3. Existing v38 vaults where `ETF-Sparpläne` was *already* promoted to the
///    root before upgrading keep `Investment` via schema step 39's backfill.
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

    // 3. Pre-v39 vault where `ETF-Sparpläne` was already promoted to root:
    //    migrating v38 -> v39 backfills `role = 'investment'`, keeping `Investment`.
    let mig_dir = crate::test_support::TempDir::new("roles-v38-promoted-etf");
    let src =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vault-v38.girocrab");
    let dst = mig_dir.join("vault-v38-promoted.girocrab");
    std::fs::copy(&src, &dst).unwrap();
    {
        // Mutate the v38 copy before opening (so before step 39 runs).
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

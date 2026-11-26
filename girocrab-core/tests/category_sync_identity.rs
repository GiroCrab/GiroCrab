//! A transient name collision on one peer must preserve both category identities.
use girocrab_core::merchants::FlowKind;
use girocrab_core::models::{BookingFilter, CategoryRole};
use girocrab_core::vault::Vault;
use rusqlite::Connection;

const KEY: [u8; 32] = [7; 32];

struct TestDir(std::path::PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn connect(path: &str) -> Connection {
    let conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "key", format!("x'{}'", hex::encode(KEY)))
        .unwrap();
    conn
}

#[test]
fn household_spending_survives_all_six_category_sync_orders() {
    let dir = TestDir(std::env::temp_dir().join(format!("category-sync-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = |name: &str| dir.0.join(name).to_string_lossy().into_owned();
    let create = |peer: &str, role| {
        let vault = Vault::open(path(peer), KEY.to_vec()).unwrap();
        let id = vault
            .create_category("Savings".into(), None, String::new(), String::new())
            .unwrap();
        vault.set_category_role(id, Some(role)).unwrap();
        let conn = connect(&path(peer));
        let uuid: String = conn
            .query_row("SELECT uuid FROM categories WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .unwrap();
        (id, uuid)
    };
    let (household, household_uuid) = create("household", CategoryRole::Consumption);
    let (investment, investment_uuid) = create("investment", CategoryRole::Investment);
    assert_ne!(household_uuid, investment_uuid);
    {
        let conn = connect(&path("household"));
        conn.execute_batch(
            "INSERT INTO account_groups (id, name) VALUES (1, 'Test');
            INSERT INTO accounts (id, group_id, external_ref, name) VALUES (1, 1, 'test', 'Test');",
        )
        .unwrap();
        conn.execute("INSERT INTO transactions (account_id, external_id, booking_date, amount_cents, category_id, category_source, category_ms)
            VALUES (1, 'purchase', '2026-01-01', -100, ?1, 'manual', 4)", [household]).unwrap();
    }
    std::fs::copy(path("investment"), path("renamed")).unwrap();
    connect(&path("renamed"))
        .execute(
            "UPDATE categories SET name = 'Brokerage', name_ms = name_ms + 1 WHERE id = ?1",
            [investment],
        )
        .unwrap();

    for order in [
        ["household", "investment", "renamed"],
        ["household", "renamed", "investment"],
        ["investment", "household", "renamed"],
        ["investment", "renamed", "household"],
        ["renamed", "household", "investment"],
        ["renamed", "investment", "household"],
    ] {
        std::fs::copy(path(order[0]), path("result")).unwrap();
        let vault = Vault::open(path("result"), KEY.to_vec()).unwrap();
        for pass in [&order[1..], &order[..]] {
            for peer in pass {
                vault.merge_peer_snapshot(path(peer)).unwrap();
            }
            let db = connect(&path("result"));
            let live_id = |uuid: &str, name: &str| -> i64 {
                db.query_row("SELECT id FROM categories WHERE uuid = ?1 AND name = ?2 AND deleted_ms IS NULL",
                    [uuid, name], |r| r.get(0)).unwrap_or_else(|err| panic!("{order:?}: {name}: {err}"))
            };
            let household = live_id(&household_uuid, "Savings");
            let investment = live_id(&investment_uuid, "Brokerage");
            let cats = vault.list_categories(None, None).unwrap();
            assert_eq!(
                cats.iter()
                    .find(|c| c.id == investment)
                    .unwrap()
                    .effective_role,
                CategoryRole::Investment
            );
            let page = vault.list_transactions(BookingFilter::default()).unwrap();
            assert_eq!(page.transactions.len(), 1);
            let booking = &page.transactions[0];
            assert_eq!(booking.category_id, Some(household), "{order:?}");
            assert_eq!(booking.flow, FlowKind::Consumption, "{order:?}");
            assert_eq!(
                (page.flow.consumption_cents, page.flow.invested_cents),
                (100, 0),
                "{order:?}"
            );
        }
    }
}

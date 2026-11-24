//! „Duplizieren“ im Kategorien-Dialog, Vorbild `AppModel+CategoryDuplicate.swift` (macOS):
//! copies name, symbol and colour of a category and its sub-categories. Bookings, rules and
//! budgets stay with the original: the copy is written only through `CategoryWriter`
//! (`create_category` + `reorder_categories`).

use std::sync::Arc;

use slint::{ComponentHandle, SharedString};

use super::{
    Category, MainWindow, Vault, VaultSlot, refresh_categories, reload_all, vault_of, with_ui,
};

/// A category as the copy needs it, in tree order like `Ui::categories`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Node<'a> {
    pub id: i64,
    pub parent: Option<i64>,
    pub name: &'a str,
    pub symbol: &'a str,
    pub color: &'a str,
}

impl<'a> From<&'a Category> for Node<'a> {
    fn from(c: &'a Category) -> Self {
        Node {
            id: c.id,
            parent: c.parent_id,
            name: &c.name,
            symbol: &c.sf_symbol,
            color: &c.color_hex,
        }
    }
}

/// One category to create. `parent` indexes an earlier step; `None` = the original's parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CopyStep {
    pub parent: Option<usize>,
    pub name: String,
    pub sf_symbol: String,
    pub color_hex: String,
}

/// „<Name> Kopie“; on a collision with a sibling „<Name> Kopie 2“, „… 3“ and so on.
pub(super) fn copy_name(original: &str, siblings: &[&str]) -> String {
    let taken = |name: &str| {
        let name = name.to_lowercase();
        siblings.iter().any(|s| s.trim().to_lowercase() == name)
    };
    let base = format!("{} Kopie", original.trim());
    if !taken(&base) {
        return base;
    }
    let mut n = 2;
    loop {
        let name = format!("{base} {n}");
        if !taken(&name) {
            return name;
        }
        n += 1;
    }
}

/// The original (renamed) and its sub-tree (names kept), `None` if `id` is gone.
pub(super) fn copy_plan(nodes: &[Node], id: i64) -> Option<Vec<CopyStep>> {
    let start = nodes.iter().position(|n| n.id == id)?;
    let root = nodes[start];
    let step = |n: &Node, parent: Option<usize>, name: String| CopyStep {
        parent,
        name,
        sf_symbol: n.symbol.to_string(),
        color_hex: n.color.to_string(),
    };
    let siblings: Vec<&str> = nodes
        .iter()
        .filter(|n| n.parent == root.parent)
        .map(|n| n.name)
        .collect();
    let mut steps = vec![step(&root, None, copy_name(root.name, &siblings))];
    let mut ids = vec![root.id];
    // Tree order: the sub-tree follows the original directly and ends at the first row whose
    // parent is not part of it.
    for n in &nodes[start + 1..] {
        let Some(parent) = ids.iter().position(|&p| Some(p) == n.parent) else {
            break;
        };
        ids.push(n.id);
        steps.push(step(n, Some(parent), n.name.to_string()));
    }
    Some(steps)
}

/// The original's siblings in their order with the copy right below the original.
pub(super) fn order_with_copy(
    nodes: &[Node],
    original: i64,
    copy: i64,
) -> Option<(Option<i64>, Vec<i64>)> {
    let parent = nodes.iter().find(|n| n.id == original)?.parent;
    let mut ids: Vec<i64> = nodes
        .iter()
        .filter(|n| n.parent == parent && n.id != copy)
        .map(|n| n.id)
        .collect();
    let at = ids.iter().position(|&i| i == original)? + 1;
    ids.insert(at, copy);
    Some((parent, ids))
}

/// Everything the copy writes. Rules, bookings and budgets have no entry here on purpose.
pub(super) trait CategoryWriter {
    fn create(
        &self,
        name: String,
        parent: Option<i64>,
        sf_symbol: String,
        color_hex: String,
    ) -> Result<i64, String>;
    fn reorder(&self, parent: Option<i64>, ids: Vec<i64>) -> Result<(), String>;
}

impl CategoryWriter for Vault {
    fn create(
        &self,
        name: String,
        parent: Option<i64>,
        sf_symbol: String,
        color_hex: String,
    ) -> Result<i64, String> {
        self.create_category(name, parent, sf_symbol, color_hex)
            .map_err(|e| e.to_string())
    }

    fn reorder(&self, parent: Option<i64>, ids: Vec<i64>) -> Result<(), String> {
        self.reorder_categories(parent, ids)
            .map_err(|e| e.to_string())
    }
}

/// Creates the copy of `id` with its sub-tree and moves it right below the original.
pub(super) fn duplicate(w: &impl CategoryWriter, nodes: &[Node], id: i64) -> Result<i64, String> {
    let steps = copy_plan(nodes, id).ok_or("Die Kategorie gibt es nicht mehr.")?;
    let root_parent = nodes.iter().find(|n| n.id == id).and_then(|n| n.parent);
    let mut created: Vec<i64> = Vec::with_capacity(steps.len());
    for s in steps {
        let parent = s.parent.map_or(root_parent, |i| Some(created[i]));
        created.push(w.create(s.name, parent, s.sf_symbol, s.color_hex)?);
    }
    let copy = created[0];
    if let Some((parent, ids)) = order_with_copy(nodes, id, copy) {
        w.reorder(parent, ids)?;
    }
    Ok(copy)
}

/// Worker-thread entry: reads the current tree, so a sync since opening the dialog counts.
pub(super) fn duplicate_in(v: &Vault, id: i64) -> Result<i64, String> {
    let cats = v.list_categories(None, None).map_err(|e| e.to_string())?;
    let nodes: Vec<Node> = cats.iter().map(Node::from).collect();
    duplicate(v, &nodes, id)
}

/// „Duplizieren“: writes on a worker thread; `Ui::moving` blocks ▲/▼ and further clicks meanwhile.
pub(super) fn wire(win: &MainWindow, vault: &VaultSlot) {
    let (weak, slot) = (win.as_weak(), vault.clone());
    win.on_category_duplicate(move || {
        let (Some(win), Some(v)) = (weak.upgrade(), vault_of(&slot)) else {
            return;
        };
        let id = with_ui(|u| {
            if u.moving {
                return None;
            }
            let id = usize::try_from(win.get_cat_selected())
                .ok()
                .and_then(|i| u.categories.get(i))?
                .id;
            u.moving = true;
            u.category_gate.reset();
            Some(id)
        });
        let Some(id) = id else {
            return;
        };
        win.set_cat_armed(false);
        win.set_cat_busy(true);
        win.set_cat_status("Wird dupliziert …".into());
        let (weak, slot) = (win.as_weak(), slot.clone());
        std::thread::spawn(move || {
            let result = duplicate_in(&v, id);
            let _ = weak.upgrade_in_event_loop(move |win| {
                with_ui(|u| u.moving = false);
                win.set_cat_busy(false);
                // Locked or swapped meanwhile: the dialog no longer shows this vault.
                let Some(v) = vault_of(&slot).filter(|cur| Arc::ptr_eq(cur, &v)) else {
                    return;
                };
                match result {
                    Ok(copy) => {
                        win.set_cat_status(SharedString::new());
                        refresh_categories(&win, &v, Some(copy));
                        reload_all(&win, &v);
                    }
                    Err(e) => {
                        // Partial copy: earlier creates stay saved, so show them. The refresh may
                        // set its own status, hence the original error is set last.
                        refresh_categories(&win, &v, Some(id));
                        reload_all(&win, &v);
                        win.set_cat_status(e.into());
                    }
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    fn node(id: i64, parent: Option<i64>, name: &'static str) -> Node<'static> {
        Node {
            id,
            parent,
            name,
            symbol: "tag",
            color: "#112233",
        }
    }

    #[test]
    fn copy_name_appends_kopie_and_numbers_collisions() {
        assert_eq!(
            copy_name("Lebensmittel", &["Lebensmittel"]),
            "Lebensmittel Kopie"
        );
        assert_eq!(
            copy_name(
                "Lebensmittel",
                &["Lebensmittel", "lebensmittel kopie", "Lebensmittel Kopie 2"]
            ),
            "Lebensmittel Kopie 3"
        );
        // Only an exact (case-insensitive) collision counts.
        assert_eq!(copy_name(" Reisen ", &["Reisen Kopie 3"]), "Reisen Kopie");
    }

    #[test]
    fn plan_copies_sub_tree_and_checks_names_per_level() {
        let nodes = [
            node(1, None, "Essen"),
            node(2, Some(1), "Restaurant"),
            node(3, Some(2), "Mittag"),
            node(4, Some(1), "Essen Kopie"),
            node(5, None, "Haushalt"),
        ];
        let plan = copy_plan(&nodes, 1).unwrap();
        let shape: Vec<(Option<usize>, &str)> =
            plan.iter().map(|s| (s.parent, s.name.as_str())).collect();
        // „Essen Kopie“ below „Essen“ is no sibling; „Haushalt“ is not part of the sub-tree.
        assert_eq!(
            shape,
            [
                (None, "Essen Kopie"),
                (Some(0), "Restaurant"),
                (Some(1), "Mittag"),
                (Some(0), "Essen Kopie"),
            ]
        );
        assert!(
            plan.iter()
                .all(|s| s.sf_symbol == "tag" && s.color_hex == "#112233")
        );
        assert_eq!(copy_plan(&nodes, 2).unwrap().len(), 2);
        assert!(copy_plan(&nodes, 99).is_none());
    }

    #[test]
    fn copy_goes_right_below_original() {
        let nodes = [
            node(1, None, "A"),
            node(2, Some(1), "A1"),
            node(3, Some(1), "A2"),
            node(4, None, "B"),
        ];
        assert_eq!(order_with_copy(&nodes, 1, 9), Some((None, vec![1, 9, 4])));
        assert_eq!(
            order_with_copy(&nodes, 2, 9),
            Some((Some(1), vec![2, 9, 3]))
        );
    }

    #[derive(Default)]
    struct Recorder {
        calls: RefCell<Vec<String>>,
        next: Cell<i64>,
    }

    impl CategoryWriter for Recorder {
        fn create(
            &self,
            name: String,
            parent: Option<i64>,
            sf_symbol: String,
            color_hex: String,
        ) -> Result<i64, String> {
            let id = 100 + self.next.get();
            self.next.set(self.next.get() + 1);
            self.calls
                .borrow_mut()
                .push(format!("create {name} {parent:?} {sf_symbol} {color_hex}"));
            Ok(id)
        }

        fn reorder(&self, parent: Option<i64>, ids: Vec<i64>) -> Result<(), String> {
            self.calls
                .borrow_mut()
                .push(format!("reorder {parent:?} {ids:?}"));
            Ok(())
        }
    }

    #[test]
    fn duplicate_writes_only_categories_no_rules_or_bookings() {
        let nodes = [
            node(1, None, "Essen"),
            node(2, Some(1), "Restaurant"),
            node(3, None, "Haushalt"),
        ];
        let rec = Recorder::default();
        assert_eq!(duplicate(&rec, &nodes, 1), Ok(100));
        assert_eq!(
            *rec.calls.borrow(),
            [
                "create Essen Kopie None tag #112233",
                "create Restaurant Some(100) tag #112233",
                "reorder None [1, 100, 3]",
            ]
        );
        // A vanished category writes nothing.
        assert!(duplicate(&rec, &nodes, 42).is_err());
        assert_eq!(rec.calls.borrow().len(), 3);
    }

    struct FailsOnReorder(Recorder);

    impl CategoryWriter for FailsOnReorder {
        fn create(
            &self,
            name: String,
            parent: Option<i64>,
            sf_symbol: String,
            color_hex: String,
        ) -> Result<i64, String> {
            self.0.create(name, parent, sf_symbol, color_hex)
        }

        fn reorder(&self, _parent: Option<i64>, _ids: Vec<i64>) -> Result<(), String> {
            Err("Sortieren fehlgeschlagen".to_string())
        }
    }

    #[test]
    fn failed_reorder_reports_original_error_and_keeps_created_categories() {
        let nodes = [node(1, None, "Essen"), node(2, Some(1), "Restaurant")];
        let w = FailsOnReorder(Recorder::default());
        assert_eq!(
            duplicate(&w, &nodes, 1),
            Err("Sortieren fehlgeschlagen".to_string())
        );
        // The creates are not rolled back: the UI has to refresh its list after the error.
        assert_eq!(w.0.calls.borrow().len(), 2);
    }
}

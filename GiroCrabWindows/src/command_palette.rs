//! Command palette, part 1: the catalog of commands and the tolerant search over it.
//! UI-free so it can be unit-tested; the window wires it in part 2 (rows, privacy masking).
//! Account names only feed the search: a hit carries the account id, never the name.

// Wired to the window in part 2; until then only the tests use it.
#![cfg_attr(not(test), allow(dead_code))]

use crate::daily::effective_privacy;
use crate::nav::Target;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    View,
    Sync,
    Export,
    Settings,
    Rules,
    Tags,
    Account,
}

impl Group {
    /// Subtitle of a row; searched too, so "konto" lists the accounts.
    pub fn label(self) -> &'static str {
        match self {
            Self::View => "Ansicht",
            Self::Sync => "Abruf",
            Self::Export => "Export",
            Self::Settings => "Einstellungen",
            Self::Rules => "Regeln",
            Self::Tags => "Tags",
            Self::Account => "Konto",
        }
    }
}

/// What running a hit does; the window maps it to its handlers (part 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Show(Target),
    SyncAll,
    ExportBookings,
    OpenSettings,
    EditRules,
    ApplyRules,
    ManageTags,
    /// Filter the booking list by this tag name.
    FilterTag(String),
}

/// Fixed commands in display order (empty query, ties in the ranking).
const CATALOG: &[(&str, Group, Action)] = &[
    (
        "Alle Konten",
        Group::View,
        Action::Show(Target::AllAccounts),
    ),
    (
        "Neue Umsätze",
        Group::View,
        Action::Show(Target::NewBookings),
    ),
    ("Vermögen", Group::View, Action::Show(Target::Wealth)),
    ("Depot", Group::View, Action::Show(Target::Depot)),
    ("Geplant", Group::View, Action::Show(Target::Planned)),
    (
        "Verträge & Abos",
        Group::View,
        Action::Show(Target::Subscriptions),
    ),
    ("Lastschriften", Group::View, Action::Show(Target::Mandates)),
    ("Belege", Group::View, Action::Show(Target::Documents)),
    (
        "Unkategorisiert",
        Group::View,
        Action::Show(Target::Uncategorized),
    ),
    ("Alle Konten abrufen", Group::Sync, Action::SyncAll),
    ("Umsätze exportieren", Group::Export, Action::ExportBookings),
    ("Einstellungen", Group::Settings, Action::OpenSettings),
    ("Regeln bearbeiten", Group::Rules, Action::EditRules),
    ("Regeln jetzt anwenden", Group::Rules, Action::ApplyRules),
    ("Tags verwalten", Group::Tags, Action::ManageTags),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Fixed title of a catalog command; `None` for accounts and tags (part 2 shows the
    /// name looked up from the action, masked in privacy mode).
    pub title: Option<&'static str>,
    pub group: Group,
    pub action: Action,
    /// Disabled hits stay in the list (after the enabled ones) but must not run.
    pub enabled: bool,
}

/// What the window knows when the palette opens.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context<'a> {
    pub unlocked: bool,
    /// The privacy toggle; a locked vault counts as private anyway.
    pub privacy: bool,
    pub has_connections: bool,
    /// `(account id, name)`; the name is only searched, never kept in a hit.
    pub accounts: &'a [(i64, &'a str)],
    pub tags: &'a [&'a str],
}

fn is_enabled(action: &Action, ctx: &Context) -> bool {
    match action {
        Action::OpenSettings => true,
        Action::SyncAll => ctx.unlocked && ctx.has_connections,
        // An export writes personal data into a file: never in privacy mode or locked.
        Action::ExportBookings => !effective_privacy(ctx.privacy, ctx.unlocked),
        _ => ctx.unlocked,
    }
}

/// Lowercase with umlauts as plain vowels and ß as "ss".
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push('a'),
            'ö' => out.push('o'),
            'ü' => out.push('u'),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    out
}

/// The folded query and, if different, its spelling with "ae"/"oe"/"ue" as plain vowels,
/// so "mueller" finds "Müller". Only the query gets this, else "neu" would miss "Neue …".
fn query_forms(query: &str) -> Vec<String> {
    let q = fold(&query.split_whitespace().collect::<Vec<_>>().join(" "));
    let plain = q.replace("ae", "a").replace("oe", "o").replace("ue", "u");
    if plain == q { vec![q] } else { vec![q, plain] }
}

/// Rank of a match, lower is better: 0 title prefix, 1 word prefix, 2 part of the title,
/// 3 every query word somewhere in title or group. Both sides already folded.
fn rank(title: &str, group: &str, q: &str) -> Option<u8> {
    if title.starts_with(q) {
        return Some(0);
    }
    if title
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(q))
    {
        return Some(1);
    }
    if title.contains(q) {
        return Some(2);
    }
    let hay = format!("{title} {group}");
    q.split_whitespace().all(|t| hay.contains(t)).then_some(3)
}

/// Sorted hits for `query`: enabled before disabled, then by rank, then catalog order
/// (catalog, tags, accounts). An empty query lists everything in catalog order.
pub fn search(query: &str, ctx: &Context) -> Vec<Hit> {
    let forms = query_forms(query);
    let mut ranked: Vec<(u8, Hit)> = Vec::new();
    let mut consider = |title: &str, hit: Hit| {
        let r = if forms[0].is_empty() {
            Some(0)
        } else {
            let (t, g) = (fold(title), fold(hit.group.label()));
            forms.iter().filter_map(|q| rank(&t, &g, q)).min()
        };
        if let Some(r) = r {
            ranked.push((r, hit));
        }
    };
    for (title, group, action) in CATALOG {
        let enabled = is_enabled(action, ctx);
        consider(
            title,
            Hit {
                title: Some(*title),
                group: *group,
                action: action.clone(),
                enabled,
            },
        );
    }
    if ctx.unlocked {
        for name in ctx.tags {
            let action = Action::FilterTag((*name).to_string());
            consider(
                name,
                Hit {
                    title: None,
                    group: Group::Tags,
                    action,
                    enabled: true,
                },
            );
        }
        for (id, name) in ctx.accounts {
            let action = Action::Show(Target::Account(*id));
            consider(
                name,
                Hit {
                    title: None,
                    group: Group::Account,
                    action,
                    enabled: true,
                },
            );
        }
    }
    ranked.sort_by_key(|(r, h)| (!h.enabled, *r));
    ranked.into_iter().map(|(_, h)| h).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unlocked<'a>(accounts: &'a [(i64, &'a str)], tags: &'a [&'a str]) -> Context<'a> {
        Context {
            unlocked: true,
            privacy: false,
            has_connections: true,
            accounts,
            tags,
        }
    }

    fn actions(hits: &[Hit]) -> Vec<Action> {
        hits.iter().map(|h| h.action.clone()).collect()
    }

    #[test]
    fn title_prefix_beats_word_prefix_beats_substring() {
        let accounts = [
            (1, "Girokonto Alltag"),
            (2, "Konto Tagesgeld"),
            (3, "Tagesgeld Plus"),
        ];
        let hits = search("tag", &unlocked(&accounts, &[]));
        assert_eq!(
            actions(&hits),
            vec![
                Action::ManageTags,
                Action::Show(Target::Account(3)),
                Action::Show(Target::Account(2)),
                Action::Show(Target::Account(1)),
            ]
        );
        // accounts are found by name but carry only their id
        assert!(hits[1..].iter().all(|h| h.title.is_none()));
    }

    #[test]
    fn umlauts_and_case_do_not_matter() {
        assert_eq!(fold("Straße MÜLLER"), "strasse muller");
        let accounts = [(7, "Müller Depot")];
        for q in ["mueller", "muller", "MÜLLER", "  Müll  "] {
            let hits = search(q, &unlocked(&accounts, &[]));
            assert_eq!(
                hits[0].action,
                Action::Show(Target::Account(7)),
                "query {q:?}"
            );
        }
        let hits = search("vermoegen", &unlocked(&[], &[]));
        assert_eq!(hits[0].action, Action::Show(Target::Wealth));
        // the query alone gets the ae/oe/ue form: "neu" still finds "Neue Umsätze"
        let hits = search("neu", &unlocked(&[], &[]));
        assert_eq!(hits[0].action, Action::Show(Target::NewBookings));
    }

    #[test]
    fn empty_query_lists_whole_catalog_in_order() {
        let accounts = [(1, "Giro"), (2, "Spar")];
        let hits = search("   ", &unlocked(&accounts, &["Urlaub"]));
        assert_eq!(hits.len(), CATALOG.len() + 3);
        assert_eq!(hits[0].action, Action::Show(Target::AllAccounts));
        assert!(hits.iter().all(|h| h.enabled));
        assert_eq!(
            hits[CATALOG.len()].action,
            Action::FilterTag("Urlaub".into())
        );
        assert_eq!(
            hits.last().unwrap().action,
            Action::Show(Target::Account(2))
        );
    }

    #[test]
    fn locked_vault_privacy_and_no_connections_disable_commands() {
        let accounts = [(1, "Giro")];
        let locked = Context {
            accounts: &accounts,
            tags: &["Urlaub"],
            ..Context::default()
        };
        let hits = search("", &locked);
        assert_eq!(hits[0].action, Action::OpenSettings);
        assert_eq!(hits.iter().filter(|h| h.enabled).count(), 1);
        assert!(
            hits.iter().all(|h| h.title.is_some()),
            "no accounts/tags while locked"
        );
        assert!(search("giro", &locked).is_empty());

        let private = Context {
            privacy: true,
            ..unlocked(&[], &[])
        };
        let export = search("export", &private);
        assert_eq!(export[0].action, Action::ExportBookings);
        assert!(!export[0].enabled);

        let offline = Context {
            has_connections: false,
            ..unlocked(&[], &[])
        };
        let sync = search("abrufen", &offline);
        assert_eq!(sync[0].action, Action::SyncAll);
        assert!(!sync[0].enabled);
        // disabled hits sort after enabled ones even with a better rank
        let all = search("alle konten", &offline);
        assert_eq!(all[0].action, Action::Show(Target::AllAccounts));
        assert_eq!(all.last().unwrap().action, Action::SyncAll);
    }
}

//! Export and import of rules and tags as a JSON file (#1057).
//!
//! A rule refers to a category, accounts and tags by their row ids, which a
//! fresh vault (e.g. after a new import) does not share. So the file carries,
//! next to the ids, what identifies them in another vault:
//!
//! - **category**: uuid, `builtin_role` (starter categories) and the path of
//!   names from the top ("Lebensmittel" › "Supermarkt");
//! - **accounts**: IBAN, account number and name (an IBAN shared by several
//!   accounts is narrowed down by number and name);
//! - **tags**: name and colour (tags are matched by name, missing ones created).
//!
//! The import resolves each of them in that order. A rule whose category or
//! (all of) whose accounts cannot be found, or whose field or match kind this
//! build does not know, is skipped rather than imported in a wider form (no
//! category, every account, all fields); the report names it. So is a rule
//! `save_rule` would refuse (empty or invalid pattern, neither category nor
//! tags), already in the preview. A rule that already exists (same name,
//! pattern, field, match kind, category, case sensitivity, amount range,
//! accounts and tags) is skipped too, so importing the same file twice adds nothing.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::i18n::{tr, trf};
use crate::models::{MatchKind, Rule, RuleField};
use crate::vault::Vault;

/// `format` of the file; the import refuses anything else.
const FORMAT: &str = "girocrab-rules-tags";
/// Newest file version this build reads and the one it writes.
const VERSION: u32 = 1;

/// What an import did, for a short summary in the app.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RuleExchangeReport {
    /// Tags that did not exist yet and were created.
    pub tags_created: u32,
    /// Tags found in the vault by name (or uuid) and reused.
    pub tags_existing: u32,
    pub rules_imported: u32,
    /// Rules not imported: already present, or category / accounts not found.
    pub rules_skipped: u32,
    /// One line per imported rule: its name, plus the accounts left out when
    /// some of them were not found.
    pub rules_imported_lines: Vec<String>,
    /// One line per skipped rule with the reason (already present, category or
    /// accounts not found, …).
    pub rules_skipped_lines: Vec<String>,
    /// Names of the tags created, in the file's order (for the details).
    pub tags_created_names: Vec<String>,
    /// Names of the tags that already existed, as the vault spells them.
    pub tags_existing_names: Vec<String>,
    /// One line per tag that could not be created (invalid name), with the reason.
    pub tags_skipped_lines: Vec<String>,
}

/// A file's contents before the import, for choosing what to import (#1057).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RuleExchangePreview {
    pub rules: Vec<RulePreview>,
    pub tags: Vec<TagPreview>,
}

/// One rule of the file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RulePreview {
    /// Position in the file, for [`Vault::import_rules_and_tags_selected`].
    pub index: u32,
    pub name: String,
    /// "Lebensmittel › Supermarkt"; empty = the rule only adds tags.
    pub category_path: String,
    /// Names of the tags the rule adds.
    pub tags: Vec<String>,
    /// Why the import would skip it (already present, category not found, …).
    pub problem: Option<String>,
    /// Imported, but with a remark (some accounts not found).
    pub note: Option<String>,
}

/// One tag of the file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TagPreview {
    pub name: String,
    pub color_hex: String,
    /// A tag of that name is already in the vault (it is reused, not created).
    pub exists: bool,
    /// Why the import would skip it (invalid name).
    pub problem: Option<String>,
}

// ------------------------------------------------------------------ file format

#[derive(Debug, Serialize, Deserialize)]
struct ExchangeFile {
    format: String,
    version: u32,
    #[serde(default)]
    exported_at: String,
    #[serde(default)]
    tags: Vec<TagEntry>,
    #[serde(default)]
    rules: Vec<RuleEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TagEntry {
    #[serde(default)]
    uuid: String,
    name: String,
    #[serde(default)]
    color_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CategoryRef {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    uuid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    builtin_role: Option<String>,
    /// Names from the top level down to the category itself.
    #[serde(default)]
    path: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccountRef {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    iban: String,
    #[serde(default)]
    account_number: String,
    #[serde(default)]
    name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RuleEntry {
    name: String,
    field: String,
    match_kind: String,
    pattern: String,
    #[serde(default)]
    case_sensitive: bool,
    #[serde(default)]
    min_amount_cents: Option<i64>,
    #[serde(default)]
    max_amount_cents: Option<i64>,
    #[serde(default = "enabled_default")]
    enabled: bool,
    /// `None`: the rule only adds tags.
    #[serde(default)]
    category: Option<CategoryRef>,
    /// Empty: all accounts.
    #[serde(default)]
    accounts: Vec<AccountRef>,
    /// Names of the tags the rule adds (entries of `tags`).
    #[serde(default)]
    tags: Vec<String>,
}

fn enabled_default() -> bool {
    true
}

fn match_kind_str(kind: MatchKind) -> &'static str {
    match kind {
        MatchKind::Keyword => "keyword",
        MatchKind::Regex => "regex",
        MatchKind::Expression => "expression",
    }
}

fn match_kind_from(s: &str) -> Option<MatchKind> {
    match s {
        "keyword" => Some(MatchKind::Keyword),
        "regex" => Some(MatchKind::Regex),
        "expression" => Some(MatchKind::Expression),
        _ => None,
    }
}

// ------------------------------------------------------------------ vault lookups

#[derive(Debug, Clone)]
struct CategoryRow {
    id: i64,
    uuid: String,
    builtin_role: Option<String>,
    path: Vec<String>,
}

/// Live categories with their path of names.
fn categories(conn: &Connection) -> CoreResult<Vec<CategoryRow>> {
    let rows: Vec<(i64, Option<i64>, String, Option<String>, Option<String>)> = conn
        .prepare(
            "SELECT id, parent_id, name, uuid, builtin_role FROM categories WHERE deleted_ms IS NULL",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
        .collect::<Result<_, _>>()?;
    let by_id: HashMap<i64, (Option<i64>, String)> = rows
        .iter()
        .map(|(id, parent, name, _, _)| (*id, (*parent, name.clone())))
        .collect();
    Ok(rows
        .iter()
        .map(|(id, _, _, uuid, role)| {
            let mut path = Vec::new();
            let mut cur = Some(*id);
            // a broken parent chain must not loop
            while let Some(c) = cur.filter(|_| path.len() < 32) {
                let Some((parent, name)) = by_id.get(&c) else {
                    break;
                };
                path.push(name.clone());
                cur = *parent;
            }
            path.reverse();
            CategoryRow {
                id: *id,
                uuid: uuid.clone().unwrap_or_default(),
                builtin_role: role.clone().filter(|r| !r.is_empty()),
                path,
            }
        })
        .collect())
}

#[derive(Debug, Clone)]
struct AccountRow {
    id: i64,
    iban: String,
    account_number: String,
    name: String,
}

fn accounts(conn: &Connection) -> CoreResult<Vec<AccountRow>> {
    Ok(conn
        .prepare(
            "SELECT id, iban, account_number, name FROM accounts WHERE deleted_ms IS NULL ORDER BY id",
        )?
        .query_map([], |r| {
            Ok(AccountRow {
                id: r.get(0)?,
                iban: r.get(1)?,
                account_number: r.get(2)?,
                name: r.get(3)?,
            })
        })?
        .collect::<Result<_, _>>()?)
}

/// Live tags: (id, uuid, name, colour), in the user's order.
fn tags(conn: &Connection) -> CoreResult<Vec<(i64, String, String, String)>> {
    Ok(conn
        .prepare(
            "SELECT id, uuid, name, color_hex FROM tags WHERE deleted_ms IS NULL ORDER BY sort_order, id",
        )?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?)
}

fn fold(s: &str) -> String {
    crate::tags::fold(s)
}

fn same_path(a: &[String], b: &[String], loose: bool) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| if loose { fold(x) == fold(y) } else { x == y })
}

/// The category in this vault: uuid, built-in role, exact path, then the path
/// ignoring case and extra spaces.
fn resolve_category(r: &CategoryRef, cats: &[CategoryRow]) -> Option<i64> {
    if !r.uuid.is_empty()
        && let Some(c) = cats.iter().find(|c| c.uuid == r.uuid)
    {
        return Some(c.id);
    }
    if let Some(role) = r.builtin_role.as_deref().filter(|s| !s.is_empty()) {
        let hits: Vec<_> = cats
            .iter()
            .filter(|c| c.builtin_role.as_deref() == Some(role))
            .collect();
        if hits.len() == 1 {
            return Some(hits[0].id);
        }
    }
    if r.path.is_empty() {
        return None;
    }
    for loose in [false, true] {
        let hits: Vec<_> = cats
            .iter()
            .filter(|c| same_path(&c.path, &r.path, loose))
            .collect();
        if hits.len() == 1 {
            return Some(hits[0].id);
        }
    }
    None
}

/// The account in this vault: a unique IBAN, then a unique account number,
/// then a unique name. Several accounts with the IBAN (e.g. sub-accounts of one
/// login) are told apart by number and name; if that fails, none is taken.
fn resolve_account(r: &AccountRef, accs: &[AccountRow]) -> Option<i64> {
    let mut pool: Vec<&AccountRow> = accs.iter().collect();
    let iban = crate::iban::normalize_iban(&r.iban);
    if !iban.is_empty() {
        let hits: Vec<_> = accs
            .iter()
            .filter(|a| crate::iban::normalize_iban(&a.iban) == iban)
            .collect();
        if hits.len() == 1 {
            return Some(hits[0].id);
        }
        if !hits.is_empty() {
            pool = hits;
        }
    }
    let number = r.account_number.trim();
    if !number.is_empty() {
        let hits: Vec<_> = pool
            .iter()
            .filter(|a| a.account_number.trim() == number)
            .collect();
        if hits.len() == 1 {
            return Some(hits[0].id);
        }
    }
    let name = fold(&r.name);
    if !name.is_empty() {
        let hits: Vec<_> = pool.iter().filter(|a| fold(&a.name) == name).collect();
        if hits.len() == 1 {
            return Some(hits[0].id);
        }
    }
    None
}

/// Reads and checks a rules/tags file.
fn parse(json: &str) -> CoreResult<ExchangeFile> {
    let foreign =
        || CoreError::invalid(tr("Die Datei enthält keine Regeln oder Tags aus GiroCrab."));
    let file: ExchangeFile = serde_json::from_str(json).map_err(|_| foreign())?;
    if file.format != FORMAT {
        return Err(foreign());
    }
    if file.version > VERSION {
        return Err(CoreError::invalid(tr(
            "Die Datei stammt aus einer neueren GiroCrab-Version. Bitte GiroCrab aktualisieren.",
        )));
    }
    Ok(file)
}

fn rule_name(e: &RuleEntry) -> String {
    if e.name.trim().is_empty() {
        e.pattern.clone()
    } else {
        e.name.clone()
    }
}

/// Every tag of the file: its list, plus names a rule uses that the list misses.
fn wanted_tags(file: &ExchangeFile) -> Vec<TagEntry> {
    let mut wanted: Vec<TagEntry> = Vec::new();
    for t in &file.tags {
        if !wanted.iter().any(|w| fold(&w.name) == fold(&t.name)) {
            wanted.push(t.clone());
        }
    }
    for r in &file.rules {
        for t in &r.tags {
            if !wanted.iter().any(|w| fold(&w.name) == fold(t)) {
                wanted.push(TagEntry {
                    uuid: String::new(),
                    name: t.clone(),
                    color_hex: String::new(),
                });
            }
        }
    }
    wanted
}

/// The live tag a file tag stands for: the one with its uuid, else the one with
/// its name (a fresh vault shares no uuid). (id, name)
fn find_tag(live: &[(i64, String, String, String)], t: &TagEntry) -> Option<(i64, String)> {
    let by_uuid = || {
        live.iter()
            .find(|(_, uuid, ..)| !t.uuid.is_empty() && *uuid == t.uuid)
    };
    let by_name = || {
        live.iter()
            .find(|(_, _, name, _)| fold(name) == fold(&t.name))
    };
    by_uuid()
        .or_else(by_name)
        .map(|(id, _, name, _)| (*id, name.clone()))
}

/// Why a tag of the file cannot be created (invalid name); `None` = fine.
fn tag_problem(name: &str) -> Option<String> {
    crate::tags::clean_name(name).err().map(|e| reason(&e))
}

/// A core error as a short reason in a list: the message alone for invalid
/// input (without „Ungültige Eingabe:“).
fn reason(err: &CoreError) -> String {
    match err {
        CoreError::InvalidInput { message } => message.clone(),
        other => other.to_string(),
    }
}

/// What an import does with a rule of the file.
enum Plan {
    Ready {
        category_id: i64,
        field: RuleField,
        match_kind: MatchKind,
        account_ids: Vec<i64>,
        /// Accounts of the rule not found here (name or IBAN).
        missing: Vec<String>,
    },
    /// Not imported; the reason, for the user.
    Skip(String),
}

/// Folded names of live tags by id, for comparing a file rule's tags (by name:
/// tags to be created have no id yet) with those of the rules here.
fn tag_name_map(live: &[(i64, String, String, String)]) -> HashMap<i64, String> {
    live.iter()
        .map(|(id, _, name, _)| (*id, fold(name)))
        .collect()
}

/// A rule's tags as sorted folded names; names that cannot become a tag are
/// left out (the rule is imported without them).
fn tag_set<'a>(names: impl Iterator<Item = &'a String>) -> Vec<String> {
    let mut set: Vec<String> = names
        .filter(|n| tag_problem(n).is_none())
        .map(|n| fold(n))
        .collect();
    set.sort_unstable();
    set.dedup();
    set
}

/// Resolves a rule's category and accounts in this vault. A rule is skipped
/// rather than widened: no category found, or none of its accounts.
/// `names` = [`tag_name_map`] of the tags `existing` rules use.
fn plan_rule(
    e: &RuleEntry,
    cats: &[CategoryRow],
    accs: &[AccountRow],
    existing: &[Rule],
    names: &HashMap<i64, String>,
) -> Plan {
    let Some(match_kind) = match_kind_from(&e.match_kind) else {
        return Plan::Skip(tr("unbekannte Suchart").into());
    };
    // `RuleField::from_str` reads anything unknown as „all fields“: a wider rule
    if !matches!(
        e.field.as_str(),
        "any" | "counterparty" | "purpose" | "booking_text" | "iban"
    ) {
        return Plan::Skip(tr("unbekanntes Feld").into());
    }
    let category_id = match &e.category {
        None => 0,
        Some(c) => match resolve_category(c, cats) {
            Some(id) => id,
            None => {
                return Plan::Skip(trf(
                    "Kategorie „{0}“ nicht gefunden",
                    &[&c.path.join(" › ")],
                ));
            }
        },
    };
    let mut account_ids = Vec::new();
    let mut missing = Vec::new();
    for a in &e.accounts {
        match resolve_account(a, accs) {
            Some(id) => account_ids.push(id),
            None => missing.push(if a.name.is_empty() {
                a.iban.clone()
            } else {
                a.name.clone()
            }),
        }
    }
    if !e.accounts.is_empty() && account_ids.is_empty() {
        // without its accounts the rule would apply to every account
        return Plan::Skip(trf("Konten nicht gefunden ({0})", &[&missing.join(", ")]));
    }
    let name = rule_name(e);
    let field = RuleField::from_str(&e.field);
    // the checks of `save_rule`, so the preview shows what the import will do
    if e.pattern.trim().is_empty() {
        return Plan::Skip(tr("Das Suchmuster ist leer.").into());
    }
    if category_id == 0 && e.tags.is_empty() {
        return Plan::Skip(
            tr("Die Regel braucht eine Kategorie oder mindestens einen Tag.").into(),
        );
    }
    let probe = Rule {
        id: 0,
        name: name.clone(),
        category_id,
        field,
        match_kind,
        pattern: e.pattern.clone(),
        case_sensitive: e.case_sensitive,
        min_amount_cents: e.min_amount_cents,
        max_amount_cents: e.max_amount_cents,
        account_ids: account_ids.clone(),
        priority: 0,
        enabled: e.enabled,
        match_count: 0,
        tag_ids: Vec::new(),
    };
    if let Err(err) = crate::rules::CompiledRule::compile_supported(probe) {
        return Plan::Skip(reason(&err));
    }
    let sorted = |ids: &[i64]| {
        let mut ids = ids.to_vec();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let accounts = sorted(&account_ids);
    let tags = tag_set(e.tags.iter());
    if existing.iter().any(|r| {
        r.name == name
            && r.pattern == e.pattern
            && r.field == field
            && r.match_kind == match_kind
            && r.category_id == category_id
            && r.case_sensitive == e.case_sensitive
            && r.min_amount_cents == e.min_amount_cents
            && r.max_amount_cents == e.max_amount_cents
            && sorted(&r.account_ids) == accounts
            && tag_set(r.tag_ids.iter().filter_map(|id| names.get(id))) == tags
    }) {
        return Plan::Skip(tr("bereits vorhanden").into());
    }
    Plan::Ready {
        category_id,
        field,
        match_kind,
        account_ids,
        missing,
    }
}

fn to_json(file: &ExchangeFile) -> CoreResult<String> {
    Ok(serde_json::to_string_pretty(file)?)
}

#[uniffi::export]
impl Vault {
    /// The rules `rule_ids` (empty = all) with the tags they add, as JSON for a
    /// file (#1057). Each rule names its category and accounts by id and by
    /// path / IBAN / name, so another vault can find them again.
    pub fn export_rules(&self, rule_ids: Vec<i64>) -> Result<String, CoreError> {
        let db = self.db()?;
        let rules: Vec<Rule> = Vault::load_rules(&db)?
            .into_iter()
            .filter(|r| rule_ids.is_empty() || rule_ids.contains(&r.id))
            .collect();
        let cats = categories(&db)?;
        let accs = accounts(&db)?;
        let all_tags = tags(&db)?;
        let mut used: Vec<i64> = rules
            .iter()
            .flat_map(|r| r.tag_ids.iter().copied())
            .collect();
        used.sort_unstable();
        used.dedup();
        let tag_name: HashMap<i64, &str> = all_tags
            .iter()
            .map(|(id, _, name, _)| (*id, name.as_str()))
            .collect();
        let file = ExchangeFile {
            format: FORMAT.into(),
            version: VERSION,
            exported_at: crate::dates::today_iso(),
            tags: all_tags
                .iter()
                .filter(|(id, ..)| used.contains(id))
                .map(|(_, uuid, name, color)| TagEntry {
                    uuid: uuid.clone(),
                    name: name.clone(),
                    color_hex: color.clone(),
                })
                .collect(),
            rules: rules
                .iter()
                .map(|r| RuleEntry {
                    name: r.name.clone(),
                    field: r.field.as_str().into(),
                    match_kind: match_kind_str(r.match_kind).into(),
                    pattern: r.pattern.clone(),
                    case_sensitive: r.case_sensitive,
                    min_amount_cents: r.min_amount_cents,
                    max_amount_cents: r.max_amount_cents,
                    enabled: r.enabled,
                    category: (r.category_id != 0).then(|| {
                        let c = cats.iter().find(|c| c.id == r.category_id);
                        CategoryRef {
                            id: r.category_id,
                            uuid: c.map(|c| c.uuid.clone()).unwrap_or_default(),
                            builtin_role: c.and_then(|c| c.builtin_role.clone()),
                            path: c.map(|c| c.path.clone()).unwrap_or_default(),
                        }
                    }),
                    accounts: r
                        .account_ids
                        .iter()
                        .map(|id| {
                            let a = accs.iter().find(|a| a.id == *id);
                            AccountRef {
                                id: *id,
                                iban: a.map(|a| a.iban.clone()).unwrap_or_default(),
                                account_number: a
                                    .map(|a| a.account_number.clone())
                                    .unwrap_or_default(),
                                name: a.map(|a| a.name.clone()).unwrap_or_default(),
                            }
                        })
                        .collect(),
                    tags: r
                        .tag_ids
                        .iter()
                        .filter_map(|id| tag_name.get(id).map(|n| n.to_string()))
                        .collect(),
                })
                .collect(),
        };
        to_json(&file)
    }

    /// The tags `tag_ids` (empty = all), as JSON for a file (#1057).
    pub fn export_tags(&self, tag_ids: Vec<i64>) -> Result<String, CoreError> {
        let db = self.db()?;
        let file = ExchangeFile {
            format: FORMAT.into(),
            version: VERSION,
            exported_at: crate::dates::today_iso(),
            tags: tags(&db)?
                .into_iter()
                .filter(|(id, ..)| tag_ids.is_empty() || tag_ids.contains(id))
                .map(|(_, uuid, name, color_hex)| TagEntry {
                    uuid,
                    name,
                    color_hex,
                })
                .collect(),
            rules: Vec::new(),
        };
        to_json(&file)
    }

    /// What a file of [`Vault::export_rules`] / [`Vault::export_tags`] holds,
    /// with what an import would do with each entry, for the selection before
    /// the import (#1057). Changes nothing.
    pub fn preview_rules_and_tags(&self, json: String) -> Result<RuleExchangePreview, CoreError> {
        let file = parse(&json)?;
        let db = self.db()?;
        let live = tags(&db)?;
        let names = tag_name_map(&live);
        let existing = Vault::load_rules(&db)?;
        let (cats, accs) = (categories(&db)?, accounts(&db)?);
        Ok(RuleExchangePreview {
            rules: file
                .rules
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let (problem, note) = match plan_rule(e, &cats, &accs, &existing, &names) {
                        Plan::Ready { missing, .. } if !missing.is_empty() => (
                            None,
                            Some(trf(
                                "ohne die nicht gefundenen Konten ({0})",
                                &[&missing.join(", ")],
                            )),
                        ),
                        Plan::Ready { .. } => (None, None),
                        Plan::Skip(reason) => (Some(reason), None),
                    };
                    RulePreview {
                        index: i as u32,
                        name: rule_name(e),
                        category_path: e
                            .category
                            .as_ref()
                            .map(|c| c.path.join(" › "))
                            .unwrap_or_default(),
                        tags: e.tags.clone(),
                        problem,
                        note,
                    }
                })
                .collect(),
            tags: wanted_tags(&file)
                .into_iter()
                .map(|t| {
                    let exists = find_tag(&live, &t).is_some();
                    TagPreview {
                        problem: (!exists).then(|| tag_problem(&t.name)).flatten(),
                        exists,
                        name: t.name,
                        color_hex: t.color_hex,
                    }
                })
                .collect(),
        })
    }

    /// Imports everything in a file of [`Vault::export_rules`] or
    /// [`Vault::export_tags`]; see [`Vault::import_rules_and_tags_selected`].
    pub fn import_rules_and_tags(&self, json: String) -> Result<RuleExchangeReport, CoreError> {
        self.import_exchange(&parse(&json)?, None, None)
    }

    /// Imports the rules at `rule_indices` (positions in the file, as
    /// [`Vault::preview_rules_and_tags`] lists them) and the tags named in
    /// `tag_names`. The tags a chosen rule adds are always imported with it.
    /// Missing tags are created (matched by name), the rules are added after the
    /// existing ones with category and accounts found again in this vault.
    /// Imported rules are not applied to bookings yet.
    pub fn import_rules_and_tags_selected(
        &self,
        json: String,
        rule_indices: Vec<u32>,
        tag_names: Vec<String>,
    ) -> Result<RuleExchangeReport, CoreError> {
        self.import_exchange(&parse(&json)?, Some(&rule_indices), Some(&tag_names))
    }
}

impl Vault {
    /// `None` = all rules / all tags of the file.
    fn import_exchange(
        &self,
        file: &ExchangeFile,
        rule_indices: Option<&[u32]>,
        tag_names: Option<&[String]>,
    ) -> CoreResult<RuleExchangeReport> {
        let mut report = RuleExchangeReport {
            tags_created: 0,
            tags_existing: 0,
            rules_imported: 0,
            rules_skipped: 0,
            rules_imported_lines: Vec::new(),
            rules_skipped_lines: Vec::new(),
            tags_created_names: Vec::new(),
            tags_existing_names: Vec::new(),
            tags_skipped_lines: Vec::new(),
        };
        let chosen: Vec<&RuleEntry> = file
            .rules
            .iter()
            .enumerate()
            .filter(|(i, _)| rule_indices.is_none_or(|ids| ids.contains(&(*i as u32))))
            .map(|(_, e)| e)
            .collect();

        let (cats, accs, mut existing, mut priority, mut names) = {
            let db = self.db()?;
            let existing = Vault::load_rules(&db)?;
            let top = existing.iter().map(|r| r.priority).max().unwrap_or(0);
            let names = tag_name_map(&tags(&db)?);
            (categories(&db)?, accounts(&db)?, existing, top, names)
        };

        // Tags: the chosen ones plus every tag a chosen rule adds that will be
        // imported (a skipped rule brings no tag along).
        let needed: Vec<String> = chosen
            .iter()
            .filter(|e| {
                matches!(
                    plan_rule(e, &cats, &accs, &existing, &names),
                    Plan::Ready { .. }
                )
            })
            .flat_map(|e| e.tags.iter().map(|t| fold(t)))
            .collect();
        let wanted: Vec<TagEntry> = wanted_tags(file)
            .into_iter()
            .filter(|t| {
                let key = fold(&t.name);
                needed.contains(&key)
                    || tag_names.is_none_or(|names| names.iter().any(|n| fold(n) == key))
            })
            .collect();
        let mut tag_ids: HashMap<String, i64> = HashMap::new();
        for t in &wanted {
            let existing = {
                let db = self.db()?;
                find_tag(&tags(&db)?, t)
            };
            let id = match existing {
                Some((id, name)) => {
                    report.tags_existing += 1;
                    report.tags_existing_names.push(name);
                    id
                }
                None => {
                    // an invalid colour must not cost the tag; an invalid name skips
                    // only this tag (a rule using it comes without it)
                    let created = self
                        .create_tag(t.name.clone(), t.color_hex.clone())
                        .or_else(|_| self.create_tag(t.name.clone(), String::new()));
                    match created {
                        Ok(tag) => {
                            names.insert(tag.id, fold(&tag.name));
                            report.tags_created += 1;
                            report.tags_created_names.push(tag.name.clone());
                            tag.id
                        }
                        Err(err) => {
                            report
                                .tags_skipped_lines
                                .push(trf("„{0}“: {1}", &[&t.name, &reason(&err)]));
                            continue;
                        }
                    }
                }
            };
            tag_ids.insert(fold(&t.name), id);
        }

        // Rules: after the existing ones, in the file's order.
        for e in chosen {
            let name = rule_name(e);
            let (category_id, field, match_kind, account_ids, missing) =
                match plan_rule(e, &cats, &accs, &existing, &names) {
                    Plan::Skip(reason) => {
                        report.rules_skipped += 1;
                        report
                            .rules_skipped_lines
                            .push(trf("„{0}“: {1}", &[&name, &reason]));
                        continue;
                    }
                    Plan::Ready {
                        category_id,
                        field,
                        match_kind,
                        account_ids,
                        missing,
                    } => (category_id, field, match_kind, account_ids, missing),
                };
            let mut rule_tags: Vec<i64> = e
                .tags
                .iter()
                .filter_map(|t| tag_ids.get(&fold(t)).copied())
                .collect();
            rule_tags.sort_unstable();
            rule_tags.dedup();
            priority += 1;
            let rule = Rule {
                id: 0,
                name: name.clone(),
                category_id,
                field,
                match_kind,
                pattern: e.pattern.clone(),
                case_sensitive: e.case_sensitive,
                min_amount_cents: e.min_amount_cents,
                max_amount_cents: e.max_amount_cents,
                account_ids,
                priority,
                enabled: e.enabled,
                match_count: 0,
                tag_ids: rule_tags,
            };
            match self.save_rule(rule.clone()) {
                Ok(id) => {
                    report.rules_imported += 1;
                    report.rules_imported_lines.push(if missing.is_empty() {
                        name.clone()
                    } else {
                        let note = trf(
                            "ohne die nicht gefundenen Konten ({0})",
                            &[&missing.join(", ")],
                        );
                        trf("„{0}“ – {1}", &[&name, &note])
                    });
                    // a second copy in the same file counts as present
                    existing.push(Rule { id, ..rule });
                }
                Err(err) => {
                    report.rules_skipped += 1;
                    report
                        .rules_skipped_lines
                        .push(trf("„{0}“: {1}", &[&name, &reason(&err)]));
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests;

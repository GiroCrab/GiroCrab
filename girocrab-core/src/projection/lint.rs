use std::path::{Path, PathBuf};

use regex::Regex;

fn has_raw_aggregate(source: &str) -> bool {
    let functions =
        Regex::new(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)").unwrap();
    let sql = Regex::new(r"(?is)\bSUM\s*\([^)]*(?:\bamount_cents|\.amount_cents)").unwrap();
    let rust = Regex::new(r"(?:\+=|saturating_add\s*\()\s*-?\s*\w+\.amount_cents|\.map\s*\(\s*\|\w+\|\s*-?\s*\w+\.amount_cents\s*\)\s*\.sum").unwrap();
    let bank_balance = Regex::new(
        r"\b\w+\.balance_cents\s*=\s*\w+\.balance_cents\.saturating_add\([^;]*amount_cents[^;]*\);",
    )
    .unwrap();
    let functions: Vec<_> = functions.captures_iter(source).collect();
    functions.iter().enumerate().any(|(index, capture)| {
        let start = capture.get(0).unwrap().start();
        let attributes: Vec<_> = source[..start]
            .trim_end()
            .lines()
            .rev()
            .take_while(|l| l.trim_start().starts_with("#["))
            .collect();
        if attributes
            .iter()
            .any(|l| l.trim() == "#[test]" || l.contains("#[tokio::test"))
        {
            return false;
        }
        let end = functions
            .get(index + 1)
            .map(|c| c.get(0).unwrap().start())
            .unwrap_or(source.len());
        let body = &source[start..end];
        if body
            .split('{')
            .next()
            .is_some_and(|s| s.contains("Vec<TransactionSplitInput>"))
        {
            return false;
        }
        let name = &capture[1];
        let category = body.contains("category_id")
            || body.contains("category_name")
            || name.contains("categor")
            || name.contains("cashflow");
        let report = bank_balance.replace_all(body, "");
        category
            && (sql.is_match(&report) || report.contains("amount_cents") && rust.is_match(&report))
    })
}

fn scan(root: &Path, path: &Path, offenders: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let relative = path.strip_prefix(root).unwrap();
        let name = relative.to_string_lossy().replace('\\', "/");
        if name == "girocrab-core/src/projection.rs"
            || name.starts_with("girocrab-core/src/projection/")
            || name.contains("/tests/")
            || name.ends_with("/tests.rs")
        {
            continue;
        }
        if entry.file_type().unwrap().is_dir() {
            scan(root, &path, offenders);
        } else if path.extension().is_some_and(|s| s == "rs")
            && has_raw_aggregate(&std::fs::read_to_string(&path).unwrap())
        {
            offenders.push(relative.to_path_buf());
        }
    }
}

#[test]
fn category_and_split_sums_only_use_the_shared_projection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut offenders = Vec::new();
    for path in [
        "girocrab-core/src",
        "GiroCrabLinux/src",
        "GiroCrabWindows/src",
    ] {
        scan(root, &root.join(path), &mut offenders);
    }
    assert!(
        offenders.is_empty(),
        "Category/split totals must use projection.rs: {}",
        offenders
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
}

#[test]
fn lint_detects_sql_and_row_aggregates_but_not_display_or_validation() {
    assert!(!has_raw_aggregate(
        "fn import_categories(rows:&[Row]) { let category_id=0; for r in rows { st.balance_cents = st.balance_cents.saturating_add(r.amount_cents); } }"
    ));
    assert!(has_raw_aggregate(
        "fn category_totals() { query(\"SELECT category_id, SUM(s.amount_cents) FROM transaction_splits s\"); }"
    ));
    assert!(has_raw_aggregate(
        "fn categories(rows: &[Row]) { for r in rows { add(r.category_name, r.amount_cents); total += r.amount_cents; } }"
    ));
    assert!(has_raw_aggregate(
        "fn cashflow_from_rows(rows: &[Row]) { let n = rows.iter().map(|r|r.amount_cents).sum::<i64>(); }"
    ));
    assert!(!has_raw_aggregate(
        "fn categories(totals: &[CategoryFlowTotal]) { totals.iter().map(|r|r.cents).sum::<i64>(); }"
    ));
    assert!(!has_raw_aggregate(
        "fn validate_splits(parts: &[Part]) { parts.iter().try_fold(0, |s,p|s.checked_add(p.amount_cents)); }"
    ));
    assert!(!has_raw_aggregate(
        "#[test]\nfn category_fixture() { query(\"SELECT category_id, SUM(amount_cents) FROM transactions\"); }"
    ));
    assert!(!has_raw_aggregate(
        "#[test]\n#[ignore]\nfn category_fixture() { query(\"SELECT category_id, SUM(amount_cents) FROM transactions\"); }"
    ));
}

#[test]
fn lint_names_the_offending_platform_file() {
    let dir = crate::test_support::TempDir::new("aggregation-lint");
    let root = dir.join("GiroCrabWindows/src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("bad.rs"),
        "fn category_totals(rows: &[Row]) { for r in rows { n += r.amount_cents; } }",
    )
    .unwrap();
    let mut offenders = Vec::new();
    scan(&dir.join(""), &root, &mut offenders);
    assert_eq!(offenders, vec![PathBuf::from("GiroCrabWindows/src/bad.rs")]);
}

use super::super::persist::documents_fetch_on;
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use jiff::civil::Date;
use sha2::{Digest, Sha256};

const MAX_LIST_BYTES: usize = 512 * 1024;
const MAX_PDF_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedStatement {
    statement_id: String,
    file_reference: String,
    for_year: i16,
    for_month: i8,
}

#[derive(Debug, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AdvanziaStatement {
    pub id: String,
    pub date: String,
    pub key: String,
}

fn statement_account(
    db: &Connection,
    id: i64,
    bank: &str,
    profile: &str,
    expected: Option<i64>,
) -> CoreResult<i64> {
    let (_, state) = connection(db, id)?;
    if state.profile_id.is_empty()
        || state.profile_id != profile
        || state.login_scope != login_scope(db, id)?
        || state.bank_account_id.as_deref() != Some(bank)
    {
        return Err(invalid("Ungültige Konto-Zuordnung."));
    }
    let (reference, _) = compatibility::reference(db, id, &state, bank, None)?;
    let account: Option<i64> = db
        .query_row(
            "SELECT id FROM accounts WHERE connection_id=?1 AND external_ref=?2 AND deleted_ms IS NULL
             AND archived_unix IS NULL AND kind='credit_card' AND currency='EUR'",
            params![id, reference],
            |r| r.get(0),
        )
        .optional()?;
    account
        .filter(|&account| expected.is_none_or(|expected| expected == account))
        .ok_or_else(|| invalid("Ungültige Konto-Zuordnung."))
}

fn parse_list(
    body: &str,
    bank: &str,
    months: u8,
    today: Date,
) -> CoreResult<Vec<AdvanziaStatement>> {
    let schema = || invalid("Ungültige Abrechnungsliste.");
    if body.len() > MAX_LIST_BYTES {
        return Err(schema());
    }
    let list: Vec<ListedStatement> = serde_json::from_str(body).map_err(|_| schema())?;
    if list.len() > 600 || !matches!(months, 0 | 12 | 24) {
        return Err(schema());
    }
    let cutoff = i32::from(today.year()) * 12 + i32::from(today.month()) - i32::from(months);
    let mut ids = HashMap::new();
    let mut keys = HashSet::new();
    let mut statements = Vec::new();
    for item in list {
        let id = uuid::Uuid::try_parse(&item.statement_id)
            .ok()
            .filter(|id| id.to_string().eq_ignore_ascii_case(&item.statement_id))
            .ok_or_else(schema)?;
        let date = Date::new(item.for_year, item.for_month, 1)
            .ok()
            .filter(|d| d.year() > 0 && *d <= today.first_of_month())
            .ok_or_else(schema)?
            .last_of_month();
        if item.file_reference.trim().is_empty()
            || item.file_reference.len() > 512
            || item.file_reference.chars().any(char::is_control)
        {
            return Err(schema());
        }
        // The file reference is an opaque identity, never a URL or a local path.
        let key = hex::encode(Sha256::digest(format!(
            "{bank}:{date}:{}",
            item.file_reference
        )));
        if ids.insert(id, key.clone()).is_some_and(|old| old != key) {
            return Err(schema());
        }
        if months != 0 && i32::from(date.year()) * 12 + i32::from(date.month()) <= cutoff {
            continue;
        }
        if keys.insert(key.clone()) {
            statements.push(AdvanziaStatement {
                id: id.to_string(),
                date: date.to_string(),
                key,
            });
        }
    }
    statements.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.id.cmp(&b.id)));
    Ok(statements)
}

fn valid_statement_date(date: &str) -> CoreResult<Date> {
    crate::dates::parse_iso(date)
        .filter(|d| d.year() > 0 && *d == d.last_of_month() && d.to_string() == date)
        .ok_or_else(|| invalid("Ungültiges Abrechnungsdatum."))
}

#[uniffi::export]
impl Vault {
    pub fn advanzia_statements_enabled(
        &self,
        connection_id: i64,
        bank_account_id: String,
        profile_id: String,
        target_account_id: Option<i64>,
    ) -> CoreResult<bool> {
        crate::banking_gate::check_connector("advanzia")?;
        let db = self.db()?;
        if !documents_fetch_on(&db, connection_id)? {
            return Ok(false);
        }
        statement_account(
            &db,
            connection_id,
            &uuid(&bank_account_id)?,
            &profile_id,
            target_account_id,
        )?;
        Ok(true)
    }

    pub fn pending_advanzia_statements(
        &self,
        connection_id: i64,
        bank_account_id: String,
        profile_id: String,
        target_account_id: Option<i64>,
        listing: String,
        months: u8,
    ) -> CoreResult<Vec<AdvanziaStatement>> {
        crate::banking_gate::check_connector("advanzia")?;
        let bank = uuid(&bank_account_id)?;
        let mut items = parse_list(&listing, &bank, months, crate::dates::today())?;
        let db = self.db()?;
        let account = statement_account(&db, connection_id, &bank, &profile_id, target_account_id)?;
        if !documents_fetch_on(&db, connection_id)? {
            return Ok(Vec::new());
        }
        let mut query = db.prepare(
            "SELECT source_url FROM booking_documents
            WHERE account_id=?1 AND source_url LIKE 'advanzia-statement:%'",
        )?;
        let known = query
            .query_map([account], |r| r.get::<_, String>(0))?
            .collect::<Result<HashSet<_>, _>>()?;
        items.retain(|item| !known.contains(&format!("advanzia-statement:{}", item.key)));
        Ok(items)
    }

    pub fn import_advanzia_statement(
        &self,
        connection_id: i64,
        bank_account_id: String,
        profile_id: String,
        target_account_id: Option<i64>,
        statement: AdvanziaStatement,
        pdf_base64: String,
    ) -> CoreResult<bool> {
        crate::banking_gate::check_connector("advanzia")?;
        let pdf = decode_pdf(&pdf_base64)?;
        valid_statement_date(&statement.date)?;
        if statement.key.len() != 64
            || !statement
                .key
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("Ungültige Abrechnungsliste."));
        }
        let db = self.db()?;
        let tx = db.unchecked_transaction()?;
        let bank = uuid(&bank_account_id)?;
        let account = statement_account(&tx, connection_id, &bank, &profile_id, target_account_id)?;
        if !documents_fetch_on(&tx, connection_id)? {
            return Ok(false);
        }
        let added = crate::importers::portal_statements::store_statement_pdf(
            &tx,
            account,
            "Advanzia",
            &statement.date,
            &pdf,
            &format!("advanzia-statement:{}", statement.key),
        )?;
        tx.commit()?;
        Ok(added)
    }
}

fn decode_pdf(encoded: &str) -> CoreResult<Vec<u8>> {
    let error = || invalid("Ungültige PDF-Abrechnung.");
    if encoded.len() > MAX_PDF_BYTES.div_ceil(3) * 4 {
        return Err(error());
    }
    let pdf = STANDARD.decode(encoded).map_err(|_| error())?;
    if pdf.len() > MAX_PDF_BYTES || !pdf.starts_with(b"%PDF-") {
        return Err(error());
    }
    Ok(pdf)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "00000000-0000-4000-8000-000000000001";

    fn item(n: u8, file: &str, year: i16, month: i8) -> Value {
        serde_json::json!({
            "statementId": format!("00000000-0000-4000-8000-{n:012}"),
            "fileReference": file, "type":"normal", "forYear":year, "forMonth":month,
        })
    }

    #[test]
    fn statement_periods_and_file_identity() {
        let today = Date::new(2026, 2, 5).unwrap();
        let first = item(1, "one.pdf", 2024, 2);
        let renewed = item(2, "one.pdf", 2024, 2);
        let corrected = item(3, "two.pdf", 2024, 2);
        let recent = item(4, "new.pdf", 2026, 2);
        let boundary = item(5, "last-year.pdf", 2025, 2);
        let list = serde_json::json!([first, renewed, corrected, recent, boundary]).to_string();
        let all = parse_list(&list, ACCOUNT, 0, today).unwrap();
        assert_eq!(
            all.iter().map(|s| s.date.as_str()).collect::<Vec<_>>(),
            ["2026-02-28", "2025-02-28", "2024-02-29", "2024-02-29"]
        );
        assert_ne!(all[2].key, all[3].key);
        assert_eq!(parse_list(&list, ACCOUNT, 12, today).unwrap().len(), 1);
        assert_eq!(parse_list(&list, ACCOUNT, 24, today).unwrap().len(), 2);
        let new_id = parse_list(
            &serde_json::json!([item(6, "one.pdf", 2024, 2)]).to_string(),
            ACCOUNT,
            0,
            today,
        )
        .unwrap();
        assert_eq!(all[2].key, new_id[0].key);
    }

    #[test]
    fn rejects_bad_lists_and_pdfs() {
        let today = Date::new(2026, 2, 5).unwrap();
        let mut wrong_id = item(1, "one.pdf", 2026, 2);
        wrong_id["statementId"] = Value::String("../other".into());
        for body in [
            "{}".into(),
            "[{]".into(),
            serde_json::json!([wrong_id]).to_string(),
            serde_json::json!([item(1, "one.pdf", 2025, 13)]).to_string(),
            serde_json::json!([item(1, "one.pdf", 2026, 3)]).to_string(),
            serde_json::json!([item(1, "", 2026, 2)]).to_string(),
            serde_json::json!([item(1, "one.pdf", 2026, 2), item(1, "other.pdf", 2026, 2)])
                .to_string(),
            " ".repeat(MAX_LIST_BYTES + 1),
        ] {
            assert!(parse_list(&body, ACCOUNT, 0, today).is_err());
        }
        assert!(parse_list("[]", ACCOUNT, 0, today).unwrap().is_empty());
        assert!(valid_statement_date("2024-02-29extra").is_err());
        assert!(valid_statement_date("2024-02-28").is_err());
        assert!(decode_pdf("!").is_err());
        assert!(decode_pdf(&STANDARD.encode(b"%PDF not a signature")).is_err());
        let mut too_large = vec![0; MAX_PDF_BYTES + 1];
        too_large[..5].copy_from_slice(b"%PDF-");
        assert!(decode_pdf(&STANDARD.encode(too_large)).is_err());
        assert!(decode_pdf(&STANDARD.encode(b"%PDF-1.7\nfictional")).is_ok());
    }

    #[test]
    fn a_regular_followup_fetch_stores_only_the_new_monthly_statement() {
        let (vault, connection, _directory) = super::super::tests::setup();
        let profile = vault.advanzia_profile(connection).unwrap().profile_id;
        let sync = |list: Value| {
            let body = super::super::tests::body("APPROVED", "fictional", "-1.00", "Test");
            vault
                .clone()
                .import_advanzia_capture(
                    connection,
                    ACCOUNT.into(),
                    profile.clone(),
                    vec![body],
                    "-42.00".into(),
                    None,
                )
                .unwrap();
            assert!(
                vault
                    .advanzia_statements_enabled(connection, ACCOUNT.into(), profile.clone(), None)
                    .unwrap()
            );
            vault
                .pending_advanzia_statements(
                    connection,
                    ACCOUNT.into(),
                    profile.clone(),
                    None,
                    list.to_string(),
                    24,
                )
                .unwrap()
        };
        let save = |statement| {
            assert!(
                vault
                    .import_advanzia_statement(
                        connection,
                        ACCOUNT.into(),
                        profile.clone(),
                        None,
                        statement,
                        STANDARD.encode(b"%PDF-1.7 fictional"),
                    )
                    .unwrap()
            );
        };
        let today = crate::dates::today();
        let previous = today.first_of_month().yesterday().unwrap();
        let old = item(1, "previous.pdf", previous.year(), previous.month());
        let new = item(2, "new.pdf", today.year(), today.month());
        let mut first = sync(serde_json::json!([old]));
        assert_eq!(first.len(), 1);
        save(first.pop().unwrap());

        let both = serde_json::json!([old, new]);
        let mut second = sync(both.clone());
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].date, today.last_of_month().to_string());
        save(second.pop().unwrap());
        assert!(sync(both).is_empty());
        let account = vault.list_accounts().unwrap()[0].id;
        let db = vault.db().unwrap();
        let stored: i64 = db.query_row(
            "SELECT COUNT(*) FROM booking_documents WHERE account_id=?1 AND doc_type='kontoauszug'",
            [account], |r| r.get(0),
        ).unwrap();
        assert_eq!(stored, 2);
    }

    #[test]
    fn statements_are_bound_and_pdf_errors_leave_the_capture_intact() {
        let (vault, connection, _directory) = super::super::tests::setup();
        let profile = vault.advanzia_profile(connection).unwrap().profile_id;
        let records = || {
            serde_json::json!([item(1, "one.pdf", 2024, 1), item(2, "two.pdf", 2024, 1)])
                .to_string()
        };
        let pending = |expected| {
            vault.pending_advanzia_statements(
                connection,
                ACCOUNT.into(),
                profile.clone(),
                expected,
                records(),
                0,
            )
        };
        assert!(
            pending(None).is_err(),
            "a previously confirmed binding is required"
        );
        let body = super::super::tests::body("APPROVED", "fictional", "-1.00", "Test");
        vault
            .clone()
            .import_advanzia_capture(
                connection,
                ACCOUNT.into(),
                profile.clone(),
                vec![body],
                "-42.00".into(),
                None,
            )
            .unwrap();
        let account = vault.list_accounts().unwrap()[0].id;
        assert!(pending(Some(account + 1)).is_err());
        assert!(
            vault
                .advanzia_statements_enabled(
                    connection,
                    ACCOUNT.into(),
                    "old profile".into(),
                    Some(account)
                )
                .is_err()
        );
        let other = "00000000-0000-4000-8000-000000000002";
        assert!(
            vault
                .advanzia_statements_enabled(
                    connection,
                    other.into(),
                    profile.clone(),
                    Some(account)
                )
                .is_err()
        );
        for statement in pending(Some(account)).unwrap() {
            let save = |pdf: &[u8]| {
                vault.import_advanzia_statement(
                    connection,
                    ACCOUNT.into(),
                    profile.clone(),
                    Some(account),
                    AdvanziaStatement {
                        id: statement.id.clone(),
                        date: statement.date.clone(),
                        key: statement.key.clone(),
                    },
                    STANDARD.encode(pdf),
                )
            };
            assert!(save(b"HTML login").is_err());
            assert!(save(b"%PDF-1.7 fake").unwrap());
            assert!(!save(b"%PDF-1.7 fake").unwrap());
        }
        assert!(pending(Some(account)).unwrap().is_empty());
        assert_eq!(vault.list_accounts().unwrap()[0].balance_cents, -4200);
        assert_eq!(super::super::tests::rows(&vault).len(), 1);
        let db = vault.db().unwrap();
        let count: i64 = db.query_row(
            "SELECT COUNT(*) FROM booking_documents WHERE account_id=?1 AND doc_date='2024-01-31' AND doc_type='kontoauszug'",
            [account], |r| r.get(0),
        ).unwrap();
        assert_eq!(count, 2);
        drop(db);
        vault.set_documents_fetch(connection, false).unwrap();
        assert!(
            !vault
                .advanzia_statements_enabled(connection, ACCOUNT.into(), profile, Some(account))
                .unwrap()
        );
    }
}

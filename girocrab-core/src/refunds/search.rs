use super::*;

pub(super) struct SearchIndex {
    books: Vec<i64>,
    merchants: HashMap<i64, String>,
    linked: HashSet<Key>,
}

fn index<'a>(v: &Vault, db: &Connection, p: &'a Resolved) -> CoreResult<&'a SearchIndex> {
    let rules = v.merchant_rules(db)?;
    Ok(p.search.get_or_init(|| {
        let mut books: Vec<_> = p.state.books.values().filter(|b| b.live).collect();
        books.sort_by(|a, b| {
            b.date
                .cmp(&a.date)
                .then_with(|| a.identity.cmp(&b.identity))
        });
        let mut names = HashMap::new();
        let merchants = books
            .iter()
            .map(|b| {
                let name = names
                    .entry((&b.counterparty, &b.purpose))
                    .or_insert_with(|| {
                        rules
                            .resolve_with_bundled(&b.counterparty, &b.purpose)
                            .0
                            .name
                    });
                (b.id, name.clone())
            })
            .collect();
        let mut linked = HashSet::new();
        for a in &p.allocations {
            if a.refund.is_none() || a.original.is_none() {
                continue;
            }
            linked.insert(a.row.key.clone());
            if let (Some(refund), Some(original)) = (&a.refund, &a.original) {
                linked.insert(Key {
                    refund: refund.clone(),
                    original: original.clone(),
                });
            }
        }
        SearchIndex {
            books: books.into_iter().map(|b| b.id).collect(),
            merchants,
            linked,
        }
    }))
}

pub(super) fn reference_tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | ')' | '+'))
        // plain numbers and real dates are no references; an impossible date-shaped value
        // (31.04.2026) is one
        .filter(|w| {
            let bare = w.trim_matches(|c: char| !c.is_alphanumeric());
            !is_date(w) && (date_shaped(w) || !bare.chars().all(|c| c.is_ascii_digit() || c == '.'))
        })
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(char::to_uppercase)
                .collect::<String>()
        })
        .filter(|w| {
            w.chars().count() >= 6 && w.chars().any(|c| c.is_ascii_digit()) && w != "NOTPROVIDED"
        })
        .collect()
}

/// `(year, month, day)` fields of a date-shaped word (`YYYY-MM-DD`, `DD.MM.YYYY`, `DD/MM/YY` …),
/// valid or not.
fn date_fields(word: &str) -> Option<(Option<i16>, Option<i16>, Option<i16>)> {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    // one separator throughout: `2026-04/17` is no date
    let sep = word.chars().find(|c| matches!(c, '-' | '/' | '.'))?;
    let fields: Vec<_> = word.split(sep).collect();
    let digits = |f: &str, n| f.len() == n && f.bytes().all(|b| b.is_ascii_digit());
    let num = |f: &str| f.parse::<i16>().ok();
    Some(match fields.as_slice() {
        [a, b, c]
            if digits(a, 4) && (digits(b, 1) || digits(b, 2)) && (digits(c, 1) || digits(c, 2)) =>
        {
            (num(a), num(b), num(c))
        }
        [a, b, c]
            if (digits(a, 1) || digits(a, 2))
                && (digits(b, 1) || digits(b, 2))
                && (digits(c, 4) || digits(c, 2)) =>
        {
            let year = num(c).map(|y| if c.len() == 2 { 2000 + y } else { y });
            (year, num(b), num(a))
        }
        _ => return None,
    })
}

fn date_shaped(word: &str) -> bool {
    date_fields(word).is_some()
}

/// A real calendar date: left out of the reference tokens. An impossible date-shaped value
/// (`2026-42-17`, `31.04.2026`) stays a reference.
fn is_date(word: &str) -> bool {
    let Some((year, month, day)) = date_fields(word) else {
        return false;
    };
    matches!((year, month, day), (Some(y), Some(m), Some(d))
        if i8::try_from(m).ok().zip(i8::try_from(d).ok())
            .is_some_and(|(m, d)| jiff::civil::Date::new(y, m, d).is_ok()))
}

fn days_between(a: &str, b: &str) -> Option<i64> {
    Some(
        crate::dates::days_between(
            crate::dates::parse_iso(a.get(..10)?)?,
            crate::dates::parse_iso(b.get(..10)?)?,
        )
        .abs(),
    )
}

fn candidates(
    v: &Vault,
    db: &Connection,
    p: &Resolved,
    own: &Endpoint,
    query: &str,
    limit: usize,
    include_exhausted: bool,
    reason: Option<RefundMatch>,
) -> CoreResult<Vec<RefundCandidate>> {
    let book = p
        .state
        .books
        .get(&own.0)
        .filter(|b| b.live)
        .ok_or_else(invalid)?;
    book.check_capacity()?;
    let amount = book
        .amount_of(&own.1)
        .filter(|a| *a != 0)
        .ok_or_else(invalid)?;
    if p.open(own) == 0 || limit == 0 {
        return Ok(Vec::new());
    }
    let query = query.trim().to_lowercase();
    let index = index(v, db, p)?;
    let mut out = Vec::new();
    let mut exhausted = Vec::new();
    // reference tokens once per booking, not per endpoint pair
    let by_reference = reason == Some(RefundMatch::SameReference);
    let own_tokens = if by_reference {
        reference_tokens(&format!("{} {}", book.purpose, book.reference))
    } else {
        HashSet::new()
    };
    let books = index
        .books
        .iter()
        .map(|id| &p.state.books[id])
        .filter(|b| b.live && b.id != book.id && b.currency == book.currency);
    for b in books {
        // suggestions look 62 days around the booking only: checked before any other work
        if reason.is_some() && !days_between(&book.date, &b.date).is_some_and(|d| d <= 62) {
            continue;
        }
        if b.check_capacity().is_err() {
            continue;
        }
        let merchant = &index.merchants[&b.id];
        let tokens = if by_reference {
            reference_tokens(&format!("{} {}", b.purpose, b.reference))
        } else {
            HashSet::new()
        };
        for (endpoint, other_amount) in b.endpoints() {
            if other_amount == 0 || (other_amount > 0) == (amount > 0) {
                continue;
            }
            if let Some(reason) = reason {
                let matches = match reason {
                    RefundMatch::SameAmount => other_amount.checked_abs() == amount.checked_abs(),
                    RefundMatch::SameMerchant => {
                        !book.counterparty.trim().is_empty()
                            && book.counterparty.trim().to_lowercase()
                                == b.counterparty.trim().to_lowercase()
                            || !merchant.is_empty() && merchant == &index.merchants[&own.0]
                    }
                    RefundMatch::SameReference => !own_tokens.is_disjoint(&tokens),
                };
                if !matches {
                    continue;
                }
            }
            let key = if amount > 0 {
                Key {
                    refund: own.clone(),
                    original: endpoint.clone(),
                }
            } else {
                Key {
                    refund: endpoint.clone(),
                    original: own.clone(),
                }
            };
            if index.linked.contains(&key) {
                continue;
            }
            let open_cents = p.open(&endpoint);
            if open_cents == 0 && (!include_exhausted || exhausted.len() >= limit) {
                continue;
            }
            let label = b.label_of(&endpoint.1);
            if !query.is_empty()
                && !format!(
                    "{} {} {} {} {}",
                    b.counterparty, b.purpose, b.reference, label, merchant
                )
                .to_lowercase()
                .contains(&query)
            {
                continue;
            }
            let category_id = b
                .parts
                .iter()
                .find(|p| p.uuid == endpoint.1)
                .map_or(b.category_id, |p| p.category_id);
            let rows = if open_cents == 0 {
                &mut exhausted
            } else {
                &mut out
            };
            rows.push(RefundCandidate {
                transaction_id: b.id,
                part: endpoint.1,
                part_label: label,
                account_id: b.account_id,
                booking_date: b.date.clone(),
                counterparty: b.counterparty.clone(),
                merchant_name: merchant.clone(),
                amount_cents: other_amount,
                open_cents,
                currency: b.currency.clone(),
                category_id,
            });
            if out.len() == limit {
                return Ok(out);
            }
        }
    }
    out.extend(exhausted.into_iter().take(limit - out.len()));
    Ok(out)
}

#[uniffi::export]
impl Vault {
    /// Both directions use the selected endpoint's sign, including zero-total bookings.
    pub fn refund_candidates(
        &self,
        transaction_id: i64,
        own_part: String,
        query: String,
        limit: u32,
    ) -> CoreResult<Vec<RefundCandidate>> {
        let db = self.db()?;
        let p = self.refund_projection(&db)?;
        candidates(
            self,
            &db,
            &p,
            &Endpoint(transaction_id, own_part),
            &query,
            limit as usize,
            true,
            None,
        )
    }

    pub fn refund_suggestions(
        &self,
        transaction_id: i64,
        own_part: String,
    ) -> CoreResult<Vec<RefundSuggestion>> {
        let db = self.db()?;
        let p = self.refund_projection(&db)?;
        let own = Endpoint(transaction_id, own_part);
        let mut out: Vec<RefundSuggestion> = Vec::new();
        for reason in [
            RefundMatch::SameAmount,
            RefundMatch::SameMerchant,
            RefundMatch::SameReference,
        ] {
            // full: no further (more expensive) search for a weaker reason
            if out.len() == 3 {
                break;
            }
            for c in candidates(self, &db, &p, &own, "", 3, false, Some(reason))? {
                if out.len() == 3 {
                    return Ok(out);
                }
                if out.iter().any(|s| {
                    s.candidate.transaction_id == c.transaction_id && s.candidate.part == c.part
                }) {
                    continue;
                }
                out.push(RefundSuggestion {
                    candidate: c,
                    reason,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod date_tests {
    use super::*;

    /// CodeRabbit (#1852): only real calendar dates are left out of the reference tokens.
    #[test]
    fn only_real_dates_are_dropped_from_references() {
        for date in ["2024-02-29", "29.02.2024", "31/12/26", "2026-1-5"] {
            assert!(is_date(date), "{date}");
        }
        for not_a_date in ["2026-42-17", "2025-02-29", "31.04.2026", "00.01.2026"] {
            assert!(!is_date(not_a_date), "{not_a_date}");
        }
        assert!(reference_tokens("Ref 2026-42-17").contains("20264217"));
        assert!(reference_tokens("Ref 2026-04-17").is_empty());
        // dotted: an impossible date stays, a real one and a plain number go
        assert!(reference_tokens("Ref 31.04.2026").contains("31042026"));
        assert!(reference_tokens("Ref 30.04.2026").is_empty());
        assert!(reference_tokens("Betrag 1.234.567").is_empty());
        assert!(reference_tokens("Betrag 1234567.").is_empty());
        assert!(!is_date("2026-04/17"));
        assert!(reference_tokens("Ref 2026-04/17").contains("20260417"));
    }
}

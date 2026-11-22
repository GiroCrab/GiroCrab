use super::*;

pub(super) struct SearchIndex {
    books: Vec<i64>,
    merchants: HashMap<i64, String>,
    linked: HashSet<Key>,
}

fn index<'a>(v: &Vault, db: &Connection, p: &'a Projection) -> CoreResult<&'a SearchIndex> {
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

fn reference_tokens(text: &str) -> HashSet<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '(' | ')' | '+'))
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit() || c == '.') && !is_date(w))
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

fn is_date(word: &str) -> bool {
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    let fields: Vec<_> = word.split(['-', '/', '.']).collect();
    let digits = |f: &str, n| f.len() == n && f.bytes().all(|b| b.is_ascii_digit());
    match fields.as_slice() {
        [a, b, c] => {
            digits(b, 2)
                && (digits(a, 4) && digits(c, 2) || digits(a, 2) && (digits(c, 4) || digits(c, 2)))
        }
        _ => false,
    }
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
    p: &Projection,
    own: &Endpoint,
    query: &str,
    limit: usize,
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
    let books = index
        .books
        .iter()
        .map(|id| &p.state.books[id])
        .filter(|b| b.live && b.id != book.id && b.currency == book.currency);
    for b in books {
        if b.check_capacity().is_err() {
            continue;
        }
        let merchant = &index.merchants[&b.id];
        for (endpoint, other_amount) in b.endpoints() {
            if other_amount == 0 || (other_amount > 0) == (amount > 0) || p.open(&endpoint) == 0 {
                continue;
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
            let open_cents = p.open(&endpoint);
            out.push(RefundCandidate {
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
        let pool = candidates(self, &db, &p, &own, "", usize::MAX)?;
        if pool.is_empty() {
            return Ok(Vec::new());
        }
        let source = p.state.books.get(&transaction_id).ok_or_else(invalid)?;
        let amount = source
            .amount_of(&own.1)
            .ok_or_else(invalid)?
            .checked_abs()
            .ok_or_else(invalid)?;
        let refs = reference_tokens(&format!("{} {}", source.purpose, source.reference));
        let merchant = &index(self, &db, &p)?.merchants[&transaction_id];
        let mut out: Vec<RefundSuggestion> = Vec::new();
        for reason in [
            RefundMatch::SameAmount,
            RefundMatch::SameMerchant,
            RefundMatch::SameReference,
        ] {
            for c in &pool {
                if out.len() == 3 {
                    return Ok(out);
                }
                if out.iter().any(|s| {
                    s.candidate.transaction_id == c.transaction_id && s.candidate.part == c.part
                }) {
                    continue;
                }
                let other = &p.state.books[&c.transaction_id];
                let matches = match reason {
                    RefundMatch::SameAmount => c.amount_cents.checked_abs() == Some(amount),
                    RefundMatch::SameMerchant => {
                        (!source.counterparty.trim().is_empty()
                            && source.counterparty.trim().to_lowercase()
                                == c.counterparty.trim().to_lowercase()
                            || !merchant.is_empty() && merchant == &c.merchant_name)
                            && days_between(&source.date, &c.booking_date).is_some_and(|d| d <= 62)
                    }
                    RefundMatch::SameReference => !refs.is_disjoint(&reference_tokens(&format!(
                        "{} {}",
                        other.purpose, other.reference
                    ))),
                };
                if matches {
                    out.push(RefundSuggestion {
                        candidate: c.clone(),
                        reason,
                    });
                }
            }
        }
        Ok(out)
    }
}

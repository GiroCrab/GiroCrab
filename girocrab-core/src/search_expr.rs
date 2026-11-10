//! Search expressions for rules, in MoneyMoney's syntax: `AND`/`OR`/`NOT`
//! (uppercase), parentheses, `"phrases"`, `*`/`?` wildcards, `/regex/`,
//! field prefixes (`name:`, `purpose:`, …) and `amount` comparisons.
//! Matching ignores case and folds umlauts (ä → ae, ß → ss). A word matches a
//! whole whitespace-separated word of the text; punctuation doesn't split.

use regex::{Regex, RegexBuilder};

use crate::i18n::{tr, trf};

/// The booking fields an expression can look at.
pub struct ExprInput<'a> {
    pub amount_cents: i64,
    pub name: &'a str,
    pub purpose: &'a str,
    pub booking_text: &'a str,
    pub remote_account: &'a str,
    pub local_account: &'a str,
    pub comment: &'a str,
    pub currency: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Field {
    Name,
    Purpose,
    BookingText,
    RemoteAccount,
    LocalAccount,
    Comment,
    Currency,
    /// Known to MoneyMoney, not stored by GiroCrab: never matches.
    Unsupported(&'static str),
}

impl Field {
    fn parse(s: &str) -> Option<Field> {
        Some(match s {
            "name" => Field::Name,
            "purpose" => Field::Purpose,
            "booking_text" => Field::BookingText,
            "remote_account" => Field::RemoteAccount,
            "local_account" => Field::LocalAccount,
            "comment" => Field::Comment,
            "currency" => Field::Currency,
            "reference" => Field::Unsupported("reference"),
            "mandate" => Field::Unsupported("mandate"),
            "creditor_id" => Field::Unsupported("creditor_id"),
            _ => return None,
        })
    }

    fn text<'a>(self, t: &ExprInput<'a>) -> &'a str {
        match self {
            Field::Name => t.name,
            Field::Purpose => t.purpose,
            Field::BookingText => t.booking_text,
            Field::RemoteAccount => t.remote_account,
            Field::LocalAccount => t.local_account,
            Field::Comment => t.comment,
            Field::Currency => t.currency,
            Field::Unsupported(_) => "",
        }
    }

    /// Account numbers are compared without their spaces, as one word.
    fn is_account(self) -> bool {
        matches!(self, Field::RemoteAccount | Field::LocalAccount)
    }
}

/// Fields searched by a term without a field prefix.
const DEFAULT_FIELDS: [Field; 4] = [
    Field::Name,
    Field::Purpose,
    Field::BookingText,
    Field::Comment,
];

#[derive(Debug, Clone, Copy, PartialEq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

enum Pattern {
    /// One word or a phrase: consecutive words, each an anchored wildcard pattern.
    Words(Vec<Regex>),
    Regex(Regex),
}

enum Node {
    Text(Option<Field>, Pattern),
    Amount(Cmp, i64),
    Not(Box<Node>),
    And(Box<Node>, Box<Node>),
    Or(Box<Node>, Box<Node>),
}

enum Tok {
    Open,
    Close,
    And,
    Or,
    Not,
    Atom(Node),
}

pub struct SearchExpr {
    root: Node,
    unsupported: Vec<&'static str>,
}

impl SearchExpr {
    pub fn parse(src: &str) -> Result<SearchExpr, String> {
        let mut unsupported = Vec::new();
        let toks = tokenize(src, &mut unsupported)?;
        if toks.is_empty() {
            return Err(tr("Der Suchausdruck ist leer.").into());
        }
        let mut p = Parser { toks, pos: 0 };
        let root = p.or()?;
        if p.pos < p.toks.len() {
            return Err(tr("Unerwartete schließende Klammer.").into());
        }
        Ok(SearchExpr { root, unsupported })
    }

    pub fn matches(&self, t: &ExprInput) -> bool {
        eval(&self.root, t)
    }

    /// Fields the expression uses that GiroCrab doesn't store, in order of appearance.
    pub fn unsupported_fields(&self) -> Vec<&'static str> {
        self.unsupported.clone()
    }
}

/// Lowercase plus ä → ae, ö → oe, ü → ue, ß → ss.
fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c => out.push(c),
        }
    }
    out
}

fn wildcard(word: &str) -> Result<Regex, String> {
    let mut src = String::from("^");
    for c in fold(word).chars() {
        match c {
            '*' => src.push_str(".*"),
            '?' => src.push('.'),
            c => src.push_str(&regex::escape(&c.to_string())),
        }
    }
    src.push('$');
    Regex::new(&src).map_err(|e| e.to_string())
}

fn words(text: &str) -> Result<Pattern, String> {
    let parts = text
        .split_whitespace()
        .map(wildcard)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Pattern::Words(parts))
}

fn regex(src: &str) -> Result<Pattern, String> {
    RegexBuilder::new(src)
        .case_insensitive(true)
        .size_limit(1 << 20)
        .build()
        .map(Pattern::Regex)
        .map_err(|e| trf("Ungültiger regulärer Ausdruck: {0}", &[&e]))
}

fn parse_cents(v: &str) -> Option<i64> {
    let (neg, digits) = match v.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let (int, frac) = match digits.split_once([',', '.']) {
        Some((i, f)) => (i, f),
        None => (digits, ""),
    };
    if int.is_empty() && frac.is_empty()
        || !int.chars().all(|c| c.is_ascii_digit())
        || !frac.chars().all(|c| c.is_ascii_digit())
        || frac.len() > 2
    {
        return None;
    }
    let int: i64 = if int.is_empty() { 0 } else { int.parse().ok()? };
    let frac: i64 = format!("{frac:0<2}").parse().ok()?;
    let cents = int.checked_mul(100)?.checked_add(frac)?;
    Some(if neg { -cents } else { cents })
}

fn amount(rest: &str) -> Result<Node, String> {
    let (cmp, value) = [
        ("==", Cmp::Eq),
        ("!=", Cmp::Ne),
        ("<=", Cmp::Le),
        (">=", Cmp::Ge),
        ("=", Cmp::Eq),
        ("<", Cmp::Lt),
        (">", Cmp::Gt),
    ]
    .iter()
    .find_map(|(op, cmp)| rest.strip_prefix(op).map(|v| (*cmp, v)))
    .ok_or_else(|| trf("Ungültiger Betrag „{0}“", &[&rest]))?;
    let cents = parse_cents(value).ok_or_else(|| trf("Ungültiger Betrag „{0}“", &[&value]))?;
    Ok(Node::Amount(cmp, cents))
}

/// Reads up to the closing `end` (a `\` escapes it) and returns the text in between.
fn delimited(chars: &[char], i: &mut usize, end: char) -> Option<String> {
    let mut out = String::new();
    while *i < chars.len() {
        let c = chars[*i];
        *i += 1;
        if c == '\\' && end == '/' && chars.get(*i) == Some(&'/') {
            out.push('/');
            *i += 1;
        } else if c == end {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

/// A quoted phrase or a `/regex/` starting at `chars[*i]`.
fn quoted(chars: &[char], i: &mut usize) -> Result<Pattern, String> {
    let open = chars[*i];
    *i += 1;
    match open {
        '"' => {
            let s = delimited(chars, i, '"')
                .ok_or_else(|| tr("Anführungszeichen nicht geschlossen.").to_string())?;
            words(&s)
        }
        _ => {
            let s = delimited(chars, i, '/')
                .ok_or_else(|| tr("Regulärer Ausdruck nicht mit / geschlossen.").to_string())?;
            regex(&s)
        }
    }
}

fn tokenize(src: &str, unsupported: &mut Vec<&'static str>) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '(' || c == ')' {
            toks.push(if c == '(' { Tok::Open } else { Tok::Close });
            i += 1;
            continue;
        }
        if c == '"' || c == '/' {
            toks.push(Tok::Atom(Node::Text(None, quoted(&chars, &mut i)?)));
            continue;
        }
        // A bare word, possibly `field:` followed by a word, phrase or regex.
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '(' | ')') {
            if chars[i] == ':' {
                break;
            }
            i += 1;
        }
        let head: String = chars[start..i].iter().collect();
        if i < chars.len() && chars[i] == ':' && is_field_name(&head) {
            let field =
                Field::parse(&head).ok_or_else(|| trf("Unbekanntes Feld „{0}“", &[&head]))?;
            if let Field::Unsupported(name) = field {
                if !unsupported.contains(&name) {
                    unsupported.push(name);
                }
            }
            i += 1;
            let pattern = if matches!(chars.get(i), Some('"') | Some('/')) {
                quoted(&chars, &mut i)?
            } else {
                let s = i;
                while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '(' | ')')
                {
                    i += 1;
                }
                let value: String = chars[s..i].iter().collect();
                if value.is_empty() {
                    return Err(trf("Nach „{0}:“ fehlt ein Suchbegriff.", &[&head]));
                }
                if field.is_account() {
                    words(&value.replace(' ', ""))?
                } else {
                    words(&value)?
                }
            };
            toks.push(Tok::Atom(Node::Text(Some(field), pattern)));
            continue;
        }
        // Not a field: the colon belongs to the word.
        while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '(' | ')') {
            i += 1;
        }
        let word: String = chars[start..i].iter().collect();
        toks.push(match word.as_str() {
            "AND" => Tok::And,
            "OR" => Tok::Or,
            "NOT" => Tok::Not,
            w if w.starts_with("amount") && w[6..].starts_with(['=', '!', '<', '>']) => {
                Tok::Atom(amount(&w[6..])?)
            }
            w => Tok::Atom(Node::Text(None, words(w)?)),
        });
    }
    Ok(toks)
}

/// `name` in `name:` – lowercase letters and underscores only, so words like
/// "Ref:123" or "https://…" stay plain words.
fn is_field_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_')
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn or(&mut self) -> Result<Node, String> {
        let mut left = self.and()?;
        while matches!(self.peek(), Some(Tok::Or)) {
            self.pos += 1;
            let right = self.operand("OR", Parser::and)?;
            left = Node::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Node, String> {
        let mut left = self.unary()?;
        loop {
            match self.peek() {
                Some(Tok::And) => {
                    self.pos += 1;
                    let right = self.operand("AND", Parser::unary)?;
                    left = Node::And(Box::new(left), Box::new(right));
                }
                // Adjacent terms: implicit AND.
                Some(Tok::Atom(_)) | Some(Tok::Not) | Some(Tok::Open) => {
                    let right = self.unary()?;
                    left = Node::And(Box::new(left), Box::new(right));
                }
                _ => return Ok(left),
            }
        }
    }

    fn unary(&mut self) -> Result<Node, String> {
        if matches!(self.peek(), Some(Tok::Not)) {
            self.pos += 1;
            let inner = self.operand("NOT", Parser::unary)?;
            return Ok(Node::Not(Box::new(inner)));
        }
        self.primary()
    }

    /// The operand after `op`; an error if the expression ends there.
    fn operand(
        &mut self,
        op: &str,
        next: fn(&mut Parser) -> Result<Node, String>,
    ) -> Result<Node, String> {
        if matches!(self.peek(), None | Some(Tok::Close)) {
            return Err(trf("Der Ausdruck endet nach „{0}“.", &[&op]));
        }
        next(self)
    }

    fn primary(&mut self) -> Result<Node, String> {
        let tok = self.toks.get_mut(self.pos);
        match tok {
            Some(Tok::Open) => {
                self.pos += 1;
                let inner = self.or()?;
                if !matches!(self.peek(), Some(Tok::Close)) {
                    return Err(tr("Klammer nicht geschlossen.").into());
                }
                self.pos += 1;
                Ok(inner)
            }
            Some(Tok::Atom(_)) => {
                let Tok::Atom(node) = std::mem::replace(&mut self.toks[self.pos], Tok::Close)
                else {
                    unreachable!()
                };
                self.pos += 1;
                Ok(node)
            }
            Some(Tok::Close) => Err(tr("Unerwartete schließende Klammer.").into()),
            Some(Tok::And) => Err(tr("„AND“ steht ohne Begriff davor.").into()),
            Some(Tok::Or) => Err(tr("„OR“ steht ohne Begriff davor.").into()),
            Some(Tok::Not) | None => Err(tr("Der Suchausdruck ist unvollständig.").into()),
        }
    }
}

fn eval(node: &Node, t: &ExprInput) -> bool {
    match node {
        Node::Not(n) => !eval(n, t),
        Node::And(a, b) => eval(a, t) && eval(b, t),
        Node::Or(a, b) => eval(a, t) || eval(b, t),
        Node::Amount(cmp, v) => {
            let a = t.amount_cents;
            match cmp {
                Cmp::Eq => a == *v,
                Cmp::Ne => a != *v,
                Cmp::Lt => a < *v,
                Cmp::Gt => a > *v,
                Cmp::Le => a <= *v,
                Cmp::Ge => a >= *v,
            }
        }
        Node::Text(Some(Field::Unsupported(_)), _) => false,
        Node::Text(Some(f), p) => text_matches(p, f.text(t), f.is_account()),
        Node::Text(None, p) => DEFAULT_FIELDS
            .iter()
            .any(|f| text_matches(p, f.text(t), false)),
    }
}

fn text_matches(p: &Pattern, text: &str, account: bool) -> bool {
    match p {
        Pattern::Regex(re) => re.is_match(text),
        Pattern::Words(pat) => {
            let folded = if account {
                fold(&text.replace(' ', ""))
            } else {
                fold(text)
            };
            let tokens: Vec<&str> = folded.split_whitespace().collect();
            !pat.is_empty()
                && tokens
                    .windows(pat.len())
                    .any(|w| w.iter().zip(pat).all(|(tok, re)| re.is_match(tok)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inp<'a>(name: &'a str, purpose: &'a str, amount_cents: i64) -> ExprInput<'a> {
        ExprInput {
            amount_cents,
            name,
            purpose,
            booking_text: "",
            remote_account: "",
            local_account: "",
            comment: "",
            currency: "EUR",
        }
    }

    fn m(expr: &str, t: &ExprInput) -> bool {
        SearchExpr::parse(expr).unwrap().matches(t)
    }

    #[test]
    fn bare_word_matches_whole_token_in_any_text_field() {
        assert!(m("REWE", &inp("REWE Markt GmbH", "", -100)));
        assert!(m("rewe", &inp("", "Einkauf REWE 123", -100)));
        assert!(!m("REWE", &inp("REWEMARKT", "", -100)));
    }

    #[test]
    fn wildcards() {
        assert!(m("Blumen*", &inp("Blumenhaus Kunz", "", -1)));
        assert!(m("*Miete*", &inp("", "Wohnungsmiete.Mai", -1)));
        assert!(m("M?ller", &inp("Miller", "", -1)));
    }

    #[test]
    fn phrase_matches_consecutive_words() {
        assert!(m("\"Bahn AG\"", &inp("DB Bahn AG", "", -1)));
        assert!(!m("\"Bahn AG\"", &inp("AG der Bahn", "", -1)));
    }

    #[test]
    fn boolean_operators_and_precedence() {
        let t = inp("EC-Karte", "Barauszahlung", -1);
        assert!(!m("\"EC-Karte\" AND (NOT \"Barauszahlung\")", &t));
        assert!(m(
            "\"EC-Karte\" AND (NOT \"Barauszahlung\")",
            &inp("EC-Karte", "", -1)
        ));
        assert!(m(
            "Buecher OR eBooks OR (Waren AND Versand)",
            &inp("", "Waren Versand", -1)
        ));
        assert!(m("REWE EDEKA", &inp("REWE EDEKA", "", -1)), "implicit AND");
        assert!(!m("REWE EDEKA", &inp("REWE", "", -1)));
        assert!(
            m("Lidl and Aldi", &inp("Lidl and Aldi", "", -1)),
            "lowercase and is a word"
        );
        assert!(!m("Lidl and Aldi", &inp("Lidl Aldi", "", -1)));
    }

    #[test]
    fn fields_restrict_the_search() {
        assert!(m("name:REWE", &inp("REWE", "", -1)));
        assert!(!m("name:REWE", &inp("", "REWE", -1)));
        assert!(m("purpose:\"Miete Mai\"", &inp("", "Miete Mai 2026", -1)));
        let mut t = inp("", "", -1);
        t.comment = "Urlaub Kroatien";
        t.remote_account = "DE89 3704 0044 0532 0130 00";
        assert!(m("comment:Urlaub", &t));
        assert!(m("remote_account:DE89370400440532013000", &t));
        assert!(m("currency:eur", &t));
    }

    #[test]
    fn amount_comparisons_in_cents() {
        assert!(m("amount<0", &inp("", "", -1)));
        assert!(!m("amount<0", &inp("", "", 0)));
        assert!(m("amount>=12,50", &inp("", "", 1250)));
        assert!(!m("amount>12,50", &inp("", "", 1250)));
        assert!(m("amount!=5.00", &inp("", "", 499)));
        assert!(m("amount=-3,5", &inp("", "", -350)));
        assert!(m("amount==-3,5", &inp("", "", -350)));
        assert!(m("amount<=0,00", &inp("", "", 0)));
    }

    #[test]
    fn umlauts_fold_both_ways() {
        assert!(m("name:Müller", &inp("MUELLER GmbH", "", -1)));
        assert!(m("name:Mueller", &inp("Müller", "", -1)));
        assert!(
            !m("Strasse", &inp("", "Hauptstraße", -1)),
            "whole token only"
        );
        assert!(m("*strasse", &inp("", "Hauptstraße", -1)));
    }

    #[test]
    fn regex_atoms() {
        assert!(m("/GA NR[0-9]{8}/", &inp("", "BahnCard GA NR12345678", -1)));
        assert!(m("purpose:/^miete/", &inp("", "Miete Mai", -1)));
        assert!(!m("name:/^miete/", &inp("", "Miete Mai", -1)));
    }

    #[test]
    fn unsupported_fields_parse_but_never_match() {
        let e = SearchExpr::parse("reference:ABC OR mandate:X OR reference:Y").unwrap();
        assert_eq!(e.unsupported_fields(), vec!["reference", "mandate"]);
        assert!(!e.matches(&inp("ABC", "X", -1)));
    }

    #[test]
    fn syntax_errors_are_reported() {
        for bad in [
            "(REWE",
            "\"REWE",
            "REWE OR",
            "NOT",
            "foo:bar",
            "amount<abc",
            "/(/",
            "",
            "REWE)",
            "AND REWE",
        ] {
            assert!(SearchExpr::parse(bad).is_err(), "{bad}");
        }
    }
}

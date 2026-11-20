//! Generates `girocrab-core/data/fints_directory.tsv` (BLZ -> bank name, FinTS 3.0 PIN/TAN URL).
//!
//! Input: `src/main/resources/blz.properties` of hbci4java (LGPL-2.1, updated with
//! every Bundesbank BLZ release), at a pinned commit:
//! https://raw.githubusercontent.com/hbci4j/hbci4java/<commit>/src/main/resources/blz.properties
//!
//! The output depends only on that file and the two lists below. After moving to a
//! new commit, list the hosts that no longer answer and report those with a known
//! successor to hbci4java (as in hbci4j/hbci4java#160); `HOSTS` bridges the time
//! until the fix is there:
//!   grep -v '^#' girocrab-core/data/fints_directory.tsv | cut -f3 | cut -d/ -f3 | sort -u \
//!     | xargs -P 16 -I{} sh -c 'curl -s -o /dev/null --max-time 20 https://{}/ || echo {}'
//! (2026-11-20: 15 hosts with 42 rows, mostly codes of merged banks and Sparkassen
//! on retired gateway names; they have no known successor and stay in the table.)
//!
//! Usage:
//!   cargo run -p gen-fints-directory -- path/to/blz.properties <hbci4java-commit>

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::{env, fs, process};

const ATRUVIA_1: &str = "https://fints1.atruvia.de/cgi-bin/hbciservlet";
const POSTBANK: &str = "https://hbci.postbank.de/banking/hbci.do";

/// Hosts the dataset lists and the endpoint used instead (`None`: the rows are left out).
const HOSTS: &[(&str, Option<&str>)] = &[
    // Earlier directory versions sent every HVB code to this endpoint and stored
    // connections keep it; both hosts answer, so switch only after a verified login on hbci-01.
    (
        "hbci-01.hypovereinsbank.de",
        Some("https://hbci.hypovereinsbank.de/hbci/hbci.do"),
    ),
    // comdirect is listed by the core itself (`fints_directory::COMDIRECT`, once instead
    // of once per bank code); Commerzbank (with the former Dresdner Bank codes) is
    // offered through PSD2.
    ("fints.comdirect.de", None),
    ("fints.commerzbank.de", None),
    ("hbci.dresdner-bank.de", None),
];

/// Rows written as listed, over the dataset: (blz, name, url); `None` keeps the
/// dataset's URL.
const PINNED: &[(&str, &str, Option<&str>)] = &[
    // Names the bank picker shows, finds and its quick picks match by prefix
    // (`AddBankAssistant.popularNames`); the dataset has "Deutsche Kreditbank Berlin",
    // "ING-DiBa", "Bnp Paribas S.A. Niederlassung Deutschland - Consorsbank", "Deutsche
    // Apotheker- und Ärztebank eG" and "Frankfurter Sparkasse" (the code 1822direkt uses).
    ("12030000", "DKB (Deutsche Kreditbank)", None),
    ("50010517", "ING", None),
    ("76030080", "Consorsbank", None),
    (
        "30060601",
        "Deutsche Apotheker- und Ärztebank (apoBank)",
        None,
    ),
    ("50050201", "1822direkt (Frankfurter Sparkasse)", None),
    // Verified logins: Volksbank Kassel Göttingen was tested on fints1, Miles & More card
    // logins go through the bank's general address (docs/miles-and-more-connector.md).
    ("52090000", "Volksbank Kassel Göttingen", Some(ATRUVIA_1)),
    (
        "50070439",
        "Miles & More Kreditkarte (Deutsche Bank)",
        Some("https://fints.deutsche-bank.de/"),
    ),
    // Retired Postbank codes, still in fints_presets::POSTBANK_BLZ for older IBANs.
    ("80010090", "Postbank (BLZ 80010090)", Some(POSTBANK)),
    ("90010085", "Postbank (BLZ 90010085)", Some(POSTBANK)),
];

fn host(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    let h = rest.split(['/', ':', '?']).next()?;
    (h.contains('.') && !h.contains('@')).then_some(h)
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let [src, commit] = args.as_slice() else {
        eprintln!("usage: cargo run -p gen-fints-directory -- <blz.properties> <hbci4java-commit>");
        process::exit(2);
    };
    if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        eprintln!("the commit must be the full 40-character hash of the hbci4java revision");
        process::exit(2);
    }
    let text = fs::read_to_string(src).unwrap_or_else(|e| {
        eprintln!("cannot read {src}: {e}");
        process::exit(1);
    });

    // blz=name|city|bic|check method|rdh host|pin/tan url|rdh version|pin/tan version|
    let mut banks: BTreeMap<&str, (&str, &str, &str)> = BTreeMap::new();
    let mut not_https = 0;
    for line in text.lines() {
        let Some((blz, rest)) = line.trim_end().split_once('=') else {
            continue;
        };
        if blz.len() != 8 || !blz.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let f: Vec<&str> = rest.split('|').collect();
        let (name, city) = (f[0].trim(), f.get(1).map_or("", |c| c.trim()));
        let url = f.get(5).map_or("", |u| u.trim());
        if url.is_empty() || name.is_empty() {
            continue;
        }
        let Some(h) = host(url) else {
            not_https += 1;
            continue;
        };
        let url = match HOSTS.iter().find(|r| r.0 == h) {
            Some(&(_, Some(to))) => to,
            Some(&(_, None)) => continue,
            None => url,
        };
        banks.insert(blz, (name, city, url));
    }
    if banks.len() < 2000 {
        eprintln!("only {} entries parsed – wrong file?", banks.len());
        process::exit(1);
    }

    // Several bank codes of one bank ("Postbank", "PSD Bank Nord" / "PSD Bank Nord eG"):
    // the city tells them apart in the bank picker.
    let key = |n: &str| n.to_lowercase().trim_end_matches(" eg").to_string();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for (name, _, _) in banks.values() {
        *seen.entry(key(name)).or_default() += 1;
    }
    // without a city, so they are written as listed
    for &(blz, name, url) in PINNED {
        let url = url
            .or_else(|| banks.get(blz).map(|b| b.2))
            .unwrap_or_else(|| {
                eprintln!("pinned row {blz} keeps the dataset's URL, but the dataset lacks it");
                process::exit(1);
            });
        banks.insert(blz, (name, "", url));
    }

    let mut out = format!(
        "# GENERATED by tools/gen-fints-directory – do not edit by hand.\n\
         #\n\
         # BLZ -> FinTS 3.0 PIN/TAN endpoint table from hbci4java's blz.properties\n\
         # (LGPL-2.1, github.com/hbci4j/hbci4java, commit {commit}).\n\
         # Host replacements (HOSTS) and pinned rows (PINNED) are listed in the generator.\n\
         # Names shared by several bank codes carry the city.\n\
         # Columns (tab-separated): blz, bank name, FinTS URL. Sorted by BLZ.\n"
    );
    for (blz, (name, city, url)) in &banks {
        let name = if city.is_empty() || seen[&key(name)] == 1 {
            name.to_string()
        } else if let Some(head) = name.strip_suffix(')') {
            // "Postbank (Giro)" -> "Postbank (Giro, Hamburg)"
            format!("{head}, {city})")
        } else {
            format!("{name} ({city})")
        };
        out.push_str(&format!("{blz}\t{name}\t{url}\n"));
    }

    let dest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../girocrab-core/data/fints_directory.tsv");
    fs::write(&dest, out).unwrap_or_else(|e| {
        eprintln!("cannot write {}: {e}", dest.display());
        process::exit(1);
    });
    println!(
        "{} entries ({not_https} without https) -> {}",
        banks.len(),
        dest.display()
    );
}

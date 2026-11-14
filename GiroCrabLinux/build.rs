use cxx_qt_build::{CxxQtBuilder, QmlFile, QmlModule};

fn main() {
    // BankingPolicy: the bundled policy is trusted for 90 days after the build.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo::rustc-env=GIROCRAB_BUILD_UNIX={now}");
    println!("cargo::rerun-if-changed=../policy/banking-policy.json");
    // QML files live in qml/ but are registered at the root of module
    // `GiroCrab` (qrc:/qt/qml/GiroCrab/Main.qml): the build runs inside qml/,
    // so the implicit directory import of every file *is* the module and the
    // singletons (AppModel, Theme, Fmt, SF) resolve to one instance each.
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    std::env::set_current_dir(manifest.join("qml")).expect("qml/ directory");
    let mut files: Vec<_> = std::fs::read_dir(".")
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .filter(|n| n.to_string_lossy().ends_with(".qml"))
        .collect();
    files.sort();
    let qml: Vec<QmlFile> = files
        .iter()
        .map(|n| {
            let text = std::fs::read_to_string(n).unwrap_or_default();
            QmlFile::from(n).singleton(text.contains("pragma Singleton"))
        })
        .collect();
    println!("cargo::rerun-if-changed={}", manifest.join("qml").display());
    println!("cargo::rerun-if-changed={}", manifest.join("cpp").display());
    println!(
        "cargo::rerun-if-changed={}",
        manifest.join("qml-optional").display()
    );
    print_web_assets(&manifest);
    english_strings(&manifest);
    CxxQtBuilder::new_qml_module(QmlModule::new("GiroCrab").qml_files(qml))
        .qt_module("Quick")
        .qt_module("QuickControls2")
        .qt_module("Widgets")
        .qt_module("PrintSupport")
        // web logins: QNetworkCookie of the QtWebEngine cookie store (cpp/webcookies.cpp)
        .qt_module("Network")
        .qrc(manifest.join("assets.qrc"))
        .cpp_file(manifest.join("cpp/prepare.cpp"))
        .cpp_file(manifest.join("cpp/printing.cpp"))
        .cpp_file(manifest.join("cpp/webcookies.h"))
        .cpp_file(manifest.join("cpp/webcookies.cpp"))
        .cpp_file(manifest.join("cpp/translator.cpp"))
        .file(manifest.join("src/bridge.rs"))
        .build();
    // Our C++ (printing.cpp, prepare.cpp, …) sits in the static library cxx-qt links after its
    // Qt libraries. With `--as-needed` (Debian arm64) the linker has already dropped a Qt
    // library nothing needed up to that point, e.g. QPrinter / QWidget::winId go missing.
    // Naming the Qt libraries once more after that library keeps them resolvable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        for lib in [
            "PrintSupport",
            "Widgets",
            "QuickControls2",
            "Quick",
            "Qml",
            "Network",
            "Gui",
            "Core",
        ] {
            println!("cargo::rustc-link-lib=dylib=Qt6{lib}");
        }
    }
}

/// The print stylesheet and paginator script of the Mac app (PrintReport.swift `css` and
/// `paginator`), so the Linux printout (QtWebEngine) never drifts from it. Written to
/// OUT_DIR/print_web_assets.rs; fails the build if the Swift file no longer has that shape.
fn print_web_assets(manifest: &std::path::Path) {
    let swift = manifest.join("../GiroCrabApp/Sources/GiroCrabUI/PrintReport.swift");
    println!("cargo::rerun-if-changed={}", swift.display());
    let text = std::fs::read_to_string(&swift)
        .unwrap_or_else(|e| panic!("{}: {e} (print stylesheet of the Mac app)", swift.display()));
    let block = |start: &str, indent: usize| -> String {
        let from = text
            .find(start)
            .unwrap_or_else(|| panic!("PrintReport.swift: `{start}` not found"));
        let open = text[from..].find("\"\"\"\n").expect("opening \"\"\"") + from + 4;
        let close = text[open..].find("\"\"\"").expect("closing \"\"\"") + open;
        text[open..close]
            .trim_end()
            .lines()
            .map(|l| {
                l.get(indent..)
                    .filter(|_| l.starts_with(&" ".repeat(indent)))
                    .unwrap_or(l.trim_start())
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let css = block("private static let css = ", 4);
    let script = block("private static func paginator(", 8);
    // The first two lines are `(function(){` and the `var W=…` line with Swift
    // interpolations; QML writes them itself (A4, footer, "Seite {p} von {n}").
    let mut lines = script.lines();
    assert_eq!(
        lines.next(),
        Some("(function(){"),
        "PrintReport.swift: paginator start changed"
    );
    let vars = lines.next().unwrap_or_default();
    assert!(
        vars.starts_with("var W=") && vars.contains("FOOTER=") && vars.contains("PAGEOF="),
        "PrintReport.swift: paginator variables changed: {vars}"
    );
    let body: Vec<&str> = lines.collect();
    assert!(
        !css.contains("\\(") && !body.iter().any(|l| l.contains("\\(")),
        "PrintReport.swift: new Swift interpolation in css/paginator, extend build.rs"
    );
    let out =
        std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("print_web_assets.rs");
    std::fs::write(
        out,
        format!(
            "pub const PRINT_CSS: &str = {css:?};\npub const PRINT_PAGINATOR: &str = {:?};\n",
            body.join("\n")
        ),
    )
    .expect("write print_web_assets.rs");
}

/// English UI texts: the Mac app's string catalog (Localizable.xcstrings, German keys) plus
/// the Linux-only texts in i18n/en-linux.json (which win), both with Qt placeholders
/// (`%1`, `%2` …). Written to OUT_DIR/en.json (German → English) for src/i18n.rs and
/// cpp/translator.cpp; fails the build if the catalog no longer has the expected shape.
fn english_strings(manifest: &std::path::Path) {
    use serde_json::Value;
    use std::collections::BTreeMap;
    let catalog = manifest.join("../GiroCrabApp/Xcode/App/Localizable.xcstrings");
    let linux = manifest.join("i18n/en-linux.json");
    println!("cargo::rerun-if-changed={}", catalog.display());
    println!("cargo::rerun-if-changed={}", linux.display());
    let read = |path: &std::path::Path| -> Value {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("{}: {e} (English UI texts)", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    };
    let root = read(&catalog);
    assert_eq!(
        root["sourceLanguage"], "de",
        "Localizable.xcstrings: sourceLanguage is no longer \"de\", extend build.rs"
    );
    let strings = root["strings"]
        .as_object()
        .expect("Localizable.xcstrings: `strings` is not an object, extend build.rs");
    // `stringUnit.value`, or the "other" form of a plural `variations`.
    let unit = |key: &str, v: &Value| -> String {
        if let Some(s) = v["stringUnit"]["value"].as_str() {
            return s.to_string();
        }
        v["variations"]["plural"]["other"]["stringUnit"]["value"]
            .as_str()
            .unwrap_or_else(|| {
                panic!(
                    "Localizable.xcstrings: English of {key:?} has an unknown shape: {v}, \
                     extend build.rs"
                )
            })
            .to_string()
    };
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (key, entry) in strings {
        let localizations = entry["localizations"].as_object().unwrap_or_else(|| {
            panic!("Localizable.xcstrings: {key:?} has no `localizations`, extend build.rs")
        });
        // Not translated yet: Qt shows the German source (scripts/i18n-report.py lists it).
        let Some(en) = localizations.get("en") else {
            continue;
        };
        let mut value = unit(key, en);
        // `%#@docs@` → the "other" form of substitution `docs`, `%arg` → its argument.
        if let Some(subs) = en.get("substitutions") {
            let subs = subs.as_object().unwrap_or_else(|| {
                panic!("Localizable.xcstrings: substitutions of {key:?} are not an object")
            });
            for (name, sub) in subs {
                let num = sub["argNum"].as_u64().unwrap_or_else(|| {
                    panic!("Localizable.xcstrings: substitution {name} of {key:?} has no argNum")
                });
                let spec = sub["formatSpecifier"].as_str().unwrap_or("@");
                let form = unit(key, sub).replace("%arg", &format!("%{num}${spec}"));
                value = value.replace(&format!("%#@{name}@"), &form);
            }
        }
        assert!(
            !value.contains("%#@"),
            "Localizable.xcstrings: unresolved substitution in {key:?}: {value}, extend build.rs"
        );
        let (key, value) = (qt_placeholders(key), qt_placeholders(&value));
        // A menu text escapes `&` (mnemonic): "Speichern && anwenden".
        if key.contains('&') {
            map.entry(key.replace('&', "&&"))
                .or_insert_with(|| value.replace('&', "&&"));
        }
        // "%@ von %@" and "%1$@ von %2$@" are the same Qt text: the first one wins.
        map.entry(key).or_insert(value);
    }
    let own = read(&linux);
    let own = own
        .as_object()
        .expect("i18n/en-linux.json: not an object (German → English)");
    let marks =
        |s: &str| -> Vec<u32> { (1..=9).filter(|n| s.contains(&format!("%{n}"))).collect() };
    for (key, value) in own {
        let value = value
            .as_str()
            .unwrap_or_else(|| panic!("i18n/en-linux.json: {key:?} is not a string"));
        assert_eq!(
            marks(key),
            marks(value),
            "i18n/en-linux.json: placeholders of {key:?} and {value:?} differ"
        );
        map.insert(key.clone(), value.to_string());
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("en.json");
    std::fs::write(out, serde_json::to_string(&map).expect("en.json")).expect("write en.json");
}

/// Swift format specifiers → Qt's numbered placeholders: `%1$@` → `%1`, otherwise numbered
/// in order (`%@ … %lld` → `%1 … %2`). `%%` becomes `%`; a lone `%` ("30 %") stays text.
fn qt_placeholders(s: &str) -> String {
    const SPECS: [&str; 9] = ["@", "lld", "llu", "ld", "lu", "d", "i", "u", "f"];
    let mut out = String::with_capacity(s.len());
    let mut next = 0;
    let mut rest = s;
    while let Some(at) = rest.find('%') {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 1..];
        if let Some(t) = tail.strip_prefix('%') {
            out.push('%');
            rest = t;
            continue;
        }
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let position = (digits > 0 && tail[digits..].starts_with('$')).then(|| &tail[..digits]);
        let spec_at = position.map_or(0, |_| digits + 1);
        match SPECS.iter().find(|p| tail[spec_at..].starts_with(*p)) {
            Some(spec) => {
                next += 1;
                match position {
                    Some(n) => out.push_str(&format!("%{n}")),
                    None => out.push_str(&format!("%{next}")),
                }
                rest = &tail[spec_at + spec.len()..];
            }
            None => {
                out.push('%');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

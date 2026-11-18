//! Prints the text GiroCrab reads from a PDF (`documents::extract_pdf_text`), the same extraction the
//! document sorting and matching use. For `scripts/analyse-dokumente.py --extractor`: the analysis then
//! reports what the app sees, not what a PDF viewer shows (a viewer reads fonts the app does not).
//!
//! Usage: cargo run -q -p girocrab-core --example pdf_text -- <file.pdf>
fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: pdf_text <file.pdf>");
        std::process::exit(2);
    };
    match std::fs::read(&path) {
        Ok(bytes) => print!("{}", girocrab_core::documents::extract_pdf_text(&bytes)),
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    }
}

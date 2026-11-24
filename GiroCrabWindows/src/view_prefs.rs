//! Geräte-lokale Ansichts-Schalter aus dem Menü „Ansicht“ (Mac-Vorbild: `ViewLayout.swift`,
//! `showStatistics` / `headerChartCollapsed` / `compactHeader`). Liegen als `view-prefs.json`
//! neben dem Tresor, gehören aber zum Gerät, nicht zum Tresor: Sperren setzt sie nicht zurück.
//! „Saldo“ und „Letzter Abruf“ sind nicht Teil dieser Datei (#1757, #1756).

use std::path::{Path, PathBuf};

use slint::ComponentHandle;

use crate::MainWindow;

const KEY_CHART: &str = "showChart";
const KEY_STATISTICS: &str = "showStatistics";
const KEY_COMPACT_HEADER: &str = "compactHeader";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewPrefs {
    pub show_chart: bool,
    pub show_statistics: bool,
    pub compact_header: bool,
}

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            show_chart: true,
            // Wie bisher eingeklappt: sonst verdrängen die Karten bei 600 px die Liste.
            show_statistics: false,
            compact_header: false,
        }
    }
}

impl ViewPrefs {
    /// Fehlende Schlüssel (alte Dateien), falsche Typen und kaputtes JSON ergeben Standardwerte.
    pub fn from_json(text: &str) -> Self {
        let d = Self::default();
        let Ok(serde_json::Value::Object(map)) = serde_json::from_str(text) else {
            return d;
        };
        let flag = |key: &str, default: bool| {
            map.get(key)
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(default)
        };
        Self {
            show_chart: flag(KEY_CHART, d.show_chart),
            show_statistics: flag(KEY_STATISTICS, d.show_statistics),
            compact_header: flag(KEY_COMPACT_HEADER, d.compact_header),
        }
    }

    pub fn to_json(self) -> String {
        serde_json::json!({
            KEY_CHART: self.show_chart,
            KEY_STATISTICS: self.show_statistics,
            KEY_COMPACT_HEADER: self.compact_header,
        })
        .to_string()
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|t| Self::from_json(&t))
            .unwrap_or_default()
    }

    /// Atomar: erst `.tmp` schreiben, dann umbenennen (ersetzt auch unter Windows).
    pub fn save(self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, self.to_json())?;
        std::fs::rename(&tmp, path)
    }

    fn apply(self, win: &MainWindow) {
        win.set_show_chart(self.show_chart);
        win.set_show_statistics(self.show_statistics);
        win.set_compact_header(self.compact_header);
    }

    fn read(win: &MainWindow) -> Self {
        Self {
            show_chart: win.get_show_chart(),
            show_statistics: win.get_show_statistics(),
            compact_header: win.get_compact_header(),
        }
    }
}

/// Datei neben dem Tresor (`%LOCALAPPDATA%\GiroCrab\view-prefs.json`).
pub fn prefs_path(vault_path: &Path) -> PathBuf {
    vault_path.with_file_name("view-prefs.json")
}

/// Lädt die Schalter ins Fenster und speichert jede Änderung. `None` (Demo/Screenshot):
/// Standardwerte, nichts wird geschrieben.
pub fn install(win: &MainWindow, path: Option<PathBuf>) {
    path.as_deref()
        .map(ViewPrefs::load)
        .unwrap_or_default()
        .apply(win);
    let weak = win.as_weak();
    win.on_view_prefs_changed(move || {
        let (Some(win), Some(path)) = (weak.upgrade(), path.as_deref()) else {
            return;
        };
        if let Err(e) = ViewPrefs::read(&win).save(path) {
            eprintln!("Ansichts-Schalter nicht gespeichert: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "girocrab-view-prefs-{}-{nanos}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn defaults_show_chart_only() {
        let d = ViewPrefs::default();
        assert!(d.show_chart);
        assert!(!d.show_statistics);
        assert!(!d.compact_header);
    }

    #[test]
    fn old_file_without_keys_gives_defaults() {
        assert_eq!(ViewPrefs::from_json("{}"), ViewPrefs::default());
        assert_eq!(
            ViewPrefs::from_json(r#"{"theme":"dark","compactRows":true}"#),
            ViewPrefs::default()
        );
    }

    #[test]
    fn partial_and_broken_values_fall_back_per_key() {
        let p = ViewPrefs::from_json(r#"{"compactHeader":true,"showChart":"ja"}"#);
        assert!(p.compact_header);
        assert!(p.show_chart, "falscher Typ ergibt den Standardwert");
        assert!(!p.show_statistics);
        assert_eq!(ViewPrefs::from_json("kaputt"), ViewPrefs::default());
        assert_eq!(ViewPrefs::from_json("[true]"), ViewPrefs::default());
    }

    #[test]
    fn save_and_load_round_trip() {
        let path = temp_file("roundtrip.json");
        assert_eq!(
            ViewPrefs::load(&path),
            ViewPrefs::default(),
            "fehlende Datei"
        );
        let prefs = ViewPrefs {
            show_chart: false,
            show_statistics: true,
            compact_header: true,
        };
        prefs.save(&path).unwrap();
        assert_eq!(ViewPrefs::load(&path), prefs);
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prefs_live_next_to_the_vault() {
        let p = prefs_path(Path::new("/data/GiroCrab/vault.sqlite"));
        assert_eq!(p, Path::new("/data/GiroCrab/view-prefs.json"));
    }
}

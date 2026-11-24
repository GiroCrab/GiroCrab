//! Geräte-lokale Ansichts-Schalter aus dem Menü „Ansicht“ (Mac-Vorbild: `ViewLayout.swift`,
//! `showStatistics` / `headerChartCollapsed` / `compactHeader`). Liegen als `view-prefs.json`
//! neben dem Tresor, gehören aber zum Gerät, nicht zum Tresor: Sperren setzt sie nicht zurück.
//! „Saldo“ und „Letzter Abruf“ sind nicht Teil dieser Datei (#1757, #1756).

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

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

    /// Atomar: erst eine `.tmp` pro Prozess schreiben, dann umbenennen (ersetzt auch unter
    /// Windows). Der Prozessname im Dateinamen verhindert, dass sich zwei Instanzen mit
    /// demselben Tresorordner die Zwischendatei wegnehmen.
    pub fn save(self, path: &Path) -> std::io::Result<()> {
        let tmp = tmp_path(path);
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

/// Zwischendatei für `save`: pro Prozess eindeutig, liegt neben der Zieldatei.
fn tmp_path(path: &Path) -> PathBuf {
    path.with_extension(format!("json.{}.tmp", std::process::id()))
}

/// Zustand der Schalter gegenüber dem Demo-Modus: Im Demo-Modus gelten die Standardwerte,
/// nichts wird gespeichert, und die normalen Werte bleiben im Speicher für den Rückweg.
#[derive(Debug)]
struct PrefsState {
    demo: bool,
    normal: ViewPrefs,
}

impl PrefsState {
    /// Ein Schalter wurde bedient. `true`: normal speichern. Im Demo-Modus nie.
    fn changed(&mut self, current: ViewPrefs) -> bool {
        if self.demo {
            return false;
        }
        self.normal = current;
        true
    }

    /// Wechsel von/in den Demo-Modus. Liefert die Werte, die das Fenster zeigen soll.
    fn set_demo(&mut self, demo: bool) -> ViewPrefs {
        self.demo = demo;
        if demo {
            ViewPrefs::default()
        } else {
            self.normal
        }
    }
}

static STATE: Mutex<Option<PrefsState>> = Mutex::new(None);

fn state() -> MutexGuard<'static, Option<PrefsState>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Lädt die Schalter ins Fenster und speichert jede Änderung. `None` (Demo/Screenshot):
/// Standardwerte, nichts wird geschrieben.
pub fn install(win: &MainWindow, path: Option<PathBuf>) {
    let initial = path.as_deref().map(ViewPrefs::load).unwrap_or_default();
    initial.apply(win);
    *state() = Some(PrefsState {
        demo: false,
        normal: initial,
    });
    let weak = win.as_weak();
    win.on_view_prefs_changed(move || {
        let (Some(win), Some(path)) = (weak.upgrade(), path.as_deref()) else {
            return;
        };
        let current = ViewPrefs::read(&win);
        if !state().as_mut().is_some_and(|s| s.changed(current)) {
            return;
        }
        if let Err(e) = current.save(path) {
            eprintln!("Ansichts-Schalter nicht gespeichert: {e}");
        }
    });
}

/// Demo-Modus der App ein- oder ausgeschaltet: im Demo-Modus Standardwerte und kein
/// Speichern, danach wieder die normalen Geräte-Schalter.
pub fn set_demo(win: &MainWindow, demo: bool) {
    let prefs = state().as_mut().map(|s| s.set_demo(demo));
    if let Some(prefs) = prefs {
        prefs.apply(win);
    }
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
        assert!(!tmp_path(&path).exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn prefs_live_next_to_the_vault() {
        let p = prefs_path(Path::new("/data/GiroCrab/vault.sqlite"));
        assert_eq!(p, Path::new("/data/GiroCrab/view-prefs.json"));
    }

    #[test]
    fn staging_file_is_per_process_and_next_to_target() {
        let target = Path::new("/data/GiroCrab/view-prefs.json");
        let tmp = tmp_path(target);
        assert_eq!(tmp.parent(), target.parent());
        assert_ne!(tmp, target.with_extension("json.tmp"));
        let name = tmp.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.contains(&std::process::id().to_string()), "{name}");
        assert!(name.ends_with(".tmp"));
    }

    fn normal_prefs() -> ViewPrefs {
        ViewPrefs {
            show_chart: false,
            show_statistics: true,
            compact_header: true,
        }
    }

    #[test]
    fn normal_mode_saves_and_remembers_the_change() {
        let mut s = PrefsState {
            demo: false,
            normal: ViewPrefs::default(),
        };
        assert!(s.changed(normal_prefs()));
        assert_eq!(s.normal, normal_prefs());
    }

    #[test]
    fn demo_mode_never_saves_and_keeps_normal_values() {
        let mut s = PrefsState {
            demo: false,
            normal: normal_prefs(),
        };
        assert_eq!(
            s.set_demo(true),
            ViewPrefs::default(),
            "Demo: Standardwerte"
        );
        let demo_toggle = ViewPrefs {
            show_chart: true,
            show_statistics: false,
            compact_header: false,
        };
        assert!(!s.changed(demo_toggle), "im Demo-Modus nicht speichern");
        assert_eq!(s.normal, normal_prefs(), "normale Werte bleiben erhalten");
        assert_eq!(
            s.set_demo(false),
            normal_prefs(),
            "Rückweg stellt sie wieder her"
        );
        assert!(s.changed(demo_toggle), "danach wird wieder gespeichert");
    }

    #[test]
    fn entering_demo_twice_does_not_lose_normal_values() {
        let mut s = PrefsState {
            demo: false,
            normal: normal_prefs(),
        };
        s.set_demo(true);
        s.set_demo(true);
        assert_eq!(s.set_demo(false), normal_prefs());
    }

    /// Normales Entsperren ruft `set_demo(false)` ohne vorherigen Demo-Modus auf
    /// (Erfolgszweig von `open`): Die geladenen Werte bleiben, Speichern bleibt an.
    #[test]
    fn unlocking_own_vault_keeps_loaded_values_and_saving() {
        let mut s = PrefsState {
            demo: false,
            normal: normal_prefs(),
        };
        assert_eq!(s.set_demo(false), normal_prefs());
        assert!(s.changed(ViewPrefs::default()), "eigener Tresor speichert");
        assert_eq!(s.normal, ViewPrefs::default());
    }
}

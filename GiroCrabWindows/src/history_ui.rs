//! Zeitraum beim ersten Abruf eines neuen Zugangs (#1043, #1767): Auswahl im Dialog
//! „Bank / Konto hinzufügen“, vorbelegt mit der Einstellung `sync.days_back`.

use girocrab_core::Vault;
use girocrab_core::connectors::HistoryRange;

/// Die Tage-Stufen der Auswahl, kürzeste zuerst; danach folgt „Maximum“ (Mac: `HistoryRange.pickerOptions`).
const DAYS: [u32; 3] = [90, 365, 730];

/// Index von „Maximum“, dem Standard von `sync.days_back`.
pub const MAX_INDEX: i32 = 3;

/// Vorbelegung aus dem Wert von `sync.days_back`: fehlt er oder ist er `max`, gilt „Maximum“.
/// Tage zwischen den Stufen ergeben die größte Stufe, die sie nicht überschreitet;
/// ungültige Werte gelten wie im Core als 90 Tage.
pub fn index_of_setting(value: Option<&str>) -> i32 {
    match value.map(str::trim) {
        None | Some("" | "max") => MAX_INDEX,
        Some(v) => {
            let days: u32 = v.parse().ok().filter(|d| *d > 0).unwrap_or(90);
            DAYS.iter().rposition(|d| *d <= days).unwrap_or(0) as i32
        }
    }
}

/// Der Zeitraum zum Index der Auswahl; alles außerhalb der Tage-Stufen ist „Maximum“.
pub fn range_of(index: i32) -> HistoryRange {
    match usize::try_from(index).ok().and_then(|i| DAYS.get(i)) {
        Some(&days) => HistoryRange::Days { days },
        None => HistoryRange::Maximum,
    }
}

/// Vorbelegung für den Dialog aus dem Tresor.
pub fn preselect(vault: &Vault) -> i32 {
    let value = vault.get_setting("sync.days_back".into()).ok().flatten();
    index_of_setting(value.as_deref())
}

/// Merkt den neu angelegten Zugang `id` für einen vollen ersten Abruf von `range` vor.
/// Schlägt das fehl, bleibt der Zugang trotzdem angelegt, damit er nicht doppelt hinzugefügt wird.
/// Der zurückgegebene Hinweis nennt dann den Fehler, sonst ist er leer.
pub fn apply(vault: &Vault, id: i64, range: HistoryRange) -> String {
    match vault.request_full_history(id, range) {
        Ok(()) => String::new(),
        Err(e) => format!(" Zeitraum beim ersten Abruf nicht gespeichert: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_max_setting_preselects_maximum() {
        assert_eq!(index_of_setting(None), MAX_INDEX);
        assert_eq!(index_of_setting(Some("max")), MAX_INDEX);
        assert_eq!(index_of_setting(Some(" max ")), MAX_INDEX);
        assert_eq!(index_of_setting(Some("")), MAX_INDEX);
    }

    #[test]
    fn day_settings_preselect_matching_or_lower_step() {
        assert_eq!(index_of_setting(Some("90")), 0);
        assert_eq!(index_of_setting(Some("365")), 1);
        assert_eq!(index_of_setting(Some("730")), 2);
        // ein Wert zwischen den Stufen (z. B. von einem anderen Gerät) überschreitet ihn nie
        assert_eq!(index_of_setting(Some("180")), 0);
        assert_eq!(index_of_setting(Some("1000")), 2);
        assert_eq!(index_of_setting(Some("30")), 0);
    }

    #[test]
    fn invalid_setting_falls_back_to_ninety_days() {
        assert_eq!(index_of_setting(Some("0")), 0);
        assert_eq!(index_of_setting(Some("abc")), 0);
    }

    #[test]
    fn index_maps_to_history_range() {
        assert_eq!(range_of(0), HistoryRange::Days { days: 90 });
        assert_eq!(range_of(1), HistoryRange::Days { days: 365 });
        assert_eq!(range_of(2), HistoryRange::Days { days: 730 });
        assert_eq!(range_of(MAX_INDEX), HistoryRange::Maximum);
        assert_eq!(range_of(-1), HistoryRange::Maximum);
    }

    #[test]
    fn preselected_index_round_trips() {
        for (setting, range) in [
            ("90", HistoryRange::Days { days: 90 }),
            ("365", HistoryRange::Days { days: 365 }),
            ("730", HistoryRange::Days { days: 730 }),
            ("max", HistoryRange::Maximum),
        ] {
            assert_eq!(range_of(index_of_setting(Some(setting))), range);
        }
    }
}

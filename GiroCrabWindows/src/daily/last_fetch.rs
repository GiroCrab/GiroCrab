//! "Abgerufen vor …" (#815): relative time of an account's last successful fetch, in German
//! local time. Pure functions only; `main.rs` supplies the clock and the connection timestamps.

const DAY: i64 = 86_400;

/// Days since 1970-01-01 -> (year, month, day) in the proleptic Gregorian calendar.
pub(super) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// (year, month, day) -> days since 1970-01-01; inverse of `civil_from_days`.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Last Sunday of a month with 31 days (March, October), as days since the epoch.
fn last_sunday(y: i64, m: i64) -> i64 {
    let day31 = days_from_civil(y, m, 31);
    // 1970-01-01 was a Thursday: (days + 4) mod 7 == 0 on Sundays.
    day31 - (day31 + 4).rem_euclid(7)
}

/// UTC offset of Europe/Berlin (the app is German-only; no time zone crate, no Win32 call):
/// CEST from the last Sunday of March to the last Sunday of October, 01:00 UTC each, else CET.
fn berlin_offset_secs(utc_secs: i64) -> i64 {
    let (y, _, _) = civil_from_days(utc_secs.div_euclid(DAY));
    let start = last_sunday(y, 3) * DAY + 3_600;
    let end = last_sunday(y, 10) * DAY + 3_600;
    if (start..end).contains(&utc_secs) {
        7_200
    } else {
        3_600
    }
}

/// Relative German text for the last successful fetch ("vor 5 Minuten", "gestern, 14:02",
/// "vor 3 Tagen", …). `None` without a fetch (offline/import accounts, never synced).
pub fn format_last_fetch(now_ms: i64, fetched_ms: Option<i64>) -> Option<String> {
    let fetched = fetched_ms.filter(|f| *f > 0)?.div_euclid(1_000);
    let now = now_ms.div_euclid(1_000);
    // A timestamp slightly in the future (clock skew) counts as "just now".
    let age = (now - fetched).max(0);
    if age < 60 {
        return Some("gerade eben".into());
    }
    if age < 3_600 {
        let n = age / 60;
        return Some(if n == 1 {
            "vor 1 Minute".into()
        } else {
            format!("vor {n} Minuten")
        });
    }
    let now_local = now + berlin_offset_secs(now);
    let local = fetched + berlin_offset_secs(fetched);
    let text = match now_local.div_euclid(DAY) - local.div_euclid(DAY) {
        ..=0 => match age / 3_600 {
            1 => "vor 1 Stunde".into(),
            n => format!("vor {n} Stunden"),
        },
        1 => {
            let sod = local.rem_euclid(DAY);
            format!("gestern, {:02}:{:02}", sod / 3_600, sod % 3_600 / 60)
        }
        n @ 2..=6 => format!("vor {n} Tagen"),
        _ => {
            let (y, m, d) = civil_from_days(local.div_euclid(DAY));
            format!("am {d:02}.{m:02}.{y:04}")
        }
    };
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC wall clock -> Unix milliseconds.
    fn ms(y: i64, m: i64, d: i64, h: i64, min: i64) -> i64 {
        (days_from_civil(y, m, d) * DAY + h * 3_600 + min * 60) * 1_000
    }

    #[test]
    fn calendar_round_trips_known_dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 23), 20_780);
        assert_eq!(civil_from_days(20_780), (2026, 11, 23));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn missing_or_invalid_timestamp_shows_nothing() {
        let now = ms(2026, 11, 23, 12, 0);
        assert_eq!(format_last_fetch(now, None), None);
        assert_eq!(format_last_fetch(now, Some(0)), None);
        assert_eq!(format_last_fetch(now, Some(-5)), None);
    }

    #[test]
    fn minutes_and_hours_on_the_same_day() {
        let now = ms(2026, 11, 23, 12, 0);
        let f = |t| format_last_fetch(now, Some(t)).unwrap();
        assert_eq!(f(now - 30_000), "gerade eben");
        assert_eq!(f(now + 90_000), "gerade eben");
        assert_eq!(f(now - 60_000), "vor 1 Minute");
        assert_eq!(f(now - 5 * 60_000), "vor 5 Minuten");
        assert_eq!(f(ms(2026, 11, 23, 11, 0)), "vor 1 Stunde");
        assert_eq!(f(ms(2026, 11, 23, 9, 0)), "vor 3 Stunden");
    }

    #[test]
    fn yesterday_days_and_older_dates() {
        let now = ms(2026, 11, 23, 12, 0);
        let f = |t| format_last_fetch(now, Some(t)).unwrap();
        // 13:02 UTC = 14:02 CET.
        assert_eq!(f(ms(2026, 11, 22, 13, 2)), "gestern, 14:02");
        assert_eq!(f(ms(2026, 11, 20, 12, 0)), "vor 3 Tagen");
        assert_eq!(f(ms(2026, 11, 17, 12, 0)), "vor 6 Tagen");
        assert_eq!(f(ms(2026, 11, 10, 12, 0)), "am 10.11.2026");
    }

    #[test]
    fn day_boundary_follows_german_local_time() {
        // 23:30 UTC is already 00:30 on the 23rd in Berlin; 21:30 UTC was 22:30 on the 22nd.
        let now = ms(2026, 11, 22, 23, 30);
        assert_eq!(
            format_last_fetch(now, Some(ms(2026, 11, 22, 21, 30))).unwrap(),
            "gestern, 22:30"
        );
        // Summer time: 12:02 UTC = 14:02 CEST.
        let now = ms(2026, 7, 15, 8, 0);
        assert_eq!(
            format_last_fetch(now, Some(ms(2026, 7, 14, 12, 2))).unwrap(),
            "gestern, 14:02"
        );
    }

    #[test]
    fn summer_time_switches_at_one_utc() {
        let at = |y, m, d, h, min| berlin_offset_secs(ms(y, m, d, h, min) / 1_000);
        assert_eq!(at(2026, 3, 29, 0, 59), 3_600);
        assert_eq!(at(2026, 3, 29, 1, 0), 7_200);
        assert_eq!(at(2026, 10, 25, 0, 59), 7_200);
        assert_eq!(at(2026, 10, 25, 1, 0), 3_600);
    }
}

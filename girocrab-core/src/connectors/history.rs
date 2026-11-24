//! How far back a sync reaches (#1043): the setting `sync.days_back` and the
//! full fetch of a login.
//!
//! A login added by the user is marked for a full fetch. Its next full sync
//! the user starts reaches back the chosen period for every account, also for
//! accounts that already have bookings, and once that sync is stored the
//! login goes on from each account's newest booking ([`since_window`]).
//! Automatic syncs and syncs of one account start an account without
//! bookings at most [`QUIET_MAX_DAYS`] back: beyond that many banks want a
//! TAN. Catching up from an account's newest booking is the same in every
//! sync, however long ago it was, so no gap is ever skipped.
//!
//! [`since_window`]: super::since_window

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{CoreError, CoreResult};
use crate::i18n::trf;
use crate::models::ConnectorKind;
use crate::vault::Vault;

/// Vault setting: how far the first fetch of a new login reaches back.
pub(crate) const DAYS_BACK_KEY: &str = "sync.days_back";

/// The first-fetch period of a sync that must not ask for an approval: PSD2
/// lets banks show 90 days of bookings without one.
pub(crate) const QUIET_MAX_DAYS: i64 = 90;

/// How far back a full fetch reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum HistoryRange {
    /// As far back as the bank delivers.
    Maximum,
    Days {
        days: u32,
    },
}

impl HistoryRange {
    /// From a setting value: `max` or a number of days; anything else (also 0,
    /// which would fetch nothing) is the earlier default of 90 days.
    pub(crate) fn parse(value: &str) -> Self {
        match value.trim() {
            "max" => Self::Maximum,
            v => Self::Days {
                days: v.parse().ok().filter(|d| *d > 0).unwrap_or(90),
            },
        }
    }

    fn setting_value(self) -> String {
        match self {
            Self::Maximum => "max".into(),
            Self::Days { days } => days.to_string(),
        }
    }

    /// Days back for `kind`, never more than its connector reaches ([`max_days`]).
    pub(crate) fn days(self, kind: ConnectorKind) -> i64 {
        match self {
            Self::Maximum => max_days(kind),
            Self::Days { days } => i64::from(days).min(max_days(kind)),
        }
    }
}

/// Further back than any of these banks keeps bookings: a connector stops
/// where the bank's history ends.
const LONGEST_DAYS: i64 = 3650;

/// How far back "Maximum" reaches for a connector.
fn max_days(kind: ConnectorKind) -> i64 {
    match kind {
        // the bank's storage period from its parameters cuts it further (`fints::statements`)
        ConnectorKind::FinTsGeneric
        | ConnectorKind::VolksbankKasselGoettingen
        | ConnectorKind::Direkt1822 => LONGEST_DAYS,
        // An account's first sync already asks for the longest history the
        // consent allows (`strategy=longest`). PSD2 banks show more than 90
        // days only right after a strong authentication (the consent), so a
        // start date further back may be refused later.
        ConnectorKind::EnableBanking
        | ConnectorKind::AmericanExpress
        | ConnectorKind::AmazonVisa => QUIET_MAX_DAYS,
        // The start date never reaches the bank: these read everything, or page
        // back from the newest booking and stop there (FFB sends it, and the
        // connector itself falls back to 2008).
        ConnectorKind::TradeRepublic
        | ConnectorKind::Bunq
        | ConnectorKind::HanseaticBank
        | ConnectorKind::Bhw
        | ConnectorKind::Uberspace
        | ConnectorKind::Ibkr
        | ConnectorKind::ScalableCapital
        | ConnectorKind::Ffb => LONGEST_DAYS,
        // The start date goes to the bank; a refusal is tried again with 90
        // days ([`retry_since`]).
        ConnectorKind::Comdirect | ConnectorKind::Kontist | ConnectorKind::VolkswagenBank => {
            LONGEST_DAYS
        }
        // the transaction search reaches three years back
        ConnectorKind::PayPal => 1090,
        // The portal keeps two years and the connector stops there itself.
        // Opel and Ford Money: dates in a web form, where a refused start date
        // need not show as an error; the longest period that could be chosen before.
        ConnectorKind::Easybank | ConnectorKind::OpelDirektbank | ConnectorKind::FordMoney => 730,
        // the start date is not used: the whole history or what the portal shows
        ConnectorKind::AlteLeipziger
        | ConnectorKind::Advanzia
        | ConnectorKind::Fnz
        | ConnectorKind::Quirion
        | ConnectorKind::Sharinbox
        | ConnectorKind::EquatePlus
        | ConnectorKind::Finpension
        | ConnectorKind::SchwaebischHall
        | ConnectorKind::AmazonOrders
        | ConnectorKind::BitcoinCore => LONGEST_DAYS,
    }
}

/// Start date of a second try after the bank answered a request from `since`
/// with `err`: 90 days back, when `since` reached further and the bank refused
/// the request (its answer, not a failed login, a rate limit, a failure on its
/// side or an incomplete list). Otherwise `err` again. A bare 200 says nothing
/// about the failure (a GraphQL error without a status of its own). The first
/// fetch of a login must not fail on its period.
pub(crate) fn retry_since(since: &str, err: CoreError) -> CoreResult<String> {
    let shorter = crate::dates::iso(crate::dates::today() - crate::dates::days(QUIET_MAX_DAYS));
    let refused = match &err {
        // a client error status, or the bank's own non-numeric code (VW `Status`)
        CoreError::Bank { code, .. } => code.parse::<u16>().map_or(true, |c| {
            (400..500).contains(&c) && !matches!(c, 401 | 403 | 429)
        }),
        _ => false,
    };
    if refused && since < shorter.as_str() {
        Ok(shorter)
    } else {
        Err(err)
    }
}

/// Warning of a sync whose start date for `account_ref` the bank refused
/// ([`retry_since`]).
pub(crate) fn refused_warning(account_ref: &str) -> String {
    trf(
        "{0}: Die Bank hat den angeforderten Zeitraum abgelehnt; abgerufen wurden die letzten 90 Tage.",
        &[&account_ref],
    )
}

/// Vault setting key: the period (`max` or days) of the login's pending full fetch.
fn key(connection_id: i64) -> String {
    format!("sync.history.{connection_id}")
}

/// The days the first fetch of an account without bookings reaches back
/// (`sync.days_back`); `user_full_sync`: a full sync the user started.
pub(crate) fn first_fetch_days(
    db: &Connection,
    kind: ConnectorKind,
    user_full_sync: bool,
) -> CoreResult<i64> {
    let value = Vault::setting_inner(db, DAYS_BACK_KEY)?.unwrap_or_default();
    let days = HistoryRange::parse(&value).days(kind);
    Ok(if user_full_sync {
        days
    } else {
        days.min(QUIET_MAX_DAYS)
    })
}

/// The stored value of the login's pending full fetch.
pub(crate) fn pending(db: &Connection, connection_id: i64) -> CoreResult<Option<String>> {
    Vault::setting_inner(db, &key(connection_id))
}

/// Marks a login the user just added for a full fetch with the period of
/// `sync.days_back`.
pub(crate) fn request_default(db: &Connection, connection_id: i64) -> CoreResult<()> {
    let value = Vault::setting_inner(db, DAYS_BACK_KEY)?.unwrap_or_default();
    Vault::set_setting_inner(
        db,
        &key(connection_id),
        &HistoryRange::parse(&value).setting_value(),
    )
}

/// The full fetch with `value` is stored: later syncs go on incrementally. A
/// period asked for in the meantime stays.
pub(crate) fn finish(db: &Connection, connection_id: i64, value: &str) -> CoreResult<()> {
    db.execute(
        "DELETE FROM settings WHERE key = ?1 AND value = ?2",
        params![key(connection_id), value],
    )?;
    Ok(())
}

/// Forgets the pending full fetch of a deleted login.
pub(crate) fn clear(db: &Connection, connection_id: i64) -> CoreResult<()> {
    db.execute("DELETE FROM settings WHERE key = ?1", [key(connection_id)])?;
    Ok(())
}

#[uniffi::export]
impl Vault {
    /// The next full sync the user starts for this login reaches back `range`
    /// for every account, also those that already have bookings (#1043);
    /// later syncs go on from each account's newest booking. Beyond 90 days
    /// many banks ask for a TAN.
    pub fn request_full_history(
        &self,
        connection_id: i64,
        range: HistoryRange,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        let live: Option<i64> = db
            .query_row(
                "SELECT id FROM bank_connections WHERE id = ?1 AND deleted_ms IS NULL",
                [connection_id],
                |r| r.get(0),
            )
            .optional()?;
        if live.is_none() {
            return Err(CoreError::NotFound {
                what: trf("Bankverbindung {0}", &[&connection_id]),
            });
        }
        Vault::set_setting_inner(&db, &key(connection_id), &range.setting_value())
    }

    /// The period the next full sync of this login reaches back, while that
    /// full fetch is pending.
    pub fn pending_full_history(
        &self,
        connection_id: i64,
    ) -> Result<Option<HistoryRange>, CoreError> {
        let db = self.db()?;
        Ok(pending(&db, connection_id)?.map(|v| HistoryRange::parse(&v)))
    }
}

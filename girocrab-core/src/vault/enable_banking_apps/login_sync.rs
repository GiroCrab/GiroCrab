//! Which Enable Banking application a bank login signs with, shared between
//! devices (#962).
//!
//! Settings are not part of the device merge (`sync_merge.rs`), and the local
//! connection id differs per device. The front ends therefore share the
//! assignment themselves (Apple: iCloud Keychain) under a key that is the same
//! on every device: a hash of `sync_merge::connection_key`, so no login name
//! leaves the vault. Each device remembers which assignment it last shared
//! (`enablebanking.synced_app.<conn>`), so it can tell whether a difference was
//! made here or on another device and never silently replaces either.

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use super::{connector_of, own_pair, uses_enable_banking};
use crate::error::CoreError;
use crate::error::CoreResult;
use crate::vault::Vault;

/// Settings key holding the assignment this device last shared for a login.
const SYNCED_PREFIX: &str = "enablebanking.synced_app";
/// Stored for "shared as the default application" (an empty value deletes the setting).
const SYNCED_DEFAULT: &str = "-";

fn synced_key(connection_id: i64) -> String {
    format!("{SYNCED_PREFIX}.{connection_id}")
}

/// The login's key across devices, without its login name.
pub(crate) fn sync_key(db: &Connection, connection_id: i64) -> CoreResult<String> {
    let key = crate::sync_merge::connection_key(db, connection_id)?;
    Ok(hex::encode(Sha256::digest(key.as_bytes())))
}

/// Removes the shared-state marker of a deleted or merged login.
pub(crate) fn forget(db: &Connection, connection_id: i64) -> CoreResult<()> {
    db.execute(
        "DELETE FROM settings WHERE key = ?1",
        [synced_key(connection_id)],
    )?;
    Ok(())
}

/// One Enable Banking login as the cross-device sync sees it. Never carries
/// the private key (see [`Vault::connection_enable_banking_key`]).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EnableBankingLoginApp {
    pub connection_id: i64,
    /// Same for this login on every device (hex SHA-256 of its merge key).
    pub sync_key: String,
    /// The login's own application; empty = it uses the default one.
    pub application_id: String,
    /// What this device last shared or took over; `None` = never, empty =
    /// the default application.
    pub synced_application_id: Option<String>,
}

#[uniffi::export]
impl Vault {
    /// Every login that signs with Enable Banking, with its cross-device key
    /// and the application it uses.
    pub fn list_enable_banking_logins(&self) -> Result<Vec<EnableBankingLoginApp>, CoreError> {
        let db = self.db()?;
        let logins: Vec<(i64, String)> = {
            let mut st = db.prepare(
                "SELECT id, connector FROM bank_connections WHERE deleted_ms IS NULL ORDER BY id",
            )?;
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?
        };
        let mut out = Vec::new();
        for (id, connector) in logins {
            if !uses_enable_banking(&db, &connector)? {
                continue;
            }
            let synced = Vault::setting_inner(&db, &synced_key(id))?.map(|v| {
                if v == SYNCED_DEFAULT {
                    String::new()
                } else {
                    v
                }
            });
            out.push(EnableBankingLoginApp {
                connection_id: id,
                sync_key: sync_key(&db, id)?,
                application_id: own_pair(&db, id)?.map(|(app, _)| app).unwrap_or_default(),
                synced_application_id: synced,
            });
        }
        Ok(out)
    }

    /// Remembers that the login's assignment `application_id` (empty = the
    /// default application) is now the shared one.
    pub fn mark_enable_banking_login_synced(
        &self,
        connection_id: i64,
        application_id: String,
    ) -> Result<(), CoreError> {
        let db = self.db()?;
        connector_of(&db, connection_id)?;
        let id = application_id.trim();
        let value = if id.is_empty() { SYNCED_DEFAULT } else { id };
        Vault::set_setting_inner(&db, &synced_key(connection_id), value)
    }
}

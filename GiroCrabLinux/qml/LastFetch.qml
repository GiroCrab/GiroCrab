pragma Singleton
import QtQuick
import QtCore

// „Abgerufen vor …“ under the balance of a single account (#815): ViewLayout.swift › lastFetchKey
// („Letzten Abruf anzeigen“ in the Darstellung menu and the settings, off by default) and
// AppModel+Loading.swift › lastFetchText(for:). Shown by StreamHeaderCard.qml.
QtObject {
    id: lf

    property Settings store: Settings {
        /// `@AppStorage(ViewLayout.lastFetchKey)`
        property bool showLastFetch: false
    }
    readonly property bool shown: store.showLastFetch
    function setShown(on) { store.showLastFetch = on }

    /// AppModel.isOffline: accounts that take manual bookings (#679) are never fetched. Re-read with
    /// the accounts (a small indexed query).
    readonly property var offlineIDs: {
        if (!AppModel.isUnlocked || !AppModel.groups.length) return []
        try { return AppModel.core("list_offline_account_ids") } catch (e) { return [] }
    }

    /// lastFetchText(for:): { text, help } of the account's bank connection, null for offline accounts.
    /// The core stores the time per connection, not per account, so accounts of one bank show the same value.
    function text(account) {
        if (!account || offlineIDs.indexOf(account.id) >= 0) return null
        const c = account.connection_id !== null ? AppModel.connections.find(c => c.id === account.connection_id) : null
        if (!c || c.last_sync_unix === null) return { text: qsTr("Noch nie abgerufen"), help: "" }
        const d = new Date(c.last_sync_unix * 1000)
        // Date.formatted(date: .long, time: .shortened)
        const exact = d.toLocaleDateString(Fmt.locale, Fmt.pattern("d. MMMM yyyy")) + ", " + d.toLocaleTimeString(Fmt.locale, "HH:mm")
        return { text: qsTr("Abgerufen %1").arg(Fmt.relative(c.last_sync_unix)), help: exact }
    }
}

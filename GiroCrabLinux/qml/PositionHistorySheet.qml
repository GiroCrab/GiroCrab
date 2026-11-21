import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// PositionHistorySheet.swift (#578): the daily value of one position, from the fetches and an
// import. It is the value of the position (quantity × price), so a purchase or a sale moves it:
// it is no price chart. `request` (PositionHistoryRequest): { accountId, key, name, currency };
// the host clears it on close.
Sheet {
    id: sheet
    property var request: null
    signal dismissed()
    /// Load: "loading" | "failed" | "loaded"
    property string loadState: "loading"
    property string failure: ""
    property var points: []
    property int generation: 0

    visible: request !== null
    sheetWidth: 560
    padding: 20
    onRequestChanged: if (request) reload()
    onClosed: dismissed()

    function reload() {
        const r = request
        if (!r) return
        const gen = ++generation
        loadState = "loading"
        AppModel.coreAsync("position_value_history", { account_id: r.accountId, position: r.key }, pts => {
            if (gen !== generation) return
            points = pts; loadState = "loaded"
        }, err => {
            if (gen !== generation) return
            failure = err && err.message ? AppModel.parseCoreError(err.message).message : String(err)
            loadState = "failed"
        })
    }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 14

        Text { Layout.fillWidth: true; text: sheet.request ? sheet.request.name : ""; font.pixelSize: Theme.body; font.weight: Font.DemiBold
               color: Theme.text; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
               Sensitive { labels: true } }

        BusyIndicator { visible: sheet.loadState === "loading"; running: visible; Layout.alignment: Qt.AlignHCenter
                        Layout.preferredHeight: 200; implicitWidth: 20; padding: 0 }
        ColumnLayout {
            visible: sheet.loadState === "failed"
            Layout.fillWidth: true
            spacing: 6
            RowLayout {
                spacing: 4
                SFIcon { symbol: "exclamationmark.triangle"; size: Theme.caption; color: Theme.warning }
                Text { text: qsTr("Wertverlauf konnte nicht geladen werden"); font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.warning }
            }
            Text { Layout.fillWidth: true; text: sheet.failure; font.pixelSize: Theme.caption2; color: Theme.tertiary
                   wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight }
            IconButton { text: qsTr("Erneut versuchen"); font.pixelSize: Theme.caption; onClicked: sheet.reload() }
        }
        Text { visible: sheet.loadState === "loaded" && sheet.points.length < 2
               text: qsTr("Noch kein Verlauf für diese Position"); font.pixelSize: Theme.callout; color: Theme.secondary }
        DepotValueChart {
            visible: sheet.loadState === "loaded" && sheet.points.length >= 2
            Layout.fillWidth: true; Layout.preferredHeight: 200
            points: visible ? sheet.points : []
            currency: sheet.request ? sheet.request.currency || "EUR" : "EUR"
            Sensitive { amounts: true }
        }
        // summary(_:): LabeledContent rows
        GridLayout {
            readonly property var first: sheet.points.length ? sheet.points[0] : null
            readonly property var last: sheet.points.length ? sheet.points[sheet.points.length - 1] : null
            id: summary
            visible: sheet.loadState === "loaded" && sheet.points.length >= 2
            Layout.fillWidth: true
            columns: 2; columnSpacing: 12; rowSpacing: 4
            Text { text: qsTr("Verlauf ab"); font.pixelSize: Theme.callout; color: Theme.text; Layout.fillWidth: true }
            Text { text: summary.first ? Fmt.medium(summary.first.date) : ""; font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.secondary }
            Text { text: summary.last && summary.last.value_cents === 0 ? qsTr("Nicht mehr im Depot seit") : qsTr("Letzter Wert")
                   font.pixelSize: Theme.callout; color: Theme.text; Layout.fillWidth: true }
            Text {
                readonly property bool sold: !!summary.last && summary.last.value_cents === 0
                text: !summary.last ? "" : sold ? Fmt.medium(summary.last.date) : Fmt.money(summary.last.value_cents, sheet.request ? sheet.request.currency || "EUR" : "EUR")
                font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.secondary
                Sensitive { amounts: true; when: !parent.sold }
            }
        }
        Text { Layout.fillWidth: true; wrapMode: Text.WordWrap
               text: qsTr("Wert der Position (Menge × Kurs). Käufe und Verkäufe verändern ihn – er ist kein Kursverlauf.")
               font.pixelSize: Theme.caption; color: Theme.secondary }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            Button { text: qsTr("Fertig"); highlighted: true; onClicked: sheet.close()
                     Keys.onReturnPressed: sheet.close() }
        }
    }
}

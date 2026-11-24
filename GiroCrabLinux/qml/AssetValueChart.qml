import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AssetAccountFrame.swift › AssetValueChart: the value chart of a holdings view. It is handed
// finished data and runs no vault query. Missing valuations are never filled in: a single value
// is a sentence, not a line. `content`: { kind: "loading" } | { kind: "unavailable", text } |
// { kind: "points", points }.
ColumnLayout {
    id: vc
    property var content: ({ kind: "loading" })
    property string currency: "EUR"
    /// Where the notes sit: under the value (right) in the wide header, across (left) otherwise.
    property int alignment: Qt.AlignLeft
    readonly property var points: content.kind === "points" ? content.points : []
    spacing: 4

    BusyIndicator { visible: vc.content.kind === "loading"; running: visible; Layout.alignment: Qt.AlignHCenter
                    implicitWidth: 18; implicitHeight: 18; padding: 0 }
    Text {
        visible: vc.content.kind === "unavailable" || (vc.content.kind === "points" && vc.points.length === 0)
        Layout.fillWidth: true
        text: vc.content.kind === "unavailable" ? vc.content.text : qsTr("Keine Bewertungen im gewählten Zeitraum")
        font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap
        horizontalAlignment: vc.alignment === Qt.AlignRight ? Text.AlignRight : Text.AlignLeft
    }
    Text {
        visible: vc.content.kind === "points" && vc.points.length === 1
        Layout.fillWidth: true
        text: vc.points.length === 1 ? qsTr("Nur eine Bewertung: %1 · %2").arg(Fmt.medium(vc.points[0].date)).arg(Fmt.money(vc.points[0].balance_cents, vc.currency)) : ""
        font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap
        horizontalAlignment: vc.alignment === Qt.AlignRight ? Text.AlignRight : Text.AlignLeft
        Sensitive { amounts: true }
    }
    // room for the min/max/0 axis labels (GitHub #203); the whole chart is protected
    BalanceChart {
        visible: vc.content.kind === "points" && vc.points.length > 1
        Layout.fillWidth: true; Layout.preferredHeight: 72
        points: vc.points
        Sensitive {}
    }
}

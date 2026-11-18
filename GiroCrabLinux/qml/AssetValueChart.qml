import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AssetAccountFrame.swift › AssetValueChart: the value chart of a holdings view. It is handed
// finished data and runs no vault query. Missing valuations are never filled in: a single value
// is a sentence, not a line. `content`: { kind: "loading" } | { kind: "unavailable", text } |
// { kind: "failed", text, retry } | { kind: "depot", points, from, to } (DepotValueHistory.chartContent:
// the value history as a step line in the depots' currency, with the coverage of the sum; `from` /
// `to` are the shown period, ISO or null, and the axis spans it).
ColumnLayout {
    id: vc
    property var content: ({ kind: "loading" })
    property string currency: "EUR"
    /// Small print under the chart (e.g. that a change in value is no return).
    property string footnote: ""
    /// What a sum leaves out (depots in another currency), shown like a partial sum.
    property string excludedNote: ""
    readonly property var points: content.kind === "depot" ? content.points : []
    readonly property bool drawsChart: content.kind === "depot" && points.length > 1
    spacing: 4

    BusyIndicator { visible: vc.content.kind === "loading"; running: visible; Layout.alignment: Qt.AlignHCenter
                    implicitWidth: 18; implicitHeight: 18; padding: 0 }
    Text {
        visible: vc.content.kind === "unavailable" || (vc.content.kind === "depot" && vc.points.length === 0)
        Layout.fillWidth: true
        text: vc.content.kind === "unavailable" ? vc.content.text : qsTr("Keine Bewertungen im gewählten Zeitraum")
        font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap
    }
    // .failed: „Verlauf konnte nicht geladen werden“ with the message and „Erneut versuchen“
    ColumnLayout {
        visible: vc.content.kind === "failed"
        Layout.fillWidth: true
        spacing: 4
        Note { symbol: "exclamationmark.triangle"; text: qsTr("Verlauf konnte nicht geladen werden"); weight: Font.DemiBold; tint: Theme.warning
               size: Theme.caption }
        Text { Layout.fillWidth: true; text: vc.content.kind === "failed" ? vc.content.text : ""; font.pixelSize: Theme.caption2; color: Theme.tertiary
               wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight }
        IconButton { visible: vc.content.kind === "failed" && !!vc.content.retry; text: qsTr("Erneut versuchen"); font.pixelSize: Theme.caption
                     onClicked: vc.content.retry() }
    }
    Text {
        visible: vc.content.kind === "depot" && vc.points.length === 1
        Layout.fillWidth: true
        text: vc.points.length === 1 ? qsTr("Nur eine Bewertung: %1 · %2").arg(Fmt.medium(vc.points[0].date)).arg(Fmt.money(vc.points[0].value_cents, vc.currency)) : ""
        font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap
        Sensitive { amounts: true }
    }
    // the whole chart is protected: the line, the axes and the hover readout alike
    DepotValueChart {
        visible: vc.drawsChart
        Layout.fillWidth: true; Layout.preferredHeight: 84
        points: vc.drawsChart ? vc.points : []
        currency: vc.currency
        period: vc.content.kind === "depot" && vc.content.from
                ? [Fmt.date(vc.content.from), vc.content.to ? Fmt.date(vc.content.to) : new Date()] : null
        Sensitive { amounts: true }
    }
    readonly property string startNote: drawsChart ? DepotValueHistory.startNote(points, content.from) : ""
    readonly property string coverageNote: drawsChart ? DepotValueHistory.coverageNote(points) : ""
    Text { visible: vc.startNote !== ""; text: vc.startNote; font.pixelSize: Theme.caption2; color: Theme.secondary }
    Note { visible: vc.coverageNote !== ""; symbol: "chart.line.uptrend.xyaxis"; text: vc.coverageNote }
    Note { visible: vc.drawsChart && vc.excludedNote !== ""; symbol: "dollarsign.circle"; text: vc.excludedNote }
    Text { visible: vc.drawsChart && vc.footnote !== ""; Layout.fillWidth: true; text: vc.footnote
           font.pixelSize: Theme.caption2; color: Theme.tertiary; wrapMode: Text.WordWrap }

    /// Label(…, systemImage:) in the warning colour, caption2 semibold.
    component Note: RowLayout {
        id: note
        property string symbol
        property string text
        property color tint: Theme.warning
        property int weight: Font.DemiBold
        property int size: Theme.caption2
        Layout.fillWidth: true
        spacing: 4
        SFIcon { symbol: note.symbol; size: note.size; color: note.tint; Layout.alignment: Qt.AlignVCenter }
        Text { Layout.fillWidth: true; text: note.text; font.pixelSize: note.size; font.weight: note.weight; color: note.tint; wrapMode: Text.WordWrap }
    }
}

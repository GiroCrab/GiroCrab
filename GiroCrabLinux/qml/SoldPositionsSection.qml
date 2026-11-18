import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore

// PositionHistorySheet.swift › SoldPositionsSection (#578): positions of a depot that are no
// longer held but have a value history, collapsed below the positions table; each opens its
// history. Open or closed is remembered (`@AppStorage("depotSoldPositionsExpanded")`, QSettings `[holdings]`).
ColumnLayout {
    id: sec
    /// ValuePosition list (`depot_value_positions`, held = false), the latest sale first.
    property var positions: []
    signal open(var position)
    Settings { id: store; category: "holdings"; property bool depotSoldPositionsExpanded: false }
    spacing: 0

    // DisclosureGroup label: „Verkaufte Positionen (3)“
    Item {
        Layout.fillWidth: true; Layout.leftMargin: 4; Layout.rightMargin: 4
        implicitHeight: head.implicitHeight + 8
        RowLayout {
            id: head
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6
            SFIcon { symbol: "chevron.right"; size: 10; color: Theme.secondary; rotation: store.depotSoldPositionsExpanded ? 90 : 0
                     Behavior on rotation { NumberAnimation { duration: 200 } } }
            Text { text: qsTr("Verkaufte Positionen (%1)").arg(sec.positions.length); font.pixelSize: Theme.callout; font.weight: Font.DemiBold; color: Theme.text }
        }
        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor
                    onClicked: store.depotSoldPositionsExpanded = !store.depotSoldPositionsExpanded }
        Accessible.role: Accessible.Button
        Accessible.name: qsTr("Verkaufte Positionen (%1)").arg(sec.positions.length)
    }
    ColumnLayout {
        visible: store.depotSoldPositionsExpanded
        Layout.fillWidth: true; Layout.leftMargin: 4; Layout.rightMargin: 4; Layout.topMargin: 6
        spacing: 0
        Repeater {
            model: sec.positions
            ColumnLayout {
                id: row
                required property var modelData
                required property int index
                Layout.fillWidth: true
                spacing: 0
                Item {
                    Layout.fillWidth: true
                    implicitHeight: line.implicitHeight + 12
                    RowLayout {
                        id: line
                        anchors.left: parent.left; anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
                        spacing: 10
                        Text { Layout.fillWidth: true; text: row.modelData.name !== "" ? row.modelData.name : row.modelData.position
                               elide: Text.ElideRight; font.pixelSize: Theme.callout; color: Theme.text
                               Sensitive { labels: true } }
                        // "verkauft 12.03.2025" after a closing 0, else "zuletzt 12.03.2025" (no longer reported)
                        Text { text: row.modelData.last_value_cents === 0 ? qsTr("verkauft %1").arg(Fmt.medium(row.modelData.last_date))
                                                                          : qsTr("zuletzt %1").arg(Fmt.medium(row.modelData.last_date))
                               font.pixelSize: Theme.callout; font.features: { "tnum": 1 }; color: Theme.secondary }
                        SFIcon { symbol: "chart.xyaxis.line"; size: 12; color: Theme.secondary }
                    }
                    MouseArea { id: rowArea; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                                onClicked: sec.open(row.modelData) }
                    ToolTip.visible: rowArea.containsMouse; ToolTip.delay: 600; ToolTip.text: qsTr("Wertverlauf anzeigen")
                    Accessible.role: Accessible.Button
                    Accessible.name: qsTr("Wertverlauf anzeigen")
                }
                Rectangle { visible: row.index < sec.positions.length - 1; Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }
            }
        }
    }
}

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtCore

// AssetCompositionCard.swift › AssetSectionCard (#564): a collapsible analysis card of a holdings
// view: a title with a chevron and a body that folds away, with its own remembered state
// (`@AppStorage(expandedKey)`, QSettings `[holdings]`).
Rectangle {
    id: sc
    property string title
    property string expandedKey
    property bool defaultExpanded: true
    property bool expanded: true
    default property alias content: body.data
    Settings { id: store; category: "holdings" }
    function load() { const v = store.value(expandedKey, defaultExpanded); expanded = v === true || v === "true" }
    Component.onCompleted: load()
    onExpandedKeyChanged: load()
    function toggle() { expanded = !expanded; store.setValue(expandedKey, expanded) }

    implicitHeight: col.implicitHeight + 24
    radius: 12
    color: Theme.card
    border.color: Theme.border
    ColumnLayout {
        id: col
        x: 16; y: 12
        width: parent.width - 32
        spacing: 10
        Item {
            Layout.fillWidth: true
            implicitHeight: head.implicitHeight
            RowLayout {
                id: head
                width: parent.width
                spacing: 8
                SFIcon { symbol: "chevron.right"; size: 10; color: Theme.secondary; rotation: sc.expanded ? 90 : 0
                         Behavior on rotation { NumberAnimation { duration: 200 } } }
                Text { text: sc.title; font.pixelSize: Theme.subheadline + 1; font.weight: Font.DemiBold; color: Theme.secondary; Layout.fillWidth: true }
            }
            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: sc.toggle() }
            Accessible.role: Accessible.Button
            Accessible.name: sc.title
            Accessible.description: sc.expanded ? qsTr("Ausgeklappt") : qsTr("Eingeklappt")
        }
        ColumnLayout { id: body; visible: sc.expanded; Layout.fillWidth: true; spacing: 10 }
    }
}

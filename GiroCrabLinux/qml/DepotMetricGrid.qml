import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// DepotKPITiles.swift › DepotMetricGrid / DepotMetricTile (#564): a row of figure tiles – one row
// when wide, rows of two otherwise, one per row where even two do not fit. The tiles of a row share
// their height. A missing figure says so; it is never shown as 0. Tiles come from
// DepotMetrics.securitiesTiles.
GridLayout {
    id: grid
    property var tiles: []
    property bool wide: true
    readonly property int perRow: wide ? Math.max(tiles.length, 1) : (width >= 2 * 170 * Theme.textScale + 10 ? 2 : 1)
    columns: perRow
    columnSpacing: 10; rowSpacing: 10
    Repeater {
        model: grid.tiles
        Rectangle {
            id: tile
            required property var modelData
            Layout.fillWidth: true; Layout.fillHeight: true
            Layout.preferredWidth: 1
            implicitHeight: col.implicitHeight + 12
            radius: 9
            color: Theme.card
            border.color: Theme.border
            readonly property bool hasValue: modelData.value !== null
            ColumnLayout {
                id: col
                x: 10; y: 6
                width: parent.width - 20
                spacing: 2
                Text { text: tile.modelData.title; font.pixelSize: Theme.caption; font.weight: Font.Medium; color: Theme.secondary
                       Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: 2 }
                Text {
                    text: tile.hasValue ? tile.modelData.value : qsTr("Nicht verfügbar")
                    font.pixelSize: Theme.title3 + 2; font.weight: Font.DemiBold; font.features: { "tnum": 1 }
                    color: tile.hasValue ? tile.modelData.tint : Theme.tertiary
                    Layout.fillWidth: true; elide: Text.ElideRight
                    fontSizeMode: Text.HorizontalFit; minimumPixelSize: Math.round((Theme.title3 + 2) * 0.75)
                    Sensitive { amounts: true; when: tile.hasValue }
                }
                Text {
                    visible: tile.hasValue && tile.modelData.detail !== null
                    text: tile.modelData.detail !== null ? tile.modelData.detail : ""
                    font.pixelSize: Theme.caption; font.features: { "tnum": 1 }
                    color: Qt.colorEqual(tile.modelData.tint, Theme.text) ? Theme.secondary : tile.modelData.tint
                    Layout.fillWidth: true; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
                    Sensitive { amounts: true }
                }
                Text {
                    text: tile.hasValue ? tile.modelData.footnote : tile.modelData.missing
                    font.pixelSize: Theme.caption2; color: Theme.tertiary
                    Layout.fillWidth: true; Layout.topMargin: 2; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight
                }
            }
            HoverHandler { id: tileHover }
            ToolTip.visible: tileHover.hovered; ToolTip.delay: 600; ToolTip.text: modelData.help
            Accessible.role: Accessible.StaticText
            Accessible.name: modelData.title
        }
    }
}

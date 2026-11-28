import QtQuick
import QtQuick.Controls

// RecipientEntry.swift › RecipientSuggestionList (#881, #978, #1475): known recipients as a
// drop-down under the name field of the transfer and the standing order form, one row each;
// pick with the mouse or ↑/↓ and Return, Esc closes it. It floats over the form below the
// field (parent: the sheet's contentItem), so the sheet keeps its size while typing.
Rectangle {
    id: list
    /// The recipient name field the list hangs under.
    property Item field
    /// The ScrollView around the form: the list follows it when it scrolls.
    property ScrollView scrollView
    property var suggestions: []
    property int highlighted: -1
    /// false while the form is not shown (review).
    property bool active: true
    signal picked(var recipient)
    /// "Nicht mehr vorschlagen" (#978).
    signal hideRequested(string iban)
    signal hoveredRow(int index)

    /// Known recipients for what was typed: only once something is typed, one row per IBAN
    /// (the most used spelling), at most five.
    function recipients(query) {
        if ((query || "").trim() === "") return []
        const seen = {}
        return AppModel.suggestRecipients(query).filter(r => { if (seen[r.iban]) return false; seen[r.iban] = true; return true }).slice(0, 5)
    }

    z: 6
    visible: active && suggestions.length > 0 && !!field && field.visible
    readonly property point at: {
        if (!field || !parent) return Qt.point(0, 0)
        if (scrollView) { void scrollView.contentItem.contentY; void scrollView.height }
        void field.y; void parent.width; void field.visible
        return field.mapToItem(parent, 0, field.height + 4)
    }
    x: at.x; y: at.y; width: field ? field.width : 0
    height: suggestionColumn.implicitHeight + 8
    radius: 8; color: Theme.card; border.color: Theme.border
    Column {
        id: suggestionColumn
        x: 4; y: 4; width: parent.width - 8
        Repeater {
            model: list.suggestions
            Rectangle {
                id: suggestionRow
                required property var modelData
                required property int index
                width: suggestionColumn.width; height: rowText.implicitHeight + 8
                radius: 6
                color: index === list.highlighted ? Theme.selection : "transparent"
                // one row per IBAN with its last use; ✕: "Nicht mehr vorschlagen" (#978). No
                // context menu: a menu takes the focus from the name field, which clears the list.
                Column {
                    id: rowText
                    x: 8; y: 4; width: parent.width - 16 - hideButton.width
                    spacing: 1
                    Text { width: parent.width; text: suggestionRow.modelData.name; font.pixelSize: Theme.body; color: Theme.text; elide: Text.ElideRight; Sensitive {} }
                    RecipientIBANLine { width: parent.width; maxWidth: parent.width; iban: suggestionRow.modelData.iban }
                    Text { width: parent.width; text: qsTr("zuletzt %1").arg(Fmt.medium(suggestionRow.modelData.last_used)); font.pixelSize: Theme.caption2; color: Theme.tertiary }
                }
                HoverHandler { onHoveredChanged: if (hovered) list.hoveredRow(suggestionRow.index) }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: list.picked(suggestionRow.modelData) }
                IconButton {
                    id: hideButton
                    anchors.right: parent.right; anchors.rightMargin: 4; anchors.verticalCenter: parent.verticalCenter
                    // a click must leave the focus in the name field (it keeps the list)
                    focusPolicy: Qt.NoFocus
                    symbol: "xmark.circle.fill"; symbolSize: 13; implicitWidth: 22; implicitHeight: 22
                    ToolTip.text: qsTr("Nicht mehr vorschlagen")
                    Accessible.name: qsTr("Nicht mehr vorschlagen")
                    onClicked: list.hideRequested(suggestionRow.modelData.iban)
                }
            }
        }
    }
}

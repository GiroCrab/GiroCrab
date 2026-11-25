import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// BalanceHeaderView.swift › AmountFilterForm (#357): popover of the amount filter, from / to as
// absolute amounts, empty = open. Opened by the amount pill of the stream header's filter row
// (StreamHeaderCard.qml), state in ListFilters.qml.
Popup {
    id: pop
    padding: 16
    margins: 8
    onImplicitHeightChanged: if (visible) place()
    onOpened: { place(); minField.focusField() }
    function place() {
        if (!parent) return
        const top = parent.mapToItem(null, 0, 0).y
        const room = parent.Window.height
        y = top + parent.height + 6 + implicitHeight <= room - 8 || top - implicitHeight - 6 < 8 ? parent.height + 6 : -implicitHeight - 6
    }
    // .frame(minWidth: 300)
    width: Math.max(300 * Theme.textScale, implicitContentWidth + leftPadding + rightPadding)
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
    background: Rectangle { radius: 9; color: Theme.card; border.color: Theme.border }
    // .onAppear: the fields start from the current range
    onAboutToShow: {
        minField.text = ListFilters.amountMin === null ? "" : Fmt.plain(ListFilters.amountMin)
        maxField.text = ListFilters.amountMax === null ? "" : Fmt.plain(ListFilters.amountMax)
    }

    /// AmountFilterForm.cents: empty = no limit (null); undefined = not a valid, non-negative amount.
    function cents(text) {
        const t = text.trim()
        if (t === "") return null
        const c = Fmt.parseCents(t)
        return c === null || c < 0 ? undefined : c
    }
    readonly property var parsedMin: cents(minField.text)
    readonly property var parsedMax: cents(maxField.text)
    readonly property bool valid: parsedMin !== undefined && parsedMax !== undefined
    function apply() {
        if (!valid) return
        ListFilters.setAmount(parsedMin, parsedMax)
        close()
    }

    contentItem: ColumnLayout {
        spacing: 12
        GridLayout {
            columns: 2
            columnSpacing: 12
            rowSpacing: 8
            Text { text: qsTr("Von"); font.pixelSize: Theme.body; color: Theme.text }
            AmountField { id: minField; placeholder: qsTr("Kein Minimum") }
            Text { text: qsTr("Bis"); font.pixelSize: Theme.body; color: Theme.text }
            AmountField { id: maxField; placeholder: qsTr("Kein Maximum") }
        }
        Text {
            Layout.fillWidth: true; wrapMode: Text.WordWrap
            text: qsTr("Betrag ohne Vorzeichen.")
            font.pixelSize: Theme.caption; color: Theme.secondary
        }
        RowLayout {
            Layout.fillWidth: true
            Button {
                text: qsTr("Zurücksetzen")
                enabled: ListFilters.amountActive || minField.text !== "" || maxField.text !== ""
                onClicked: { minField.text = ""; maxField.text = ""; ListFilters.clearAmount(); pop.close() }
            }
            Item { Layout.fillWidth: true }
            Button { text: qsTr("Abbrechen"); onClicked: pop.close() }
            Button { text: qsTr("Übernehmen"); highlighted: true; enabled: pop.valid; onClicked: pop.apply() }
        }
    }

    /// TextField (125 pt, right-aligned, monospaced digits) with „€“ after it; Return applies.
    component AmountField: RowLayout {
        property alias text: field.text
        property alias placeholder: field.placeholderText
        function focusField() { field.forceActiveFocus() }
        spacing: 4
        TextField {
            id: field
            Layout.preferredWidth: 125 * Theme.textScale
            horizontalAlignment: TextInput.AlignRight
            font.pixelSize: Theme.body; font.features: { "tnum": 1 }
            onAccepted: pop.apply()
        }
        Text { text: "€"; font.pixelSize: Theme.body; color: Theme.text }
    }
}

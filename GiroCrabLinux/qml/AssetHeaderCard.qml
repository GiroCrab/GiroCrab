import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AssetAccountFrame.swift › AssetHeaderCard (#564): header card of a holdings view: title, current
// value, a value chart and the footer row with the period menu, documents and kind-specific actions.
// Wide: name, details and actions on the left, the value with its chart on the right (about 42 %).
// Narrow: name and value first, the chart across the full width below. Minimised: no chart.
// The cash views' header (StreamHeaderCard) stays separate.
Card {
    id: hc
    padding: 16
    property var account
    property string title
    property real valueCents: 0
    property string valueLabel
    /// A securities depot has no pay cycle: its period menu hides "since last salary".
    property bool periodForDepot: false
    property string valueHelp: ""
    property string currency: account && account.currency ? account.currency : "EUR"
    /// Lines under the title (bank, status, count).
    property alias subtitle: subtitleColumn.data
    /// The note under the value (difference to the positions), in the warning colour.
    property string valueNote: ""
    property string valueNoteHelp: ""
    /// The chart content for AssetValueChart.
    property var chartContent: ({ kind: "loading" })
    /// Kind-specific actions at the end of the footer.
    property alias actions: actionsRow.data

    readonly property bool compact: AppModel.compactHeader
    readonly property bool wide: width >= 640 * Theme.textScale && !compact
    readonly property string compactTitle: compact ? qsTr("Kopfbereich einblenden") : qsTr("Kopfbereich minimieren")
    implicitHeight: grid.implicitHeight + 2 * padding

    function bankDetails(bic) {
        const a = account
        const bank = bic ? (a.bank_name ? "BIC: " + bic + " (" + a.bank_name + ")" : "BIC: " + bic) : (a.bank_name ? "Bank: " + a.bank_name : "")
        return [a.owner ? "Name: " + a.owner : "", "IBAN: " + Fmt.iban(a.iban), bank].filter(s => s).join("\n")
    }

    GridLayout {
        id: grid
        width: parent.width
        columns: hc.wide ? 2 : 1
        columnSpacing: 20; rowSpacing: 9

        // left column (wide) / first rows (narrow)
        ColumnLayout {
            id: leftColumn
            // the implicit width of long lines never pushes the value column out of the card
            Layout.fillWidth: true; Layout.preferredWidth: 1; Layout.minimumWidth: 0; Layout.alignment: Qt.AlignTop
            spacing: 8
            RowLayout {
                Layout.fillWidth: true
                spacing: 8
                ColumnLayout {
                    spacing: 3
                    Layout.fillWidth: true
                    Layout.maximumWidth: implicitWidth   // the chevron sits right after the title
                    Layout.alignment: Qt.AlignTop
                    Text { text: hc.title; font.pixelSize: hc.compact ? Theme.body + 2 : Theme.title2 + 3; font.weight: Font.DemiBold; color: Theme.text
                           elide: Text.ElideRight; Layout.fillWidth: true
                           Sensitive {} }
                    ColumnLayout { id: subtitleColumn; visible: !hc.compact; spacing: 3; Layout.fillWidth: true }
                    TapHandler {
                        acceptedButtons: Qt.RightButton
                        enabled: hc.account !== null && hc.account !== undefined
                        onTapped: { copyMenu.bic = AppModel.run(() => AppModel.core("bic_for_iban", { iban: hc.account.iban })) || ""; copyMenu.popup() }
                    }
                    // AccountCopyMenu
                    AppMenu {
                        id: copyMenu
                        property string bic: ""
                        AppMenuSeparator { contentItem: Text { text: qsTr("In Zwischenablage kopieren"); font.pixelSize: Theme.caption; color: Theme.secondary; leftPadding: 8 } }
                        AppMenuItem { text: qsTr("Bankverbindung"); visible: !!hc.account && hc.account.iban !== ""; height: visible ? implicitHeight : 0
                                      onTriggered: AppModel.copyToClipboard(hc.bankDetails(copyMenu.bic)) }
                        AppMenuItem { text: qsTr("IBAN"); visible: !!hc.account && hc.account.iban !== ""; height: visible ? implicitHeight : 0
                                      onTriggered: AppModel.copyToClipboard(hc.account.iban) }
                        AppMenuItem { text: qsTr("BIC"); visible: copyMenu.bic !== ""; height: visible ? implicitHeight : 0
                                      onTriggered: AppModel.copyToClipboard(copyMenu.bic) }
                        AppMenuItem { text: qsTr("Kontostand"); onTriggered: AppModel.copyToClipboard(Fmt.clipboard(hc.account.balance_cents)) }
                    }
                }
                // Minimise the header: more room for the content (GitHub #358); the chart goes away with it.
                Item {
                    Layout.alignment: Qt.AlignTop
                    activeFocusOnTab: true
                    Keys.onSpacePressed: AppModel.compactHeader = !AppModel.compactHeader
                    Keys.onReturnPressed: AppModel.compactHeader = !AppModel.compactHeader
                    Keys.onEnterPressed: AppModel.compactHeader = !AppModel.compactHeader
                    Accessible.onPressAction: AppModel.compactHeader = !AppModel.compactHeader
                    implicitWidth: compactIcon.width + 8; implicitHeight: compactIcon.height + 8
                    SFIcon { id: compactIcon; anchors.centerIn: parent; symbol: "chevron.down"; size: 10; color: Theme.secondary
                             rotation: hc.compact ? 0 : 180 }
                    MouseArea { id: compactArea; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                                onClicked: AppModel.compactHeader = !AppModel.compactHeader }
                    ToolTip.visible: compactArea.containsMouse; ToolTip.delay: 600; ToolTip.text: hc.compactTitle
                    Accessible.role: Accessible.Button; Accessible.name: hc.compactTitle
                }
                Item { Layout.fillWidth: true; visible: !hc.wide }
                ValueBlock { visible: !hc.wide; Layout.alignment: Qt.AlignTop }
            }
            Item { visible: hc.wide; Layout.fillHeight: true }
            AssetValueChart { visible: !hc.wide && !hc.compact; Layout.fillWidth: true; content: hc.chartContent; currency: hc.currency }
            // Period and documents stay at hand; the kind-specific actions follow, on a second line
            // if the card is too narrow for one (ViewThatFits).
            GridLayout {
                Layout.fillWidth: true
                columns: leftColumn.width >= controlsRow.implicitWidth + actionsRow.implicitWidth + 32 ? 3 : 1
                columnSpacing: 12; rowSpacing: 6
                RowLayout {
                    id: controlsRow
                    spacing: 12
                    PeriodMenuButton { forDepot: hc.periodForDepot }
                    IconButton { text: qsTr("Dokumente"); symbol: "doc.text"; symbolSize: 12; font.pixelSize: Theme.caption
                                 onClicked: AppModel.openAccountDocuments(hc.account) }
                }
                Item { visible: parent.columns === 3; Layout.fillWidth: true }
                RowLayout { id: actionsRow; spacing: 12 }
            }
        }
        // right column (wide): the value with its chart below
        ColumnLayout {
            visible: hc.wide
            Layout.alignment: Qt.AlignTop
            Layout.preferredWidth: Math.max(280 * Theme.textScale, hc.width * 0.42)
            Layout.minimumWidth: Layout.preferredWidth
            Layout.maximumWidth: Layout.preferredWidth
            spacing: 8
            ValueBlock { Layout.alignment: Qt.AlignRight }
            AssetValueChart { Layout.fillWidth: true; content: hc.chartContent; currency: hc.currency; alignment: Qt.AlignRight }
        }
    }
    /// The current value: hovering the chart never replaces this number.
    component ValueBlock: ColumnLayout {
        spacing: 2
        Text {
            Layout.alignment: Qt.AlignRight
            text: Fmt.money(hc.valueCents, hc.currency)
            font.pixelSize: hc.compact ? Theme.title3 + 3 : Theme.largeTitle + 4; font.weight: Font.DemiBold; font.features: { "tnum": 1 }
            color: Theme.text
            Sensitive { amounts: true }
            HoverHandler { id: valueHover }
            ToolTip.visible: valueHover.hovered && hc.valueHelp !== ""; ToolTip.delay: 600; ToolTip.text: hc.valueHelp
            TapHandler { acceptedButtons: Qt.RightButton; onTapped: valueMenu.popup() }
            AppMenu { id: valueMenu
                AppMenuItem { text: qsTr("%1 kopieren").arg(hc.valueLabel); icon.name: SF.icon("doc.on.doc")
                              onTriggered: AppModel.copyToClipboard(Fmt.clipboard(hc.valueCents)) } }
        }
        Text { visible: !hc.compact; Layout.alignment: Qt.AlignRight; text: hc.valueLabel; font.pixelSize: Theme.caption; color: Theme.secondary }
        Text {
            visible: !hc.compact && hc.valueNote !== ""
            Layout.alignment: Qt.AlignRight
            Layout.maximumWidth: Math.max(200, hc.width * 0.45)
            text: hc.valueNote
            color: Theme.warning
            font.pixelSize: Theme.caption2; font.features: { "tnum": 1 }
            horizontalAlignment: Text.AlignRight; wrapMode: Text.WordWrap
            Sensitive { amounts: true }
            HoverHandler { id: noteHover }
            ToolTip.visible: noteHover.hovered && hc.valueNoteHelp !== ""; ToolTip.delay: 600; ToolTip.text: hc.valueNoteHelp
        }
    }
}

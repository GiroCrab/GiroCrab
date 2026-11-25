import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// ExportSheet.swift › ExportAccountPicker (#831): account choice of the export sheet as a
// popover under the „Konten“ button – searchable, grouped by bank, a bank's checkbox toggles
// all its accounts. `singleChoice` (CAMT.053) picks exactly one account.
Popup {
    id: pop
    /// [{ bank, accounts }]
    property var groups: []
    /// id -> true (the sheet's choice; `picked` hands a new one back)
    property var selectedIDs: ({})
    property bool singleChoice: false
    property var isUsable: a => true
    signal picked(var ids)
    property string search: ""

    // ReportDialogs.swift › capturablePopover (#384): drawn into the bug report picture
    onOpened: { place(); ReportDialogs.track(pop) }
    onClosed: { search = ""; ReportDialogs.untrack(pop) }
    Component.onDestruction: ReportDialogs.untrack(pop)
    // .capturablePopover(arrowEdge: .bottom): below the button, above it when there is no room
    function place() {
        if (!parent) return
        const top = parent.mapToItem(null, 0, 0).y
        const room = parent.Window.height
        y = top + parent.height + 6 + height <= room - 8 || top - height - 6 < 8 ? parent.height + 6 : -height - 6
        x = parent.width - width
    }
    margins: 8
    padding: 0
    width: 340 * Theme.textScale
    height: 380 * Theme.textScale
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
    background: Rectangle { radius: 9; color: Theme.card; border.color: Theme.border }

    readonly property var allIDs: groups.reduce((out, g) => out.concat(g.accounts.map(a => a.id)), [])
    readonly property int selectedCount: Object.keys(selectedIDs).length
    readonly property var shownGroups: {
        const q = search.trim()
        if (q === "") return groups
        const fq = Fmt.fold(q)
        return groups.map(g => ({ bank: g.bank, accounts: Fmt.fold(g.bank).indexOf(fq) >= 0
                                   ? g.accounts : g.accounts.filter(a => AppModel.matchesSidebarSearch(a, q)) }))
                     .filter(g => g.accounts.length > 0)
    }
    function toggle(account) {
        let s
        if (singleChoice) { s = {}; s[account.id] = true }
        else { s = Object.assign({}, selectedIDs); if (s[account.id]) delete s[account.id]; else s[account.id] = true }
        picked(s)
    }
    function subtitle(a) {
        return [a.owner || "", a.iban ? Fmt.iban(a.iban) : ""].filter(x => x !== "").join(" · ")
    }

    contentItem: ColumnLayout {
        spacing: 0
        TextField {
            visible: pop.allIDs.length > 8
            Layout.fillWidth: true; Layout.margins: 10
            placeholderText: qsTr("Konto, Bank oder IBAN")
            text: pop.search
            onTextEdited: pop.search = text
        }
        Rectangle { visible: pop.allIDs.length > 8; Layout.fillWidth: true; height: 1; color: Theme.border }
        ListView {
            id: list
            Layout.fillWidth: true; Layout.fillHeight: true
            clip: true
            ScrollBar.vertical: AppScrollBar {}
            model: pop.shownGroups
            delegate: Column {
                id: group
                required property var modelData
                width: list.width
                readonly property var usableIDs: modelData.accounts.filter(pop.isUsable).map(a => a.id)
                readonly property int chosen: usableIDs.filter(id => !!pop.selectedIDs[id]).length
                readonly property bool allOn: usableIDs.length > 0 && chosen === usableIDs.length
                // bankHeader (only with several banks)
                RowLayout {
                    visible: pop.groups.length > 1
                    x: 10; width: parent.width - 20; height: 28
                    spacing: 8
                    CheckBox {
                        visible: !pop.singleChoice
                        padding: 0
                        tristate: true
                        checkState: group.allOn ? Qt.Checked : group.chosen === 0 ? Qt.Unchecked : Qt.PartiallyChecked
                        nextCheckState: () => checkState
                        Accessible.name: group.allOn ? qsTr("Alle von %1 abwählen").arg(group.modelData.bank) : qsTr("Alle von %1 wählen").arg(group.modelData.bank)
                        onClicked: {
                            const s = Object.assign({}, pop.selectedIDs)
                            for (const id of group.usableIDs) { if (group.allOn) delete s[id]; else s[id] = true }
                            pop.picked(s)
                        }
                    }
                    Text { text: group.modelData.bank; font.pixelSize: Theme.subheadline; font.weight: Font.DemiBold; color: Theme.text
                           elide: Text.ElideRight; Layout.fillWidth: true }
                }
                Repeater {
                    model: group.modelData.accounts
                    Item {
                        id: row
                        required property var modelData
                        readonly property bool on: !!pop.selectedIDs[modelData.id]
                        readonly property bool usable: pop.isUsable(modelData)
                        readonly property string caption: usable ? pop.subtitle(modelData) : ""
                        width: group.width
                        implicitHeight: rowLayout.implicitHeight + 10
                        opacity: usable ? 1 : 0.5
                        RowLayout {
                            id: rowLayout
                            x: 10; width: parent.width - 20; anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            CheckBox {
                                visible: !pop.singleChoice
                                padding: 0; Layout.alignment: Qt.AlignTop
                                checked: row.on; enabled: row.usable
                                onClicked: pop.toggle(row.modelData)
                            }
                            RadioButton {
                                visible: pop.singleChoice
                                padding: 0; Layout.alignment: Qt.AlignTop
                                checked: row.on; enabled: row.usable
                                onClicked: pop.toggle(row.modelData)
                            }
                            ColumnLayout {
                                spacing: 1; Layout.fillWidth: true
                                Text { text: row.modelData.name; color: Theme.text; font.pixelSize: Theme.body
                                       elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                                Text { visible: !row.usable; text: qsTr("Nicht für Darlehen und Depots")
                                       font.pixelSize: Theme.caption; color: Theme.secondary }
                                Text { visible: row.usable && row.caption !== ""; text: row.caption
                                       font.pixelSize: Theme.caption; color: Theme.secondary
                                       elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                            }
                        }
                        MouseArea { anchors.fill: parent; anchors.leftMargin: 34; enabled: row.usable; onClicked: pop.toggle(row.modelData) }
                    }
                }
            }
        }
        Rectangle { visible: !pop.singleChoice; Layout.fillWidth: true; height: 1; color: Theme.border }
        RowLayout {
            visible: !pop.singleChoice
            Layout.fillWidth: true; Layout.margins: 10
            spacing: 8
            Button {
                flat: true; text: qsTr("Alle")
                enabled: pop.selectedCount !== pop.allIDs.length
                onClicked: { const s = {}; for (const id of pop.allIDs) s[id] = true; pop.picked(s) }
            }
            Button { flat: true; text: qsTr("Keine"); enabled: pop.selectedCount > 0; onClicked: pop.picked({}) }
            Item { Layout.fillWidth: true }
            Text { text: qsTr("%1 von %2 gewählt").arg(pop.selectedCount).arg(pop.allIDs.length)
                   font.pixelSize: Theme.caption; color: Theme.secondary }
        }
    }
}

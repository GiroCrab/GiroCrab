import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// BalanceHeaderView.swift › PeriodMenuButton (#564): the period menu of the depot and Sachwerte
// headers and of Vermögen › Wertpapiere. A depot has no pay cycle, so `forDepot` hides
// "since last salary" and names the period it really uses (DepotState.depotPeriodTitle).
AbstractButton {
    id: pm
    property bool forDepot: false
    property int fontSize: Theme.caption
    property color tint: Theme.secondary
    padding: 0
    focusPolicy: Qt.TabFocus
    implicitWidth: r.implicitWidth; implicitHeight: Math.max(20, r.implicitHeight)
    background: Rectangle {
        visible: pm.activeFocus
        anchors.fill: parent; anchors.margins: -3; radius: 5
        color: "transparent"; border.color: Theme.accent; border.width: 1
    }
    contentItem: RowLayout {
        id: r
        spacing: 5
        SFIcon { symbol: "calendar"; size: pm.fontSize + 2; color: pm.tint }
        Text { text: pm.forDepot ? DepotState.depotPeriodTitle : AppModel.periodTitle; font.pixelSize: pm.fontSize; color: pm.tint }
        SFIcon { symbol: "chevron.down"; size: 9; color: pm.tint }
    }
    HoverHandler { cursorShape: Qt.PointingHandCursor }
    onClicked: { menu.rebuild(); menu.popup() }
    // sections as in the usual period menus (#382), rebuilt on every open (they count back from today)
    Component { id: periodEntry
        AppMenuItem { property string periodID; text: AppModel.periodTitleOf(periodID); checkable: true
                      checked: AppModel.period === periodID; onTriggered: AppModel.choosePeriod(periodID) } }
    Component { id: periodDivider; AppMenuSeparator {} }
    AppMenu {
        id: menu
        property var entries: []
        function rebuild() {
            for (const o of entries) removeItem(o)
            const built = []
            for (const section of AppModel.periodMenu()) {
                const ids = section.filter(id => !(pm.forDepot && id === "sinceSalary"))
                for (const id of ids) built.push(periodEntry.createObject(null, { periodID: id }))
                built.push(periodDivider.createObject(null))
            }
            built.forEach((o, i) => insertItem(i, o))
            entries = built
        }
        AppMenuItem {
            text: AppModel.period === "custom" ? qsTr("Benutzerdefiniert: %1 …").arg(AppModel.periodTitle) : qsTr("Benutzerdefiniert …")
            checkable: true; checked: AppModel.period === "custom"
            onTriggered: { checked = Qt.binding(() => AppModel.period === "custom"); AppModel.showCustomPeriod = true }
        }
    }
}

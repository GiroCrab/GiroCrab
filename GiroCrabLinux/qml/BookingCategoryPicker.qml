import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// BookingCategoryMenu.swift › BookingCategoryPicker (#1857): the popover of a row's category
// label. „Ohne Kategorie“, the history's suggestion (#664) for a single uncategorised booking,
// then the sidebar's category order in one scroll list, indented by depth; a category with
// sub-categories is a heading, not a choice. Acts on the multiple selection when the row is
// part of it. One popover for the whole list (BookingStreamView.qml), opened at the label of
// the clicked row. Keyboard: ↑/↓ move between the choices, Return or Space picks, Esc closes.
Popup {
    id: pop
    property var booking: null
    // ReportDialogs.swift › capturablePopover (#384): drawn into the bug report picture
    onOpened: { place(); ReportDialogs.track(pop); list.byKeyboard = false; list.currentIndex = -1; list.forceActiveFocus() }
    onClosed: ReportDialogs.untrack(pop)
    Component.onDestruction: ReportDialogs.untrack(pop)
    padding: 0
    margins: 8
    /// .popover(arrowEdge: .bottom): below the label, above it when there is no room below.
    function place() {
        if (!parent) return
        const top = parent.mapToItem(null, 0, 0).y
        const room = parent.Window.height
        y = top + parent.height + 6 + implicitHeight <= room - 8 || top - implicitHeight - 6 < 8 ? parent.height + 6 : -implicitHeight - 6
        x = 0
    }
    /// Opens at `anchor` (the row's category label) for `b`.
    function show(b, anchor) {
        booking = b
        parent = anchor
        open()
    }
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    background: Rectangle { radius: 9; color: Theme.card; border.color: Theme.border }
    width: 238 * Theme.textScale

    readonly property var targets: BookingDetails.categorizeTargets(booking)
    /// targets.allSatisfy { $0.categoryId == booking.categoryId }: only then is a choice ticked.
    readonly property bool sharedCategory: booking !== null && targets.every(t => t.category_id === booking.category_id)
    readonly property var suggested: targets.length === 1 ? CategorySuggestions.category(CategorySuggestions.forBooking(booking)) : null

    /// BookingCategoryPicker.rows(for:): the tree, flattened depth-first.
    function rows(nodes, depth) {
        let out = []
        for (const n of nodes) {
            out.push({ kind: "category", category: n.category, depth: depth, isGroup: !!n.children })
            if (n.children) out = out.concat(rows(n.children, depth + 1))
        }
        return out
    }
    readonly property var categoryRows: rows(AppModel.categoryTree, 0)
    readonly property var entries: {
        const head = [{ kind: "none" }]
        if (suggested) head.push({ kind: "suggestion", category: suggested }, { kind: "divider" })
        return head.concat(categoryRows)
    }
    /// .frame(height: min(360, 12 + entryCount * 26))
    readonly property int entryCount: categoryRows.length + (suggested === null ? 1 : 2)

    function isChoice(i) { const e = entries[i]; return !!e && e.kind !== "divider" && !(e.kind === "category" && e.isGroup) }
    function isSelected(e) {
        if (!booking || !sharedCategory) return false
        if (e.kind === "none") return booking.category_id === null
        return e.kind === "category" && booking.category_id === e.category.id
    }
    /// choose(_:): dismiss, then AppModel.setCategoryFromPicker – the targets that already have the
    /// category keep it (and its source: picking a rule's category again does not turn it manual).
    function choose(id) {
        const changed = targets.filter(t => t.category_id !== id)
        close()
        AppModel.setCategoryFor(changed, id)
    }
    function activate(i) {
        if (!isChoice(i)) return
        const e = entries[i]
        choose(e.kind === "none" ? null : e.category.id)
    }
    function step(from, delta) {
        for (let i = from + delta; i >= 0 && i < entries.length; i += delta)
            if (isChoice(i)) return i
        return from
    }

    contentItem: ColumnLayout {
        spacing: 0
        Text {
            visible: pop.targets.length > 1
            text: qsTr("%1 Buchungen ausgewählt").arg(pop.targets.length)
            font.pixelSize: Theme.caption; font.weight: Font.Medium; color: Theme.secondary
            Layout.fillWidth: true; Layout.margins: 8
        }
        Rectangle { visible: pop.targets.length > 1; Layout.fillWidth: true; implicitHeight: 1; color: Theme.border }
        ListView {
            id: list
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(360, 12 + pop.entryCount * 26) * Theme.textScale
            Layout.margins: 6
            clip: true
            spacing: 2
            boundsBehavior: Flickable.StopAtBounds
            model: pop.visible ? pop.entries : []
            currentIndex: -1
            /// the highlight only after an arrow key (plain buttons on the Mac show none for the mouse)
            property bool byKeyboard: false
            ScrollBar.vertical: AppScrollBar {}
            // .onAppear { proxy.scrollTo(booking.categoryId, anchor: .center) }
            onCountChanged: if (count > 0 && pop.booking && pop.booking.category_id !== null) {
                const i = pop.entries.findIndex(e => e.kind === "category" && e.category.id === pop.booking.category_id)
                if (i >= 0) positionViewAtIndex(i, ListView.Center)
            }
            Keys.onDownPressed: { byKeyboard = true; currentIndex = pop.step(currentIndex, 1) }
            Keys.onUpPressed: { byKeyboard = true; currentIndex = pop.step(currentIndex < 0 ? pop.entries.length : currentIndex, -1) }
            Keys.onReturnPressed: pop.activate(currentIndex)
            Keys.onEnterPressed: pop.activate(currentIndex)
            Keys.onSpacePressed: pop.activate(currentIndex)
            delegate: Item {
                id: entry
                required property var modelData
                required property int index
                readonly property bool divider: modelData.kind === "divider"
                readonly property bool selected: pop.isSelected(entry.modelData)
                readonly property bool choice: pop.isChoice(entry.index)
                readonly property var category: entry.modelData.kind === "none" ? null : modelData.category
                readonly property int depth: modelData.kind === "category" ? modelData.depth : 0
                readonly property color tint: modelData.kind === "none" ? Theme.warning
                    : modelData.kind === "suggestion" ? Theme.secondary : Theme.readable(category.color_hex)
                readonly property string title: modelData.kind === "none" ? SidebarModel.uncategorizedLabel
                    : modelData.kind === "suggestion" ? qsTr("Vorschlag übernehmen: %1").arg(category.name) : category.name
                width: ListView.view.width
                implicitHeight: divider ? 1 : option.implicitHeight + 8
                Accessible.role: modelData.kind === "category" && modelData.isGroup ? Accessible.Heading : Accessible.Button
                Accessible.name: divider ? "" : AppModel.masking(title, AppModel.redactLabels)
                Accessible.selected: selected
                Accessible.onPressAction: pop.activate(entry.index)
                Rectangle { visible: entry.divider; anchors.fill: parent; color: Theme.border }
                Rectangle {
                    visible: !entry.divider && (entry.selected || (list.byKeyboard && list.currentIndex === entry.index))
                    anchors.fill: parent; radius: 5
                    color: Theme.withAlpha(Theme.accent, entry.selected ? 0.14 : 0.08)
                }
                RowLayout {
                    id: option
                    visible: !entry.divider
                    anchors.verticalCenter: parent.verticalCenter
                    x: 6 + entry.depth * 12 * Theme.textScale
                    width: parent.width - x - 8
                    spacing: 7
                    SFIcon {
                        symbol: entry.modelData.kind === "none" ? "questionmark.folder"
                            : entry.modelData.kind === "suggestion" ? "clock.arrow.circlepath"
                            : (entry.category.sf_symbol || "tag.fill")
                        size: 12; color: entry.tint
                        Layout.preferredWidth: 16 * Theme.textScale
                    }
                    Text {
                        text: entry.title
                        font.pixelSize: Theme.callout + 1; color: entry.tint
                        elide: Text.ElideRight; Layout.fillWidth: true
                        Sensitive { labels: true }
                    }
                    SFIcon { visible: entry.selected; symbol: "checkmark"; size: 11; color: Theme.accent }
                }
                MouseArea {
                    anchors.fill: parent
                    enabled: entry.choice
                    onClicked: pop.activate(entry.index)
                }
            }
        }
    }
}

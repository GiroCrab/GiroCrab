import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// PlanningUI.swift › SplitSheet: split one booking (`AppModel.splitBooking`)
// into category parts that add up to its amount.
Sheet {
    id: sheet
    readonly property var booking: AppModel.splitBooking
    /// [{ uid, categoryID, amount, note }] – edits bump `revision` instead of
    /// rebuilding the list, so the field being typed in keeps its focus.
    /// The first part keeps the rest (#1663): its amount follows from the others, so the
    /// parts always add up and only what is split off is typed.
    property var parts: []
    property int revision: 0
    property int nextUid: 0
    property string error: ""
    /// The booking is split already (loaded with the parts).
    property bool hasSplits: false
    /// The part whose amount field takes the focus once its row exists.
    property int focusUid: -1
    /// The part ½ ⅓ ¼ fill: the one typed in last, else the last part.
    property int pickUid: -1
    visible: booking !== null
    sheetWidth: 600
    padding: 22
    onClosed: { AppModel.splitBooking = null; parts = [] }

    readonly property real total: booking ? Math.abs(booking.amount_cents) : 0
    readonly property int sign: booking && booking.amount_cents < 0 ? -1 : 1
    // Amounts are entered in the booking's direction: a negative part counter-books the others
    // (+59 = +200 and -141, #427). The first part takes what is left; no part may be 0 (SplitMath).
    /// A part split off as typed: an amount or a share ("50 %"); null when unreadable. An empty
    /// field counts as 0, so the rest shows the whole amount until something is typed.
    function cents(p) { return String(p.amount).trim() === "" ? 0 : AppModel.splitPartCents(p.amount, total) }
    readonly property var splitOff: { revision; return parts.slice(1).map(p => cents(p)) }
    /// What the first part keeps; null while a part is unreadable.
    readonly property var rest: splitOff.some(c => c === null) ? null : total - splitOff.reduce((s, c) => s + c, 0)
    readonly property bool valid: parts.length >= 2 && rest !== null && rest !== 0 && splitOff.every(c => c !== 0)
    /// Why "Speichern" is greyed out (#383): say so instead of leaving it silent.
    readonly property string problem: {
        revision
        if (valid || parts.length === 0 || error !== "") return ""
        return parts.slice(1).some(p => String(p.amount).trim() !== "" && cents(p) === null)
            ? qsTr("Teilbeträge sind Beträge wie „43,71“ oder Anteile wie „50 %“.")
            : qsTr("Jeder Teil braucht einen Betrag ungleich 0.")
    }
    function presetAmount() {
        const percent = AppModel.splitPresetPercent
        const share = percent === null ? null : AppModel.splitShareCents(total, percent, 100)
        return share === null ? "" : Fmt.plain(share)
    }
    function part(categoryID, amount, note) { return { uid: nextUid++, categoryID: categoryID, amount: amount, note: note } }

    onAboutToShow: {
        error = ""; hasSplits = false; parts = []
        const b = booking
        AppModel.splits(b, existing => {
            if (sheet.booking !== b) return
            if (existing === null) {
                // never offer a fresh split over one that could not be read
                error = qsTr("Die bestehende Aufteilung konnte nicht geladen werden.")
                return
            }
            hasSplits = existing.length > 0
            parts = existing.length === 0
                // the share picked in the row menu (#1663) goes into the second part
                ? [part(b.category_id, "", ""), part(null, presetAmount(), "")]
                : existing.map(s => part(s.category_id, Fmt.plain(sign * s.amount_cents), s.note))
            if (!hasSplits) focusUid = parts[1].uid
        })
    }
    function save() {
        if (!valid) return
        const amounts = [rest].concat(splitOff)
        const inputs = parts.map((p, i) => ({ category_id: p.categoryID, amount_cents: sign * amounts[i], note: p.note }))
        error = AppModel.saveSplits(inputs, booking)
        if (error === "") close()
    }
    /// The quick picks (#1663): `1 / denominator` of the booking into the part typed in last
    /// (else the last part); the odd cent stays with the rest.
    function pick(denominator) {
        let index = parts.findIndex(p => p.uid === pickUid)
        if (index < 1) index = parts.length - 1
        if (index < 1) return
        const share = AppModel.splitShareCents(total, 1, denominator)
        parts[index].amount = share === null ? "" : Fmt.plain(share)
        // rebuild the rows: the amount field shows the new text
        parts = parts.slice()
        revision++
    }
    /// A share typed as "50 %" turns into the amount it stands for once the field is left;
    /// anything else stays. Returns the field's new text.
    function settle(index) {
        const p = parts[index]
        if (!p || !String(p.amount).includes("%")) return null
        const c = AppModel.splitPartCents(p.amount, total)
        if (c === null) return null
        p.amount = Fmt.plain(c)
        revision++
        return p.amount
    }

    readonly property var categoryOptions: [{ id: null, title: SidebarModel.uncategorizedLabel }]
        .concat(AppModel.categories.map(c => ({ id: c.id, title: c.path })))

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 14
        Text { text: qsTr("Umsatz aufteilen"); font.pixelSize: Theme.title3 + 1; font.weight: Font.Bold; color: Theme.text }
        RowLayout {
            Layout.fillWidth: true
            spacing: 8
            Text {
                Layout.fillWidth: true
                text: sheet.booking ? sheet.booking.merchant_name + " · " + Fmt.money(sheet.booking.amount_cents) : ""
                font.pixelSize: Theme.body; color: Theme.secondary
                elide: Text.ElideRight
                Sensitive { amounts: true }   // [.personal, .amounts]
            }
            // one click to split off a half, third or quarter (#1663)
            Button { text: "½"; implicitWidth: 34; font.pixelSize: Theme.body + 1; ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Die Hälfte abspalten"); onClicked: sheet.pick(2) }
            Button { text: "⅓"; implicitWidth: 34; font.pixelSize: Theme.body + 1; ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Ein Drittel abspalten"); onClicked: sheet.pick(3) }
            Button { text: "¼"; implicitWidth: 34; font.pixelSize: Theme.body + 1; ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("Ein Viertel abspalten"); onClicked: sheet.pick(4) }
        }
        Repeater {
            model: sheet.parts
            RowLayout {
                id: row
                required property var modelData
                required property int index
                readonly property bool isRest: index === 0
                Layout.fillWidth: true
                spacing: 8
                AppComboBox {
                    Layout.preferredWidth: 190
                    Accessible.name: qsTr("Kategorie")   // Picker("Kategorie", …) (ff91866)
                    model: sheet.categoryOptions
                    textRole: "title"
                    currentIndex: Math.max(0, sheet.categoryOptions.findIndex(o => o.id === row.modelData.categoryID))
                    onActivated: i => { sheet.parts[row.index].categoryID = sheet.categoryOptions[i].id; sheet.revision++ }
                }
                // the rest: what the parts below leave, never typed (#1663)
                TextField {
                    visible: row.isRest
                    Layout.preferredWidth: 90
                    text: sheet.rest === null ? "–" : Fmt.plain(sheet.rest)
                    readOnly: true
                    enabled: false
                    horizontalAlignment: TextInput.AlignRight
                    font.features: { "tnum": 1 }
                    Accessible.name: qsTr("Rest")
                    HoverHandler { id: restHover }
                    ToolTip.visible: restHover.hovered; ToolTip.delay: 600
                    ToolTip.text: qsTr("Bekommt automatisch, was die anderen Teile übrig lassen")
                    Sensitive { amounts: true }
                }
                TextField {
                    id: amountField
                    visible: !row.isRest
                    Layout.preferredWidth: 90
                    text: row.modelData.amount
                    placeholderText: Fmt.plain(0)
                    horizontalAlignment: TextInput.AlignRight
                    font.features: { "tnum": 1 }
                    onTextEdited: { sheet.parts[row.index].amount = text; sheet.revision++ }
                    onActiveFocusChanged: {
                        if (activeFocus) { sheet.pickUid = row.modelData.uid; return }
                        // "50 %" becomes the amount once the field is left (#1663)
                        const settled = sheet.settle(row.index)
                        if (settled !== null) text = settled
                    }
                    onAccepted: { const settled = sheet.settle(row.index); if (settled !== null) text = settled }
                    ToolTip.visible: hovered; ToolTip.delay: 600
                    ToolTip.text: qsTr("Betrag oder Anteil, z. B. „50 %“")
                    Sensitive { amounts: true }
                    Component.onCompleted: if (!row.isRest && row.modelData.uid === sheet.focusUid) { forceActiveFocus(); sheet.focusUid = -1 }
                }
                TextField {
                    Layout.fillWidth: true
                    text: row.modelData.note
                    placeholderText: qsTr("Notiz")
                    onTextEdited: sheet.parts[row.index].note = text
                }
                IconButton {
                    visible: !row.isRest && sheet.parts.length > 2
                    symbol: "minus.circle"
                    ToolTip.text: qsTr("Aufteilung entfernen")
                    Accessible.name: qsTr("Aufteilung entfernen")
                    onClicked: sheet.parts = sheet.parts.filter(p => p.uid !== row.modelData.uid)
                }
            }
        }
        IconButton {
            text: qsTr("Teil hinzufügen"); symbol: "plus"; symbolSize: 13
            onClicked: {
                const p = sheet.part(null, "", "")
                sheet.focusUid = p.uid
                sheet.parts = sheet.parts.concat([p])
            }
        }
        Text {
            visible: sheet.problem !== ""
            text: sheet.problem
            font.pixelSize: Theme.callout + 1; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true
        }
        Text { visible: sheet.error !== ""; text: sheet.error; font.pixelSize: Theme.callout + 1; color: Theme.negative; wrapMode: Text.WordWrap; Layout.fillWidth: true }
        RowLayout {
            Layout.fillWidth: true
            Button {
                visible: sheet.hasSplits
                text: qsTr("Aufteilung aufheben")
                palette.buttonText: Theme.negative
                onClicked: { sheet.error = AppModel.saveSplits([], sheet.booking); if (sheet.error === "") sheet.close() }
            }
            Item { Layout.fillWidth: true }
            Button { text: qsTr("Abbrechen"); onClicked: sheet.close() }
            Button { text: qsTr("Speichern"); highlighted: true; enabled: sheet.valid; onClicked: sheet.save() }
        }
    }
}

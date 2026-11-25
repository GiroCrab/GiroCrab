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
    /// The stored split was read (an empty result counts). Until then, and after a failed read,
    /// nothing can be added, picked or saved: a draft must never replace a split it has not seen.
    property bool ready: false
    /// The loaded parts no longer add up to the booking (its amount changed after saving): the
    /// first part shows the rest instead of what was stored (#1663), so the sheet says so.
    property bool storedPartsDiffer: false
    /// The share picked in the row menu, when the booking is split already and it was not
    /// applied (#1663); "" otherwise. Not seeing that a booking is split is #1662.
    property string ignoredShare: ""
    /// The part whose amount field takes the focus once its row exists.
    property int focusUid: -1
    /// The part the share buttons fill: the one typed in last, else the last part.
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
    /// The rest is below 0 and counter-books the other parts: allowed (#427), but computed
    /// (#1663), so a typo gets there unnoticed. Warn, never block.
    readonly property bool restIsNegative: rest !== null && rest < 0
    readonly property bool valid: ready && parts.length >= 2 && rest !== null && rest !== 0 && splitOff.every(c => c !== 0)
    /// Why "Speichern" is greyed out (#383): say so instead of leaving it silent; nothing while
    /// nothing is typed yet (SplitDraft.problem).
    readonly property string problem: {
        revision
        if (valid || parts.length === 0 || error !== "") return ""
        if (parts.slice(1).every(p => String(p.amount).trim() === "")) return ""
        return parts.slice(1).some(p => String(p.amount).trim() !== "" && cents(p) === null)
            ? qsTr("Teilbeträge sind Beträge wie „43,71“ oder Anteile wie „50 %“.")
            : qsTr("Jeder Teil braucht einen Betrag ungleich 0.")
    }
    function presetAmount() {
        const percent = AppModel.splitPresetPercent
        const share = percent === null ? null : AppModel.splitShareCents(total, percent, 100)
        return share === null ? "" : Fmt.plain(share)
    }
    function part(categoryID, amount, note, uuid) { return { uid: nextUid++, uuid: uuid || null, categoryID: categoryID, amount: amount, note: note } }

    onAboutToShow: {
        error = ""; hasSplits = false; storedPartsDiffer = false; ignoredShare = ""; parts = []; ready = false
        const b = booking
        AppModel.splits(b, existing => {
            if (sheet.booking !== b) return
            if (existing === null) {
                // never offer a fresh split over one that could not be read
                error = qsTr("Die bestehende Aufteilung konnte nicht geladen werden.")
                return
            }
            hasSplits = existing.length > 0
            // the share picked in the row menu (#1663) goes into the second part
            const loaded = hasSplits
                ? existing.map(s => part(s.category_id, Fmt.plain(sign * s.amount_cents), s.note, s.uuid))
                : [part(b.category_id, "", ""), part(null, presetAmount(), "")]
            // before `parts`: the Repeater builds the rows (and reads focusUid) on assignment
            if (!hasSplits) focusUid = loaded[1].uid
            parts = loaded
            ready = true
            if (hasSplits) {
                storedPartsDiffer = existing.reduce((s, x) => s + sign * x.amount_cents, 0) !== total
                const percent = AppModel.splitPresetPercent
                ignoredShare = percent === null ? "" : qsTr("%1 %").arg(percent)
            }
        })
    }
    function save() {
        if (!valid) return
        const amounts = [rest].concat(splitOff)
        const inputs = parts.map((p, i) => ({ uuid: p.uuid, category_id: p.categoryID, amount_cents: sign * amounts[i], note: p.note }))
        error = AppModel.saveSplits(inputs, booking)
        if (error === "") close()
    }
    /// The share buttons (#1663): `percent` of the booking into the part typed in last
    /// (else the last part); the odd cent stays with the rest.
    function pick(percent) {
        let index = parts.findIndex(p => p.uid === pickUid)
        if (index < 1) index = parts.length - 1
        if (index < 1) return
        const share = AppModel.splitShareCents(total, percent, 100)
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
            // one click to split off 7 %, 19 % or 50 % (#1663)
            Repeater {
                model: AppModel.splitShares
                Button {
                    required property int modelData
                    text: qsTr("%1 %").arg(modelData)
                    font.pixelSize: Theme.body
                    ToolTip.visible: hovered; ToolTip.delay: 600; ToolTip.text: qsTr("%1 abspalten").arg(text)
                    enabled: sheet.ready
                    onClicked: sheet.pick(modelData)
                }
            }
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
                    Layout.preferredWidth: 120 * Theme.textScale   // holds "−1.234.567,89" at every text size
                    text: sheet.rest === null ? "–" : Fmt.plain(sheet.rest)
                    readOnly: true
                    enabled: false
                    horizontalAlignment: TextInput.AlignRight
                    font.features: { "tnum": 1 }
                    Accessible.name: qsTr("Rest")
                    Binding on color { when: sheet.restIsNegative; value: Theme.warning }
                    HoverHandler { id: restHover }
                    ToolTip.visible: restHover.hovered; ToolTip.delay: 600
                    ToolTip.text: qsTr("Bekommt automatisch, was die anderen Teile übrig lassen")
                    Sensitive { amounts: true }
                }
                TextField {
                    id: amountField
                    visible: !row.isRest
                    Layout.preferredWidth: 120 * Theme.textScale   // holds "−1.234.567,89" at every text size
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
                    Sensitive {}   // notes are personal (.sensitive())
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
            enabled: sheet.ready
            onClicked: {
                const p = sheet.part(null, "", "")
                sheet.focusUid = p.uid
                sheet.parts = sheet.parts.concat([p])
            }
        }
        Text {
            visible: sheet.ignoredShare !== ""
            text: qsTr("Die Buchung ist schon aufgeteilt, deshalb wurde „%1“ nicht eingetragen.").arg(sheet.ignoredShare)
            font.pixelSize: Theme.callout + 1; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true
        }
        Text {
            visible: sheet.storedPartsDiffer
            text: qsTr("Die gespeicherten Teile ergeben nicht mehr den Betrag der Buchung. Die erste Zeile gleicht die Differenz aus; bitte vor dem Speichern prüfen.")
            font.pixelSize: Theme.callout + 1; color: Theme.warning; wrapMode: Text.WordWrap; Layout.fillWidth: true
        }
        Text {
            visible: sheet.problem !== ""
            text: sheet.problem
            font.pixelSize: Theme.callout + 1; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true
        }
        Text {
            visible: sheet.restIsNegative && sheet.error === ""
            text: qsTr("Der Rest in der ersten Zeile ist negativ und bucht die anderen Teile gegen. Ist das nicht gewollt, bitte die Beträge prüfen.")
            font.pixelSize: Theme.callout + 1; color: Theme.warning; wrapMode: Text.WordWrap; Layout.fillWidth: true
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

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// MerchantRecognitionSettings.swift (#1373): Einstellungen › Händler – the master switch of the
// merchant recognition, the merchant logos, the user's own merchants and the bundled ones with a
// switch each. Needs an open vault. Layout and rules: docs/HAENDLER_UI.md.
Item {
    id: pane

    /// MerchantRecognition: { enabled, builtin: [BuiltinMerchant], custom: [CustomMerchant] }, null
    /// before the first read and after a lock or vault switch.
    property var recognition: null
    /// Why the last read failed; the rows of the last good read stay.
    property string loadError: ""
    /// Counts the loads: only the newest one may show its result (quick toggles).
    property int loads: 0
    property string search: ""
    /// Switches flipped here whose write and reload are still running ("" = master switch),
    /// so a toggle does not jump back for the length of a reload.
    property var pending: ({})
    /// Writes still running per switch: a read may only replace a shown switch position once
    /// its write is done.
    property var inFlight: ({})
    /// Why the last change could not be stored (shown here: an alert of the main window would
    /// hide behind the Settings window).
    property string writeError: ""
    /// Position of each bundled merchant in the list, fixed when the pane opens or the search
    /// changes: a switched row must not jump away under the pointer.
    property var order: ({})
    /// The own merchant asked about in the delete dialog.
    property var deleting: null

    readonly property bool masterOn: pending[""] !== undefined ? pending[""]
                                     : recognition !== null ? recognition.enabled : AppModel.merchantRecognitionEnabled
    readonly property string query: search.trim().toLocaleLowerCase()
    function has(text) { return text.toLocaleLowerCase().indexOf(query) >= 0 }

    /// .task(id: vaultRevision): read and sort whenever the pane appears.
    readonly property bool shown: visible && AppModel.showSettings
    onShownChanged: if (shown) reload(true)
    onSearchChanged: sortBuiltin()
    Connections {
        target: AppModel
        // a vault swap (demo ↔ own) or a lock: forget the old vault's switches, read and sort
        function onVaultGenerationChanged() {
            pane.recognition = null
            pane.pending = ({})
            pane.order = ({})
            pane.writeError = ""
            if (pane.shown) pane.reload(true)
        }
        // a change here, a reload or a sync: read it again, rows stay in place
        function onMerchantSettingsRevisionChanged() { if (pane.shown) pane.reload(false) }
    }

    // MARK: Own merchants

    /// The own merchants the search finds (by name and search word).
    readonly property var customRows: (recognition ? recognition.custom : [])
        .filter(m => query === "" || has(m.name) || m.needles.some(n => has(n)))

    // MARK: Bundled merchants

    /// Filtered by name, key and search word, in the order `sortBuiltin` fixed.
    readonly property var builtinRows: {
        const o = order
        const rows = (recognition ? recognition.builtin : [])
            .filter(m => query === "" || has(m.name) || has(m.key) || m.needles.some(n => has(n)))
        const at = m => o[m.key] !== undefined ? o[m.key] : Number.MAX_SAFE_INTEGER
        return rows.sort((a, b) => at(a) - at(b))
    }
    readonly property int builtinCount: recognition ? recognition.builtin.length : 0
    readonly property int builtinOff: recognition ? recognition.builtin.filter(m => !isOn(m)).length : 0

    /// Switched-off and replaced ones first (undoing is near), then by name. Only when the pane
    /// opens or the search changes, never after a switch (`order`).
    function sortBuiltin() {
        const sorted = (recognition ? recognition.builtin : []).slice().sort((a, b) => {
            const onA = isActive(a), onB = isActive(b)
            if (onA !== onB) return onA ? 1 : -1
            return a.name.localeCompare(b.name)
        })
        const o = {}
        sorted.forEach((m, i) => o[m.key] = i)
        order = o
    }

    function isOn(m) { return pending[m.key] !== undefined ? pending[m.key] : m.enabled }
    /// Recognised: switched on and not replaced by an own copy.
    function isActive(m) { return isOn(m) && !m.replaced }

    /// MerchantKind.title: "Art" of a bundled merchant.
    function kindTitle(kind) {
        return ({ Supermarket: qsTr("Supermarkt"), Butcher: qsTr("Metzgerei"), FoodRescue: qsTr("Lebensmittelrettung"),
                  Restaurant: qsTr("Restaurant"), Delivery: qsTr("Lieferdienst"), Drugstore: qsTr("Drogerie"),
                  Insurance: qsTr("Versicherung"), Mobility: qsTr("Mobilität"), Fuel: qsTr("Tankstelle"),
                  Streaming: qsTr("Streaming"), Telecom: qsTr("Telekommunikation"), Utilities: qsTr("Energie & Versorgung"),
                  OnlineShop: qsTr("Onlineshop"), Electronics: qsTr("Elektronik"), Games: qsTr("Spiele"),
                  Fitness: qsTr("Fitness"), Travel: qsTr("Reisen"), PaymentService: qsTr("Zahlungsdienst"),
                  Bank: qsTr("Bank"), Broker: qsTr("Broker"), EtfIssuer: qsTr("ETF-Anbieter"), Etf: qsTr("ETF"),
                  Crypto: qsTr("Krypto"), Stock: qsTr("Aktie") })[kind] || qsTr("Sonstige")
    }

    // MARK: Actions

    function setMaster(on) {
        flip("", on, (done, failed) => AppModel.setMerchantRecognitionEnabled(on, done, failed))
    }
    function setBuiltin(m, on) {
        flip(m.key, on, (done, failed) => AppModel.setBuiltinMerchantEnabled(m.key, on, done, failed))
    }
    /// Shows the switch `key` in its new position at once and stores it. The list reads again
    /// after the change (`merchantSettingsRevision`); a failed write puts the switch back,
    /// unless it was flipped again meanwhile, and says why.
    function flip(key, on, write) {
        const p = Object.assign({}, pending)
        p[key] = on
        pending = p
        inFlight[key] = (inFlight[key] || 0) + 1
        const settled = () => { inFlight[key] = Math.max(0, (inFlight[key] || 1) - 1) }
        write(() => { settled(); writeError = "" }, message => {
            settled()
            if (pending[key] === on) {
                const back = Object.assign({}, pending)
                delete back[key]
                pending = back
            }
            writeError = message
            reload(false)
        })
    }
    function remove(m) {
        AppModel.deleteCustomMerchant(m.uuid, () => writeError = "", message => writeError = message)
    }
    function baseName(m) {
        if (m.based_on === "") return null
        const b = recognition ? recognition.builtin.find(x => x.key === m.based_on) : null
        return b ? b.name : m.based_on
    }
    function deleteMessage(m) {
        if (!m) return ""
        const custom = recognition ? recognition.custom : []
        const otherCopies = custom.some(c => c.uuid !== m.uuid && c.based_on === m.based_on)
        const original = recognition ? recognition.builtin.find(x => x.key === m.based_on) : null
        const base = baseName(m)
        if (base !== null && !otherCopies && original && original.enabled)
            return qsTr("Der eingebaute Händler „%1“ wird wieder erkannt.").arg(base)
        return qsTr("Seine Buchungen zeigen wieder den erkannten Händler oder ihre Gegenseite.")
    }

    /// A load overtaken by a newer one drops its result; a failed one keeps the rows and says so.
    /// `sorting`: fix the order of the bundled list anew (see `order`).
    function reload(sorting) {
        const load = ++loads
        AppModel.merchantRecognition(found => {
            if (load !== loads) return
            if (found === null) {
                // locked or switched meanwhile: show nothing of that vault any more
                recognition = null
                pending = ({})
                order = ({})
                return
            }
            recognition = found
            // a switch keeps its new position only while its write is still running
            const kept = {}
            for (const key in pending) {
                const b = key === "" ? null : found.builtin.find(m => m.key === key)
                const stored = key === "" ? found.enabled : b ? b.enabled : undefined
                if ((inFlight[key] || 0) > 0 && stored !== pending[key]) kept[key] = pending[key]
            }
            pending = kept
            if (sorting || Object.keys(order).length === 0) sortBuiltin()
            loadError = ""
        }, message => {
            if (load !== loads) return
            loadError = message
        })
    }

    CustomMerchantEditor { id: editor; onSaved: pane.reload(false) }
    ConfirmDialog {
        id: confirmDelete
        heading: pane.deleting ? qsTr("„%1“ löschen?").arg(AppModel.masking(pane.deleting.name)) : ""
        message: pane.deleteMessage(pane.deleting)
        actionTitle: qsTr("Löschen")
        onConfirmed: pane.remove(pane.deleting)
    }

    ScrollView {
        ScrollBar.vertical: ScrollViewBar {}
        id: scroll
        anchors.fill: parent
        contentWidth: availableWidth
        ColumnLayout {
            width: scroll.availableWidth - 40
            x: 20
            spacing: 18
            Item { implicitHeight: 2 }
            SettingsFormSection {
                title: qsTr("Händlererkennung")
                CheckBox {
                    text: qsTr("Händler erkennen")
                    checked: pane.masterOn
                    onToggled: {
                        pane.setMaster(checked)
                        checked = Qt.binding(() => pane.masterOn)
                    }
                }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                       text: qsTr("Erkennt den Händler einer Buchung aus Gegenseite und Verwendungszweck und zeigt seinen Namen und sein Zeichen. Ausgeschaltet zeigen Buchungen die Gegenseite (ohne Gegenseite den Anfang des Verwendungszwecks), ohne Händlernamen und Logo. Was als Konsum, Investition oder wiederkehrende Zahlung zählt, ändert sich dadurch nicht.") }
            }
            SettingsFormSection {
                visible: pane.writeError !== ""
                ProblemLabel { text: qsTr("Die Änderung konnte nicht gespeichert werden") }
                Text { Layout.fillWidth: true; text: pane.writeError; wrapMode: Text.WordWrap; maximumLineCount: 3; elide: Text.ElideRight
                       font.pixelSize: Theme.caption2; color: Theme.tertiary }
            }
            // SettingsView.swift › MerchantLogoSettingsSection (GitHub #33), moved here from
            // „Allgemein“ (#1373); off while the recognition is off
            SettingsFormSection {
                title: qsTr("Händler-Logos")
                enabled: pane.masterOn
                footer: qsTr("Ohne Anbieter zeigt GiroCrab mitgelieferte Markenzeichen, sonst Monogramme und Kategorie-Symbole. Mit Anbieter werden Favicons geladen und verschlüsselt im Tresor zwischengespeichert. Übertragen wird nur die Händler-Domain (z. B. lidl.de) – keine Beträge, keine Kontodaten.")
                SettingsFormRow {
                    label: qsTr("Logos aus dem Internet laden")
                    AppComboBox {
                        Layout.preferredWidth: 200 * Theme.textScale
                        model: [{ id: "off", title: qsTr("Aus") }, { id: "duckduckgo", title: qsTr("DuckDuckGo") },
                                { id: "google", title: qsTr("Google") }]
                        textRole: "title"; valueRole: "id"
                        currentIndex: Math.max(0, ["off", "duckduckgo", "google"].indexOf(MerchantLogos.provider))
                        onActivated: MerchantLogos.setProvider(currentValue)
                    }
                }
                Button {
                    text: qsTr("Logo-Cache leeren")
                    onClicked: MerchantLogos.clearLogoCache()
                }
            }
            SettingsFormSection {
                visible: pane.loadError !== ""
                ProblemLabel { text: qsTr("Die Händlerliste konnte nicht geladen werden") }
                Text { Layout.fillWidth: true; text: pane.loadError; wrapMode: Text.WordWrap; maximumLineCount: 2; elide: Text.ElideRight
                       font.pixelSize: Theme.caption2; color: Theme.tertiary }
                Button { text: qsTr("Erneut versuchen"); onClicked: pane.reload(true) }
            }
            SettingsFormSection {
                enabled: pane.masterOn
                TextField {
                    Layout.fillWidth: true
                    placeholderText: qsTr("Händler oder Suchwort suchen")
                    text: pane.search
                    onTextEdited: pane.search = text
                    Sensitive { when: parent.text !== "" }   // typed text
                }
            }
            // Linux: without the Mac's last sentence about iCloud sync (no iCloud here)
            SettingsFormSection {
                title: qsTr("Eigene Händler")
                enabled: pane.masterOn
                footer: qsTr("Eigene Händler gehen den eingebauten vor. Gesucht wird zuerst in der Gegenseite; passen dort mehrere, gewinnt das längste Suchwort. Ein Suchwort passt nur als ganzes Wort (auch mehrere Wörter wie „muster shop“), Groß- und Kleinschreibung zählen nicht.")
                Text {
                    visible: pane.query !== "" && pane.customRows.length === 0 && pane.recognition !== null && pane.recognition.custom.length > 0
                    text: qsTr("Kein eigener Händler passt zur Suche.")
                    font.pixelSize: Theme.callout; color: Theme.secondary
                }
                Repeater {
                    model: pane.customRows
                    RowLayout {
                        id: crow
                        required property var modelData
                        readonly property var m: modelData
                        readonly property var base: pane.baseName(m)
                        Layout.fillWidth: true
                        spacing: 10
                        MerchantSignet { merchant: crow.m.merchant; size: 28 }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 1
                            Text { Layout.fillWidth: true; text: crow.m.name; elide: Text.ElideRight; font.pixelSize: Theme.body; color: Theme.text
                                   Sensitive {} }   // a name the user typed
                            Text { Layout.fillWidth: true; text: AppModel.merchantNeedlesText(crow.m.needles); wrapMode: Text.WordWrap
                                   maximumLineCount: 2; elide: Text.ElideRight; font.pixelSize: Theme.caption; color: Theme.secondary
                                   Sensitive {} }
                            Text { visible: crow.base !== null; text: crow.base !== null ? qsTr("Kopie von „%1“").arg(crow.base) : ""
                                   font.pixelSize: Theme.caption2; color: Theme.tertiary }
                        }
                        IconButton {
                            symbol: "pencil"
                            ToolTip.text: qsTr("Name und Suchwörter ändern")
                            Accessible.name: qsTr("„%1“ bearbeiten …").arg(AppModel.masking(crow.m.name))
                            onClicked: editor.edit(crow.m)
                        }
                        IconButton {
                            symbol: "trash"
                            ToolTip.text: qsTr("Eigenen Händler löschen")
                            Accessible.name: qsTr("„%1“ löschen …").arg(AppModel.masking(crow.m.name))
                            onClicked: { pane.deleting = crow.m; confirmDelete.open() }
                        }
                        TapHandler { acceptedButtons: Qt.RightButton; onTapped: customMenu.popup() }
                        AppMenu {
                            id: customMenu
                            AppMenuItem { text: qsTr("Bearbeiten …"); icon.name: SF.icon("pencil"); onTriggered: editor.edit(crow.m) }
                            AppMenuItem { text: qsTr("Löschen …"); icon.name: SF.icon("trash")
                                          onTriggered: { pane.deleting = crow.m; confirmDelete.open() } }
                        }
                    }
                }
                IconButton {
                    symbol: "plus"
                    text: qsTr("Eigenen Händler hinzufügen …")
                    tint: Theme.accent
                    onClicked: editor.create()
                }
            }
            SettingsFormSection {
                title: pane.builtinOff > 0 ? qsTr("Eingebaute Händler (%1, %2 ausgeschaltet)").arg(pane.builtinCount).arg(pane.builtinOff)
                                           : qsTr("Eingebaute Händler (%1)").arg(pane.builtinCount)
                enabled: pane.masterOn
                footer: qsTr("Ein ausgeschalteter Händler wird nicht mehr erkannt; die Buchung zeigt dann den nächsten passenden Händler oder ihre Gegenseite. „Bearbeiten“ legt eine Kopie als eigenen Händler an, die den eingebauten ersetzt, solange es sie gibt; die Liste selbst bleibt unverändert.")
                Text {
                    visible: pane.recognition !== null && pane.builtinRows.length === 0
                    text: qsTr("Kein eingebauter Händler passt zur Suche.")
                    font.pixelSize: Theme.callout; color: Theme.secondary
                }
                Repeater {
                    model: pane.builtinRows
                    RowLayout {
                        id: brow
                        required property var modelData
                        readonly property var m: modelData
                        readonly property bool on: pane.isOn(m)
                        readonly property bool active: pane.isActive(m)
                        Layout.fillWidth: true
                        spacing: 10
                        MerchantSignet { merchant: brow.m.merchant; size: 28 }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 1
                            Text { Layout.fillWidth: true; text: brow.m.name; elide: Text.ElideRight; font.pixelSize: Theme.body
                                   color: brow.active ? Theme.text : Theme.secondary }
                            Text { Layout.fillWidth: true; text: brow.m.needles.join(", "); wrapMode: Text.WordWrap
                                   maximumLineCount: 2; elide: Text.ElideRight; font.pixelSize: Theme.caption; color: Theme.secondary }
                            Text { visible: brow.m.replaced; text: qsTr("Ersetzt durch eigenen Händler")
                                   font.pixelSize: Theme.caption2; color: Theme.tertiary }
                        }
                        Text { text: pane.kindTitle(brow.m.kind); font.pixelSize: Theme.caption; color: Theme.tertiary }
                        IconButton {
                            symbol: "pencil"
                            ToolTip.text: qsTr("Als eigenen Händler bearbeiten")
                            Accessible.name: qsTr("„%1“ als eigenen Händler bearbeiten").arg(brow.m.name)
                            onClicked: editor.copy(brow.m)
                        }
                        Switch {
                            enabled: !brow.m.replaced
                            checked: brow.active
                            Accessible.name: brow.m.name
                            onToggled: {
                                pane.setBuiltin(brow.m, checked)
                                checked = Qt.binding(() => brow.active)
                            }
                            ToolTip.visible: hovered
                            ToolTip.delay: 600
                            ToolTip.text: brow.m.replaced ? qsTr("Ein eigener Händler ersetzt ihn; löschen Sie die Kopie, um ihn wieder zu erkennen.")
                                        : brow.on ? qsTr("Diesen Händler nicht mehr erkennen") : qsTr("Diesen Händler wieder erkennen")
                        }
                        TapHandler { acceptedButtons: Qt.RightButton; onTapped: builtinMenu.popup() }
                        AppMenu {
                            id: builtinMenu
                            AppMenuItem { text: qsTr("Als eigenen Händler bearbeiten …"); icon.name: SF.icon("pencil"); onTriggered: editor.copy(brow.m) }
                            AppMenuItem {
                                visible: !brow.m.replaced; height: visible ? implicitHeight : 0
                                text: brow.on ? qsTr("Ausschalten") : qsTr("Einschalten")
                                icon.name: SF.icon(brow.on ? "nosign" : "checkmark.circle")
                                onTriggered: pane.setBuiltin(brow.m, !brow.on)
                            }
                        }
                    }
                }
            }
            Item { implicitHeight: 8 }
        }
    }

    // Label("…", systemImage: "exclamationmark.triangle") in the warning colour
    component ProblemLabel: RowLayout {
        property alias text: label.text
        Layout.fillWidth: true
        spacing: 6
        SFIcon { symbol: "exclamationmark.triangle"; size: 13; color: Theme.warning }
        Text { id: label; Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.warning }
    }
}

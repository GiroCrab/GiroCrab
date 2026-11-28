import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// StandingOrderSheet.swift: Dauerauftrag anlegen, ändern oder löschen (GitHub #496) – a
// form, then a read-only review, then the bank's TAN request (TwoFactorSheet). A deletion
// starts at the review. State and schedule logic: Payments.qml (StandingDraft); open it
// with Payments.openStandingOrder(mode, …). Also holds RootView's alert "Status des
// Dauerauftrags unklar". The recipient is entered as in the transfer form (#1475): suggestions
// while typing, Umbuchung to an own account, templates, paste and dropped invoices.
Sheet {
    id: sheet
    readonly property var draft: Payments.standingDraft
    readonly property var limits: Payments.standingLimits
    readonly property bool isDelete: draft.action === "Delete"
    property bool reviewing: false
    property bool demoReviewShown: false

    readonly property var account: AppModel.allAccounts.find(a => a.id === draft.account_id) || null
    readonly property bool pinStored: {
        if (!account || account.connection_id === null) return false
        if (AppModel.isDemo) return true
        const st = AppModel.connectionStatus(account.connection_id)
        return st ? st.has_pin : false
    }
    readonly property bool ibanValid: AppModel.validateIban(draft.iban)
    readonly property var cents: Payments.standingCents(draft)
    /// Unit and interval in one list: weekly … yearly (what the bank does not offer is left out).
    readonly property var rhythmChoices: Payments.rhythmChoices(draft, limits)
    readonly property var days: Payments.allowedDays(draft, limits)
    readonly property var firstRange: Payments.firstDateRange(limits)
    /// Choices of the execution day: [{ day, title }].
    readonly property var dayChoices: {
        if (draft.unit === "W") return (days.length ? days : [1, 2, 3, 4, 5, 6, 7]).map(d => ({ day: d, title: Payments.weekdayName(d) }))
        const list = (days.length ? days.filter(d => d < 31) : [...Array(30).keys()].map(i => i + 1)).map(d => ({ day: d, title: d + "." }))
        if (!days.length || days.indexOf(31) >= 0) list.push({ day: 31, title: qsTr("Letzter Tag des Monats") })
        return list
    }
    readonly property string heading: draft.action === "Create" ? (reviewing ? qsTr("Dauerauftrag prüfen") : qsTr("Neuer Dauerauftrag"))
                                    : draft.action === "Change" ? (reviewing ? qsTr("Änderung prüfen") : qsTr("Dauerauftrag ändern"))
                                    : qsTr("Dauerauftrag löschen")

    readonly property bool editing: !reviewing && !isDelete
    readonly property bool ownTransfer: !!draft.ownTransfer
    readonly property string bicText: draft.bic.trim()
    readonly property bool bicValid: bicText === "" || AppModel.looksLikeBIC(bicText)
    /// Own accounts an Umbuchung may go to (not the sending account, the same rules as the transfer).
    readonly property var ownTargets: AppModel.ownTransferTargets(account)
    property var ownTargetID: null
    /// The recipient list under the name field (RecipientSuggestions, shared with the transfer form).
    property var suggestions: []
    property int highlighted: -1
    /// Name just taken from the list: typing it must not reopen the list.
    property var pickedName: null
    /// The person changed the name. Until then (a form opened with a name, as "Dauerauftrag
    /// ändern" does) the list stays closed, although the field gets the focus.
    property bool nameEdited: false
    /// What a paste, a template or a dropped invoice filled (#663).
    property string importNote: ""
    property bool readingInvoice: false
    /// .onChange(of: draft.accountID) for every way the sender changes.
    readonly property int senderID: draft.account_id === null || draft.account_id === undefined ? -1 : draft.account_id
    onSenderIDChanged: if (visible && editing) Qt.callLater(senderChanged)

    visible: Payments.showStandingSheet
    sheetWidth: 560
    padding: 0
    // Esc closes the recipient list first.
    closePolicy: AppModel.transferBusy || suggestions.length > 0 ? Popup.NoAutoClose : Popup.CloseOnEscape
    onAboutToShow: {
        reviewing = Payments.standingDraft.action === "Delete"
            || (AppModel.demoSheet === "standing-order-confirm" && !demoReviewShown) // screenshots
        demoReviewShown = true
        suggestions = []
        pickedName = null
        nameEdited = false
        importNote = ""
        ownTargetID = null
        detectOwnTransfer()
    }
    onClosed: if (Payments.showStandingSheet) Payments.cancelStandingSheet()

    /// Fills name and IBAN from the chosen own account (owner, else account name).
    function applyOwnTarget(id) {
        const t = ownTargets.find(a => a.id === id)
        if (!t) return
        Payments.setStanding({ name: t.owner === "" ? t.name : t.owner, iban: AppModel.groupedIBAN(t.iban), bic: "" })
        suggestions = []
    }
    /// Switches between an order to someone else and an Umbuchung: leaving Umbuchung empties the
    /// fields again, so no own account stays in a field that looks typed.
    function setOwnTransfer(on) {
        if (on === ownTransfer) return
        Payments.setStanding({ ownTransfer: on })
        if (on) {
            if (!ownTargets.some(a => a.id === ownTargetID)) ownTargetID = ownTargets.length ? ownTargets[0].id : null
            applyOwnTarget(ownTargetID)
        } else {
            Payments.setStanding({ name: "", iban: "", bic: "" })
        }
    }
    /// An order whose IBAN is an own account opens as Umbuchung (the stored name stays).
    function detectOwnTransfer() {
        if (isDelete) return
        const key = s => (s || "").replace(/\s/g, "").toUpperCase()
        const own = key(draft.iban) === "" ? null : ownTargets.find(a => key(a.iban) === key(draft.iban))
        if (!own) return
        ownTargetID = own.id
        if (!ownTransfer) Payments.setStanding({ ownTransfer: true })
    }
    /// The sending account is no target: keep a valid one, or fall back to an order to someone else.
    function senderChanged() {
        if (!ownTransfer) return
        if (ownTargets.length === 0) { setOwnTransfer(false); return }
        if (!ownTargets.some(a => a.id === ownTargetID)) {
            ownTargetID = ownTargets[0].id
            applyOwnTarget(ownTargetID)
        }
    }
    function refreshSuggestions() {
        suggestions = nameEdited && nameField.activeFocus && draft.name !== pickedName ? suggestionList.recipients(draft.name) : []
    }
    onSuggestionsChanged: highlighted = -1
    function moveHighlight(by) {
        if (!suggestions.length) return false
        highlighted = Math.min(Math.max(highlighted + by, 0), suggestions.length - 1)
        return true
    }
    /// Takes name and IBAN of a known recipient and closes the list.
    function pick(r) {
        pickedName = r.name
        suggestions = []
        Payments.takeStandingRecipient(r.name, r.iban)
    }
    /// "Nicht mehr vorschlagen" (#978): the list is read again in place.
    function hideRecipient(iban) {
        AppModel.setRecipientHidden(iban, true)
        suggestions = suggestionList.recipients(draft.name)
    }
    function pasteFromClipboard() { report(PaymentParser.parseText(AppModel.clipboardText())) }
    function importInvoice(url) {
        // read now: the sheet may be closed or reopened before the file is read
        const token = Payments.standingImportToken
        importNote = qsTr("Lese Rechnung …")
        const path = decodeURIComponent(url.toString().replace(/^file:\/\//, ""))
        readingInvoice = true
        PaymentParser.parseFile(path, p => {
            readingInvoice = false
            sheet.report(p || {}, token)
        })
    }
    /// `token`: a dropped invoice, read later; a late result (review begun, sheet closed or
    /// reopened) changes nothing and leaves no "Lese Rechnung …" behind.
    function report(p, token) {
        if (token !== undefined && token !== Payments.standingImportToken) { importNote = ""; return }
        if (PaymentParser.isEmpty(p)) { importNote = qsTr("Nichts Erkennbares gefunden"); return }
        if (token !== undefined) {
            if (!Payments.fillStandingDraftIfToken(PaymentParser.patch(p), token)) { importNote = ""; return }
        } else {
            Payments.fillStandingDraft(PaymentParser.patch(p))
        }
        importNote = qsTr("Felder ausgefüllt – bitte prüfen")
    }

    // ⇧Strg+V; Strg+V outside a text field (inside one, normal paste keeps working).
    Shortcut { sequences: ["Ctrl+Shift+V", StandardKey.Paste]; enabled: sheet.visible && sheet.editing && !templatesButton.dialogOpen
               onActivated: sheet.pasteFromClipboard() }

    function start() { if (!AppModel.transferBusy) Payments.startStandingOrder() }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 0

        // EntryHeader: title left, the helpers of the transfer form (templates, recipients, paste) right
        RowLayout {
            id: head
            Layout.fillWidth: true
            Layout.leftMargin: 20; Layout.rightMargin: 20; Layout.topMargin: 14; Layout.bottomMargin: 14
            spacing: 12
            Text {
                text: sheet.heading
                font.pixelSize: Theme.title2; font.weight: Font.DemiBold; color: Theme.text
                Accessible.role: Accessible.Heading
            }
            Item { Layout.fillWidth: true }
            TransferTemplatesMenu {
                id: templatesButton
                visible: sheet.editing
                standing: true
            }
            Button {
                visible: sheet.editing
                flat: true; icon.name: SF.icon("person.2"); text: qsTr("Empfänger")
                onClicked: recipientsDialog.open()
                ToolTip.text: qsTr("Empfänger verwalten: veraltete IBANs nicht mehr vorschlagen")
                ToolTip.visible: hovered; ToolTip.delay: 600
            }
            Button {
                visible: sheet.editing
                flat: true; icon.name: SF.icon("doc.on.clipboard"); text: qsTr("Einfügen")
                onClicked: sheet.pasteFromClipboard()
                ToolTip.text: qsTr("Erkennt IBAN, Betrag, Empfänger und Zweck (⇧Strg+V; Strg+V außerhalb von Textfeldern). Rechnung (PDF, ZUGFeRD, GiroCode) hierher ziehen.")
                ToolTip.visible: hovered; ToolTip.delay: 600
            }
        }
        // OwnTransferSwitch: Überweisung ⇄ Umbuchung under the title, as in the transfer form (#1475);
        // offered when there is an own account to book to
        Segmented {
            id: kindSwitch
            visible: sheet.editing && (sheet.ownTargets.length > 0 || sheet.ownTransfer)
            Layout.alignment: Qt.AlignHCenter
            Layout.bottomMargin: 10
            options: [{ id: "single", title: qsTr("Überweisung") }, { id: "own", title: qsTr("Umbuchung") }]
            current: sheet.ownTransfer ? "own" : "single"
            onActivated: id => sheet.setOwnTransfer(id === "own")
            Accessible.name: qsTr("Art")
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.border }

        ScrollView {
            id: scroll
            ScrollBar.vertical: ScrollViewBar {}
            Layout.fillWidth: true
            readonly property real chrome: head.implicitHeight + 28 + (kindSwitch.visible ? kindSwitch.implicitHeight + 10 : 0) + messages.implicitHeight + buttonRow.implicitHeight + 32
            readonly property real body: (sheet.reviewing ? review.implicitHeight : form.implicitHeight) + 40
            Layout.preferredHeight: Math.min(sheet.reviewing ? body : Math.max(body, 420), sheet.maxBodyHeight - chrome)
            contentWidth: availableWidth
            // several children (form / review): the ScrollView cannot guess the height itself
            contentHeight: body
            clip: true

            // MARK: editor
            ColumnLayout {
                id: form
                visible: !sheet.reviewing
                x: 20; y: 20; width: scroll.availableWidth - 40
                spacing: 18

                FormSection {
                    FormRow {
                        label: qsTr("Von Konto")
                        enabled: sheet.draft.action !== "Change"
                        AppComboBox {
                            Layout.preferredWidth: 280
                            model: Payments.standingOrderAccounts.map(a => a.name + " · " + Fmt.iban(a.iban))
                            currentIndex: Payments.standingOrderAccounts.findIndex(a => a.id === sheet.draft.account_id)
                            displayText: currentIndex < 0 && sheet.account ? sheet.account.name + " · " + Fmt.iban(sheet.account.iban) : currentText
                            onActivated: i => Payments.setStanding({ account_id: Payments.standingOrderAccounts[i].id })
                            Sensitive {}
                        }
                    }
                }
                // Recipient: an own account (Umbuchung) or name, IBAN and BIC
                FormSection {
                    footer: sheet.importNote
                    FormRow {
                        visible: sheet.ownTransfer
                        label: qsTr("Zielkonto")
                        AppComboBox {
                            Layout.preferredWidth: 280
                            model: sheet.ownTargets.map(a => a.name + " · " + Fmt.iban(a.iban))
                            currentIndex: sheet.ownTargets.findIndex(a => a.id === sheet.ownTargetID)
                            onActivated: i => { sheet.ownTargetID = sheet.ownTargets[i].id; sheet.applyOwnTarget(sheet.ownTargetID) }
                            Sensitive {}
                        }
                    }
                    FormRow {
                        visible: sheet.ownTransfer
                        label: qsTr("Empfänger")
                        Text { text: sheet.draft.name; font.pixelSize: Theme.body; color: Theme.text; Sensitive {} }
                    }
                    FormRow {
                        visible: !sheet.ownTransfer
                        label: qsTr("Empfänger")
                        TextField {
                            id: nameField
                            Layout.preferredWidth: 280; horizontalAlignment: TextInput.AlignRight
                            placeholderText: qsTr("Name")
                            text: sheet.draft.name
                            Accessible.name: qsTr("Empfänger")
                            onTextEdited: {
                                if (text !== sheet.pickedName) sheet.pickedName = null
                                sheet.nameEdited = true
                                Payments.setStanding({ name: text })
                                sheet.refreshSuggestions()
                            }
                            onActiveFocusChanged: sheet.refreshSuggestions()
                            // the recipient list: ↑/↓, Return, Esc
                            Keys.onDownPressed: e => e.accepted = sheet.moveHighlight(1)
                            Keys.onUpPressed: e => e.accepted = sheet.moveHighlight(-1)
                            Keys.onEscapePressed: e => { e.accepted = sheet.suggestions.length > 0; sheet.suggestions = [] }
                            Sensitive {}
                        }
                    }
                    FormRow {
                        visible: !sheet.ownTransfer
                        label: qsTr("IBAN")
                        // .withBankLine: the bank under field and mark, the line kept while there is none (#1584)
                        ColumnLayout {
                            spacing: 4
                            RowLayout {
                                spacing: 8
                                TextField {
                                    Layout.preferredWidth: 280; horizontalAlignment: TextInput.AlignRight
                                    placeholderText: qsTr("DE00 …")
                                    font.family: "monospace"
                                    inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhPreferUppercase
                                    text: sheet.draft.iban
                                    onTextEdited: Payments.setStanding({ iban: AppModel.groupedIBAN(text) })
                                    Sensitive {}
                                }
                                // FieldValidity: nothing while the field is empty (the place stays)
                                ValidMark { opacity: sheet.draft.iban !== "" ? 1 : 0; ok: sheet.ibanValid
                                            ToolTip.text: sheet.ibanValid ? qsTr("Prüfsumme korrekt") : qsTr("IBAN ungültig (Prüfsumme/Länge)") }
                            }
                            IBANBankLine { iban: sheet.draft.iban; Layout.preferredWidth: 1 }
                        }
                    }
                    FormRow {
                        visible: !sheet.ownTransfer
                        label: qsTr("BIC")
                        RowLayout {
                            spacing: 8
                            TextField {
                                // 8 or 11 characters: as wide as the amount (HIG: size fits the input)
                                Layout.preferredWidth: 160 * Theme.textScale; horizontalAlignment: TextInput.AlignRight
                                placeholderText: qsTr("optional")
                                font.family: "monospace"
                                inputMethodHints: Qt.ImhNoPredictiveText
                                text: sheet.draft.bic
                                onTextEdited: Payments.setStanding({ bic: text })
                                Sensitive {}
                            }
                            ValidMark { opacity: sheet.bicText !== "" ? 1 : 0; ok: sheet.bicValid
                                        ToolTip.text: sheet.bicValid ? qsTr("BIC gültig") : qsTr("BIC ungültig") }
                        }
                    }
                }
                FormSection {
                    FormRow {
                        label: qsTr("Betrag")
                        TextField {
                            Layout.preferredWidth: 160; horizontalAlignment: TextInput.AlignRight
                            placeholderText: "0,00"
                            font.features: { "tnum": 1 }
                            text: sheet.draft.amountText
                            onTextEdited: Payments.setStanding({ amountText: text })
                            Sensitive { amounts: true }
                        }
                    }
                    FormRow {
                        label: qsTr("Verwendungszweck")
                        TextField {
                            Layout.preferredWidth: 280; horizontalAlignment: TextInput.AlignRight
                            placeholderText: qsTr("optional")
                            maximumLength: 140
                            text: sheet.draft.purpose
                            onTextEdited: Payments.setStanding({ purpose: text })
                            Sensitive {}
                        }
                    }
                }
                FormSection {
                    FormRow {
                        label: qsTr("Rhythmus")
                        AppComboBox {
                            Layout.preferredWidth: 200
                            model: sheet.rhythmChoices.map(r => Payments.rhythmText(r))
                            currentIndex: sheet.rhythmChoices.findIndex(r => r.unit === sheet.draft.unit && r.interval === sheet.draft.interval)
                            // .onChange(of: unit): another unit fits the day to it and to the bank's limits
                            onActivated: i => {
                                const r = sheet.rhythmChoices[i]
                                Payments.setStanding({ unit: r.unit, interval: r.interval }, r.unit !== sheet.draft.unit ? "unit" : undefined)
                            }
                        }
                    }
                    FormRow {
                        label: sheet.draft.unit === "W" ? qsTr("Wochentag") : qsTr("Ausführungstag")
                        AppComboBox {
                            Layout.preferredWidth: 200
                            model: sheet.dayChoices.map(c => c.title)
                            currentIndex: sheet.dayChoices.findIndex(c => c.day === sheet.draft.execDay)
                            // The first execution and the execution day describe the same schedule: the one changed last wins.
                            onActivated: i => Payments.setStanding({ execDay: sheet.dayChoices[i].day }, "date")
                        }
                    }
                    FormRow {
                        label: qsTr("Erste Ausführung")
                        DateField {
                            date: sheet.draft.firstDate
                            minimum: sheet.firstRange.from
                            maximum: sheet.firstRange.to
                            onPicked: d => Payments.setStanding({ firstDate: d }, "day")
                        }
                    }
                    FormRow {
                        label: qsTr("Mit Enddatum")
                        Switch { checked: sheet.draft.hasEnd; onToggled: Payments.setStanding({ hasEnd: checked }) }
                    }
                    FormRow {
                        visible: sheet.draft.hasEnd
                        label: qsTr("Letzte Ausführung")
                        DateField {
                            date: sheet.draft.lastDate
                            minimum: sheet.draft.firstDate
                            onPicked: d => Payments.setStanding({ lastDate: d })
                        }
                    }
                }
            }

            // MARK: review
            ColumnLayout {
                id: review
                visible: sheet.reviewing
                x: 20; y: 20; width: scroll.availableWidth - 40
                spacing: 12
                Note {
                    visible: sheet.isDelete; symbol: "exclamationmark.triangle"
                    text: qsTr("Der Dauerauftrag wird bei der Bank gelöscht. Danach werden keine weiteren Zahlungen mehr ausgeführt.")
                }
                Card {
                    Layout.fillWidth: true
                    ColumnLayout {
                        width: parent.width
                        spacing: 10
                        Text { text: Fmt.money(sheet.cents === null ? 0 : sheet.cents); font.pixelSize: Theme.title; font.weight: Font.DemiBold
                               font.features: { "tnum": 1 }; color: Theme.text; Sensitive { amounts: true } }
                        Text { text: Payments.rhythmText(sheet.draft) + " · " + Payments.execDayText(sheet.draft)
                               font.pixelSize: Theme.callout; color: Theme.secondary }
                        Line { label: qsTr("Von Konto"); value: sheet.account ? sheet.account.name + " · " + Fmt.iban(sheet.account.iban) : "" }
                        Line { label: qsTr("Empfänger"); value: sheet.draft.name }
                        Line { label: qsTr("IBAN"); value: sheet.draft.iban; mono: true }
                        Line { readonly property string bank: AppModel.bankNameForIban(sheet.draft.iban) || ""
                               visible: bank !== ""; label: qsTr("Bank"); value: bank }
                        Line { visible: sheet.draft.purpose !== ""; label: qsTr("Verwendungszweck"); value: sheet.draft.purpose }
                        Line { label: qsTr("Erste Ausführung"); value: Fmt.medium(Fmt.iso(sheet.draft.firstDate)); personal: false }
                        Line { label: qsTr("Letzte Ausführung"); personal: false
                               value: sheet.draft.hasEnd ? Fmt.medium(Fmt.iso(sheet.draft.lastDate)) : qsTr("bis auf Widerruf") }
                    }
                }
                TextField {
                    visible: !sheet.pinStored
                    Layout.preferredWidth: 360 * Theme.textScale
                    echoMode: TextInput.Password
                    placeholderText: qsTr("Banking-PIN") + " (" + qsTr("wird nicht gespeichert") + ")"
                    text: sheet.draft.pin
                    onTextEdited: Payments.setStanding({ pin: text })
                }
                Note {
                    visible: !sheet.isDelete; symbol: "exclamationmark.shield"
                    text: qsTr("Bitte Empfänger und IBAN genau prüfen. Die Bank bestätigt den Auftrag mit einer TAN.")
                }
            }
        }

        RowLayout {
            id: messages
            visible: AppModel.transferError !== ""
            Layout.fillWidth: true
            Layout.leftMargin: 20; Layout.rightMargin: 20; Layout.topMargin: 8; Layout.bottomMargin: 8
            spacing: 6
            SFIcon { symbol: "exclamationmark.triangle.fill"; size: 14; color: Theme.negative; Layout.alignment: Qt.AlignTop }
            TextEdit { Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WordWrap
                       text: AppModel.transferError; color: Theme.negative; font.pixelSize: Theme.callout }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.border }

        // MARK: buttons
        RowLayout {
            id: buttonRow
            Layout.fillWidth: true
            Layout.margins: 14
            spacing: 8
            Button { text: qsTr("Abbrechen"); onClicked: Payments.cancelStandingSheet() }
            Item { Layout.fillWidth: true }
            Button { visible: sheet.reviewing && !sheet.isDelete; text: qsTr("Zurück"); onClicked: { sheet.reviewing = false; AppModel.transferError = "" } }
            // HIG: a destructive action is never the default button (Return does nothing here)
            Button {
                visible: sheet.isDelete
                text: qsTr("Dauerauftrag löschen")
                palette.buttonText: Theme.negative
                enabled: !AppModel.transferBusy
                onClicked: Payments.startStandingOrder()
            }
            Button {
                visible: sheet.reviewing && !sheet.isDelete
                text: qsTr("Jetzt mit TAN freigeben"); highlighted: true
                enabled: !AppModel.transferBusy
                onClicked: sheet.start()
            }
            Button {
                id: nextButton
                visible: !sheet.reviewing
                text: qsTr("Weiter"); highlighted: true
                enabled: Payments.standingIsValid(sheet.draft)
                onClicked: {
                    AppModel.transferError = ""
                    Payments.invalidateStandingImports()  // an invoice still being read must not change what is under review
                    sheet.suggestions = []
                    sheet.reviewing = true
                }
            }
        }
    }

    // .keyboardShortcut(.defaultAction) – not for a deletion
    Shortcut {
        sequences: ["Return", "Enter"]
        enabled: sheet.visible && !sheet.isDelete && !templatesButton.dialogOpen && !recipientsDialog.visible
        onActivated: {
            // Return takes the recipient chosen with ↑/↓
            if (sheet.editing && sheet.highlighted >= 0 && sheet.highlighted < sheet.suggestions.length) {
                sheet.pick(sheet.suggestions[sheet.highlighted])
                return
            }
            if (!sheet.reviewing) { if (nextButton.enabled) nextButton.clicked() } else sheet.start()
        }
    }

    RecipientSuggestionList {
        id: suggestionList
        parent: sheet.contentItem
        active: sheet.editing && !sheet.ownTransfer
        field: nameField
        scrollView: scroll
        suggestions: sheet.suggestions
        highlighted: sheet.highlighted
        onPicked: r => sheet.pick(r)
        onHideRequested: iban => sheet.hideRecipient(iban)
        onHoveredRow: i => sheet.highlighted = i
    }

    // .capturableSheet { KnownRecipientsSheet() }: what was hidden or shown again there applies to an open list at once
    KnownRecipientsDialog { id: recipientsDialog; onClosed: sheet.refreshSuggestions() }

    // .onDrop: Rechnung (PDF, ZUGFeRD-XML, Bild oder PDF mit GiroCode) hierher ziehen.
    DropArea {
        parent: sheet.contentItem
        anchors.fill: parent
        enabled: sheet.editing && !sheet.readingInvoice
        onDropped: drop => { if (drop.hasUrls) sheet.importInvoice(drop.urls[0]) }
        Rectangle {
            anchors.fill: parent; anchors.margins: 4
            visible: parent.containsDrag
            radius: 12; color: Theme.withAlpha(Theme.accent, 0.06)
            border.color: Theme.accent; border.width: 2
        }
    }

    // Linux only, as in the transfer form: while a dropped invoice is read, a spinner over the
    // form says so (the footer text can be scrolled out of view here).
    Rectangle {
        parent: sheet.contentItem
        anchors.fill: parent
        visible: sheet.readingInvoice
        z: 10
        radius: 12; color: Theme.withAlpha(Theme.card, 0.8)
        MouseArea { anchors.fill: parent; acceptedButtons: Qt.AllButtons; hoverEnabled: true; onWheel: w => w.accepted = true }
        Column {
            anchors.centerIn: parent
            spacing: 10
            BusyIndicator { anchors.horizontalCenter: parent.horizontalCenter; running: parent.parent.visible
                            implicitWidth: 40; implicitHeight: 40 }
            Text { anchors.horizontalCenter: parent.horizontalCenter; text: qsTr("Lese Rechnung …")
                   font.pixelSize: Theme.body; color: Theme.text }
        }
    }

    // RootView: .alert("Status des Dauerauftrags unklar")
    Sheet {
        id: unknownDialog
        reportCapturable: false   // an alert on the Mac (ReportDialogs.qml)
        sheetWidth: 440
        visible: Payments.standingUnknown !== null
        onClosed: Payments.standingUnknown = null
        ColumnLayout {
            width: unknownDialog.availableWidth
            spacing: 12
            Text { text: qsTr("Status des Dauerauftrags unklar"); font.pixelSize: Theme.body + 2; font.weight: Font.Bold; color: Theme.text
                   wrapMode: Text.WordWrap; Layout.fillWidth: true }
            TextEdit {
                Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.WordWrap; color: Theme.text; font.pixelSize: Theme.body
                text: qsTr("Der Auftrag hat die Bank erreicht, aber die Antwort ist nicht eindeutig. Bitte zuerst im Online-Banking prüfen, ob der Dauerauftrag angelegt, geändert oder gelöscht wurde, bevor Sie es erneut versuchen.\n\nBank: %1")
                      .arg(Payments.standingUnknown || "")
            }
            RowLayout {
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Verstanden"); highlighted: true; onClicked: unknownDialog.close() }
            }
        }
    }

    /// FieldValidity: the check mark or cross beside the IBAN or BIC field, the tooltip as its label.
    component ValidMark: SFIcon {
        property bool ok
        symbol: ok ? "checkmark.circle.fill" : "xmark.circle.fill"
        size: 16
        color: ok ? Theme.inflow : Theme.negative
        HoverHandler { id: hover }
        ToolTip.visible: hover.hovered && opacity > 0 && ToolTip.text !== ""
        Accessible.role: Accessible.Graphic
        Accessible.name: ToolTip.text
        Accessible.ignored: opacity === 0
    }
    component Note: RowLayout {
        property alias text: noteText.text
        property alias symbol: noteIcon.symbol
        Layout.fillWidth: true
        spacing: 6
        SFIcon { id: noteIcon; size: 14; color: Theme.secondary; Layout.alignment: Qt.AlignTop }
        Text { id: noteText; Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.callout; color: Theme.secondary }
    }
    /// line(_:_:): label above, the value never truncated (recipient and IBAN are what the person checks).
    component Line: ColumnLayout {
        property string label
        property string value
        property bool mono: false
        property bool personal: true
        Layout.fillWidth: true
        spacing: 2
        Text { text: parent.label; font.pixelSize: Theme.caption; color: Theme.secondary }
        TextEdit {
            Layout.fillWidth: true; readOnly: true; selectByMouse: true; wrapMode: Text.Wrap
            text: parent.value; font.pixelSize: Theme.callout; color: Theme.text
            font.family: parent.mono ? "monospace" : Qt.application.font.family
            Sensitive { when: parent.parent.personal }
        }
    }
}

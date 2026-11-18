import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// ExportSheet.swift – Umsätze exportieren (GitHub #53 / #372, compact dialog #831):
// one grouped form: format → period → accounts → options. It opens on the last period
// and accounts (limited to the sidebar's scope); „Exportieren …“ hands over to the save
// dialog (Main.qml), several files to a folder dialog.
// PDF = white-label Umsatzliste; CAMT.053 needs exactly one account (#372), no loan or depot (#483).
Sheet {
    id: sheet
    sheetWidth: 460
    padding: 0

    // .formStyle(.grouped) on macOS: bold section headers, hairlines between the rows.
    component GroupedSection: FormSection {
        separators: true
        headerFont.pixelSize: Theme.body; headerFont.weight: Font.Bold; headerColor: Theme.text
    }
    component ToggleRow: FormRow {
        id: tr
        property bool checked
        signal toggled(bool on)
        Switch { checked: tr.checked; onToggled: tr.toggled(checked) }
    }

    property var options: BookingExport.load()
    property bool busy: false
    /// Export error, shown on the sheet itself ("" = none; the app's alert would stay hidden behind it).
    property string failure: ""
    property bool showDetails: false
    /// Booking count for the current choices, loaded on a worker thread (`null` while
    /// loading). Refreshed only when `previewKey` changes.
    property var previewCount: null
    property int previewGeneration: 0
    /// id -> true
    property var selectedIDs: ({})
    property date customFromDate: new Date()
    property date customToDate: new Date()
    /// Saved account choice, read once; a context-menu export writes it back unchanged.
    property var savedAccountIDs: []
    /// isSelectionExport when the sheet opened (finish() resets the model flag before onClosed).
    property bool openedForSelection: false

    function setOption(key, value) {
        const o = Object.assign({}, options)
        o[key] = value
        applyOptions(o)
    }
    /// `.onChange(of: options.format)`: CAMT.053 picks one account it can cover.
    function applyOptions(o) {
        const formatChanged = o.format !== options.format
        options = o
        if (formatChanged && BookingExport.isCamt(o.format) && !isSelectionExport && !camtAccountOK && firstCamtAccount) {
            const s = {}; s[firstCamtAccount.id] = true; selectedIDs = s
        }
    }

    /// The three format choices; CAMT.053 has a version of its own.
    readonly property string formatFamily: BookingExport.isCamt(options.format) ? "camt" : options.format
    function setFormatFamily(family) {
        if (family === "camt") { if (!BookingExport.isCamt(options.format)) setOption("format", "camt053v08") }
        else setOption("format", family)
    }

    readonly property var selectedIDList: Object.keys(selectedIDs).map(Number).sort((a, b) => a - b)
    /// The marked bookings: those of the context menu, else the list's selection.
    readonly property var markedIDs: {
        if (AppModel.exportBookingIDs !== null) return AppModel.exportBookingIDs
        const ids = Object.keys(AppModel.selectedBookingIDs).map(Number)
        return ids.length ? ids : (AppModel.selectedBookingID !== null ? [AppModel.selectedBookingID] : [])
    }
    readonly property int selectionCount: markedIDs.length
    onSelectionCountChanged: if (selectionCount === 0 && !isSelectionExport && options.scope === "selection") setOption("scope", "filtered")
    readonly property var exportIDs: options.scope === "selection" ? markedIDs : null

    /// Every active account; the sidebar scope only decides what is ticked on open.
    readonly property var poolAccounts: AppModel.allAccounts.filter(a => a.archived_unix === null || a.archived_unix === undefined)
    /// Same bank grouping as the sidebar (`bankGroupKey`), „Ohne Bank“ last.
    readonly property var accountsByBank: {
        const grouped = {}
        for (const a of poolAccounts) {
            const key = AppModel.bankGroupKey(a)
            if (!grouped[key]) grouped[key] = []
            grouped[key].push(a)
        }
        return Object.keys(grouped).sort((a, b) => {
            if ((a === "") !== (b === "")) return a === "" ? 1 : -1
            return a.localeCompare(b, undefined, { sensitivity: "base" })
        }).map(bank => ({ bank: bank === "" ? qsTr("Ohne Bank") : bank, accounts: grouped[bank].sort(BookingExport.byName) }))
    }
    readonly property var selectedAccounts: poolAccounts.filter(a => !!selectedIDs[a.id])

    /// Context-menu „Umsatz exportieren“ — lock to the marked bookings (no account/period chrome).
    readonly property bool isSelectionExport: AppModel.exportPreferSelection
    /// „Pro Konto eine Datei“ only when several accounts can produce separate files.
    readonly property bool showsOutputMode: options.scope !== "selection" && !BookingExport.isCamt(options.format)
        && selectedAccounts.length > 1
    readonly property bool isBulk: showsOutputMode && options.accountOutput === "perAccount"
    /// A loan's balance is no sum of its own bookings, a depot has none: CAMT.053
    /// refuses both in the core, so the picker does too (#483).
    function camtUsable(account) { return account.kind !== "Loan" && account.kind !== "Brokerage" }
    /// Exactly one account a CAMT.053 statement can cover.
    function camtSelectionOK(ids) {
        const account = ids.length === 1 ? AppModel.account(ids[0]) : null
        return !!account && camtUsable(account)
    }
    readonly property bool camtAccountOK: !BookingExport.isCamt(options.format) || camtSelectionOK(effectiveAccountIDs)
    /// First account a CAMT.053 statement can cover.
    readonly property var firstCamtAccount: selectedAccounts.find(camtUsable) || poolAccounts.find(camtUsable) || null
    readonly property bool canExport: !busy && camtAccountOK && effectiveAccountIDs.length > 0 && (isBulk || (previewCount || 0) > 0)

    readonly property var accountIDsForSelectedBookings: {
        const ids = exportIDs || []
        const out = {}
        for (const b of AppModel.bookings) if (ids.indexOf(b.id) >= 0) out[b.account_id] = true
        return Object.keys(out).map(Number).sort((a, b) => a - b)
    }
    /// Accounts used for the export query (selection → accounts of the marked bookings).
    readonly property var effectiveAccountIDs: options.scope === "selection" && accountIDsForSelectedBookings.length
        ? accountIDsForSelectedBookings : selectedIDList

    /// Column and layout toggles don't change the count, so they don't reload it.
    readonly property string previewKey: JSON.stringify({
        f: options.format, s: options.scope, p: options.periodPreset, cf: options.customFrom, ct: options.customTo,
        d: options.direction, a: effectiveAccountIDs, b: exportIDs })
    onPreviewKeyChanged: if (visible) previewTimer.restart()

    function period(o) { return o.scope === "selection" ? "current" : o.periodPreset }

    readonly property string countText: {
        if (options.scope !== "selection" && selectedIDList.length === 0) return qsTr("Bitte mindestens ein Konto wählen.")
        if (previewCount === null) return "…"
        if (previewCount === 0) return qsTr("Keine Umsätze")
        const rows = previewCount === 1 ? qsTr("1 Umsatz") : qsTr("%1 Umsätze").arg(previewCount)
        return isBulk ? qsTr("%1 in %2 Dateien").arg(rows).arg(selectedAccounts.length) : rows
    }
    readonly property string sheetTitle: isSelectionExport && selectionCount <= 1 ? qsTr("Umsatz exportieren") : qsTr("Umsätze exportieren")

    /// Saves the choices when the sheet closes. The context-menu export keeps the saved
    /// accounts (it ticks the marked bookings' accounts).
    function persistOptions() {
        const snapshot = Object.assign({}, options)
        snapshot.scope = "filtered"
        snapshot.selectedAccountIDs = openedForSelection ? savedAccountIDs : selectedIDList
        BookingExport.save(snapshot)
    }

    // MARK: period pop-up (the marked bookings or a period preset)
    readonly property var periodChoices: BookingExport.menuCases.filter(p => p !== "sinceSalary" || !!AppModel.salaryCycle)
    function periodTitle(preset) {
        if (preset === "current") return qsTr("Aktuelle Ansicht (%1)").arg(AppModel.periodTitle)
        if (preset === "sinceSalary" && AppModel.salaryCycle)
            return qsTr("Seit %1").arg(PrintReport.statementDate(AppModel.salaryCycle.last_date))
        return BookingExport.presetTitle(preset)
    }
    readonly property var periodEntries: {
        const out = []
        if (selectionCount > 0)
            out.push({ id: "selection", title: selectionCount === 1 ? qsTr("Markierter Umsatz") : qsTr("Markierte Umsätze (%1)").arg(selectionCount) })
        for (const p of periodChoices) out.push({ id: p, title: periodTitle(p) })
        return out
    }
    readonly property string periodChoice: options.scope === "selection" ? "selection" : options.periodPreset
    function setPeriodChoice(id) {
        const o = Object.assign({}, options)
        if (id === "selection") o.scope = "selection"
        else { o.scope = "filtered"; o.periodPreset = id }
        applyOptions(o)
        if (id === "custom") syncCustomDatesFromOptions()
    }
    function syncCustomDatesFromOptions() {
        if (!options.customFrom || !options.customTo) {
            const r = BookingExport.previousMonthRange()
            const o = Object.assign({}, options)
            o.customFrom = r[0] || Fmt.iso(new Date())
            o.customTo = r[1] || Fmt.iso(new Date())
            options = o
        }
        let from = Fmt.date(options.customFrom) || new Date(), to = Fmt.date(options.customTo) || new Date()
        if (from > to) { const t = from; from = to; to = t }
        customFromDate = from
        customToDate = to
    }
    function setCustomDate(isFrom, d) {
        if (isFrom) customFromDate = d; else customToDate = d
        setOption(isFrom ? "customFrom" : "customTo", Fmt.iso(d))
    }

    // MARK: accounts
    readonly property string accountSummary: {
        const n = selectedIDList.length
        if (n === 0) return qsTr("Keine")
        if (n === 1 && selectedAccounts.length) return selectedAccounts[0].name
        if (n === poolAccounts.length) return qsTr("Alle Konten")
        return qsTr("%1 Konten").arg(n)
    }

    // MARK: - Lifecycle
    function bootstrap() {
        options = BookingExport.load()
        failure = ""
        savedAccountIDs = options.selectedAccountIDs
        openedForSelection = isSelectionExport
        const o = Object.assign({}, options)
        let ids
        if (isSelectionExport) {
            // Context menu: only the marked bookings.
            o.scope = "selection"
            options = o
            ids = accountIDsForSelectedBookings.length ? accountIDsForSelectedBookings : poolAccounts.map(a => a.id)
        } else {
            // Menu / toolbar: period and accounts as last chosen; the accounts within the
            // sidebar's scope (an account or group), else all of the sidebar's accounts.
            o.scope = "filtered"
            options = o
            if (o.periodPreset === "custom") syncCustomDatesFromOptions()
            const pool = poolAccounts.map(a => a.id)
            const sidebar = AppModel.selectionAccountIDs()
            const scope = sidebar.length ? sidebar.filter(id => pool.indexOf(id) >= 0) : pool
            ids = o.selectedAccountIDs.filter(id => scope.indexOf(id) >= 0)
            if (!ids.length) ids = scope
            if (BookingExport.isCamt(o.format) && !camtSelectionOK(ids)) {
                selectedIDs = ({})
                ids = firstCamtAccount ? [firstCamtAccount.id] : []
            }
            showDetails = BookingExport.hasNonDefaultContent(o)
        }
        const s = {}
        for (const id of ids) s[id] = true
        selectedIDs = s
        previewTimer.restart()
    }

    onAboutToShow: bootstrap()
    onClosed: {
        persistOptions()
        AppModel.showExportSheet = false
    }

    /// Loads the booking count on a worker thread; a newer choice makes an older load stale.
    Timer {
        id: previewTimer
        interval: 60
        onTriggered: sheet.refreshPreview()
    }
    function refreshPreview() {
        const gen = ++previewGeneration
        const o = options, accountIDs = effectiveAccountIDs, bookingIDs = exportIDs
        if (!accountIDs.length) { previewCount = 0; return }
        previewCount = null
        const preset = period(o)
        BookingExport.loadExportBookings(accountIDs, preset, bookingIDs, o, BookingExport.isCamt(o.format), (bookings, balances, tagNote) => {
            if (gen !== previewGeneration) return
            previewCount = BookingExport.exportReport(bookings, accountIDs, preset, bookingIDs, o, tagNote).rows.length
        }, () => {
            if (gen !== previewGeneration) return
            previewCount = 0
        })
    }

    /// A CAMT.053 refusal of the core (InvalidInput) arrives as its plain message (dispatch.rs).
    function fail(message) {
        busy = false
        failure = message
    }
    /// Closes the sheet, then runs `present` (the save dialog must not open over the sheet).
    /// close() unloads the sheet (lazy Loader in Main.qml): hand over BookingExport's function
    /// itself, not a closure over this sheet's context.
    function finish(present, a) {
        busy = false
        AppModel.exportBookingIDs = null
        AppModel.exportPreferSelection = false
        close()
        Qt.callLater(present, a)
    }
    function exportFiles() {
        if (!canExport) return
        const ids = effectiveAccountIDs
        busy = true
        const o = Object.assign({}, options)
        if (isBulk) {
            // One file per account; accounts without bookings in the period are skipped
            // (one after the other: the PDF renderer takes one job at a time).
            const files = []
            const next = i => {
                if (i >= ids.length) {
                    if (!files.length) { fail(qsTr("Keine Umsätze für den Export.")); return }
                    finish(BookingExport.presentExportFiles, files)
                    return
                }
                BookingExport.buildFile([ids[i]], null, o, file => { if (file) files.push(file); next(i + 1) }, fail)
            }
            next(0)
            return
        }
        BookingExport.buildFile(ids, exportIDs, o, file => {
            if (!file) { fail(qsTr("Keine Umsätze für den Export.")); return }
            finish(BookingExport.presentExportFile, file)
        }, fail)
    }

    ColumnLayout {
        width: sheet.availableWidth
        spacing: 0

        // navigationTitle + toolbar (cancellation / confirmation)
        RowLayout {
            id: head
            Layout.fillWidth: true
            Layout.margins: 12
            spacing: 8
            Button { text: qsTr("Abbrechen"); enabled: !sheet.busy; onClicked: sheet.close() }
            Item { Layout.fillWidth: true }
            Text { text: sheet.sheetTitle; font.pixelSize: Theme.body + 1; font.weight: Font.DemiBold; color: Theme.text }
            Item { Layout.fillWidth: true }
            Button {
                text: sheet.busy ? qsTr("Wird erstellt …") : qsTr("Exportieren …"); highlighted: true
                enabled: sheet.canExport
                onClicked: sheet.exportFiles()
                Shortcut { sequences: ["Return", "Enter"]; enabled: sheet.visible && parent.enabled; onActivated: sheet.exportFiles() }
            }
        }
        Rectangle { Layout.fillWidth: true; height: 1; color: Theme.border }

        ScrollView {
            id: scroll
            ScrollBar.vertical: ScrollViewBar {}
            Layout.fillWidth: true
            Layout.preferredHeight: Math.min(form.implicitHeight + 32, sheet.maxBodyHeight - head.implicitHeight - 25)
            contentWidth: availableWidth
            clip: true
            ColumnLayout {
                id: form
                x: 16; y: 16; width: scroll.availableWidth - 32
                spacing: 18

                // MARK: formatSection
                GroupedSection {
                    footer: BookingExport.format(sheet.options.format).subtitle
                    FormRow {
                        label: qsTr("Format")
                        Segmented {
                            options: [{ id: "pdf", title: qsTr("PDF") }, { id: "csv", title: "CSV" }, { id: "camt", title: qsTr("CAMT.053") }]
                            current: sheet.formatFamily
                            onActivated: id => sheet.setFormatFamily(id)
                        }
                    }
                    FormRow {
                        visible: BookingExport.isCamt(sheet.options.format)
                        label: qsTr("Version")
                        AppComboBox {
                            Layout.preferredWidth: 230
                            model: [qsTr("Aktuell"), qsTr("Älter, für ältere Programme")]
                            currentIndex: sheet.options.format === "camt053v02" ? 1 : 0
                            onActivated: i => sheet.setOption("format", i === 1 ? "camt053v02" : "camt053v08")
                        }
                    }
                }

                // MARK: scopeSection
                GroupedSection {
                    footer: BookingExport.isCamt(sheet.options.format) && !sheet.camtAccountOK
                        ? (sheet.effectiveAccountIDs.length === 1 ? qsTr("Für Darlehen und Depots gibt es keinen CAMT.053-Auszug.")
                                                                  : qsTr("CAMT.053 gilt immer für genau ein Konto."))
                        : sheet.options.scope === "selection" && BookingExport.isCamt(sheet.options.format)
                        ? qsTr("CAMT.053 für markierte Buchungen enthält deren Zeitraum mit den echten Salden des Kontos.")
                        : sheet.isBulk ? qsTr("Du wählst gleich einen Ordner für die Dateien.") : ""
                    FormRow {
                        visible: sheet.isSelectionExport
                        label: sheet.selectionCount === 1 ? qsTr("Dieser Umsatz") : qsTr("Auswahl")
                        value: sheet.selectionCount === 1 ? qsTr("1 Buchung") : qsTr("%1 Buchungen").arg(sheet.selectionCount)
                    }
                    FormRow {
                        visible: !sheet.isSelectionExport
                        label: qsTr("Zeitraum")
                        AppComboBox {
                            Layout.preferredWidth: 250
                            textRole: "title"
                            model: sheet.periodEntries
                            currentIndex: sheet.periodEntries.findIndex(e => e.id === sheet.periodChoice)
                            onActivated: i => sheet.setPeriodChoice(sheet.periodEntries[i].id)
                        }
                    }
                    FormRow {
                        visible: !sheet.isSelectionExport && sheet.options.scope === "filtered" && sheet.options.periodPreset === "custom"
                        label: qsTr("Von")
                        DateField { date: sheet.customFromDate; maximum: sheet.customToDate; onPicked: d => sheet.setCustomDate(true, d) }
                    }
                    FormRow {
                        visible: !sheet.isSelectionExport && sheet.options.scope === "filtered" && sheet.options.periodPreset === "custom"
                        label: qsTr("Bis")
                        DateField { date: sheet.customToDate; minimum: sheet.customFromDate; onPicked: d => sheet.setCustomDate(false, d) }
                    }
                    FormRow {
                        visible: !sheet.isSelectionExport && sheet.options.scope === "filtered"
                        label: qsTr("Konten")
                        // accountButton: summary + chevron, opens the account popover
                        Item {
                            id: accountButton
                            implicitWidth: accountRow.implicitWidth; implicitHeight: accountRow.implicitHeight + 4
                            Layout.maximumWidth: 260
                            RowLayout {
                                id: accountRow
                                anchors.verticalCenter: parent.verticalCenter
                                width: Math.min(implicitWidth, parent.width)
                                spacing: 4
                                Text { text: sheet.accountSummary; color: Theme.text; font.pixelSize: Theme.body
                                       elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                                Text { text: "▾"; font.pixelSize: Theme.caption; color: Theme.secondary }   // chevron.up.chevron.down
                            }
                            MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor
                                        onClicked: accountPicker.visible ? accountPicker.close() : accountPicker.open() }
                            ExportAccountPicker {
                                id: accountPicker
                                groups: sheet.accountsByBank
                                selectedIDs: sheet.selectedIDs
                                singleChoice: BookingExport.isCamt(sheet.options.format)
                                isUsable: a => !BookingExport.isCamt(sheet.options.format) || sheet.camtUsable(a)
                                onPicked: s => sheet.selectedIDs = s
                            }
                        }
                    }
                    ToggleRow {
                        visible: !sheet.isSelectionExport && sheet.options.scope === "filtered" && sheet.showsOutputMode
                        label: qsTr("Pro Konto eine Datei")
                        checked: sheet.options.accountOutput === "perAccount"
                        onToggled: on => sheet.setOption("accountOutput", on ? "perAccount" : "combined")
                    }
                    FormRow {
                        label: qsTr("Umfang")
                        value: sheet.countText
                    }
                }

                // MARK: detailsSection – DisclosureGroup „Optionen“
                GroupedSection {
                    visible: sheet.options.format === "pdf" || sheet.options.format === "csv"
                    footer: sheet.options.format === "pdf" ? qsTr("Anfangssaldo und Endsaldo stehen immer im Kopf.") : ""
                    Item {
                        Layout.fillWidth: true
                        implicitHeight: 24
                        RowLayout {
                            anchors.fill: parent
                            spacing: 6
                            Text { text: qsTr("Optionen"); color: Theme.text; font.pixelSize: Theme.body; Layout.fillWidth: true }
                            SFIcon { symbol: sheet.showDetails ? "chevron.down" : "chevron.right"; size: 10; color: Theme.secondary }
                        }
                        MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: sheet.showDetails = !sheet.showDetails }
                    }
                    ToggleRow { visible: sheet.showDetails; label: qsTr("Kategorie")
                                checked: sheet.options.showCategory; onToggled: on => sheet.setOption("showCategory", on) }
                    ToggleRow { visible: sheet.showDetails; label: qsTr("Notizen")
                                checked: sheet.options.showNote; onToggled: on => sheet.setOption("showNote", on) }
                    ToggleRow { visible: sheet.showDetails && sheet.options.format === "csv"; label: qsTr("IBAN der Gegenseite")
                                checked: sheet.options.showIban; onToggled: on => sheet.setOption("showIban", on) }
                    ToggleRow { visible: sheet.showDetails && sheet.options.format === "csv"; label: qsTr("Tags")
                                checked: sheet.options.showTags; onToggled: on => sheet.setOption("showTags", on) }
                    ToggleRow { visible: sheet.showDetails; label: qsTr("Tagessaldo-Spalte")
                                checked: sheet.options.showDailyBalance; onToggled: on => sheet.setOption("showDailyBalance", on) }
                    // the balance after each booking, CSV only (#577)
                    ToggleRow { visible: sheet.showDetails && sheet.options.format === "csv"; label: qsTr("Saldo-Spalte (nach jeder Buchung)")
                                checked: sheet.options.showRunningBalance; onToggled: on => sheet.setOption("showRunningBalance", on) }
                    ToggleRow { visible: sheet.showDetails && sheet.options.format === "pdf"; label: qsTr("Nach Konto gruppieren")
                                checked: sheet.options.groupByAccount; onToggled: on => sheet.setOption("groupByAccount", on) }
                    ToggleRow { visible: sheet.showDetails; label: qsTr("Nur erste Zeile Verwendungszweck")
                                checked: sheet.options.firstLineOnly; onToggled: on => sheet.setOption("firstLineOnly", on) }
                    FormRow {
                        visible: sheet.showDetails
                        label: qsTr("Reihenfolge")
                        AppComboBox {
                            Layout.preferredWidth: 170
                            model: [qsTr("Älteste zuerst"), qsTr("Neueste zuerst")]
                            currentIndex: sheet.options.newestFirst ? 1 : 0
                            onActivated: i => sheet.setOption("newestFirst", i === 1)
                        }
                    }
                }
            }
        }
    }

    // .alert("Export fehlgeschlagen", isPresented: failure != nil)
    Sheet {
        id: failureDialog
        reportCapturable: false   // an alert on the Mac (ReportDialogs.qml)
        sheetWidth: 420
        visible: sheet.failure !== ""
        onClosed: sheet.failure = ""
        ColumnLayout {
            width: failureDialog.availableWidth
            spacing: 12
            Text { text: qsTr("Export fehlgeschlagen"); font.pixelSize: Theme.body + 2; font.weight: Font.Bold; color: Theme.text
                   wrapMode: Text.WordWrap; Layout.fillWidth: true }
            TextEdit { text: sheet.failure; readOnly: true; selectByMouse: true; wrapMode: Text.WordWrap; color: Theme.text
                       font.pixelSize: Theme.body; Layout.fillWidth: true }
            RowLayout {
                Item { Layout.fillWidth: true }
                Button { text: qsTr("OK"); highlighted: true; onClicked: failureDialog.close() }
            }
        }
    }
}

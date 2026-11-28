import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// OfflineAccountsUI.swift › offlineAccountSheets(_:) (#679, #81): "Neues Offline-Konto …",
// "Tilgung übernehmen" (#1981), "Saldo korrigieren …" and the question of "Buchung löschen …",
// driven by OfflineAccounts.
// The booking editor (ManualBookingSheet.qml) is loaded by Main.qml.
Item {
    // OfflineAccountSheet: name, kind, starting balance (a loan: original amount and remaining debt) and note
    Sheet {
        id: newAccount
        sheetWidth: 440
        visible: OfflineAccounts.showNewAccount
        property string kind: "Cash"
        readonly property bool isLoan: kind === "Loan"
        onAboutToShow: {
            kind = "Cash"
            nameField.text = ""; balanceField.text = ""; loanOriginalField.text = ""; loanRemainingField.text = ""; noteField.text = ""
            Qt.callLater(() => nameField.forceActiveFocus())
        }
        onClosed: OfflineAccounts.showNewAccount = false
        /// An empty field is a starting balance of 0; a typo is not. A loan: the remaining debt (≥ 0).
        readonly property var openingCents: {
            if (isLoan) { const c = Fmt.parseCents(loanRemainingField.text); return c !== null && c >= 0 ? c : null }
            return balanceField.text.trim() === "" ? 0 : Fmt.parseCents(balanceField.text.trim())
        }
        readonly property var originalLoanCents: {
            if (!isLoan) return null
            const c = Fmt.parseCents(loanOriginalField.text)
            return c !== null && c > 0 ? c : null
        }
        readonly property bool canCreateAccount: openingCents !== null && (!isLoan || originalLoanCents !== null)
        function create() {
            if (!canCreateAccount) return
            const cents = openingCents, kind = newAccount.kind, original = originalLoanCents
            const trimmed = nameField.text.trim()
            const name = trimmed === "" ? OfflineAccounts.kindTitle(kind) : nameField.text, note = noteField.text
            close()
            OfflineAccounts.createOfflineAccount(name, note, cents, kind, original)
        }
        ColumnLayout {
            width: newAccount.availableWidth
            spacing: 14
            Text { text: qsTr("Neues Offline-Konto"); font.pixelSize: Theme.title3; font.weight: Font.Bold; color: Theme.text; Accessible.role: Accessible.Heading }
            Text { text: qsTr("Ein Konto ohne Bankanbindung. Die Buchungen tragen Sie selbst ein.")
                   font.pixelSize: Theme.callout; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true }
            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: 12; rowSpacing: 8
                Text { text: qsTr("Kontoname"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                TextField { id: nameField; Layout.fillWidth: true; placeholderText: OfflineAccounts.kindTitle(newAccount.kind); onAccepted: newAccount.create(); Sensitive {} }
                Text { text: qsTr("Kontoart"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                AppComboBox {
                    Layout.fillWidth: true
                    model: OfflineAccounts.kinds
                    textRole: "title"
                    currentIndex: Math.max(0, OfflineAccounts.kinds.findIndex(k => k.id === newAccount.kind))
                    onActivated: i => newAccount.kind = OfflineAccounts.kinds[i].id
                }
                Text { visible: !newAccount.isLoan; text: qsTr("Startsaldo"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                TextField { id: balanceField; visible: !newAccount.isLoan; Layout.fillWidth: true; placeholderText: Fmt.plain(0); horizontalAlignment: TextInput.AlignRight
                            font.features: { "tnum": 1 }; onAccepted: newAccount.create(); Sensitive { amounts: true } }
                Text { visible: newAccount.isLoan; text: qsTr("Ursprünglicher Kreditbetrag"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                TextField { id: loanOriginalField; visible: newAccount.isLoan; Layout.fillWidth: true; horizontalAlignment: TextInput.AlignRight
                            font.features: { "tnum": 1 }; onAccepted: newAccount.create(); Sensitive { amounts: true } }
                Text { visible: newAccount.isLoan; text: qsTr("Restschuld zum Start"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                TextField { id: loanRemainingField; visible: newAccount.isLoan; Layout.fillWidth: true; horizontalAlignment: TextInput.AlignRight
                            font.features: { "tnum": 1 }; onAccepted: newAccount.create(); Sensitive { amounts: true } }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Text { text: qsTr("Konto-Notiz"); font.pixelSize: Theme.body; color: Theme.text }
                ScrollView {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 60
                    ScrollBar.vertical: ScrollViewBar {}
                    TextArea {
                        id: noteField
                        wrapMode: TextEdit.Wrap
                        font.pixelSize: Theme.body
                        background: Rectangle { radius: 6; color: "transparent"; border.color: Theme.withAlpha(Theme.tertiary, 0.5) }
                        Sensitive {}
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Abbrechen"); onClicked: newAccount.close() }
                Button { text: qsTr("Anlegen"); highlighted: true; enabled: newAccount.canCreateAccount; onClicked: newAccount.create() }
            }
        }
    }

    // OfflineLoanPaymentSheet (#1981): confirms the principal of a source payment before creating its
    // counter-booking in the offline loan
    Sheet {
        id: loanPayment
        sheetWidth: 440
        readonly property var request: OfflineAccounts.loanPaymentRequest
        property string saveError: ""
        /// The typed principal: > 0 and at most the debited amount, null otherwise.
        readonly property var principalCents: {
            const c = Fmt.parseCents(principalField.text)
            return request && c !== null && c > 0 && c <= Math.abs(request.booking.amount_cents) ? c : null
        }
        visible: request !== null
        onAboutToShow: {
            principalField.text = ""; saveError = ""
            Qt.callLater(() => principalField.forceActiveFocus())
        }
        onClosed: OfflineAccounts.loanPaymentRequest = null
        function save() {
            const cents = principalCents
            if (cents === null) return
            try {
                OfflineAccounts.mirrorLoanPayment(request, cents)
                close()
            } catch (e) {
                saveError = AppModel.parseCoreError(e.message).message
            }
        }
        ColumnLayout {
            width: loanPayment.availableWidth
            spacing: 14
            Text { text: qsTr("Tilgung übernehmen"); font.pixelSize: Theme.title3; font.weight: Font.Bold; color: Theme.text; Accessible.role: Accessible.Heading }
            GridLayout {
                Layout.fillWidth: true
                columns: 2
                columnSpacing: 12; rowSpacing: 8
                Text { text: qsTr("Konto"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                Text { text: loanPayment.request ? loanPayment.request.account.name : ""; font.pixelSize: Theme.body; color: Theme.text
                       elide: Text.ElideRight; Layout.fillWidth: true; Sensitive {} }
                Text { text: qsTr("Abbuchung"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                Text { text: loanPayment.request ? Fmt.money(loanPayment.request.booking.amount_cents, loanPayment.request.account.currency) : ""
                       font.pixelSize: Theme.body; font.features: { "tnum": 1 }; color: Theme.text; Layout.fillWidth: true; Sensitive { amounts: true } }
                Text { text: qsTr("Tilgungsanteil"); font.pixelSize: Theme.body; color: Theme.text; Layout.alignment: Qt.AlignRight }
                TextField { id: principalField; Layout.fillWidth: true; horizontalAlignment: TextInput.AlignRight; font.features: { "tnum": 1 }
                            onAccepted: loanPayment.save(); Sensitive { amounts: true } }
            }
            Text { text: qsTr("Geben Sie den Anteil der Rate ein, der die Restschuld senkt.")
                   font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true }
            Text { visible: loanPayment.saveError !== ""; text: loanPayment.saveError; font.pixelSize: Theme.caption; color: Theme.negative
                   wrapMode: Text.WordWrap; Layout.fillWidth: true }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Abbrechen"); onClicked: loanPayment.close() }
                Button { text: qsTr("Übernehmen"); highlighted: true; enabled: loanPayment.principalCents !== null; onClicked: loanPayment.save() }
            }
        }
    }

    // BalanceCorrectionSheet: books the difference to the real balance as a correction booking
    Sheet {
        id: correction
        sheetWidth: 440
        readonly property var request: OfflineAccounts.correctionRequest
        readonly property var account: request ? (AppModel.accountsByID[request.accountId] || null) : null
        readonly property var targetCents: Fmt.parseCents(targetField.text)
        visible: request !== null
        onAboutToShow: {
            // read the request directly: aboutToShow comes before the bindings above are updated
            const req = OfflineAccounts.correctionRequest
            const a = req ? AppModel.accountsByID[req.accountId] : null
            targetField.text = a ? Fmt.plain(a.balance_cents) : ""
            Qt.callLater(() => { targetField.forceActiveFocus(); targetField.selectAll() })
        }
        onClosed: OfflineAccounts.correctionRequest = null
        function apply() {
            const cents = targetCents
            if (cents === null || !request) return
            const id = request.accountId
            close()
            OfflineAccounts.correctBalance(id, cents)
        }
        ColumnLayout {
            width: correction.availableWidth
            spacing: 14
            Text { text: qsTr("Saldo korrigieren"); font.pixelSize: Theme.title3; font.weight: Font.Bold; color: Theme.text; Accessible.role: Accessible.Heading }
            Text {
                visible: correction.account !== null
                text: correction.account ? qsTr("Aktueller Saldo: %1").arg(Fmt.money(correction.account.balance_cents, correction.account.currency)) : ""
                font.pixelSize: Theme.callout; color: Theme.secondary
                Sensitive { amounts: true }
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: 12
                Text { text: qsTr("Tatsächlicher Saldo"); font.pixelSize: Theme.body; color: Theme.text }
                TextField { id: targetField; Layout.fillWidth: true; horizontalAlignment: TextInput.AlignRight; font.features: { "tnum": 1 }
                            onAccepted: correction.apply(); Sensitive { amounts: true } }
            }
            Text { text: qsTr("Der Unterschied wird als Buchung „Saldokorrektur“ mit heutigem Datum angelegt. Den Startsaldo ändern Sie über „Einstellungen …“ im Kontextmenü des Kontos.")
                   font.pixelSize: Theme.caption; color: Theme.tertiary; wrapMode: Text.WordWrap; Layout.fillWidth: true }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Abbrechen"); onClicked: correction.close() }
                Button { text: qsTr("Korrigieren"); highlighted: true; enabled: correction.targetCents !== null; onClicked: correction.apply() }
            }
        }
    }

    // .alert("Buchung löschen?")
    ConfirmDialog {
        visible: OfflineAccounts.deleting !== null
        heading: qsTr("Buchung löschen?")
        message: qsTr("Die Buchung wird aus dem Konto entfernt und der Saldo neu berechnet. Das lässt sich nicht widerrufen.")
        actionTitle: qsTr("Löschen")
        // kept here: the dialog closes (and clears `deleting`) before `confirmed` arrives
        property var booking: null
        onAboutToShow: booking = OfflineAccounts.deleting
        onConfirmed: { const b = booking; booking = null; if (b) OfflineAccounts.deleteManualBooking(b) }
        onClosed: OfflineAccounts.deleting = null
    }
}

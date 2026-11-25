import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// ExternalAccessSettings.swift: Einstellungen › Analyse (#136). Read-only access for local
// programs (analysis tools, AI assistants) over a local socket. Off by default; accounts are
// an explicit selection, IBANs/account numbers and notes stay hidden unless switched on. The
// access token is shown once, only its hash is stored. State and queue: ExternalAccess.qml.
// Linux: „diesem Rechner“ instead of „diesem Mac“; no App-Translocation hint; the MCP entry
// also names the socket (girocrab-mcp knows a default path only on macOS).
ScrollView {
    ScrollBar.vertical: ScrollViewBar {}
    id: pane
    contentWidth: availableWidth

    readonly property var scope: ExternalAccess.scope
    readonly property bool hasToken: ExternalAccess.hasToken
    readonly property bool scopeEditable: !!scope && !ExternalAccess.scopeReadFailed
    /// `if scope.enabled { accountsSection … }`
    readonly property bool enabledScope: AppModel.holdsRealData && !!scope && scope.enabled
    property var clientConfig: null
    property string currentHelperPath: ExternalAccess.helperPath
    function refreshAssistantConfig() {
        if (!shown || !hasToken) {
            clientConfig = null
            currentHelperPath = ExternalAccess.helperPath
            return
        }
        clientConfig = AppModel.coreOrNull("local_api_client_config", { token: ExternalAccess.newToken || null })
        currentHelperPath = ExternalAccess.helperPath
    }
    /// .onAppear / .onDisappear of the pane (tab switch or window closed)
    readonly property bool shown: visible && AppModel.showSettings
    onShownChanged: {
        if (shown) {
            ExternalAccess.sessionGeneration++
            ExternalAccess.reconcile()
            refreshAssistantConfig()
            return
        }
        ExternalAccess.forgetNewToken()
        forgetCheck()
    }
    readonly property var config: clientConfig

    // CheckState: "" (idle), "running", or a LocalApiCheck name ("works", "failed", …)
    property string check: ""
    property string checkMessage: ""
    property int checkGeneration: 0
    // A new or revoked key makes an earlier (or running) test stale.
    Connections {
        target: ExternalAccess
        function onNewTokenChanged() { pane.forgetCheck(); pane.refreshAssistantConfig() }
        function onHasTokenChanged() { pane.forgetCheck(); pane.refreshAssistantConfig() }
    }

    function startCheck() {
        AppModel.coreOrNull("local_api_check_cancel")
        const gen = ++checkGeneration
        check = "running"
        AppModel.coreAsync("local_api_check", { token: ExternalAccess.newToken || null }, r => {
            if (gen !== pane.checkGeneration) return
            pane.checkMessage = r.message || ""
            pane.check = r.result
        }, err => {
            if (gen !== pane.checkGeneration) return
            pane.checkMessage = err
            pane.check = "failed"
        })
    }
    /// Cancelling also ends the helper process.
    function forgetCheck() {
        if (check === "running") AppModel.coreOrNull("local_api_check_cancel")
        checkGeneration++
        check = ""
    }
    function checkText(result) {
        switch (result) {
        case "works": return qsTr("Die Verbindung funktioniert: Das Hilfsprogramm erreicht GiroCrab, und der Schlüssel wird angenommen.")
        case "reachableWithoutKey": return qsTr("GiroCrab ist erreichbar. Den Schlüssel kann der Test nur direkt nach dem Erzeugen prüfen; ein falscher Schlüssel zeigt sich sonst beim ersten Abruf Ihres Programms.")
        case "notReachable": return qsTr("Nicht erreichbar: Die Schnittstelle läuft gerade nicht (siehe Status oben).")
        case "keyRejected": return qsTr("Der Schlüssel wird nicht angenommen. Erzeugen Sie einen neuen.")
        case "turnedOff": return qsTr("Die externe Analyse ist ausgeschaltet.")
        case "locked": return qsTr("GiroCrab ist gesperrt.")
        case "busy": return qsTr("Alle Verbindungen sind belegt. Versuchen Sie es gleich noch einmal.")
        case "noAnswer": return qsTr("Das Hilfsprogramm hat nicht rechtzeitig geantwortet. Versuchen Sie es noch einmal.")
        default: return qsTr("Der Test ist fehlgeschlagen: %1").arg(checkMessage)
        }
    }
    function status() {
        if (!hasToken) return qsTr("Wartet auf einen Zugangsschlüssel")
        if (ExternalAccess.startError !== "") return qsTr("Nicht gestartet: %1").arg(ExternalAccess.startError)
        if (scope.account_ids.length === 0) return qsTr("Aktiv, noch kein Konto geteilt")
        return ExternalAccess.running ? qsTr("Aktiv") : qsTr("Wird gestartet …")
    }
    /// A switch of the scope; every change is saved at once.
    function setField(key, on) {
        if (!scope) return
        const s = Object.assign({}, scope)
        s[key] = on
        ExternalAccess.updateScope(s)
    }
    function setAccount(id, on) {
        if (!scope) return
        const s = Object.assign({}, scope)
        s.account_ids = scope.account_ids.filter(x => x !== id)
        if (on) s.account_ids.push(id)
        ExternalAccess.updateScope(s)
    }

    // .confirmationDialog("Zugriff widerrufen?")
    ConfirmDialog {
        id: confirmRevoke
        heading: qsTr("Zugriff widerrufen?")
        message: qsTr("Programme mit dem bisherigen Zugangsschlüssel erhalten sofort keine Daten mehr. Für neuen Zugriff erzeugen Sie einen neuen Schlüssel.")
        actionTitle: qsTr("Widerrufen")
        onConfirmed: ExternalAccess.revokeToken()
    }

    component Note: Text {
        Layout.fillWidth: true; wrapMode: Text.WordWrap
        font.pixelSize: Theme.callout; color: Theme.secondary
    }
    /// Label(…, systemImage: "exclamationmark.triangle") / "checkmark.circle"
    component IconNote: RowLayout {
        property alias text: label.text
        property string symbol: "exclamationmark.triangle"
        Layout.fillWidth: true; spacing: 6
        SFIcon { symbol: parent.symbol; size: 14; color: Theme.secondary; Layout.alignment: Qt.AlignTop }
        Text { id: label; Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.callout; color: Theme.text }
    }

    ColumnLayout {
        width: pane.availableWidth - 40
        x: 20
        spacing: 18
        Item { implicitHeight: 2 }

        SettingsFormSection {
            visible: !AppModel.holdsRealData
            title: qsTr("Externe Analyse")
            Note { text: qsTr("Die externe Analyse ist nur mit Ihrem eigenen, entsperrten Tresor verfügbar – nicht mit den Demo-Daten.") }
        }

        SettingsFormSection {
            visible: AppModel.holdsRealData && ExternalAccess.scopeReadFailed
            title: qsTr("Externe Analyse")
            IconNote {
                text: ExternalAccess.startError !== ""
                    ? qsTr("Einstellungen konnten nicht geladen werden: %1").arg(ExternalAccess.startError)
                    : qsTr("Einstellungen konnten nicht geladen werden.")
            }
            Button {
                text: qsTr("Erneut versuchen")
                onClicked: ExternalAccess.reconcile()
            }
        }

        // accessSection
        SettingsFormSection {
            visible: AppModel.holdsRealData && !!pane.scope
            enabled: pane.scopeEditable
            title: qsTr("Externe Analyse")
            CheckBox {
                text: qsTr("Lesenden Zugriff für Programme auf diesem Rechner erlauben")
                checked: !!pane.scope && pane.scope.enabled
                onToggled: pane.setField("enabled", checked)
            }
            Note { text: qsTr("Programme auf diesem Rechner, die Sie einrichten, können ausgewählte Konten, Buchungen, Kategorien und Summen lesen. Sie können nichts ändern, keine Zahlungen auslösen und sehen keine Zugangsdaten. Der Zugriff besteht nur, solange GiroCrab läuft und der Tresor entsperrt ist.") }
            IconNote { text: qsTr("Programme wie KI-Assistenten können die geteilten Daten an einen Cloud-Dienst übertragen. Teilen Sie nur, was dort verarbeitet werden darf.") }
            Note { text: qsTr("Beim Ausschalten wird der Zugangsschlüssel widerrufen. Zum erneuten Einschalten erzeugen Sie einen neuen.") }
            SettingsFormRow {
                visible: !!pane.scope && pane.scope.enabled
                label: qsTr("Status")
                value: visible ? pane.status() : ""
            }
        }

        // accountsSection
        SettingsFormSection {
            visible: pane.enabledScope
            enabled: pane.scopeEditable
            title: qsTr("Geteilte Konten")
            Repeater {
                model: AppModel.groups
                ColumnLayout {
                    id: groupRows
                    required property var modelData
                    Layout.fillWidth: true
                    spacing: 8
                    Repeater {
                        model: groupRows.modelData.accounts
                        RowLayout {
                            id: accountRow
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: 2
                            CheckBox {
                                id: box
                                checked: !!pane.scope && pane.scope.account_ids.indexOf(accountRow.modelData.id) >= 0
                                onToggled: pane.setAccount(accountRow.modelData.id, checked)
                                Accessible.name: accountRow.modelData.name
                            }
                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2
                                Text { text: accountRow.modelData.name; font.pixelSize: Theme.body; color: Theme.text
                                       Layout.fillWidth: true; elide: Text.ElideRight
                                       Sensitive {} }
                                Text { text: groupRows.modelData.name; font.pixelSize: Theme.caption; color: Theme.secondary
                                       Layout.fillWidth: true; elide: Text.ElideRight
                                       Sensitive { labels: true } }
                                TapHandler { onTapped: { box.toggle(); box.toggled() } }
                            }
                        }
                    }
                }
            }
            Note { text: qsTr("Neue Konten werden erst geteilt, wenn Sie sie hier auswählen.") }
        }

        // fieldsSection
        SettingsFormSection {
            visible: pane.enabledScope
            enabled: pane.scopeEditable
            title: qsTr("Geteilte Felder")
            Repeater {
                model: [{ key: "share_counterparty", title: qsTr("Empfänger und Auftraggeber") },
                        { key: "share_purpose", title: qsTr("Verwendungszweck und Buchungstext") },
                        { key: "share_account_numbers", title: qsTr("IBANs und Kontonummern") },
                        { key: "share_notes", title: qsTr("Notizen") }]
                CheckBox {
                    required property var modelData
                    text: modelData.title
                    checked: !!pane.scope && pane.scope[modelData.key]
                    onToggled: pane.setField(modelData.key, checked)
                }
            }
            Note { text: qsTr("Beträge, Daten, Kategorien und Salden werden immer geteilt. Ohne IBANs und Kontonummern werden auch lange Ziffernfolgen in Texten unkenntlich gemacht. Die Suche steht nur zur Verfügung, wenn die ersten drei Felder geteilt sind.") }
        }

        // tokenSection
        SettingsFormSection {
            visible: pane.enabledScope
            enabled: pane.scopeEditable
            title: qsTr("Zugangsschlüssel")
            SettingsFormRow {
                visible: ExternalAccess.newToken !== ""
                label: qsTr("Neuer Schlüssel")
                fillValue: true
                TextInput {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 220 * Theme.textScale
                    text: ExternalAccess.newToken
                    // starts at „gcapi_…“ like the Mac's truncated line (lineLimit(1))
                    readOnly: true; selectByMouse: true; clip: true; autoScroll: false
                    font.family: "monospace"; font.pixelSize: Theme.caption; color: Theme.text
                    Sensitive {}
                }
                IconButton {
                    symbol: "doc.on.doc"
                    ToolTip.text: qsTr("Kopieren")
                    Accessible.name: qsTr("Kopieren")
                    // VaultKeyManager.copyToPasteboard: cleared from the clipboard after 60 s
                    onClicked: AppModel.copySecret(ExternalAccess.newToken)
                }
            }
            Note {
                visible: ExternalAccess.newToken !== ""
                text: qsTr("Der Schlüssel wird nur jetzt angezeigt. Tragen Sie ihn im Programm ein, das zugreifen soll. Kopiert bleibt er 60 Sekunden in der Zwischenablage.")
            }
            RowLayout {
                spacing: 8
                Button {
                    text: !pane.hasToken ? qsTr("Zugangsschlüssel erzeugen") : qsTr("Neuen Schlüssel erzeugen")
                    onClicked: ExternalAccess.createToken()
                }
                Button {
                    visible: pane.hasToken
                    text: qsTr("Zugriff widerrufen")
                    palette.buttonText: Theme.negative
                    onClicked: confirmRevoke.open()
                }
            }
            SettingsFormRow {
                label: qsTr("Socket")
                value: ExternalAccess.socketPath || "–"
                selectable: true
            }
        }

        // assistantSection: setup for an MCP client – the helper, a ready entry for the
        // client's configuration and a test of the whole path.
        SettingsFormSection {
            visible: pane.enabledScope && pane.hasToken
            title: qsTr("Für KI-Assistenten")
            Note { text: qsTr("Für KI-Assistenten und andere MCP-Clients liegt GiroCrab ein Hilfsprogramm bei. Tragen Sie den folgenden Eintrag in die MCP-Konfiguration Ihres Programms ein; es startet das Hilfsprogramm dann selbst.") }
            TextEdit {
                visible: pane.config !== null
                Layout.fillWidth: true
                text: pane.config || ""
                readOnly: true; selectByMouse: true
                wrapMode: TextEdit.WrapAnywhere
                font.family: "monospace"; font.pixelSize: Theme.caption; color: Theme.text
                Sensitive {}   // the key, and a path that may name the user
            }
            Note {
                visible: pane.config !== null && ExternalAccess.newToken === ""
                text: qsTr("Den Schlüssel sehen Sie nur direkt nach dem Erzeugen: Ersetzen Sie „gcapi_…“ durch Ihren Schlüssel, oder erzeugen Sie einen neuen, dann steht er bereits im Eintrag.")
            }
            RowLayout {
                visible: pane.config !== null
                spacing: 8
                Button {
                    text: qsTr("Eintrag kopieren")
                    // With the key in it: cleared from the clipboard after 60 s.
                    onClicked: ExternalAccess.newToken !== "" ? AppModel.copySecret(pane.config) : AppModel.copyToClipboard(pane.config)
                }
                Button {
                    text: qsTr("Programmpfad kopieren")
                    enabled: pane.currentHelperPath !== ""
                    onClicked: AppModel.copyToClipboard(pane.currentHelperPath)
                }
                Button {
                    text: qsTr("Verbindung testen")
                    enabled: pane.check !== "running"
                    onClicked: pane.startCheck()
                }
            }
            Spinner { visible: pane.check === "running"; size: 16 }
            IconNote {
                visible: pane.check !== "" && pane.check !== "running"
                symbol: pane.check === "works" ? "checkmark.circle" : "exclamationmark.triangle"
                text: visible ? pane.checkText(pane.check) : ""
            }
            ColumnLayout {
                visible: pane.config === null
                Layout.fillWidth: true
                spacing: 8
                Note {
                    text: ExternalAccess.devBuild
                        ? qsTr("Dieser Entwickler-Build enthält das Hilfsprogramm nicht. Bauen Sie es mit „cargo build --release -p girocrab-mcp“.")
                        : qsTr("Das Hilfsprogramm ist in diesem Build nicht enthalten.")
                }
                Button {
                    text: qsTr("Erneut suchen")
                    onClicked: pane.refreshAssistantConfig()
                }
            }
        }
    }
}

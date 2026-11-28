import QtCore
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

// RuleTagExchangeUI.swift › RuleTagExchange (#1057): rules and tags as a JSON file – the save
// and open panels, the choice of what to import and the summary after it. Shared by
// RuleEditorView.qml and TagManagerSheet.qml; call `exportRules(ids)`, `exportTags(ids)` (empty =
// all) or `startImport()`. The file is read and written in the core (dispatch.rs), the import
// reloads rules and tags (AppModel+RuleExchange.swift › importRulesAndTags).
Item {
    id: ex
    /// { kind: "rules" | "tags", ids } while the save panel is open.
    property var exportRequest: null
    /// { json, preview } of the file read, while the user chooses what to import.
    property var pending: null
    /// RuleExchangeReport of the last import, while the summary is open.
    property var report: null

    function exportRules(ids) { startExport({ kind: "rules", ids: ids }) }
    function exportTags(ids) { startExport({ kind: "tags", ids: ids }) }
    function startImport() { openDialog.open() }

    /// RuleTagExport.defaultFilename
    function defaultFilename(r) {
        if (r.kind === "rules") return r.ids.length === 1 ? qsTr("GiroCrab-Regel") : qsTr("GiroCrab-Regeln")
        return r.ids.length === 1 ? qsTr("GiroCrab-Tag") : qsTr("GiroCrab-Tags")
    }
    function startExport(request) {
        exportRequest = request
        saveDialog.selectedFile = saveDialog.currentFolder + "/" + defaultFilename(request) + ".json"
        saveDialog.open()
    }
    function fail(title, err) {
        AppModel.errorTitle = title
        AppModel.errorMessage = AppModel.parseCoreError(err).message
    }
    function reloadRulesAndTags() { TagsModel.reload(); AppModel.reloadRules() }

    // .fileExporter(contentType: .json, defaultFilename:)
    FileDialog {
        id: saveDialog
        title: qsTr("Exportieren …")
        fileMode: FileDialog.SaveFile
        defaultSuffix: "json"
        nameFilters: [qsTr("JSON-Datei (*.json)")]
        currentFolder: StandardPaths.writableLocation(StandardPaths.DocumentsLocation)
        onAccepted: {
            const r = ex.exportRequest
            ex.exportRequest = null
            AppModel.coreAsync("export_rules_tags_file", { kind: r.kind, ids: r.ids, path: AppModel.localPath(selectedFile) },
                               () => {}, err => ex.fail(qsTr("Export fehlgeschlagen"), err))
        }
        onRejected: ex.exportRequest = null
    }

    // .fileImporter(allowedContentTypes: [.json]) → AppModel.previewRulesTagsFile
    FileDialog {
        id: openDialog
        title: qsTr("Importieren …")
        fileMode: FileDialog.OpenFile
        nameFilters: [qsTr("JSON-Datei (*.json)"), qsTr("Alle Dateien (*)")]
        currentFolder: StandardPaths.writableLocation(StandardPaths.DocumentsLocation)
        onAccepted: AppModel.coreAsync("preview_rules_tags_file", { path: AppModel.localPath(selectedFile) }, r => {
            ex.pending = r
            selectionSheet.open()
        }, err => ex.fail(qsTr("Import fehlgeschlagen"), err))
    }

    /// AppModel+RuleExchange.swift › importRulesAndTags
    function runImport(json, rules, tags) {
        AppModel.coreAsync("import_rules_and_tags_selected", { json: json, rule_indices: rules, tag_names: tags }, r => {
            reloadRulesAndTags()
            report = r
            reportSheet.open()
        }, err => {
            // tags created before the failure are in the vault: show them
            reloadRulesAndTags()
            fail(qsTr("Import fehlgeschlagen"), err)
        })
    }

    // MARK: - RuleExchangeSelectionSheet: every rule and tag of the file, all ticked; untick what
    // should stay out. The tags a ticked rule adds always come along (ticked and locked).
    Sheet {
        id: selectionSheet
        sheetWidth: 520
        readonly property var preview: ex.pending ? ex.pending.preview : ({ rules: [], tags: [] })
        /// Ticked rule positions and tag names ({ key: true }).
        property var rules: ({})
        property var tags: ({})
        onAboutToShow: {
            const r = {}, t = {}
            for (const x of preview.rules) r[x.index] = true
            for (const x of preview.tags) t[x.name] = true
            rules = r; tags = t
        }
        onClosed: ex.pending = null
        readonly property var ruleIndices: Object.keys(rules).map(Number).sort((a, b) => a - b)
        /// requiredTags: tag keys a ticked rule that will be imported adds (a skipped one brings none).
        readonly property var required: {
            const keys = {}
            for (const r of preview.rules) if (rules[r.index] && r.problem === null)
                for (const name of r.tags) keys[TagsModel.tagKey(name)] = true
            return keys
        }
        function isLocked(tag) { return required[TagsModel.tagKey(tag.name)] === true }
        function isTicked(tag) { return tags[tag.name] === true || isLocked(tag) }
        readonly property var tickedTags: preview.tags.filter(t => isTicked(t)).map(t => t.name)
        function setAll(kind, on) {
            const s = {}
            if (on) for (const x of preview[kind]) s[kind === "rules" ? x.index : x.name] = true
            if (kind === "rules") rules = s; else tags = s
        }
        function toggle(kind, key, on) {
            const s = Object.assign({}, kind === "rules" ? rules : tags)
            if (on) s[key] = true; else delete s[key]
            if (kind === "rules") rules = s; else tags = s
        }

        ColumnLayout {
            width: selectionSheet.availableWidth
            spacing: 12
            Text { text: qsTr("Import auswählen"); font.pixelSize: Theme.body + 2; font.weight: Font.Bold; color: Theme.text }
            Text { text: qsTr("Alles ist ausgewählt. Was nicht importiert werden soll, abwählen.")
                   font.pixelSize: Theme.callout; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true }
            Flickable {
                id: selectionFlick
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(sections.implicitHeight, 380 * Theme.textScale)
                contentHeight: sections.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: AppScrollBar {}
                ColumnLayout {
                    id: sections
                    width: selectionFlick.width
                    spacing: 14
                    ColumnLayout {
                        visible: selectionSheet.preview.rules.length > 0
                        Layout.fillWidth: true
                        spacing: 6
                        SectionHeader {
                            title: qsTr("Regeln (%1 von %2)").arg(selectionSheet.ruleIndices.length).arg(selectionSheet.preview.rules.length)
                            onAll: selectionSheet.setAll("rules", true)
                            onNone: selectionSheet.setAll("rules", false)
                        }
                        Repeater {
                            model: selectionSheet.preview.rules
                            RowLayout {
                                id: ruleRow
                                required property var modelData
                                Layout.fillWidth: true
                                spacing: 6
                                CheckBox {
                                    id: ruleBox
                                    Layout.alignment: Qt.AlignTop
                                    checked: selectionSheet.rules[ruleRow.modelData.index] === true
                                    // the click broke the binding: the state lives in the sheet („Alle“ / „Keine“)
                                    onToggled: {
                                        selectionSheet.toggle("rules", ruleRow.modelData.index, ruleBox.checked)
                                        ruleBox.checked = Qt.binding(() => selectionSheet.rules[ruleRow.modelData.index] === true)
                                    }
                                    Accessible.name: ruleRow.modelData.name
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 1
                                    // rule names are mostly payees
                                    Text { text: ruleRow.modelData.name; font.pixelSize: Theme.body; color: Theme.text
                                           wrapMode: Text.WordWrap; Layout.fillWidth: true; Sensitive {} }
                                    Text { visible: ruleRow.modelData.category_path !== ""; text: "→ " + ruleRow.modelData.category_path
                                           font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true
                                           Sensitive { labels: true } }
                                    Text { visible: ruleRow.modelData.tags.length > 0; text: ruleRow.modelData.tags.map(t => "#" + t).join(" ")
                                           font.pixelSize: Theme.caption; color: Theme.secondary; wrapMode: Text.WordWrap; Layout.fillWidth: true
                                           Sensitive {} }
                                    Text { visible: text !== ""
                                           text: ruleRow.modelData.problem !== null ? qsTr("Wird übersprungen: %1").arg(ruleRow.modelData.problem)
                                                 : (ruleRow.modelData.note || "")
                                           font.pixelSize: Theme.caption; color: Theme.warning; wrapMode: Text.WordWrap; Layout.fillWidth: true
                                           Sensitive {} }
                                }
                            }
                        }
                    }
                    ColumnLayout {
                        visible: selectionSheet.preview.tags.length > 0
                        Layout.fillWidth: true
                        spacing: 6
                        SectionHeader {
                            title: qsTr("Tags (%1 von %2)").arg(selectionSheet.tickedTags.length).arg(selectionSheet.preview.tags.length)
                            onAll: selectionSheet.setAll("tags", true)
                            onNone: selectionSheet.setAll("tags", false)
                        }
                        Repeater {
                            model: selectionSheet.preview.tags
                            RowLayout {
                                id: tagRow
                                required property var modelData
                                readonly property bool locked: selectionSheet.isLocked(tagRow.modelData)
                                Layout.fillWidth: true
                                spacing: 6
                                CheckBox {
                                    id: tagBox
                                    Layout.alignment: Qt.AlignTop
                                    enabled: !tagRow.locked
                                    checked: selectionSheet.isTicked(tagRow.modelData)
                                    onToggled: {
                                        selectionSheet.toggle("tags", tagRow.modelData.name, tagBox.checked)
                                        tagBox.checked = Qt.binding(() => selectionSheet.isTicked(tagRow.modelData))
                                    }
                                    Accessible.name: "#" + tagRow.modelData.name
                                }
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 1
                                    // tag names are always covered in a report (#228)
                                    Text { text: "#" + tagRow.modelData.name; font.pixelSize: Theme.body; color: Theme.text
                                           wrapMode: Text.WordWrap; Layout.fillWidth: true; Sensitive {} }
                                    Text { text: tagRow.modelData.problem !== null ? qsTr("Wird übersprungen: %1").arg(tagRow.modelData.problem)
                                                 : tagRow.locked ? qsTr("von einer ausgewählten Regel verwendet")
                                                 : tagRow.modelData.exists ? qsTr("bereits vorhanden, wird übernommen") : qsTr("neu")
                                           font.pixelSize: Theme.caption; color: tagRow.modelData.problem !== null ? Theme.warning : Theme.secondary
                                           wrapMode: Text.WordWrap; Layout.fillWidth: true }
                                }
                            }
                        }
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: qsTr("Abbrechen"); onClicked: selectionSheet.close() }
                Button {
                    text: qsTr("Importieren (%1)").arg(selectionSheet.ruleIndices.length + selectionSheet.tickedTags.length)
                    highlighted: true
                    enabled: selectionSheet.ruleIndices.length + selectionSheet.tickedTags.length > 0
                    onClicked: {
                        const json = ex.pending.json, rules = selectionSheet.ruleIndices, tags = selectionSheet.tickedTags
                        selectionSheet.close()
                        ex.runImport(json, rules, tags)
                    }
                }
            }
        }
    }

    /// The title of a section with „Alle“ / „Keine“.
    component SectionHeader: RowLayout {
        id: sh
        property string title
        signal all()
        signal none()
        Layout.fillWidth: true
        Text { text: sh.title; font.pixelSize: Theme.subheadline; font.weight: Font.DemiBold; color: Theme.text; Layout.fillWidth: true }
        Button { flat: true; text: qsTr("Alle"); onClicked: sh.all() }
        Button { flat: true; text: qsTr("Keine"); onClicked: sh.none() }
    }

    // MARK: - RuleExchangeReportSheet: the counts, and below them „Details“ (folded by default)
    // with the rules and tags imported or skipped.
    Sheet {
        id: reportSheet
        sheetWidth: 460
        property bool showDetails: false
        readonly property var r: ex.report
        onAboutToShow: showDetails = false
        onClosed: ex.report = null
        /// summaryLines: rules only when the file had rules, tags only when it had tags.
        readonly property var summaryLines: {
            if (!r) return []
            const lines = []
            if (r.rules_imported + r.rules_skipped > 0)
                lines.push(qsTr("Regeln importiert: %1 · übersprungen: %2").arg(r.rules_imported).arg(r.rules_skipped))
            if (r.tags_created + r.tags_existing > 0 || r.tags_skipped_lines.length > 0)
                lines.push(qsTr("Tags importiert: %1 · übersprungen: %2").arg(r.tags_created).arg(r.tags_existing + r.tags_skipped_lines.length))
            if (lines.length === 0) lines.push(qsTr("Die Datei enthält keine Regeln oder Tags."))
            if (r.rules_imported > 0) lines.push(qsTr("Bestehende Buchungen ändern sich erst mit „Alle Regeln anwenden“."))
            return lines
        }
        /// detailSections: imported before skipped, rules before tags; empty ones left out.
        readonly property var sections: !r ? [] : [
            { title: qsTr("Importierte Regeln"), lines: r.rules_imported_lines },
            { title: qsTr("Übersprungene Regeln"), lines: r.rules_skipped_lines },
            { title: qsTr("Importierte Tags"), lines: r.tags_created_names },
            // already there (reused by the rules) or not creatable (invalid name)
            { title: qsTr("Übersprungene Tags"),
              lines: r.tags_existing_names.map(n => qsTr("„%1“: bereits vorhanden").arg(n)).concat(r.tags_skipped_lines) },
        ].filter(s => s.lines.length > 0)

        ColumnLayout {
            width: reportSheet.availableWidth
            spacing: 12
            Text { text: qsTr("Import abgeschlossen"); font.pixelSize: Theme.body + 2; font.weight: Font.Bold; color: Theme.text }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Repeater {
                    model: reportSheet.summaryLines
                    Text { required property string modelData
                           text: modelData; font.pixelSize: Theme.body; color: Theme.text; wrapMode: Text.WordWrap; Layout.fillWidth: true }
                }
            }
            // DisclosureGroup „Details“
            Item {
                visible: reportSheet.sections.length > 0
                Layout.fillWidth: true
                implicitHeight: 24
                RowLayout {
                    anchors.fill: parent
                    spacing: 6
                    SFIcon { symbol: reportSheet.showDetails ? "chevron.down" : "chevron.right"; size: 10; color: Theme.secondary }
                    Text { text: qsTr("Details"); font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text; Layout.fillWidth: true }
                }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: reportSheet.showDetails = !reportSheet.showDetails }
                Accessible.role: Accessible.Button
                Accessible.name: qsTr("Details")
            }
            Flickable {
                id: detailsFlick
                visible: reportSheet.showDetails && reportSheet.sections.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: Math.min(detailColumn.implicitHeight, 240 * Theme.textScale)
                contentHeight: detailColumn.implicitHeight
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: AppScrollBar {}
                ColumnLayout {
                    id: detailColumn
                    width: detailsFlick.width
                    spacing: 10
                    Repeater {
                        model: reportSheet.sections
                        ColumnLayout {
                            id: section
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: 2
                            Text { text: section.modelData.title; font.pixelSize: Theme.caption; font.weight: Font.DemiBold; color: Theme.secondary }
                            Repeater {
                                model: section.modelData.lines
                                // rule, account and tag names
                                Text { required property string modelData
                                       text: modelData; font.pixelSize: Theme.callout; color: Theme.text; wrapMode: Text.WordWrap
                                       Layout.fillWidth: true; Sensitive {} }
                            }
                        }
                    }
                }
            }
            RowLayout {
                Layout.fillWidth: true
                Item { Layout.fillWidth: true }
                Button { text: qsTr("OK"); highlighted: true; onClicked: reportSheet.close() }
            }
        }
    }
}

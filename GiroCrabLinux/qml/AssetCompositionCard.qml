import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AssetCompositionCard.swift (#564): "Zusammensetzung" of the depot and Sachwerte: the current
// inventory grouped by position, asset class or instrument. Unclassified items stay in the sum as
// their own grey slice; a search in the list never changes the base.
SliceCompositionCard {
    id: acc
    /// ClassifiedAsset list (the whole current inventory).
    property var assets: []
    /// The groupings on offer; with one entry there is no switch.
    property var modes: ["position", "assetClass", "instrument"]
    /// "position" | "assetClass" | "instrument"; the host owns it (`modeRequested`).
    property string mode: "position"
    signal modeRequested(string mode)

    readonly property int excluded: DepotMetrics.excludedCount(assets)
    readonly property int unclassified: assets.filter(a => a.classification.asset_class === "unclassified" && a.holding.value_cents > 0).length

    slices: DepotMetrics.compositionGroups(assets, mode)
    foldsRing: mode === "position"
    grouping: Segmented {
        visible: acc.modes.length > 1
        fontSize: Theme.caption
        options: acc.modes.map(m => ({ id: m, title: m === "position" ? qsTr("Nach Position") : m === "assetClass" ? qsTr("Nach Anlageklasse") : qsTr("Nach Instrument") }))
        current: acc.mode
        onActivated: id => { if (id !== acc.mode) { acc.modeRequested(id); acc.filterRequested(null) } }
    }
    footer: [
        Text {
            visible: acc.excluded > 0
            Layout.fillWidth: true; wrapMode: Text.WordWrap
            text: acc.excluded === 1 ? qsTr("1 Position ohne positiven Wert ist nicht dargestellt und nicht in der Summe.")
                                     : qsTr("%1 Positionen ohne positiven Wert sind nicht dargestellt und nicht in der Summe.").arg(acc.excluded)
            font.pixelSize: Theme.caption; color: Theme.secondary
        },
        Text {
            visible: acc.mode === "position" && acc.unclassified > 0
            Layout.fillWidth: true; wrapMode: Text.WordWrap
            text: acc.unclassified === 1 ? qsTr("1 Position ohne Klassifizierung – im Kontextmenü zuordnen.")
                                         : qsTr("%1 Positionen ohne Klassifizierung – im Kontextmenü einer Position zuordnen.").arg(acc.unclassified)
            font.pixelSize: Theme.caption; color: Theme.secondary
        }
    ]
}

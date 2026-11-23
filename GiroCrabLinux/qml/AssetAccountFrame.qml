import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AssetAccountFrame.swift (#564): page frame of the holdings views (securities depot, Sachwerte):
// banners, header, figures, analysis and the actual content, in one scroll view. The frame knows
// no account kind and loads nothing: each view brings its children (header, figures, analysis,
// content) in that order. The search is the toolbar field (DepotState.query), never the booking search.
Rectangle {
    id: frame
    color: Theme.canvas
    default property alias content: column.data
    /// Whether the window is wide enough for the table layouts (700 pt × text size).
    readonly property bool wide: column.width >= 700 * Theme.textScale
    property real minContentHeight: 0
    /// A click on the empty space around the cards (Sachwerte: ends the item selection).
    signal backgroundTapped()

    ColumnLayout {
        anchors.fill: parent
        spacing: 0
        BankingPolicyBanner { Layout.fillWidth: true }
        AnnouncementBanner { Layout.fillWidth: true }
        Flickable {
            id: flick
            Layout.fillWidth: true; Layout.fillHeight: true
            contentWidth: width
            contentHeight: Math.max(column.implicitHeight, frame.minContentHeight) + 36
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}
            // a background layer, so it never sees clicks on the rows
            MouseArea { width: flick.width; height: flick.contentHeight; onClicked: frame.backgroundTapped() }
            ColumnLayout {
                id: column
                x: 20; y: 18
                width: flick.width - 40
                spacing: 14
                // Marketing screenshots (`--screenshot`) look like a real vault: no demo banner.
                BookingStreamView.DemoBanner { visible: AppModel.demoActive && !AppModel.screenshotMode; Layout.fillWidth: true }
            }
        }
    }
}

import QtQuick
import QtQuick.Controls
import QtQuick.Effects

// PieHover.swift › hoverCardHost / HoverCardPlacement (#889): a card at the mouse pointer, drawn
// above everything (a card inside the chart would be clipped at its edge and covered by its
// neighbours). It lies right of and below the pointer and flips to the left or above where it
// would run over the window's edge. Takes no clicks. Look of GiroCrab's cards (PieDetailCard).
Rectangle {
    id: hc
    /// The item `point` is given in; the card shows while `shown` and `source` is set.
    property Item source: null
    property point point
    property bool shown: false
    default property alias content: column.data
    /// `.frame(minWidth:)` of the content
    property real minimumWidth: 0
    readonly property real gap: 14

    parent: Overlay.overlay
    visible: shown && source !== null && parent !== null && source.visible
    enabled: false
    z: 1000
    width: Math.max(minimumWidth, column.implicitWidth) + 16
    height: column.implicitHeight + 12
    radius: 8
    color: Theme.card
    border.color: Theme.border
    readonly property point at: visible ? source.mapToItem(parent, point.x, point.y) : Qt.point(0, 0)
    x: visible ? Math.max(at.x + gap + width > parent.width ? at.x - gap - width : at.x + gap, 0) : 0
    y: visible ? Math.max(at.y + gap + height > parent.height ? at.y - gap - height : at.y + gap, 0) : 0
    layer.enabled: visible
    layer.effect: MultiEffect {
        shadowEnabled: true
        shadowColor: "#000000"
        shadowOpacity: 0.12
        shadowBlur: 0.4
        shadowVerticalOffset: 2
    }
    Column {
        id: column
        x: 8; y: 6
        width: Math.max(hc.minimumWidth, implicitWidth)
        spacing: 2
    }
}

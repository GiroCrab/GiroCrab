import QtQuick
import QtQuick.Layouts

// DirectDebitsView.swift: "Lastschriften" – mandates and collection runs.
// Placeholder slot (WP0); mandate list, inspector and collection follow.
Rectangle {
    color: Theme.canvas
    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: 16
        spacing: 12
        Text { Layout.leftMargin: 20; text: qsTr("Lastschriften"); font.pixelSize: Theme.title2; font.weight: Font.DemiBold; color: Theme.text }
        // ContentUnavailableView
        Item {
            Layout.fillWidth: true; Layout.fillHeight: true
            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 420)
                spacing: 8
                SFIcon { Layout.alignment: Qt.AlignHCenter; symbol: "doc.text"; size: 36; color: Theme.tertiary }
                Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Keine Mandate"); font.pixelSize: Theme.title3; font.weight: Font.DemiBold; color: Theme.text }
                Text { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.WordWrap; font.pixelSize: Theme.body; color: Theme.secondary
                       text: qsTr("Lege ein Mandat an oder importiere eine Mitgliederliste (CSV) oder eine SEPA-Lastschriftdatei.") }
            }
        }
    }
}

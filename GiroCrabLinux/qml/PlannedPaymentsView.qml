import QtQuick
import QtQuick.Layouts

// PlannedPaymentsView.swift: "Geplant" – payments ordered at the bank, not yet booked.
// Placeholder slot (WP0); the list, totals and filters follow.
Rectangle {
    color: Theme.canvas
    ColumnLayout {
        anchors.fill: parent
        anchors.topMargin: 16
        spacing: 12
        ColumnLayout {
            Layout.leftMargin: 20; Layout.rightMargin: 20
            spacing: 4
            Text { text: qsTr("Geplant"); font.pixelSize: Theme.title2; font.weight: Font.DemiBold; color: Theme.text }
            Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                   text: qsTr("Zahlungen, die bei der Bank bereits beauftragt, aber noch nicht gebucht sind.") }
        }
        // ContentUnavailableView
        Item {
            Layout.fillWidth: true; Layout.fillHeight: true
            ColumnLayout {
                anchors.centerIn: parent
                width: Math.min(parent.width - 40, 420)
                spacing: 8
                SFIcon { Layout.alignment: Qt.AlignHCenter; symbol: "calendar.badge.clock"; size: 36; color: Theme.tertiary }
                Text { Layout.alignment: Qt.AlignHCenter; text: qsTr("Keine geplanten Zahlungen"); font.pixelSize: Theme.title3; font.weight: Font.DemiBold; color: Theme.text }
                Text { Layout.fillWidth: true; horizontalAlignment: Text.AlignHCenter; wrapMode: Text.WordWrap; font.pixelSize: Theme.body; color: Theme.secondary
                       text: qsTr("Terminüberweisungen und Daueraufträge deiner Bank erscheinen hier bis zur Ausführung.") }
            }
        }
    }
}

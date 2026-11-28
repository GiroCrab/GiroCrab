import QtQuick
import QtQuick.Controls

// DepotValueChart.swift (#578): the value line of one or more securities depots. A value holds
// until the next known one (a step line, never a smoothed curve: nothing is suggested between two
// valuations), a partial sum is dashed, and the line breaks where another depot joins. Amounts use
// the depots' currency. Hovering names the day, its value and the change since the start of the
// period. Drawn on a Canvas like BalanceChart.qml; the host covers it with `Sensitive { amounts: true }`.
Item {
    id: chart
    property var points: []
    property string currency: "EUR"
    /// The shown period as [from, to] Date objects: the axis spans it even where the first value
    /// comes later (null: the data).
    property var period: null
    /// The ISO day under the pointer ("" = none).
    property string hoveredDay: ""

    readonly property var segments: DepotValueHistory.segments(DepotValueHistory.drawn(points))
    /// The point whose value holds on the hovered day.
    readonly property var hovered: hoveredDay !== "" ? DepotValueHistory.valueOn(hoveredDay, points) : null

    /// yDomain: close to the values (a depot's value moves by a few percent: from zero the line
    /// would look flat), with a little air above and below. In cents.
    readonly property var yDomain: {
        if (!points.length) return { lo: 0, hi: 100 }
        let lo = Infinity, hi = -Infinity
        for (const p of points) { lo = Math.min(lo, p.value_cents); hi = Math.max(hi, p.value_cents) }
        const pad = Math.max((hi - lo) * 0.12, Math.abs(hi) * 0.004, 100)
        return { lo: lo - pad, hi: hi + pad }
    }
    /// xDomain in ms: the period, widened to the data, at least one day.
    readonly property var xDomain: {
        const first = points.length ? Fmt.date(points[0].date) : new Date()
        const last = points.length ? Fmt.date(points[points.length - 1].date) : first
        const lo = Math.min(period ? period[0].getTime() : first.getTime(), first.getTime())
        const hi = Math.max(period ? period[1].getTime() : last.getTime(), last.getTime())
        return { lo: lo, hi: Math.max(hi, lo + 86400000) }
    }
    /// More than about a year on the axis: the labels name the year (two "1. Jan." would be ambiguous).
    readonly property bool longSpan: xDomain.hi - xDomain.lo > 400 * 86400000

    /// AxisMarks(values: .automatic(desiredCount: 3)) on the value axis: round steps inside the domain.
    readonly property var yTicks: {
        const span = yDomain.hi - yDomain.lo
        if (span <= 0) return []
        const raw = span / 3
        const mag = Math.pow(10, Math.floor(Math.log10(raw)))
        const step = [1, 2, 2.5, 5, 10].map(f => f * mag).find(s => s >= raw) || 10 * mag
        const out = []
        for (let v = Math.ceil(yDomain.lo / step) * step; v <= yDomain.hi; v += step) out.push(v)
        return out
    }
    /// The same on the time axis: calendar steps (days, weeks or months) giving about three marks.
    readonly property var xTicks: {
        const lo = new Date(xDomain.lo), hi = xDomain.hi
        const days = (hi - xDomain.lo) / 86400000
        const marks = (dates) => dates.filter(d => d.getTime() >= xDomain.lo && d.getTime() <= hi)
        if (days <= 45) {
            for (const stride of [1, 2, 7, 14]) {
                const out = []
                const d = new Date(lo.getFullYear(), lo.getMonth(), lo.getDate())
                if (stride === 7 || stride === 14) d.setDate(d.getDate() + ((8 - d.getDay()) % 7))   // Mondays
                for (; d.getTime() <= hi; d.setDate(d.getDate() + stride)) out.push(new Date(d))
                const kept = marks(out)
                if (kept.length <= 4) return kept
            }
        }
        for (const stride of [1, 2, 3, 6, 12, 24, 60, 120]) {
            const out = []
            const start = new Date(lo.getFullYear(), stride >= 12 ? 0 : lo.getMonth(), 1)
            if (stride < 12) start.setMonth(start.getMonth() - start.getMonth() % stride)
            else start.setFullYear(start.getFullYear() - start.getFullYear() % (stride / 12))
            for (const d = start; d.getTime() <= hi; d.setMonth(d.getMonth() + stride)) out.push(new Date(d))
            const kept = marks(out)
            if (kept.length <= 3) return kept
        }
        return []
    }

    FontMetrics { id: metrics; font.pixelSize: Theme.caption2; font.features: { "tnum": 1 } }
    readonly property real labelWidth: {
        let w = 0
        for (const v of yTicks) w = Math.max(w, metrics.advanceWidth(Fmt.money(Math.round(v), currency)))
        return Math.ceil(w) + 6
    }
    readonly property real plotWidth: Math.max(1, width - labelWidth)
    readonly property int axisHeight: Theme.scaled(16)
    readonly property real plotHeight: Math.max(1, height - axisHeight)
    function xAt(ms) { return (ms - xDomain.lo) / (xDomain.hi - xDomain.lo) * plotWidth }
    function yAt(c) { return plotHeight - (c - yDomain.lo) / (yDomain.hi - yDomain.lo) * plotHeight }
    function xOfDay(iso) { return xAt(Fmt.date(iso).getTime()) }

    onPointsChanged: hoveredDay = ""
    onSegmentsChanged: canvas.requestPaint()
    onHoveredDayChanged: canvas.requestPaint()
    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()
    onYDomainChanged: canvas.requestPaint()
    onXDomainChanged: canvas.requestPaint()
    onVisibleChanged: if (!visible) hoveredDay = ""
    Connections { target: Theme; function onDarkChanged() { canvas.requestPaint() } }

    Canvas {
        id: canvas
        anchors.fill: parent
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            if (chart.plotWidth <= 1 || !chart.points.length) return
            // AxisGridLine per value mark
            ctx.save()
            ctx.strokeStyle = Theme.withAlpha(Theme.tertiary, 0.35); ctx.lineWidth = 0.5
            for (const v of chart.yTicks) {
                const y = Math.round(chart.yAt(v)) + 0.5
                ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(chart.plotWidth, y); ctx.stroke()
            }
            ctx.restore()
            // .interpolationMethod(.stepEnd): the value holds until the next point, then steps
            const trace = (seg) => {
                ctx.moveTo(chart.xOfDay(seg[0].date), chart.yAt(seg[0].value_cents))
                for (let i = 1; i < seg.length; i++) {
                    const x = chart.xOfDay(seg[i].date)
                    ctx.lineTo(x, chart.yAt(seg[i - 1].value_cents))
                    ctx.lineTo(x, chart.yAt(seg[i].value_cents))
                }
            }
            for (const seg of chart.segments) {
                const partial = DepotValueHistory.isPartial(seg)
                // a soft fill under the solid stretches; a partial sum is only a dashed line
                if (!partial && seg.length > 1) {
                    ctx.beginPath()
                    trace(seg)
                    ctx.lineTo(chart.xOfDay(seg[seg.length - 1].date), chart.plotHeight)
                    ctx.lineTo(chart.xOfDay(seg[0].date), chart.plotHeight)
                    ctx.closePath()
                    const g = ctx.createLinearGradient(0, 0, 0, chart.plotHeight)
                    g.addColorStop(0, Theme.withAlpha(Theme.accent, 0.14)); g.addColorStop(1, Theme.withAlpha(Theme.accent, 0.01))
                    ctx.fillStyle = g; ctx.fill()
                }
                ctx.save()
                ctx.beginPath()
                trace(seg)
                if (seg.length === 1) ctx.lineTo(chart.xOfDay(seg[0].date) + 1, chart.yAt(seg[0].value_cents))
                ctx.strokeStyle = Theme.accent; ctx.lineWidth = 2
                if (partial) ctx.setLineDash([2, 1.5])   // [4, 3] pt at line width 2
                ctx.stroke()
                ctx.restore()
            }
            // RuleMark and PointMark of the hovered day
            if (chart.hovered) {
                const x = chart.xOfDay(chart.hoveredDay)
                ctx.strokeStyle = Theme.tertiary; ctx.lineWidth = 1
                ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, chart.plotHeight); ctx.stroke()
                ctx.fillStyle = Theme.accent
                ctx.beginPath(); ctx.arc(x, chart.yAt(chart.hovered.value_cents), 3.5, 0, 2 * Math.PI); ctx.fill()
            }
        }
    }
    // chartYAxis: trailing labels in the depots' currency
    Repeater {
        model: chart.points.length ? chart.yTicks : []
        Text {
            required property var modelData
            text: Fmt.money(Math.round(modelData), chart.currency)
            font.pixelSize: Theme.caption2; font.features: { "tnum": 1 }; color: Theme.secondary
            x: chart.plotWidth + 4
            y: Math.max(0, Math.min(chart.plotHeight - height, chart.yAt(modelData) - height / 2))
            Sensitive { amounts: true }
        }
    }
    // chartXAxis: "Nov. 2026" over more than about a year, else "9. Nov."
    Repeater {
        model: chart.points.length ? chart.xTicks : []
        Text {
            required property var modelData
            text: modelData.toLocaleDateString(Fmt.locale, chart.longSpan ? "MMM yyyy" : Fmt.pattern("d. MMM"))
            font.pixelSize: Theme.caption2; color: Theme.secondary
            y: chart.plotHeight + 2
            x: Math.max(0, Math.min(chart.plotWidth - width, chart.xAt(modelData.getTime()) - width / 2))
        }
    }
    // readout: "09.11.2026 · 54.170,00 € · +1.200,00 € (+2,3 %)"; the change only against the same coverage
    Rectangle {
        id: readout
        readonly property var change: chart.hovered && chart.points.length ? DepotValueHistory.change(chart.hovered, chart.points[0]) : null
        visible: !!chart.hovered
        x: 0; y: 0
        width: readoutRow.implicitWidth + 12; height: readoutRow.implicitHeight + 4
        radius: 5
        color: Theme.withAlpha(Theme.card, 0.92); border.color: Theme.border
        Row {
            id: readoutRow
            anchors.centerIn: parent
            spacing: 6
            Text { text: chart.hoveredDay !== "" ? Fmt.medium(chart.hoveredDay) : ""; font.pixelSize: Theme.caption2; color: Theme.secondary }
            Text { text: chart.hovered ? Fmt.money(chart.hovered.value_cents, chart.currency) : ""
                   font.pixelSize: Theme.caption2; font.weight: Font.DemiBold; font.features: { "tnum": 1 }; color: Theme.text
                   Sensitive { amounts: true } }
            Text {
                visible: !!readout.change
                text: readout.change ? DepotMetrics.signedMoney(readout.change.cents, chart.currency)
                                       + (readout.change.percent !== null ? " (" + DepotMetrics.percent(readout.change.percent, true, 1) + ")" : "") : ""
                font.pixelSize: Theme.caption2; font.features: { "tnum": 1 }
                color: readout.change ? DepotMetrics.tint(readout.change.cents) : Theme.text
                Sensitive { amounts: true }
            }
        }
    }
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.NoButton
        onPositionChanged: m => {
            if (m.x > chart.plotWidth) { chart.hoveredDay = ""; return }
            const ms = chart.xDomain.lo + m.x / chart.plotWidth * (chart.xDomain.hi - chart.xDomain.lo)
            chart.hoveredDay = Fmt.iso(new Date(ms + 43200000))   // the nearest day
        }
        onExited: chart.hoveredDay = ""
    }
    Accessible.role: Accessible.Chart
    Accessible.name: points.length < 1 || (AppModel.effectiveRedaction & AppModel.redactAmounts) !== 0 ? ""
        : qsTr("Depotwert von %1 am %2 auf %3 am %4").arg(Fmt.money(points[0].value_cents, currency)).arg(points[0].date)
              .arg(Fmt.money(points[points.length - 1].value_cents, currency)).arg(points[points.length - 1].date)
}

pragma Singleton
import QtQuick

// DepotValueHistory.swift (#578, #761): the value history of securities depots for the chart
// (`depot_value_history`). Pure: the period filter, the segments and the coverage note live
// here, without a view. A point is { date: "YYYY-MM-DD", value_cents, covered_depots, total_depots }.
QtObject {
    /// How far back the view asks the core; the period then cuts what it shows.
    readonly property int lookbackDays: 3650

    /// chartContent(_:period:): the header chart of all `points` cut to `period` ([from, to],
    /// ISO days, null = open), or why there is none.
    function chartContent(points, period) {
        if (!points || points.length === 0)
            return { kind: "unavailable", text: qsTr("Noch kein Verlauf: der Wert wird ab dem ersten Abruf festgehalten") }
        const from = period ? period[0] : null, to = period ? period[1] : null
        return { kind: "depot", points: inPeriod(points, from, to), from: from, to: to }
    }

    /// points(_:from:to:): the points inside from…to (ISO dates, null = open), oldest first.
    /// The core sends `YYYY-MM-DD`, so the comparison is a plain string comparison.
    function inPeriod(all, from, to) {
        return all.filter(p => (!from || p.date >= from) && (!to || p.date <= to))
    }

    /// DepotValueSegment.isPartial: not every depot had a value, the line is dashed.
    function isPartial(segment) {
        return segment.length > 0 && segment[0].covered_depots < segment[0].total_depots
    }

    /// segments(_:): splits the points wherever the number of covered depots changes.
    function segments(points) {
        const result = []
        for (const p of points) {
            const last = result.length ? result[result.length - 1] : null
            const tail = last ? last[last.length - 1] : null
            if (tail && tail.covered_depots === p.covered_depots && tail.total_depots === p.total_depots) last.push(p)
            else result.push([p])
        }
        return result
    }

    /// assetPoints(_:): value series of Sachwerte (DailyBalance) as chart points: full coverage,
    /// the leading days on which nothing was held yet (value 0) cut away.
    function assetPoints(balances) {
        const first = balances.findIndex(b => b.balance_cents !== 0)
        if (first < 0) return []
        return balances.slice(first).map(b => ({ date: b.date, value_cents: b.balance_cents, covered_depots: 1, total_depots: 1 }))
    }

    /// drawn(_:maxPoints:): equal values in a row collapse into the first of them (lossless for a
    /// step line); a series still longer than `maxPoints` keeps every n-th point (the first and the
    /// last point always stay, so the line reaches the last day even after an unchanged stretch).
    function drawn(points, maxPoints) {
        const max = maxPoints === undefined ? 900 : maxPoints
        const collapsed = []
        for (const p of points) {
            const last = collapsed.length ? collapsed[collapsed.length - 1] : null
            if (last && last.value_cents === p.value_cents && last.covered_depots === p.covered_depots
                && last.total_depots === p.total_depots) continue
            collapsed.push(p)
        }
        if (points.length && collapsed[collapsed.length - 1].date !== points[points.length - 1].date)
            collapsed.push(points[points.length - 1])
        if (collapsed.length <= max || max <= 2) return collapsed
        const step = Math.ceil(collapsed.length / max)
        const thinned = [collapsed[0]]
        for (let i = step; i < collapsed.length - 1; i += step) thinned.push(collapsed[i])
        thinned.push(collapsed[collapsed.length - 1])
        return thinned
    }

    /// The one note under every depot value chart (depot view and Vermögen).
    readonly property string footnote: qsTr("Wertänderung, keine Rendite: Käufe, Verkäufe und Übertragungen verändern den Wert.")

    /// value(on:in:): the point that holds on `day` (ISO): the last one up to it.
    function valueOn(day, points) {
        for (let i = points.length - 1; i >= 0; i--) if (points[i].date <= day) return points[i]
        return null
    }

    /// change(of:since:): the change since the first point of the period, only between days with
    /// the same coverage (when another depot joined, the difference is that depot's value, not a
    /// gain). { cents, percent (null when the start value is not positive) } or null.
    function change(point, first) {
        if (point.covered_depots !== first.covered_depots || point.total_depots !== first.total_depots) return null
        const cents = point.value_cents - first.value_cents
        return { cents: cents, percent: first.value_cents > 0 ? cents / first.value_cents * 100 : null }
    }

    /// startNote(_:from:): "Werte ab 19.07.2026" when the first value is clearly later than the
    /// start of the shown period (the left of the chart is empty then).
    function startNote(points, from) {
        if (!from || !points.length) return ""
        const start = Fmt.date(from), first = Fmt.date(points[0].date)
        if (!start || !first || first - start <= 3 * 86400000) return ""
        return qsTr("Werte ab %1").arg(Fmt.medium(points[0].date))
    }

    /// chartCurrency(_:): one depot's own currency, else EUR (the core sums only EUR depots).
    function chartCurrency(currencies) {
        return currencies.length === 1 && currencies[0] ? currencies[0] : "EUR"
    }

    /// excludedNote(_:): "Ohne Depots in anderer Währung (1)" when a sum over several depots leaves some out.
    function excludedNote(currencies) {
        if (currencies.length <= 1) return ""
        const other = currencies.filter(c => c && c !== "EUR").length
        return other > 0 ? qsTr("Ohne Depots in anderer Währung (%1)").arg(other) : ""
    }

    /// coverageNote(_:): "Teilsumme · 2 von 3 Depots" when some day of the shown period is not
    /// fully covered (the lowest coverage in it), else "".
    function coverageNote(points) {
        let worst = null
        for (const p of points)
            if (p.covered_depots < p.total_depots && (!worst || p.covered_depots < worst.covered_depots)) worst = p
        return worst ? qsTr("Teilsumme · %1 von %2 Depots").arg(worst.covered_depots).arg(worst.total_depots) : ""
    }
}

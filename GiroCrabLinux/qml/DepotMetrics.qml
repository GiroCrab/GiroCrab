pragma Singleton
import QtQuick

// DepotMetrics.swift (#564, #625): the pure figures of the holdings views (securities depot,
// Sachwerte account, Vermögen › Wertpapiere / Sachwerte). No UI state and no vault access; the
// views never compute sums of their own. Three data kinds stay apart: holdings (current
// inventory), security transactions (activities) and cash bookings (clearing account).
// Also DepotDetailView.swift › DepotFormat and DepotKPITiles.swift › DepotKPIs.
QtObject {
    id: dm

    // MARK: DepotFormat
    /// "+1,25 %": two digits, a typographic minus, a plus only when asked.
    function percent(value, signed, digits) {
        const d = digits === undefined ? 2 : digits
        const f = Number(value).toLocaleString(Qt.locale(), "f", d)
        return (signed && value > 0 ? "+" : "") + f.replace("-", "−") + " %"
    }
    function tint(cents) { return cents > 0 ? Theme.inflow : cents < 0 ? Theme.negative : Theme.text }
    function coverage(f) { return f.covered < f.total ? qsTr("für %1 von %2 Positionen").arg(f.covered).arg(f.total) : null }
    /// Money.signed(_:currency:)
    function signedMoney(cents, code) { return cents > 0 ? "+" + Fmt.money(cents, code) : Fmt.money(cents, code) }
    function quantityText(q) { return NumberText.format(q) }

    // MARK: HoldingFigures
    /// Tolerance when comparing the reported value with quantity × price: rounding and small price lags.
    function matchesValue(computed, value) {
        if (!isFinite(computed)) return false
        return Math.abs(computed - value) <= Math.max(value * 0.02, 2)
    }
    /// "perUnit" (value ≈ quantity × price), "percentOfNominal" (bonds) or "unknown".
    function priceBasis(h, isBond) {
        if (!isFinite(h.quantity) || !(h.quantity > 0) || h.current_price_cents <= 0 || h.value_cents <= 0) return "unknown"
        const perUnit = h.quantity * h.current_price_cents
        if (matchesValue(perUnit, h.value_cents)) return "perUnit"
        if (isBond && matchesValue(perUnit / 100, h.value_cents)) return "percentOfNominal"
        return "unknown"
    }
    function has(v) { return v !== null && v !== undefined }
    /// { cents, costCents, reported, percent }: the bank's figure first, else value − cost where plausible.
    function makeGain(cents, cost, reported) { return { cents: cents, costCents: cost, reported: reported, percent: cost > 0 ? cents / cost * 100 : null } }
    function gainOf(h, basis) {
        if (has(h.profit_loss_purchase_cents)) return makeGain(h.profit_loss_purchase_cents, h.value_cents - h.profit_loss_purchase_cents, true)
        if (basis === "perUnit") {
            const g = calculatedGain(h)
            return g ? makeGain(g.cents, h.value_cents - g.cents, false) : null
        }
        if (basis === "percentOfNominal") {
            if (h.purchase_price_cents <= 0) return null
            const cost = Math.round(h.quantity * h.purchase_price_cents / 100)
            if (!isFinite(cost) || cost < 1 || h.value_cents < 0) return null
            return makeGain(h.value_cents - cost, cost, false)
        }
        return null
    }
    /// Change against the previous day, only from the bank's previous-day value (or P&L).
    function dayChange(h) {
        if (has(h.prev_day_value_cents) && h.prev_day_value_cents > 0)
            return { cents: h.value_cents - h.prev_day_value_cents, previousValueCents: h.prev_day_value_cents }
        if (has(h.profit_loss_prev_day_cents)) {
            const prev = h.value_cents - h.profit_loss_prev_day_cents
            if (prev > 0) return { cents: h.profit_loss_prev_day_cents, previousValueCents: prev }
        }
        return null
    }
    /// cost = quantity × purchase price; missing cost prices stay unknown.
    function calculatedGain(h) {
        if (!isFinite(h.quantity) || !(h.quantity > 0) || h.purchase_price_cents <= 0 || h.value_cents < 0) return null
        const cost = Math.round(h.quantity * h.purchase_price_cents)
        if (!isFinite(cost) || cost < 1) return null
        const g = h.value_cents - cost
        return { cents: g, percent: g / cost * 100 }
    }
    /// One position as plain text: name, ISIN/WKN, quantity, price, value.
    function clipboardLine(h) {
        const id = h.isin || h.wkn
        const qty = Number(h.quantity).toLocaleString(Qt.locale(), "f", 4).replace(/[.,]?0+$/, "")
        return [h.name, id, qty + " × " + Fmt.money(h.current_price_cents), Fmt.money(h.value_cents)].filter(x => x).join(" · ")
    }
    function isBond(asset) { return asset.classification.asset_class === "bond" }
    function assetGain(asset) { return gainOf(asset.holding, priceBasis(asset.holding, isBond(asset))) }
    /// Holding.nativeCurrency(accountCurrency:) (#625): the position's own currency when it differs.
    function nativeCurrency(h, accountCurrency) {
        const c = h.currency || ""
        return c === "" || c.toUpperCase() === String(accountCurrency).toUpperCase() ? null : c
    }

    // MARK: DepotValue
    /// The bank's depot value; without one the position sum stands in; a larger gap is shown.
    function resolveValue(reportedCents, holdings) {
        const sum = holdings.reduce((t, h) => t + h.value_cents, 0)
        if (reportedCents === 0 && holdings.length > 0 && sum !== 0)
            return { cents: sum, source: "positionSum", positionSumCents: sum, positionCount: holdings.length, differenceCents: null }
        const diff = reportedCents - sum
        const flagged = holdings.length > 0 && Math.abs(diff) > Math.max(holdings.length, 100)
        return { cents: reportedCents, source: "reported", positionSumCents: sum, positionCount: holdings.length, differenceCents: flagged ? diff : null }
    }

    // MARK: DepotMetrics (totals over the current inventory)
    /// DepotFigure { amountCents, baseCents, covered, total, calculated, percent } or null.
    function figure(f) { f.percent = f.baseCents > 0 ? f.amountCents / f.baseCents * 100 : null; return f }
    function unrealized(gains) {
        const f = { amountCents: 0, baseCents: 0, covered: 0, total: gains.length, calculated: 0 }
        for (const g of gains) {
            if (!g) continue
            f.amountCents += g.cents; f.baseCents += Math.max(g.costCents, 0); f.covered += 1
            if (!g.reported) f.calculated += 1
        }
        return f.covered > 0 ? figure(f) : null
    }
    function dayChangeSum(holdings) {
        const f = { amountCents: 0, baseCents: 0, covered: 0, total: holdings.length, calculated: 0 }
        for (const h of holdings) {
            const d = dayChange(h)
            if (!d) continue
            f.amountCents += d.cents; f.baseCents += d.previousValueCents; f.covered += 1
        }
        return f.covered > 0 ? figure(f) : null
    }
    function share(valueCents, totalCents) { return totalCents > 0 && valueCents >= 0 ? valueCents / totalCents : null }
    /// Every word of the query occurs in one of the fields (case and diacritics ignored).
    function fold(s) {
        const t = String(s)
        return (typeof t.normalize === "function" ? t.normalize("NFD") : t).replace(/[\u0300-\u036f]/g, "").toLowerCase()
    }
    function matches(query, fields) {
        const words = String(query).split(/\s+/).filter(w => w !== "")
        if (!words.length) return true
        const hay = fold(fields.join(" "))
        return words.every(w => hay.indexOf(fold(w)) >= 0)
    }

    // MARK: DepotSort { key: value | gain | name, ascending }
    function toggledSort(sort, key) {
        return sort.key === key ? { key: key, ascending: !sort.ascending } : { key: key, ascending: key === "name" }
    }

    // MARK: DepotPosition
    function displayName(h) { return h.name !== "" ? h.name : (h.isin !== "" ? h.isin : h.wkn) }
    /// `gainFn` decides where the gain comes from (securities: bank or cost price; Sachwerte: purchase data).
    function makePositions(assets, gainFn, hidesPriceFn) {
        const total = assets.reduce((t, a) => t + Math.max(a.holding.value_cents, 0), 0)
        const seen = {}
        return assets.map(a => {
            const h = a.holding
            const base = h.isin + "|" + h.wkn + "|" + h.name + "|" + (h.custody_type || "")
            const n = seen[base] || 0
            seen[base] = n + 1
            return { id: n === 0 ? base : base + "#" + n, asset: a, holding: h,
                     basis: priceBasis(h, isBond(a)), gain: gainFn ? gainFn(a) : assetGain(a),
                     share: share(h.value_cents, total), hidesPrice: hidesPriceFn ? hidesPriceFn(a) : false,
                     isOwnItem: has(a.asset_item_id), displayName: displayName(h), identifier: h.isin || h.wkn }
        })
    }
    function compareNames(a, b) { return a.displayName.localeCompare(b.displayName, undefined, { numeric: true, sensitivity: "base" }) }
    /// Positions without a figure always come last, whatever the direction.
    function sortedPositions(items, sort) {
        const field = sort.key === "gain" ? (p => p.gain ? p.gain.cents : null) : (p => p.holding.value_cents)
        return items.slice().sort((a, b) => {
            if (sort.key === "name") return sort.ascending ? compareNames(a, b) : compareNames(b, a)
            const x = field(a), y = field(b)
            if (x !== null && y !== null) return x === y ? compareNames(a, b) : (sort.ascending ? x - y : y - x)
            if (x !== null) return -1
            if (y !== null) return 1
            return compareNames(a, b)
        })
    }
    function filteredByQuery(items, query) {
        return items.filter(p => matches(query, [p.holding.name, p.holding.isin, p.holding.wkn]))
    }

    // MARK: activities (SecurityActivityTotals)
    /// ISO dates compare as strings; an open end is null.
    function inPeriod(date, from, to) {
        if (from === null && to === null) return true
        if (!date) return false
        if (from !== null && date < from) return false
        if (to !== null && date > to) return false
        return true
    }
    function activityTotals(transactions, currency, from, to) {
        const r = { distributionsCents: 0, distributionCount: 0, feesCents: 0, taxesCents: 0, count: 0, otherCurrencyCount: 0 }
        for (const t of transactions) {
            if (!inPeriod(t.booking_date, from, to)) continue
            if (t.currency !== currency) { r.otherCurrencyCount++; continue }
            r.count++
            if (t.direction === "Dividend") { r.distributionsCents += t.value_cents; r.distributionCount++ }
            else if (t.direction === "Fee") r.feesCents += t.value_cents !== 0 ? Math.abs(t.value_cents) : t.fees_cents
            else if (t.direction === "Tax") r.taxesCents += t.value_cents !== 0 ? Math.abs(t.value_cents) : t.taxes_cents
            if (t.direction !== "Fee") r.feesCents += t.fees_cents
            if (t.direction !== "Tax") r.taxesCents += t.taxes_cents
        }
        r.isEmpty = r.count === 0
        r.costsCents = r.feesCents + r.taxesCents
        return r
    }

    // MARK: DepotKPIs – the figure tiles { title, value, detail, tint, footnote, missing, help }
    function otherCurrencyNote(t) { return t.otherCurrencyCount > 0 ? " · " + qsTr("%1 in Fremdwährung nicht enthalten").arg(t.otherCurrencyCount) : "" }
    function securitiesTiles(assets, transactions, currency, period, from, to) {
        const totals = activityTotals(transactions, currency, from, to)
        const unreal = unrealized(assets.map(assetGain))
        const day = dayChangeSum(assets.map(a => a.holding))
        return [
            { title: qsTr("Unrealisierte GuV"),
              value: unreal ? signedMoney(unreal.amountCents, currency) : null,
              detail: unreal && unreal.percent !== null ? percent(unreal.percent, true) : null,
              tint: unreal ? tint(unreal.amountCents) : Theme.text,
              footnote: [qsTr("Aktueller Bestand seit Einstand"), unreal ? coverage(unreal) : null,
                         unreal && unreal.calculated > 0 ? qsTr("teils berechnet") : null].filter(x => x).join(" · "),
              missing: qsTr("Keine Einstandsdaten geliefert"),
              help: qsTr("Bewertung des aktuellen Bestands gegenüber dem Einstand, nicht realisiert und unabhängig vom Zeitraum. Bankwert, sonst aus dem Einstandskurs berechnet.") },
            { title: qsTr("Tagesveränderung"),
              value: day ? signedMoney(day.amountCents, currency) : null,
              detail: day && day.percent !== null ? percent(day.percent, true) : null,
              tint: day ? tint(day.amountCents) : Theme.text,
              footnote: [qsTr("Aktueller Bestand zum Vortag"), day ? coverage(day) : null].filter(x => x).join(" · "),
              missing: qsTr("Keine Vortagswerte geliefert"),
              help: qsTr("Aus den Vortageswerten der Bank. Käufe und Verkäufe von heute sind nicht herausgerechnet.") },
            { title: qsTr("Ausschüttungen"),
              value: totals.isEmpty ? null : Fmt.money(totals.distributionsCents, currency),
              detail: totals.isEmpty ? null : qsTr("%1 Zahlungen").arg(totals.distributionCount),
              tint: Theme.inflow,
              footnote: qsTr("Brutto · %1").arg(period) + otherCurrencyNote(totals),
              missing: qsTr("Keine Aktivitäten im Zeitraum erfasst"),
              help: qsTr("Summe der erfassten Ausschüttungen vor Quellensteuer. Gilt für den gewählten Zeitraum, nicht für den Bestand. Fehlende Einträge bedeuten nicht, dass es keine Ausschüttungen gab.") },
            { title: qsTr("Gebühren & Steuern"),
              value: totals.isEmpty ? null : Fmt.money(-totals.costsCents, currency),
              detail: totals.isEmpty ? null : qsTr("Gebühren %1 · Steuern %2").arg(Fmt.money(totals.feesCents, currency)).arg(Fmt.money(totals.taxesCents, currency)),
              tint: totals.costsCents > 0 ? Theme.negative : Theme.text,
              footnote: qsTr("Zeitraum: %1").arg(period) + otherCurrencyNote(totals),
              missing: qsTr("Keine Aktivitäten im Zeitraum erfasst"),
              help: qsTr("Gebühren und Steuern aus den erfassten Wertpapieraktivitäten, eigene Zeilen und eingebettete Beträge ohne Doppelzählung. Fehlende Einträge bedeuten nicht, dass keine Kosten anfielen.") }
        ]
    }
}

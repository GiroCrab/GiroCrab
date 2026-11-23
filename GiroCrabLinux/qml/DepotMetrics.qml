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
    /// One position as plain text: name, ISIN/WKN, quantity, price, value (in trading currency when set).
    function clipboardLine(h, accountCurrency) {
        const acc = accountCurrency || "EUR"
        const trade = nativeCurrency(h, acc)
        const price = (trade !== null && has(h.native_price_cents))
            ? Fmt.money(h.native_price_cents, trade)
            : Fmt.money(h.current_price_cents, acc)
        const val = (trade !== null && has(h.native_value_cents))
            ? Fmt.money(h.native_value_cents, trade)
            : Fmt.money(h.value_cents, acc)
        const id = h.isin || h.wkn
        const qty = Number(h.quantity).toLocaleString(Qt.locale(), "f", 4).replace(/[.,]?0+$/, "")
        return [h.name, id, qty + " × " + price, val].filter(x => x).join(" · ")
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
    /// One row per security across several depots: quantity, value, share and gain add up.
    function mergedPositions(items) {
        // keyed by name when there is no ISIN/WKN: no prototype ("constructor" is a name too)
        const groups = Object.create(null)
        for (const p of items) { const k = compositionKey(p.holding); (groups[k] = groups[k] || []).push(p) }
        const seen = Object.create(null), out = []
        for (const item of items) {
            const key = compositionKey(item.holding)
            if (seen[key]) continue
            seen[key] = true
            const group = groups[key]
            if (group.length < 2) { out.push(item); continue }
            const lead = group.reduce((m, p) => p.holding.value_cents > m.holding.value_cents ? p : m, group[0])
            const h = Object.assign({}, lead.holding)
            h.quantity = group.reduce((t, p) => t + p.holding.quantity, 0)
            h.value_cents = group.reduce((t, p) => t + p.holding.value_cents, 0)
            h.prev_day_value_cents = null
            h.profit_loss_purchase_cents = null
            const gains = group.map(p => p.gain).filter(g => g)
            const gain = gains.length === 0 ? null : makeGain(gains.reduce((t, g) => t + g.cents, 0), gains.reduce((t, g) => t + g.costCents, 0),
                                                             gains.length === group.length && gains.every(g => g.reported))
            const shares = group.map(p => p.share).filter(s => s !== null)
            const asset = { holding: h, classification: lead.asset.classification, asset_item_id: lead.asset.asset_item_id }
            out.push({ id: key, asset: asset, holding: h, basis: lead.basis, gain: gain,
                       share: shares.length ? shares.reduce((t, s) => t + s, 0) : null, hidesPrice: group.every(p => p.hidesPrice),
                       isOwnItem: has(lead.asset.asset_item_id), displayName: displayName(h), identifier: h.isin || h.wkn })
        }
        return out
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
    /// Only the positions behind a clicked slice; a stale filter (judged against the whole inventory) is ignored.
    function filteredByKeys(items, keys, inventory) {
        if (!keys) return items
        const base = inventory || items
        if (!base.some(p => keys.indexOf(compositionKey(p.holding)) >= 0)) return items
        return items.filter(p => keys.indexOf(compositionKey(p.holding)) >= 0)
    }
    function filteredByQuery(items, query) {
        return items.filter(p => matches(query, [p.holding.name, p.holding.isin, p.holding.wkn]))
    }
    /// Two key sets (memberKeys) hold the same keys.
    function sameKeys(a, b) {
        if (!a || !b || a.length !== b.length) return false
        return a.every(k => b.indexOf(k) >= 0)
    }

    // MARK: DepotColors – FNV-1a of the business key into the palette; colliding keys step to the next free one
    readonly property var palette: [
        Theme.c("#2F6FDB", "#6EA0F5"), Theme.c("#1F7A5C", "#5EC79F"), Theme.c("#D98A00", "#F5B94A"), Theme.c("#8E4FC4", "#B98AE6"),
        Theme.c("#D1495B", "#F27F8E"), Theme.c("#0E8FA8", "#5CC6DB"), Theme.c("#7C9A2D", "#A9C95C"), Theme.c("#B0568F", "#DB8FC0"),
        Theme.c("#7A5C2E", "#C9A66B"), Theme.c("#C95A86", "#F2A3C0"), Theme.c("#4A5A9A", "#8FA0E0"), Theme.c("#2E8B8B", "#6FD0D0"),
        Theme.c("#9C4F2E", "#E0936B"), Theme.c("#6B8F3A", "#B5D56B"), Theme.c("#8A8A3A", "#D6D68A"), Theme.c("#5F5F7A", "#A9A9C4")]
    /// WealthPalette.color(i) / WealthPalette.rest
    function wealthColor(i) { return palette[i % 8] }
    readonly property color restColor: Theme.c("#B4BEC8", "#6B7785")
    function utf8(s) {
        const out = []
        for (const ch of String(s)) {
            let c = ch.codePointAt(0)
            if (c < 0x80) out.push(c)
            else if (c < 0x800) out.push(0xC0 | c >> 6, 0x80 | c & 63)
            else if (c < 0x10000) out.push(0xE0 | c >> 12, 0x80 | c >> 6 & 63, 0x80 | c & 63)
            else out.push(0xF0 | c >> 18, 0x80 | c >> 12 & 63, 0x80 | c >> 6 & 63, 0x80 | c & 63)
        }
        return out
    }
    /// hash % 16 only needs the low 16 bits of the 64-bit FNV-1a hash (16 divides 2^64, and the
    /// low bits of a product depend only on the low bits of its factors).
    function startIndex(key) {
        let h = 0x2325   // 0xcbf29ce484222325 mod 2^16
        for (const b of utf8(key)) h = ((h ^ b) * 0x1b3) & 0xffff   // prime 0x100000001b3 mod 2^16
        return h % palette.length
    }
    function colorIndices(keys) {
        const unique = Array.from(new Set(keys)).sort()
        const taken = {}, out = Object.create(null)
        let count = 0
        for (const key of unique) {
            let i = startIndex(key)
            if (count < palette.length) {
                while (taken[i]) i = (i + 1) % palette.length
                taken[i] = true; count++
            }
            out[key] = i
        }
        return out
    }

    // MARK: DepotComposition
    readonly property string restID: "rest"
    readonly property int maxPositionSlices: 7
    readonly property int maxRankedSlices: 10
    readonly property int maxTiles: 12
    function compositionKey(h) { return h.isin !== "" ? h.isin : (h.wkn !== "" ? h.wkn : h.name) }
    function drawable(assets) { return assets.filter(a => a.holding.value_cents > 0) }
    function excludedCount(assets) { return assets.length - drawable(assets).length }
    /// DepotSlice { id, label, cents, count, color, unclassified, personal, memberKeys }, largest first.
    function compositionGroups(assets, mode) {
        const items = drawable(assets)
        if (mode === "position") {
            const cents = Object.create(null), labels = Object.create(null), own = Object.create(null)
            const idx = colorIndices(items.map(a => compositionKey(a.holding)))
            for (const a of items) {
                const h = a.holding, k = compositionKey(h)
                cents[k] = (cents[k] || 0) + h.value_cents
                if (labels[k] === undefined) labels[k] = displayName(h)
                if (has(a.asset_item_id)) own[k] = true
            }
            return Object.keys(cents).sort((x, y) => cents[x] !== cents[y] ? cents[y] - cents[x] : (x < y ? -1 : x > y ? 1 : 0)).map(k => ({
                id: k, label: labels[k] || k, cents: cents[k], count: 1, color: palette[idx[k]], unclassified: false,
                personal: !!own[k], memberKeys: [k] }))
        }
        const by = Object.create(null)
        for (const a of items) {
            const c = a.classification
            const key = mode === "assetClass" ? c.asset_class : c.instrument
            const g = by[key] || (by[key] = {
                id: key, label: mode === "assetClass" ? AssetClassification.classTitle(key) : AssetClassification.instrumentTitle(key),
                order: mode === "assetClass" ? AssetClassification.classIndex(key) : AssetClassification.instrumentIndex(key),
                cents: 0, count: 0, unclassified: key === "unclassified", personal: false, memberKeys: [] })
            g.cents += a.holding.value_cents
            g.count += 1
            const k = compositionKey(a.holding)
            if (g.memberKeys.indexOf(k) < 0) g.memberKeys.push(k)
        }
        return Object.values(by).map(g => Object.assign(g, { color: g.unclassified ? Theme.secondary : wealthColor(g.order) }))
            .sort((a, b) => a.cents !== b.cents ? b.cents - a.cents : a.label.localeCompare(b.label))
    }
    /// The `keep` largest slices and one "Weitere (n)" slice with the rest; one remaining slice is no rest.
    function foldSlices(slices, keep) {
        if (slices.length <= keep + 1) return slices
        const rest = slices.slice(keep)
        const n = rest.reduce((t, s) => t + s.count, 0)
        const keys = []
        for (const s of rest) for (const k of s.memberKeys) if (keys.indexOf(k) < 0) keys.push(k)
        return slices.slice(0, keep).concat([{ id: restID, label: qsTr("Weitere (%1)").arg(n), cents: rest.reduce((t, s) => t + s.cents, 0),
                                               count: n, color: restColor, unclassified: false, personal: false, memberKeys: keys }])
    }
    function ranked(slices, keep) {
        const folded = foldSlices(slices, keep)
        const rest = folded.length && folded[folded.length - 1].id === restID ? folded[folded.length - 1] : null
        return { shown: folded.filter(s => s.id !== restID), rest: rest }
    }

    // MARK: Treemap (squarified)
    function treemap(values, rw, rh, preferredAspect) {
        if (preferredAspect && rh > 0 && rw > rh * preferredAspect) {
            const vw = rh * preferredAspect, factor = rw / vw
            return treemap(values, vw, rh, 0).map(r => r.w === 0 && r.h === 0 ? r : { x: r.x * factor, y: r.y, w: r.w * factor, h: r.h })
        }
        const positive = values.map(v => isFinite(v) ? Math.max(v, 0) : 0)
        const total = positive.reduce((t, v) => t + v, 0)
        const out = values.map(() => ({ x: 0, y: 0, w: 0, h: 0 }))
        if (!(total > 0) || rw <= 0 || rh <= 0) return out
        const scale = rw * rh / total
        const areas = positive.map(v => v * scale)
        let free = { x: 0, y: 0, w: rw, h: rh }
        let row = []
        const worst = (idx, side) => {
            const sum = idx.reduce((t, i) => t + areas[i], 0)
            const vals = idx.map(i => areas[i])
            const mx = Math.max(...vals), mn = Math.min(...vals)
            if (!(sum > 0) || !(side > 0) || !(mn > 0)) return Infinity
            return Math.max(side * side * mx / (sum * sum), sum * sum / (side * side * mn))
        }
        const place = () => {
            const sum = row.reduce((t, i) => t + areas[i], 0)
            if (!(sum > 0)) { row = []; return }
            if (free.w >= free.h) {
                const w = sum / free.h
                let y = free.y
                for (const i of row) { const h = areas[i] / w; out[i] = { x: free.x, y: y, w: w, h: h }; y += h }
                free = { x: free.x + w, y: free.y, w: Math.max(free.w - w, 0), h: free.h }
            } else {
                const h = sum / free.w
                let x = free.x
                for (const i of row) { const w = areas[i] / h; out[i] = { x: x, y: free.y, w: w, h: h }; x += w }
                free = { x: free.x, y: free.y + h, w: free.w, h: Math.max(free.h - h, 0) }
            }
            row = []
        }
        let i = 0
        while (i < values.length) {
            if (!(areas[i] > 0)) { i++; continue }
            const side = Math.min(free.w, free.h)
            if (row.length === 0 || worst(row.concat([i]), side) <= worst(row, side)) { row.push(i); i++ }
            else place()
        }
        place()
        return out
    }
    /// TileInk: near-black on light fills, white on dark ones (WCAG relative luminance).
    function tileInk(c) {
        const ch = v => v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)
        const l = 0.2126 * ch(c.r) + 0.7152 * ch(c.g) + 0.0722 * ch(c.b)
        return l > 0.4 ? Qt.rgba(0.11, 0.11, 0.12, 1) : "white"
    }

    // MARK: activities (DepotActivityFilter, DepotActivities, SecurityActivityTotals)
    readonly property var activityFilters: [
        { id: "all", title: qsTr("Alle") }, { id: "buys", title: qsTr("Käufe") }, { id: "sells", title: qsTr("Verkäufe") },
        { id: "distributions", title: qsTr("Ausschüttungen") }, { id: "costs", title: qsTr("Kosten") }]
    function keepsActivity(filter, t) {
        switch (filter) {
        case "buys": return t.direction === "Buy"
        case "sells": return t.direction === "Sell"
        case "distributions": return t.direction === "Dividend"
        // own fee/tax lines and every activity that carries fees or taxes
        case "costs": return t.direction === "Fee" || t.direction === "Tax" || t.fees_cents !== 0 || t.taxes_cents !== 0
        default: return true
        }
    }
    /// The bank's free text never serves as the kind (it can carry personal data).
    function kindTitle(t) {
        switch (t.direction) {
        case "Buy": return qsTr("Kauf")
        case "Sell": return qsTr("Verkauf")
        case "Dividend": return qsTr("Ausschüttung")
        case "Fee": return qsTr("Gebühr")
        case "Tax": return qsTr("Steuer")
        default: return qsTr("Sonstige")
        }
    }
    function feeCents(t) { return t.direction === "Fee" ? Math.abs(t.value_cents !== 0 ? t.value_cents : t.fees_cents) : t.fees_cents }
    function taxCents(t) { return t.direction === "Tax" ? Math.abs(t.value_cents !== 0 ? t.value_cents : t.taxes_cents) : t.taxes_cents }
    /// ISO dates compare as strings; an open end is null.
    function inPeriod(date, from, to) {
        if (from === null && to === null) return true
        if (!date) return false
        if (from !== null && date < from) return false
        if (to !== null && date > to) return false
        return true
    }
    function filteredActivities(all, accountId, filter, query, from, to) {
        return all.filter(t => t.account_id === accountId && inPeriod(t.booking_date, from, to) && keepsActivity(filter, t)
                               && matches(query, [t.name, t.isin, t.wkn]))
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

    // MARK: Sachwerte (WealthView.tangibleGain / tangibleDayChange, TangiblePositions)
    /// Gain against the purchase data: the exact total paid while nothing changed, else the cost price.
    function tangibleGain(item) {
        const h = item.holding, paid = item.setup.purchase_price_cents
        if (has(paid) && paid > 0 && h.value_cents >= 0 && h.quantity > 0 && h.quantity === item.setup.quantity) {
            const g = h.value_cents - paid
            return { cents: g, percent: g / paid * 100 }
        }
        return calculatedGain(h)
    }
    /// Today's price of one piece against yesterday's, times the pieces held; manual items have no
    /// daily price. Per piece, so an unchanged price is exactly 0.
    function tangibleDayChange(item, previousUnitCents) {
        const h = item.holding
        if (item.setup.price_source === "manual" || item.sold || !has(previousUnitCents) || !(previousUnitCents > 0)
                || !(h.current_price_cents > 0) || !isFinite(h.quantity) || !(h.quantity > 0)) return null
        const change = Math.round(h.quantity * (h.current_price_cents - previousUnitCents))
        return isFinite(change) ? change : null
    }
    function tangibleTiles(items, previousUnits, currency) {
        const held = items.filter(i => !i.sold)
        const rows = held.map(item => ({ value: item.holding.value_cents, gain: tangibleGain(item),
                                         day: tangibleDayChange(item, previousUnits[item.id]) }))
        const largest = held.reduce((m, i) => m === null || i.holding.value_cents > m.holding.value_cents ? i : m, null)
        return tangibleTilesFrom(rows.reduce((t, r) => t + r.value, 0), rows.length,
            rows.map(r => r.gain ? makeGain(r.gain.cents, r.value - r.gain.cents, false) : null),
            rows.filter(r => r.day !== null).map(r => ({ cents: r.day, previousValueCents: r.value - r.day })),
            held.filter(i => i.setup.price_source !== "manual").length,
            largest ? { name: largest.setup.name, cents: largest.holding.value_cents } : null, currency)
    }
    function tangibleTilesFrom(valueCents, count, gains, days, automaticCount, largest, currency) {
        const largestShare = largest && valueCents > 0 ? { name: largest.name, percent: largest.cents / valueCents * 100 } : null
        const unreal = unrealized(gains)
        const known = gains.filter(g => g)
        const cost = known.reduce((t, g) => t + Math.max(g.costCents, 0), 0)
        const dayCents = days.reduce((t, d) => t + d.cents, 0)
        const dayBase = days.reduce((t, d) => t + d.previousValueCents, 0)
        const cov = (covered, total) => covered < total ? qsTr("für %1 von %2 Positionen").arg(covered).arg(total) : null
        return [
            { title: qsTr("Einstandswert"), value: known.length > 0 ? Fmt.money(cost, currency) : null, detail: null, tint: Theme.text,
              footnote: [qsTr("Aktueller Bestand"), cov(known.length, count)].filter(x => x).join(" · "),
              missing: qsTr("Keine Einstandsdaten vorhanden"),
              help: qsTr("Summe der Einstandswerte der gehaltenen Objekte, soweit bekannt.") },
            { title: qsTr("Unrealisierte GuV"),
              value: unreal ? signedMoney(unreal.amountCents, currency) : null,
              detail: unreal && unreal.percent !== null ? percent(unreal.percent, true) : null,
              tint: unreal ? tint(unreal.amountCents) : Theme.text,
              footnote: [qsTr("Aktueller Bestand seit Einstand"), cov(known.length, count)].filter(x => x).join(" · "),
              missing: qsTr("Keine Einstandsdaten vorhanden"),
              help: qsTr("Wert heute gegenüber dem Einstand, nicht realisiert. Nur Objekte mit bekanntem Einstandspreis.") },
            { title: qsTr("Tagesveränderung"),
              value: days.length === 0 ? null : signedMoney(dayCents, currency),
              detail: days.length === 0 || dayBase <= 0 ? null : percent(dayCents / dayBase * 100, true),
              tint: days.length === 0 ? Theme.text : tint(dayCents),
              footnote: [qsTr("Aktueller Bestand zum Vortag"), cov(days.length, automaticCount)].filter(x => x).join(" · "),
              missing: qsTr("Keine Tagespreise vorhanden"),
              help: qsTr("Nur automatisch bepreiste Objekte (Edelmetalle) haben einen Tagespreis; manuelle Bewertungen ändern sich nicht täglich.") },
            { title: qsTr("Größte Position"),
              value: largestShare ? percent(largestShare.percent, false, 1) : null,
              detail: largestShare ? largestShare.name : null, detailPersonal: true, tint: Theme.text,
              footnote: qsTr("Anteil am gesamten Sachwert"),
              missing: qsTr("Noch keine Sachwerte"),
              help: qsTr("Das Objekt mit dem höchsten Wert und sein Anteil am gesamten Sachwert.") }
        ]
    }
    /// The held items as ClassifiedAssets: built from the items, classification attached where loaded.
    function tangibleAssets(items, classified) {
        return items.filter(i => !i.sold).map(item => {
            const holding = Object.assign({}, item.holding, { name: item.setup.name })
            const c = classified.find(a => a.asset_item_id === item.id)
            const classification = c ? c.classification
                : { security_key: null, asset_class: "unclassified", asset_class_source: "Unknown", instrument: "unclassified", instrument_source: "Unknown" }
            return { holding: holding, classification: classification, asset_item_id: item.id }
        })
    }
    function tangiblePositions(items, classified) {
        const byId = {}
        for (const i of items) byId[i.id] = i
        return makePositions(tangibleAssets(items, classified), a => {
            const item = byId[a.asset_item_id]
            const g = item ? tangibleGain(item) : null
            return g ? makeGain(g.cents, a.holding.value_cents - g.cents, false) : null
        }, a => {
            // one piece of a manually valued item (house, car) has no market price of its own
            const item = byId[a.asset_item_id]
            return !!item && item.setup.price_source === "manual" && item.holding.quantity === 1
        })
    }
}

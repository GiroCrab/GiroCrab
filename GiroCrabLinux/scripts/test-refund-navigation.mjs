import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import { test } from "node:test";

const source = readFileSync(new URL("../qml/AppModel.qml", import.meta.url), "utf8");
function bodyAfter(marker, text = source) {
    assert.ok(text.includes(marker));
    const start = text.indexOf("{", text.indexOf(marker));
    assert.ok(start >= 0);
    let depth = 1, end = start + 1;
    for (; depth && end < text.length; end++) {
        if (text[end] === "{") depth++;
        if (text[end] === "}") depth--;
    }
    assert.equal(depth, 0);
    return text.slice(start + 1, end - 1);
}
const reveal = bodyAfter("function revealBooking(id)");
const timerSource = source.slice(source.indexOf("property Timer revealTimer:"));
const timerBody = bodyAfter("onTriggered:", timerSource);

function fixture() {
    const pending = [], selected = [], cancelled = [];
    let publish;
    const s = {
        isUnlocked: true, vaultGeneration: 1, listGeneration: 1, listLimit: 200,
        bookings: [], sidebarSelection: "account:42", period: "month",
        directionFilter: "Income", statusFilter: "checked", scope: "Investments", searchText: "hidden",
        TagFilter: { clear() { s.tagsCleared = true; } },
        MerchantFocus: { sidebarChanged() { s.merchantCleared = true; } },
        UndoManager: {}, merchantStatsTimer: { stop() {} },
        searchTimer: { stop() {} },
        revealTimer: { request: 0, restart() {}, stop() { s.stopped = true; } },
        cancelLoad(channel) { cancelled.push(channel); },
        dropPendingPage() { s.listGeneration++; }, endSelectionLoad() {}, vaultReplaced() {},
        isLoadPending() { return false; }, restart() { s.waiting = true; },
        core() { throw new Error("UI-thread core read"); },
        run() { throw new Error("UI-thread read"); },
        currentFilter() { return { date_from: null, date_to: null, limit: s.listLimit }; },
        loadOffMain(channel, request, apply, failed, forceAsync) {
            assert.equal(channel, "revealBooking");
            assert.equal(forceAsync, true);
            pending.push({ calls: request(), apply });
        },
        reloadBookings(then) { publish = then; },
        selectBooking(id) { selected.push(id); },
    };
    s.model = s;
    for (const key of ["bookingID", "request", "vault"]) {
        Object.defineProperty(s, key, { get: () => s.revealTimer[key] });
    }
    vm.createContext(s);
    const jump = id => { s.target = id; vm.runInContext(`(function(id) {${reveal}})(target)`, s); };
    const fire = () => vm.runInContext(`(function() {${timerBody}})()`, s);
    const swap = () => vm.runInContext(`(function() {${bodyAfter("function vaultSwitched()")}})()`, s);
    return { s, pending, selected, cancelled, jump, fire, swap, publish: () => publish?.() };
}

test("jump clears every excluding filter and loads only through the linked day off-main", () => {
    const f = fixture();
    f.jump(42);
    for (const [key, value] of Object.entries({ sidebarSelection: "all", period: "all", directionFilter: "All", statusFilter: "all", scope: "All", searchText: "" })) {
        assert.equal(f.s[key], value);
    }
    assert.ok(f.s.tagsCleared && f.s.merchantCleared);
    f.fire();
    assert.equal(f.pending[0].calls[0][0], "booking_date");
    f.pending[0].apply(["2020-01-01"]);
    assert.equal(f.pending[1].calls[0][0], "count_transactions");
    assert.equal(f.pending[1].calls[0][1].filter.date_from, "2020-01-01");
    f.pending[1].apply([730]);
    assert.equal(f.s.listLimit, 730);
    f.s.bookings = [{ id: 42 }];
    f.publish();
    assert.deepEqual(f.selected, [42]);
});

test("loaded booking selects without a read; missing booking does not select", () => {
    const f = fixture();
    f.s.bookings = [{ id: 42 }]; f.jump(42); f.fire();
    assert.deepEqual(f.selected, [42]); assert.equal(f.pending.length, 0);
    const missing = fixture(); missing.jump(42); missing.fire(); missing.pending[0].apply([null]);
    assert.equal(missing.pending.length, 1); assert.deepEqual(missing.selected, []);
});

test("jump waits for the filter-reset list load before looking up its day", () => {
    const f = fixture(); f.jump(42); f.s.selectionLoadRunning = true; f.fire();
    assert.ok(f.s.waiting); assert.equal(f.pending.length, 0);
    f.s.selectionLoadRunning = false; f.fire();
    assert.equal(f.pending[0].calls[0][0], "booking_date");
});

test("lock, vault swap, newer jump and changed list reject stale work", () => {
    const locked = fixture(); locked.s.isUnlocked = false; locked.jump(42);
    assert.equal(locked.s.revealTimer.request, 0);
    for (const stage of [0, 1, 2]) {
        const f = fixture(); f.jump(42); f.fire();
        if (stage >= 1) f.pending[0].apply(["2020-01-01"]);
        if (stage >= 2) f.pending[1].apply([730]);
        f.swap();
        assert.ok(f.s.stopped && f.cancelled.includes("revealBooking"));
        f.s.bookings = [{ id: 42 }];
        if (stage === 0) f.pending[0].apply(["2020-01-01"]);
        if (stage === 1) f.pending[1].apply([730]);
        if (stage === 2) f.publish();
        assert.deepEqual(f.selected, []);
    }
    const newer = fixture(); newer.jump(42); newer.fire(); newer.jump(43);
    newer.pending[0].apply(["2020-01-01"]); assert.equal(newer.pending.length, 1);
    const changed = fixture(); changed.jump(42); changed.fire(); changed.s.listGeneration++;
    changed.pending[0].apply(["2020-01-01"]); assert.equal(changed.pending.length, 1);
});

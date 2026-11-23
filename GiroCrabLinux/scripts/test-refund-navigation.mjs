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
test("selection totals inherit account currencies without mixing them", () => {
    const text = readFileSync(new URL("../qml/SelectionSummaryBar.qml", import.meta.url), "utf8");
    const totals = bodyAfter("readonly property var totals:", text);
    const s = { shown: true, AppModel: {
        selectedBookings: [{account_id:1,currency:"",amount_cents:100}, {account_id:2,currency:"EUR",amount_cents:200}],
        accountsByID: {1:{currency:"USD"},2:{currency:"EUR"}}
    }};
    vm.createContext(s);
    assert.deepEqual(JSON.parse(JSON.stringify(vm.runInContext(`(function(){${totals}})()`, s))),
        [{currency:"EUR",cents:200},{currency:"USD",cents:100}]);
    s.shown = false;
    assert.equal(vm.runInContext(`(function(){${totals}})()`, s).length, 0);
});
test("zero-total split bookings retain their amount for metadata saves", () => {
    const text=readFileSync(new URL("../qml/ManualBookingSheet.qml",import.meta.url),"utf8");
    const amount=bodyAfter("readonly property var amountCents:",text);
    for (const [hasSplits,existing,input,expected] of [[true,0,"",0],[true,-1200,"invalid",-1200],[false,0,"0",null],[false,0,"12",-1200]]) {
        const s={sheet:{hasSplits},OfflineAccounts:{bookingRequest:{booking:{amount_cents:existing}}},hasSplits,amountField:{text:input},flow:"out",Fmt:{parseCents:t=>/^\d+$/.test(t)?Number(t)*100:null}};
        vm.createContext(s);
        assert.equal(vm.runInContext(`(function(){${amount}})()`,s),expected);
    }
});
test("refund mutation resets busy on identity, queue and worker errors", () => {
    for (const failure of ["identity", "worker", "queue", "locked", "ok"]) {
        const s = { refundEditing:false, isUnlocked:true, vaultLoaded:failure!=="locked", vaultGeneration:1,
            UndoManager:{busy:false,phase:"normal",branch:0,register(){}}, refundLinksRevision:0,
            core(){if(failure==="identity")throw Error("identity failed");return "identity";},
            run(fn){try{return fn();}catch(e){s.errorMessage=e.message;}},
            loadOffMain(){}, coreAsync(method,args,apply,failed){if(failure==="queue")throw Error("queue failed");if(failure==="worker")failed("worker failed");else apply({changed:false});},
            qsTr:s=>s };
        vm.createContext(s);
        vm.runInContext(`(function(pair,action,token){${bodyAfter("function changeRefundLink(pair, action, token)")}})({},"unlink")`,s);
        assert.equal(s.refundEditing,false,failure);
        assert.equal(s.UndoManager.busy,false,failure);
    }
});
test("refund card retains rows on refresh and displays failures inline", () => {
    const card=readFileSync(new URL("../qml/RefundLinksCard.qml",import.meta.url),"utf8");
    const reload=bodyAfter("function reload(clear)",card);
    const s={rows:[{id:1}],error:"",booking:{id:1},AppModel:{isUnlocked:true,loadOffMain(channel,request,apply,failed){this.apply=apply;this.failed=failed;}}};
    vm.createContext(s);
    vm.runInContext(`function reload(clear){${reload}}; reload(false)`,s);
    assert.equal(s.rows.length,1);
    s.AppModel.failed("database unavailable");
    assert.equal(s.error,"database unavailable");
    assert.equal(s.AppModel.errorMessage,undefined);
    assert.equal(s.rows.length,0);
    s.rows=[{id:1}];
    vm.runInContext("reload(true)",s);
    assert.equal(s.rows.length,0);
});
const timerSource = source.slice(source.indexOf("property Timer revealTimer:"));
const timerBody = bodyAfter("onTriggered:", timerSource);

test("expense pie coverage survives list-only reloads", () => {
    const pie = readFileSync(new URL("../qml/HeaderPieChart.qml", import.meta.url), "utf8");
    assert.ok(!pie.includes("AppModel.listGeneration"));
    assert.ok(pie.includes("AppModel.refundLinksRevision"));
    assert.ok(pie.includes("AppModel.vaultGeneration"));
});

test("asynchronous undo cannot restore an invalidated redo branch", () => {
    const undo = readFileSync(new URL("../qml/UndoManager.qml", import.meta.url), "utf8");
    const s = { phase: 0, branch: 0, undoStack: [], redoStack: [] };
    vm.createContext(s);
    const register = (phase, branch) => {
        s.recordedPhase = phase;
        s.recordedBranch = branch;
        vm.runInContext(`(function(name, fn, recordedPhase, recordedBranch) {${bodyAfter("function register(", undo)}})("refund", () => {}, recordedPhase, recordedBranch)`, s);
    };
    register(1, 0);
    assert.equal(s.redoStack.length, 1);
    register(0);
    assert.equal(s.redoStack.length, 0);
    register(1, 0);
    assert.equal(s.redoStack.length, 0);
    register(1, s.branch);
    assert.equal(s.redoStack.length, 1);
});

test("refund edits capture a vault token before queuing the worker", () => {
    let queued, applied;
    const s = {
        refundEditing: false, isUnlocked: true, vaultLoaded:true, vaultGeneration: 1,
        UndoManager: { phase: 0, branch: 0 },
        core(method) { assert.equal(method, "refund_vault_identity"); return "old-vault"; },
        run(fn) {return fn();},
        coreAsync(method,args,apply) { assert.equal(method,"edit_refund"); queued = args; applied = apply; },
    };
    vm.createContext(s);
    vm.runInContext(`(function(pair, action, token) {${bodyAfter("function changeRefundLink(")}})({}, "link", null)`, s);
    s.core = () => "new-vault";
    assert.equal(queued.vault_identity, "old-vault");
    assert.equal(typeof applied, "function");
});

test("refund demo seeding captures a vault token before queuing the worker", () => {
    let queued;
    const s = {
        categories: [],
        core(method) { assert.equal(method, "refund_vault_identity"); return "old-vault"; },
        loadOffMain(channel, request) { assert.equal(channel, "refundDemo"); queued = request; },
    };
    vm.createContext(s);
    vm.runInContext(`(function(name) {${bodyAfter("function presentDemoSheet(")}})("refund-linked")`, s);
    s.core = () => "new-vault";
    assert.equal(queued()[0][0], "seed_refund_demo");
    assert.equal(queued()[0][1].vault_identity, "old-vault");
    assert.equal(queued()[0][1].linked, true);
});

function fixture() {
    const pending = [], selected = [], cancelled = [];
    let publish;
    const s = {
        isUnlocked: true, vaultGeneration: 1, listGeneration: 1, listLimit: 200,
        bookings: [], sidebarSelection: "account:42", period: "month",
        directionFilter: "Income", statusFilter: "checked", scope: "Investments", searchText: "hidden",
        TagFilter: { clear() { s.tagsCleared = true; } },
        DepotState: { presentation: "bookings" },
        categoryAccountFilter: { 9: true },
        ListFilters: { amountMin: 10000, amountMax: 20000, clearAmount() { this.amountMin = null; this.amountMax = null; } },
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

test("loaded counterpart leaves non-booking pages without resetting list filters", () => {
    for (const page of ["documents", "wealth", "planned", "directDebits", "account:42"]) {
        const f = fixture();
        f.s.bookings = [{ id: 42 }]; f.s.sidebarSelection = page;
        f.jump(42); f.fire();
        assert.equal(f.s.sidebarSelection, page === "account:42" ? page : "all");
        assert.equal(f.s.period, "month");
        assert.equal(f.s.searchText, "hidden");
        assert.deepEqual(f.selected, [42]);
    }
});

test("loaded counterpart leaves the securities depot presentation", () => {
    const f = fixture();
    f.s.bookings = [{ id: 42 }];
    f.s.DepotState.presentation = "securities";
    f.jump(42); f.fire();
    assert.equal(f.s.sidebarSelection, "all");
    assert.deepEqual(f.selected, [42]);
});

test("missing counterpart clears the new account and amount filters", () => {
    const f = fixture();
    f.jump(42);
    assert.deepEqual(Object.keys(f.s.categoryAccountFilter), []);
    assert.equal(f.s.ListFilters.amountMin, null);
    assert.equal(f.s.ListFilters.amountMax, null);
});

test("a missing selected split retains the picker until explicit replacement", () => {
    const sheet = readFileSync(new URL("../qml/RefundSheet.qml", import.meta.url), "utf8");
    let failed;
    const s = { AppModel: { refundBooking: {id: 42}, loadOffMain(channel, req, apply, error) { failed = error; } },
        searchTimer: {stop() {}}, ownPart: "deleted", search: "", ownParts: [{part: "remaining"}], booking: {id: 42} };
    vm.createContext(s);
    vm.runInContext(`(function() {${bodyAfter("function reload()", sheet)}})()`, s);
    failed("missing part");
    assert.equal(s.ownPart, "deleted");
    assert.equal(s.ownParts[0].part, "remaining");
    assert.equal(s.error, "missing part");
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

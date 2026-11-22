import GiroCrabCore
import JavaScriptCore
import XCTest
@testable import GiroCrabUI

/// The portal's interface is undocumented; these tests pin down what the page script does with
/// a request the portal itself made, using invented data only.
final class AirPlusTests: XCTestCase {
    func testPortalDomainsAllowLoginAndPortalOnly() {
        let domains = PortalDomains(kind: .airPlus)
        XCTAssertTrue(domains.allows("businesstravelportal.airplus.com"))
        XCTAssertTrue(domains.allows("sso.service.airplus.com"))
        XCTAssertFalse(domains.allows("airplus.evil.example"))
        XCTAssertFalse(domains.allows("example.com"))
        XCTAssertEqual(ConnectorKind.airPlus.portalURL?.host, "businesstravelportal.airplus.com")
    }

    func testScriptNeverTouchesCookiesOrStorage() {
        for forbidden in ["document.cookie", "localStorage", "sessionStorage", "postMessage"] {
            XCTAssertFalse(AirPlusScripts.hook.contains(forbidden), forbidden)
        }
    }

    // MARK: page script in JavaScriptCore

    /// Just enough browser to run the hook: location, URL, XMLHttpRequest, fetch, timers.
    private func makeContext(origin: String = AirPlusScripts.origin) -> JSContext {
        let context = JSContext()!
        var exception: String?
        context.exceptionHandler = { _, value in exception = value?.toString() }
        let polyfill = """
        var window = this;
        var location = {origin: '\(origin)', href: '\(origin)/dashboard'};
        window.fetchCalls = [];
        window.collected = null; window.failure = null;
        window.URL = function(u, base) {
          var s = String(u);
          if (!/^https?:/.test(s)) s = (base ? String(base).replace(/^(https?:\\/\\/[^\\/]+).*$/, '$1') : '') + s;
          var m = s.match(/^(https?:\\/\\/[^\\/?#]+)([^?#]*)/);
          this.origin = m[1]; this.pathname = m[2]; this.href = s;
        };
        window.Headers = function() {};
        window.AbortController = function() { this.signal = {}; this.abort = function() {}; };
        window.setTimeout = function() { return 1; };
        window.clearTimeout = function() {};
        window.XMLHttpRequest = function() { this.listeners = []; this.status = 200; };
        XMLHttpRequest.prototype.open = function(m, u) { this.opened = u; };
        XMLHttpRequest.prototype.setRequestHeader = function(n, v) { (this.sent = this.sent || {})[n] = v; };
        XMLHttpRequest.prototype.addEventListener = function(n, f) { this.listeners.push(f); };
        XMLHttpRequest.prototype.send = function(b) {
          if (typeof this.onload !== 'function') return;   // the portal's own request: tests fire its listeners
          var body = JSON.parse(b);
          window.fetchCalls.push({url: this.opened, headers: this.sent, body: body});
          var rows = [];
          var total = window.mockTotal || 70;
          var first = (body.PageIndex - 1) * body.PageSize;
          for (var i = first; i < Math.min(total, first + body.PageSize); i++) {
            rows.push({AuthorizationNo: 'id-' + i, AuthorizationDate: '2026-03-04T00:00:00+00:00',
              SalesDate: '2026-03-03T00:00:00+00:00', MerchantName: 'BEISPIEL ' + i, MerchantCityName: 'MUSTERSTADT',
              SettledAmount: {Value: 10 + i, Currency: {Code: 'EUR'}}, TransactionType: 'HotelStay', CardNumber: 'must-not-leak'});
          }
          if (window.onReplay) window.onReplay();
          this.status = window.replyStatus || 200;
          this.responseText = JSON.stringify({Item: {Authorizations: rows, TotalRowCount: total}});
          this.onload();
        };
        """
        context.evaluateScript(polyfill)
        context.evaluateScript(AirPlusScripts.hook)
        XCTAssertNil(exception)
        return context
    }

    /// The portal opens the card's booking list: the hook remembers that request.
    private func portalLoadsBookingList(_ context: JSContext) {
        context.evaluateScript("""
        var x = new XMLHttpRequest();
        x.open('POST', '/call/directive/VbExampleDirective/GetAccountCardTransactionListTransaction');
        x.setRequestHeader('x-xsrf-token', 'invented-token');
        x.setRequestHeader('X-Invented-Header', 'kept');
        x.send(JSON.stringify({Card: {CardFinancialAccountId: '170000000001', MaskedCardNumber: '531954******0000',
          LimitUsed: {Value: 123.45, Currency: {Code: 'EUR'}}}, PageIndex: 1, PageSize: 5, Skip: 0, Take: 5,
          Sort: [{Field: 'postingDate', Dir: 'desc'}], Filters: []}));
        x.listeners.forEach(function(f) { f(); });
        """)
    }

    private func collect(_ context: JSContext) -> [String: Any]? {
        context.evaluateScript("""
        window.__gcAirPlus.collect().then(function(r) { window.collected = r; }, function(e) { window.failure = String(e.message); });
        """)
        guard let json = context.objectForKeyedSubscript("collected").toString(), json != "null", json != "undefined",
              let data = json.data(using: .utf8) else { return nil }
        return try? JSONSerialization.jsonObject(with: data) as? [String: Any]
    }

    func testNotReadyUntilThePortalLoadedTheBookingList() {
        let context = makeContext()
        XCTAssertEqual(context.evaluateScript("window.__gcAirPlus.ready()").toBool(), false)
        portalLoadsBookingList(context)
        XCTAssertEqual(context.evaluateScript("window.__gcAirPlus.ready()").toBool(), true)
    }

    func testReplaysTheRequestPageByPageAndHandsOverOnlyWhitelistedFields() throws {
        let context = makeContext()
        portalLoadsBookingList(context)
        let snapshot = try XCTUnwrap(collect(context))
        XCTAssertEqual(snapshot["version"] as? Int, 1)
        let card = try XCTUnwrap(snapshot["card"] as? [String: Any])
        XCTAssertEqual(card["id"] as? String, "170000000001")
        XCTAssertEqual(card["limitUsed"] as? Double, 123.45)
        XCTAssertEqual(card["currency"] as? String, "EUR")
        let rows = try XCTUnwrap(snapshot["transactions"] as? [[String: Any]])
        XCTAssertEqual(rows.count, 70)
        XCTAssertEqual(rows[0]["id"] as? String, "id-0")
        XCTAssertEqual(rows[69]["amount"] as? Double, 79)
        XCTAssertEqual(Set(rows[0].keys), ["id", "bookingDate", "salesDate", "merchant", "city", "amount", "currency", "type"])
        XCTAssertFalse((context.objectForKeyedSubscript("collected").toString() ?? "").contains("must-not-leak"))

        let calls = try XCTUnwrap(context.evaluateScript("JSON.stringify(window.fetchCalls.map(function(c) { return [c.body.PageIndex, c.body.Skip, c.body.Take, c.headers['x-xsrf-token'], c.headers['X-Invented-Header'], c.headers['Content-Type']]; }))").toString())
        XCTAssertEqual(calls, #"[[1,0,50,"invented-token","kept","application/json"],[2,50,50,"invented-token","kept","application/json"]]"#)
    }

    func testReportsHTTPFailuresAndForgetsTheRequestAfterLogout() throws {
        let context = makeContext()
        portalLoadsBookingList(context)
        context.evaluateScript("window.replyStatus = 401;")
        XCTAssertNil(collect(context))
        XCTAssertEqual(context.objectForKeyedSubscript("failure").toString(),
            "AirPlus: Abruf fehlgeschlagen (HTTP 401). Bitte erneut anmelden.")
        XCTAssertEqual(context.evaluateScript("window.__gcAirPlus.ready()").toBool(), false)
    }

    func testRefusesOutsideThePortalOrigin() {
        let context = makeContext(origin: "https://example.com")
        XCTAssertNil(context.objectForKeyedSubscript("__gcAirPlus").toObject() as? [String: Any],
            "the hook installs nothing on other origins")
        XCTAssertEqual(context.evaluateScript("typeof window.__gcAirPlus").toString(), "undefined")
    }

    func testFailsWithoutACapturedRequest() {
        let context = makeContext()
        XCTAssertNil(collect(context))
        XCTAssertEqual(context.objectForKeyedSubscript("failure").toString(),
            "AirPlus: Keine Karte erkannt.")
    }

    func testShowsTheMessageThePageScriptThrew() {
        let thrown = NSError(domain: "WKErrorDomain", code: 4, userInfo: [
            NSLocalizedDescriptionKey: "A JavaScript exception occurred",
            "WKJavaScriptExceptionMessage": "AirPlus: Abruf fehlgeschlagen (HTTP 400). Bitte erneut anmelden."])
        XCTAssertEqual(AirPlusWebSheet.message(thrown), "AirPlus: Abruf fehlgeschlagen (HTTP 400). Bitte erneut anmelden.")
        XCTAssertEqual(AirPlusWebSheet.message(NSError(domain: "x", code: 1, userInfo: [NSLocalizedDescriptionKey: "plain"])), "plain")
    }

    func testStopsWithAnErrorWhenTheListIsLongerThanThePageLimit() {
        let context = makeContext()
        portalLoadsBookingList(context)
        context.evaluateScript("window.mockTotal = 20000;")
        XCTAssertNil(collect(context))
        XCTAssertEqual(context.objectForKeyedSubscript("failure").toString(),
            "AirPlus: Die Umsatzliste ist zu lang und wurde nicht vollständig abgerufen.")
    }

    func testKeepsTheRequestOfTheCardItStartedWith() throws {
        let context = makeContext()
        portalLoadsBookingList(context)
        // while the pages load the portal sends the list request of another card
        context.evaluateScript("""
        window.onReplay = function() {
          window.onReplay = null;
          var y = new XMLHttpRequest();
          y.open('POST', '/call/directive/VbExampleDirective/GetAccountCardTransactionListTransaction');
          y.setRequestHeader('x-xsrf-token', 'other-token');
          y.send(JSON.stringify({Card: {CardFinancialAccountId: '999999999999'}, PageIndex: 1, PageSize: 5}));
          y.listeners.forEach(function(f) { f(); });
        };
        """)
        let snapshot = try XCTUnwrap(collect(context))
        XCTAssertEqual((snapshot["card"] as? [String: Any])?["id"] as? String, "170000000001")
        let ids = try XCTUnwrap(context.evaluateScript("JSON.stringify(window.fetchCalls.map(function(c) { return [c.body.Card.CardFinancialAccountId, c.headers['x-xsrf-token']]; }))").toString())
        XCTAssertEqual(ids, #"[["170000000001","invented-token"],["170000000001","invented-token"]]"#)
    }
}

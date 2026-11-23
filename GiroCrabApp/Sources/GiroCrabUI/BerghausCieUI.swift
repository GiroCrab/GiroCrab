import SwiftUI
import WebKit
import GiroCrabCore

/// Berghaus & Cie wine investment portal. Design page: `docs/berghauscie-connector.md`.
///
/// The user signs in (e-mail and password) in the visible web view. The portal then talks to a
/// GraphQL service with its own session; the page script remembers the authorization the portal
/// itself sends, in a JavaScript closure that never reaches Swift, logs or the vault, and asks
/// that service for the wine depot only. The sign-in request is never read or replayed.
/// Only the selected fields (bottles, purchase price, valuation, wine name) cross the bridge.
enum BerghausCieScripts {
    static let hook = #"""
    (() => {
      const pageOrigin = 'https://my.berghauscie.de';
      if (location.origin !== pageOrigin || window.__gcBerghausCie) return;
      const api = 'https://bc-cms.herokuapp.com/graphql';
      const SOURCE = 'WINE_SEARCHER_EUROPE_AVERAGE_EURO';
      const nativeFetch = window.fetch;
      let authorization = null;
      let signedIn = false;
      const fail = message => { throw new Error('Berghaus & Cie: ' + message); };
      const trusted = url => {
        try { const u = new URL(url, location.href); return u.origin + u.pathname === api; } catch { return false; }
      };
      // signed in = the service answered with a user (never the sign-in response's token)
      const inspect = text => {
        try {
          const parsed = JSON.parse(text);
          const answer = Array.isArray(parsed) ? parsed[0] : parsed;
          if (answer && answer.data && answer.data.me && typeof answer.data.me === 'object') signedIn = true;
          if (answer && answer.data && answer.data.tokenAuth && answer.data.tokenAuth.token) signedIn = true;
        } catch {}
      };
      const remember = headers => {
        try { const value = new Headers(headers || {}).get('authorization'); if (value) authorization = value; } catch {}
      };
      window.fetch = function(input, init) {
        const url = input instanceof Request ? input.url : String(input);
        const headers = init?.headers || (input instanceof Request ? input.headers : {});
        const response = nativeFetch.apply(this, arguments);
        if (trusted(url)) {
          response.then(r => { if (r.ok) { remember(headers); r.clone().text().then(inspect).catch(() => {}); } }).catch(() => {});
        }
        return response;
      };
      const open = XMLHttpRequest.prototype.open;
      const setHeader = XMLHttpRequest.prototype.setRequestHeader;
      const requests = new WeakMap();
      XMLHttpRequest.prototype.open = function(method, url) {
        const request = {url: String(url), headers: {}};
        requests.set(this, request);
        this.addEventListener('load', () => {
          if (this.status >= 200 && this.status < 300 && trusted(request.url)) {
            remember(request.headers);
            try { inspect(this.responseText); } catch {}
          }
        }, {once:true});
        return open.apply(this, arguments);
      };
      XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
        const request = requests.get(this);
        if (request && trusted(request.url)) request.headers[name] = value;
        return setHeader.apply(this, arguments);
      };
      // Only what the depot needs: no names, addresses, invoices or images.
      const QUERY = '{ clientsDepot { id amount caskOf buyPriceBtl bottleSize { title factorForLiterPrices } ' +
        'vintage { title year wine { lwin } valuationRel { sourceOfValuation datetimeCreated value } } ' +
        'depotPositionStatusRel { status datetimeStatusUpdate } } }';
      // positions that left the depot (sold, delivered to the customer, cancelled): exact status
      // names only, so a negated or compound status (NOT_DELIVERED) stays in the depot
      const GONE = /^(SOLD|DELIVERED|WITHDRAWN|CANCELLED|CANCELED|RETURNED)$/;
      const request = async () => {
        if (location.origin !== pageOrigin || !signedIn) fail('Bitte erneut anmelden.');
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 30000);
        try {
          const headers = {'Content-Type':'application/json', 'Accept':'application/json'};
          if (authorization) headers.Authorization = authorization;
          const r = await nativeFetch.call(window, api, {
            method: 'POST', credentials: 'include', redirect: 'error', headers,
            body: JSON.stringify({query: QUERY}), signal: controller.signal
          });
          if (r.status === 401 || r.status === 403) { signedIn = false; authorization = null; }
          if (!r.ok) fail('Abruf fehlgeschlagen (HTTP ' + r.status + '). Bitte erneut anmelden.');
          return await r.json();
        } catch (e) {
          if (e && /^Berghaus/.test(e.message)) throw e;
          fail('Abruf fehlgeschlagen.');
        } finally { clearTimeout(timeout); }
      };
      const cents = n => Math.round(n * 100) / 100;
      const whole = n => Number.isInteger(n) && n > 0;
      const parse = answer => {
        if (!answer || answer.errors || !answer.data || !Array.isArray(answer.data.clientsDepot))
          fail('Keine Depotdaten erkannt. Bitte erneut anmelden.');
        const items = [];
        const problems = new Map();
        const problem = reason => problems.set(reason, (problems.get(reason) || 0) + 1);
        for (const p of answer.data.clientsDepot) {
          const status = (p.depotPositionStatusRel || []).slice()
            .sort((a, b) => String(a.datetimeStatusUpdate).localeCompare(String(b.datetimeStatusUpdate))).pop();
          if (status && GONE.test(String(status.status).trim().toUpperCase())) continue;
          const v = p.vintage || {};
          // the newest valuation of the source decides; an unusable newest one is an error, never
          // a reason to fall back to an older value
          const valuations = (v.valuationRel || []).filter(x => x && x.sourceOfValuation === SOURCE && x.datetimeCreated)
            .sort((a, b) => String(a.datetimeCreated).localeCompare(String(b.datetimeCreated)));
          const newest = valuations[valuations.length - 1];
          const last = newest && Number.isFinite(newest.value) && newest.value >= 0 ? newest : null;
          const factor = p.bottleSize && p.bottleSize.factorForLiterPrices;
          // a case holds `caskOf` bottles, `amount` is the number of cases
          if (!whole(p.amount) || !whole(p.caskOf)) { problem('Menge'); continue; }
          if (!Number.isFinite(p.buyPriceBtl) || p.buyPriceBtl < 0) { problem('Einstandspreis'); continue; }
          if (!Number.isFinite(factor) || factor <= 0) { problem('Flaschengröße'); continue; }
          if (!last) { problem('Bewertung'); continue; }
          // the valuation is the price of a 0.75 l bottle; larger bottles scale with their volume
          const bottles = p.amount * p.caskOf;
          const exact = last.value * factor / 0.75;
          const size = String(p.bottleSize.title || '').trim();
          const title = String(v.title || '').replace(/\s+/g, ' ').trim();
          items.push({id:String(p.id), lwin:v.wine && v.wine.lwin != null ? String(v.wine.lwin) : null,
            name:(size && size !== '0,75l' ? title + ' (' + size + ')' : title).slice(0, 200),
            bottles, price:cents(exact), purchasePrice:p.buyPriceBtl,
            valueDate:String(last.datetimeCreated).slice(0, 10), value:cents(bottles * exact)});
        }
        // never import a partly understood depot: it would remove the positions left out
        if (problems.size > 0)
          fail(('Unbekanntes Depotformat: ' + [...problems].map(([r, n]) => n + '× ' + r).join(', ')).slice(0, 150));
        return items;
      };
      window.__gcBerghausCie = {
        ready: () => location.origin === pageOrigin && signedIn,
        collect: async () => JSON.stringify({version: 1, asOf: new Date().toISOString().slice(0, 10),
          depot: {items: parse(await request())}})
      };
    })();
    """#

    static let probe = #"""
    location.origin === 'https://my.berghauscie.de' && !!window.__gcBerghausCie?.ready()
    """#
}

struct BerghausCieWebSheet: View {
    @Environment(\.reportRedaction) private var redaction
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let connection: BankConnection
    @State private var browser = PortalBrowser(url: URL(string: "https://my.berghauscie.de/")!,
        domains: PortalDomains(kind: .berghausCie), captureTraffic: false, userScript: BerghausCieScripts.hook)
    @State private var uiDelegate = AmazonUIDelegate()
    @State private var ready = false
    @State private var running = false
    @State private var progress = ""
    @State private var notice: String?
    @State private var task: Task<Void, Never>?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("\(redaction.masking(connection.displayName)) – Berghaus & Cie").appFont(.headline)
                    if running { Text(progress).appFont(.caption) }
                    else if let notice { Text(notice).appFont(.caption).foregroundStyle(Theme.warning) }
                    else { Text(ready ? String(localized: "Angemeldet – der Bestand kann abgerufen werden.") : String(localized: "Melden Sie sich bei Berghaus & Cie an (E-Mail und Passwort).")).appFont(.caption) }
                }
                Spacer()
                if running || browser.loading { ProgressView().controlSize(.small) }
            }.padding(14)
            Divider()
            PortalWebView(browser: browser).macFrame(minWidth: 760, minHeight: 620)
            Divider()
            HStack {
                Button("Schließen", role: .cancel) { task?.cancel(); close() }.keyboardShortcut(.cancelAction)
                Spacer()
                Button("Bestand abrufen") { start() }.buttonStyle(.borderedProminent).disabled(!ready || running)
            }.padding(12)
        }
        .webPageDialogs(uiDelegate)
        .task {
            browser.webView.uiDelegate = uiDelegate
            while !Task.isCancelled {
                let authenticated = (try? await browser.webView.evaluateJavaScript(BerghausCieScripts.probe)) as? Bool ?? false
                let ok = browser.onPortal && authenticated
                if ready != ok { ready = ok }
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
        .onDisappear { task?.cancel(); browser.teardown() }
        .interactiveDismissDisabled(running)
    }

    private func run(_ script: String) async throws -> String {
        try Task.checkCancellation()
        let guardScript = "if (location.origin !== 'https://my.berghauscie.de' || !window.__gcBerghausCie) throw new Error('Berghaus & Cie: Bitte erneut anmelden.');\n"
        let result: Any?
        do {
            result = try await browser.webView.callAsyncJavaScript(guardScript + script, arguments: [:], in: nil, contentWorld: .page)
        } catch {
            try Task.checkCancellation()
            // The page script's own message; WebKit's generic text says nothing. Text that
            // carries a URL is never shown (it can hold tokens).
            let message = ((error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String) ?? ""
            if !message.isEmpty, message.count <= 160, !message.contains("://") {
                throw BerghausCieError.portal(message)
            }
            throw BerghausCieError.unreadable
        }
        try Task.checkCancellation()
        guard let string = result as? String else { throw BerghausCieError.unreadable }
        return string
    }

    private func start() {
        running = true
        notice = nil
        task = Task {
            guard let pin = model.pinVault() else { running = false; return }
            let connectionId = connection.id
            do {
                progress = String(localized: "Bestand abrufen …")
                let snapshot = try await run("return await window.__gcBerghausCie.collect();")
                let summary = try await pin.run { try $0.importBerghauscieSnapshot(connectionId: connectionId, snapshotJson: snapshot) }
                if model.isCurrent(pin) {
                    model.toast = String(localized: "Berghaus & Cie: \(summary.accountsUpdated) Depot aktualisiert.")
                    close()
                }
            } catch is CancellationError {
            } catch { notice = describe(error) }
            await pin.checkpoint()
            running = false
            model.reloadAll()
            // accounts another login already delivers wait for a choice (#346)
            model.checkPendingAccountSelection()
        }
    }

    private func close() { model.webLoginConnection = nil; dismiss() }
}

private enum BerghausCieError: LocalizedError {
    case unreadable
    case portal(String)
    var errorDescription: String? {
        switch self {
        case .unreadable: return String(localized: "Berghaus & Cie: Die Portalantwort konnte nicht gelesen werden.")
        case .portal(let message): return message
        }
    }
}

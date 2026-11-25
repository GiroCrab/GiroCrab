import SwiftUI
import WebKit
import GiroCrabCore

/// AXA Mitarbeiter Shareplan (employee savings plan portal). Design page: `docs/axashareplan-connector.md`.
///
/// The user signs in (credentials, SMS code) in the visible web view. The page script remembers the
/// headers the portal itself sends to its data interface, in a JavaScript closure that never reaches
/// Swift, logs or the vault, and asks that interface for the plan's funds only (two read queries
/// with fixed bodies). The sign-in requests are never read or replayed. Only the selected fields
/// (fund name and identifier, shares, prices, invested amount, value) cross the bridge.
enum AxaShareplanScripts {
    static let hook = #"""
    (() => {
      const pageOrigin = 'https://espace-salarie.axa-mon-epargne-entreprises.com';
      if (location.origin !== pageOrigin || window.__gcAxaShareplan) return;
      const nativeFetch = window.fetch;
      // The data interface lives under /<version>/api/b2b/; the version changes with portal releases.
      const API = /^\/(\d+\.\d+\.\d+)\/api\/b2b(\/.*)$/;
      // Only calls that need a signed-in user count as "signed in" and give us the portal's headers.
      const PROFILE = /^\/profile\/(me|affiliations)$/;
      let prefix = null;
      let headers = {};
      let signedIn = false;
      const fail = message => { throw new Error('AXA Shareplan: ' + message); };
      const parts = url => {
        try {
          const u = new URL(url, location.href);
          const m = u.origin === pageOrigin ? API.exec(u.pathname) : null;
          return m ? {base: '/' + m[1] + '/api/b2b', path: m[2]} : null;
        } catch { return null; }
      };
      const remember = (url, sent) => {
        const p = parts(url);
        if (!p || !PROFILE.test(p.path)) return;
        prefix = p.base;
        try { new Headers(sent || {}).forEach((value, name) => { headers[name] = value; }); } catch {}
        signedIn = true;
      };
      window.fetch = function(input, init) {
        const url = input instanceof Request ? input.url : String(input);
        const sent = init?.headers || (input instanceof Request ? input.headers : {});
        const response = nativeFetch.apply(this, arguments);
        if (parts(url)) response.then(r => { if (r.ok) remember(url, sent); }).catch(() => {});
        return response;
      };
      const open = XMLHttpRequest.prototype.open;
      const setHeader = XMLHttpRequest.prototype.setRequestHeader;
      const requests = new WeakMap();
      XMLHttpRequest.prototype.open = function(method, url) {
        const request = {url: String(url), headers: {}};
        requests.set(this, request);
        this.addEventListener('load', () => {
          if (this.status >= 200 && this.status < 300) remember(request.url, request.headers);
        }, {once:true});
        return open.apply(this, arguments);
      };
      XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
        const request = requests.get(this);
        if (request && parts(request.url)) request.headers[name] = value;
        return setHeader.apply(this, arguments);
      };
      // Read queries with fixed bodies (the portal uses POST for filters): the plan's funds and their names.
      const call = async (path, body) => {
        if (location.origin !== pageOrigin || !signedIn || !prefix) fail('Bitte erneut anmelden.');
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 30000);
        try {
          const sent = Object.assign({}, headers, {'content-type': 'application/json', 'accept': 'application/json'});
          const r = await nativeFetch.call(window, prefix + path, {
            method: 'POST', credentials: 'same-origin', redirect: 'error', headers: sent,
            body: JSON.stringify(body), signal: controller.signal
          });
          if (r.status === 401 || r.status === 403) { signedIn = false; headers = {}; }
          if (!r.ok) fail('Abruf fehlgeschlagen (HTTP ' + r.status + '). Bitte erneut anmelden.');
          return await r.json();
        } catch (e) {
          if (e && /^AXA Shareplan/.test(e.message)) throw e;
          fail('Abruf fehlgeschlagen.');
        } finally { clearTimeout(timeout); }
      };
      const LIMIT = 500;
      const ISIN = /^[A-Z]{2}[A-Z0-9]{9}\d$/;
      // amounts are whole minor units (cents) of EUR
      const money = m => m && m.currency === 'EUR' && Number.isInteger(m.value) && m.value >= 0 ? m.value : null;
      const parse = (portfolio, instruments) => {
        if (!portfolio || !portfolio.portfolio || !Array.isArray(portfolio.portfolio.positions))
          fail('Keine Depotdaten erkannt. Bitte erneut anmelden.');
        const positions = portfolio.portfolio.positions;
        if (positions.length >= LIMIT) fail('Mehr Positionen als abrufbar.');
        const names = new Map();
        for (const i of (instruments && Array.isArray(instruments.instruments) ? instruments.instruments : []))
          if (i && Number.isInteger(i.instrument_id)) names.set(i.instrument_id, i);
        const items = [];
        const problems = new Map();
        const problem = reason => problems.set(reason, (problems.get(reason) || 0) + 1);
        for (const p of positions) {
          if (!Number.isInteger(p.instrument_id) || p.instrument_id <= 0) { problem('Kennung'); continue; }
          if (!Number.isFinite(p.shares_owned) || p.shares_owned < 0) { problem('Stückzahl'); continue; }
          // fully sold or transferred: not a position any more
          if (p.shares_owned === 0) continue;
          const price = Number.isInteger(p.market_price) && p.market_price >= 0 ? p.market_price : null;
          const average = money(p.avg_price), invested = money(p.total_invested), value = money(p.total_value);
          if (price === null) { problem('Kurs'); continue; }
          if (average === null || invested === null) { problem('Einstand'); continue; }
          if (value === null) { problem('Wert'); continue; }
          const i = names.get(p.instrument_id) || {};
          const name = String(i.name || p.ticker || '').replace(/\s+/g, ' ').trim().slice(0, 200);
          if (!name) { problem('Name'); continue; }
          items.push({id: String(p.instrument_id), isin: ISIN.test(String(i.isin || '')) ? String(i.isin) : null, name,
            quantity: p.shares_owned, priceCents: price, averagePriceCents: average, investedCents: invested, valueCents: value});
        }
        // never import a partly understood depot: it would remove the positions left out
        if (problems.size > 0)
          fail(('Unbekanntes Depotformat: ' + [...problems].map(([r, n]) => n + '× ' + r).join(', ')).slice(0, 150));
        return items;
      };
      window.__gcAxaShareplan = {
        ready: () => location.origin === pageOrigin && signedIn,
        collect: async () => {
          const portfolio = await call('/profile/portfolio', {type: 'mutual-fund', offset: 0, limit: LIMIT});
          const ids = (portfolio?.portfolio?.positions || []).map(p => p.instrument_id).filter(Number.isInteger);
          const instruments = ids.length ? await call('/instruments', {instrument_ids: ids}) : null;
          return JSON.stringify({version: 1, asOf: new Date().toISOString().slice(0, 10), depot: {items: parse(portfolio, instruments)}});
        }
      };
    })();
    """#

    static let probe = #"""
    location.origin === 'https://espace-salarie.axa-mon-epargne-entreprises.com' && !!window.__gcAxaShareplan?.ready()
    """#
}

struct AxaShareplanWebSheet: View {
    @Environment(\.reportRedaction) private var redaction
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let connection: BankConnection
    @State private var browser = PortalBrowser(url: URL(string: "https://espace-salarie.axa-mon-epargne-entreprises.com/")!,
        domains: PortalDomains(kind: .axaShareplan), captureTraffic: false, userScript: AxaShareplanScripts.hook)
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
                    Text("\(redaction.masking(connection.displayName)) – AXA Mitarbeiter Shareplan").appFont(.headline)
                    if running { Text(progress).appFont(.caption) }
                    else if let notice { Text(notice).appFont(.caption).foregroundStyle(Theme.warning) }
                    else { Text(ready ? String(localized: "Angemeldet – der Bestand kann abgerufen werden.") : String(localized: "Melden Sie sich beim AXA Mitarbeiter Shareplan an (Zugangsdaten und SMS-Code).")).appFont(.caption) }
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
                let authenticated = (try? await browser.webView.evaluateJavaScript(AxaShareplanScripts.probe)) as? Bool ?? false
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
        let guardScript = "if (location.origin !== 'https://espace-salarie.axa-mon-epargne-entreprises.com' || !window.__gcAxaShareplan) throw new Error('AXA Shareplan: Bitte erneut anmelden.');\n"
        let result: Any?
        do {
            result = try await browser.webView.callAsyncJavaScript(guardScript + script, arguments: [:], in: nil, contentWorld: .page)
        } catch {
            try Task.checkCancellation()
            // The page script's own message; WebKit's generic text says nothing. Text that
            // carries a URL is never shown (it can hold tokens).
            let message = ((error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String) ?? ""
            if !message.isEmpty, message.count <= 160, !message.contains("://") {
                throw AxaShareplanError.portal(message)
            }
            throw AxaShareplanError.unreadable
        }
        try Task.checkCancellation()
        guard let string = result as? String else { throw AxaShareplanError.unreadable }
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
                let snapshot = try await run("return await window.__gcAxaShareplan.collect();")
                let summary = try await pin.run { try $0.importAxashareplanSnapshot(connectionId: connectionId, snapshotJson: snapshot) }
                if model.isCurrent(pin) {
                    model.toast = String(localized: "AXA Shareplan: \(summary.accountsUpdated) Depot aktualisiert.")
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

private enum AxaShareplanError: LocalizedError {
    case unreadable
    case portal(String)
    var errorDescription: String? {
        switch self {
        case .unreadable: return String(localized: "AXA Shareplan: Die Portalantwort konnte nicht gelesen werden.")
        case .portal(let message): return message
        }
    }
}

import SwiftUI
import WebKit
import GiroCrabCore

/// AirPlus Corporate Card (#1143). Design: `docs/airplus-connector.md`.
///
/// Login (OpenID Connect + SMS TAN) and all requests stay in the page. The script remembers
/// the portal's own request for the card's booking list (URL, anti-forgery header, body) in
/// its closure and replays it page by page; neither the header nor cookies reach Swift, logs
/// or the vault. Only whitelisted booking fields and two card values are handed over.
enum AirPlusScripts {
    static let origin = "https://businesstravelportal.airplus.com"

    /// Installed before the portal's first request.
    static let hook = #"""
    (() => {
      const origin = 'https://businesstravelportal.airplus.com';
      if (location.origin !== origin || window.__gcAirPlus) return;
      const listName = 'GetAccountCardTransactionListTransaction';
      let captured = null;   // {url, headers, body}
      const trusted = url => {
        try { const u = new URL(url, location.href);
          return u.origin === origin && (u.pathname.startsWith('/call/directive/') || u.pathname.startsWith('/services/'));
        } catch { return false; }
      };
      const open = XMLHttpRequest.prototype.open;
      const setHeader = XMLHttpRequest.prototype.setRequestHeader;
      const send = XMLHttpRequest.prototype.send;
      const requests = new WeakMap();
      XMLHttpRequest.prototype.open = function(method, url) {
        requests.set(this, {url: new URL(String(url), location.href).href, headers: {}});
        return open.apply(this, arguments);
      };
      XMLHttpRequest.prototype.setRequestHeader = function(name, value) {
        const r = requests.get(this);
        if (r) r.headers[String(name)] = String(value);
        return setHeader.apply(this, arguments);
      };
      XMLHttpRequest.prototype.send = function(body) {
        const r = requests.get(this);
        if (r && trusted(r.url) && new URL(r.url).pathname.endsWith('/' + listName) && typeof body === 'string') {
          this.addEventListener('load', () => {
            if (this.status >= 200 && this.status < 300) {
              try { captured = {url: r.url, headers: r.headers, body: JSON.parse(body)}; } catch {}
            }
          }, {once: true});
        }
        return send.apply(this, arguments);
      };
      // Replays the portal's own request the way the portal sent it: XMLHttpRequest, same headers,
      // cookies of the page. The original methods are used, so the replay is not captured again.
      const replay = (cap, body) => new Promise((resolve, reject) => {
        const x = new XMLHttpRequest();
        open.call(x, 'POST', cap.url, true);
        const headers = Object.assign({'Accept': 'application/json', 'Content-Type': 'application/json'}, cap.headers);
        for (const name of Object.keys(headers)) setHeader.call(x, name, headers[name]);
        x.timeout = 30000;
        x.onload = () => resolve({status: x.status, text: x.responseText});
        x.onerror = () => reject(new Error('AirPlus: Netzwerkfehler beim Abruf. Bitte erneut anmelden.'));
        x.ontimeout = () => reject(new Error('AirPlus: Zeitüberschreitung beim Abruf.'));
        send.call(x, JSON.stringify(body));
      });
      // `cap` is the request pinned at the start of one collection: a request the portal
      // sends meanwhile (another card) must not mix into the pages of this one.
      const page = async (cap, index, size) => {
        if (location.origin !== origin || !cap || !trusted(cap.url))
          throw new Error('AirPlus: Bitte erneut anmelden und die Umsatzübersicht der Karte öffnen.');
        const body = Object.assign({}, cap.body, {PageIndex: index, PageSize: size, Skip: (index - 1) * size, Take: size});
        const r = await replay(cap, body);
        if (r.status < 200 || r.status >= 300) {
          if (r.status === 401 || r.status === 403) captured = null;
          throw new Error('AirPlus: Abruf fehlgeschlagen (HTTP ' + r.status + '). Bitte erneut anmelden.');
        }
        try { return JSON.parse(r.text); } catch { throw new Error('AirPlus: Ungültige Antwort des Portals.'); }
      };
      const row = t => ({
        id: t.AuthorizationNo, bookingDate: t.AuthorizationDate, salesDate: t.SalesDate,
        merchant: t.MerchantName, city: t.MerchantCityName, amount: t.SettledAmount?.Value,
        currency: t.SettledAmount?.Currency?.Code, type: t.TransactionType
      });
      window.__gcAirPlus = {
        ready: () => !!captured,
        collect: async () => {
          const size = 50, maxPages = 200;
          const cap = captured;
          const card = cap?.body?.Card;
          if (!card?.CardFinancialAccountId) throw new Error('AirPlus: Keine Karte erkannt.');
          const rows = [];
          let complete = false;
          for (let index = 1; index <= maxPages; index++) {
            const data = await page(cap, index, size);
            if (!Array.isArray(data?.Item?.Authorizations)) throw new Error('AirPlus: Ungültige Umsatzliste.');
            rows.push(...data.Item.Authorizations.map(row));
            const total = data.Item.TotalRowCount;
            if (data.Item.Authorizations.length < size || (Number.isFinite(total) && rows.length >= total)) { complete = true; break; }
          }
          if (!complete) throw new Error('AirPlus: Die Umsatzliste ist zu lang und wurde nicht vollständig abgerufen.');
          const used = card.LimitUsed?.Value;
          const limit = card.AvailableLimit?.Value;
          return JSON.stringify({
            version: 1,
            card: {id: String(card.CardFinancialAccountId), maskedNumber: card.MaskedCardNumber,
              currency: rows[0]?.currency || card.LimitUsed?.Currency?.Code || 'EUR',
              limitUsed: Number.isFinite(used) ? used : null,
              creditLimit: Number.isFinite(limit) ? limit : null,
              balanceDate: new Date().toISOString().slice(0, 10)},
            transactions: rows
          });
        }
      };
    })();
    """#

    static let probe = #"""
    location.origin === 'https://businesstravelportal.airplus.com' && !!window.__gcAirPlus?.ready()
    """#
}

struct AirPlusWebSheet: View {
    @Environment(\.reportRedaction) private var redaction
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let connection: BankConnection
    @State private var browser = PortalBrowser(url: URL(string: "https://businesstravelportal.airplus.com/")!,
        domains: PortalDomains(kind: .airPlus), captureTraffic: false, userScript: AirPlusScripts.hook)
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
                    Text("\(redaction.masking(connection.displayName)) – AirPlus").appFont(.headline)
                    if running { Text(progress).appFont(.caption) }
                    else if let notice { Text(notice).appFont(.caption).foregroundStyle(Theme.warning) }
                    else { Text(ready
                        ? String(localized: "Umsatzübersicht erkannt – die Umsätze können abgerufen werden.")
                        : String(localized: "Melden Sie sich bei AirPlus an, bestätigen Sie die TAN und öffnen Sie die Umsatzübersicht Ihrer Karte.")).appFont(.caption) }
                }
                Spacer()
                if running || browser.loading { ProgressView().controlSize(.small) }
            }.padding(14)
            Divider()
            PortalWebView(browser: browser).macFrame(minWidth: 760, minHeight: 620)
            Divider()
            HStack {
                Button("Schließen", role: .cancel) { task?.cancel(); close() }.keyboardShortcut(.cancelAction)
                Button("Neu anmelden") { browser.webView.load(URLRequest(url: URL(string: AirPlusScripts.origin + "/")!)) }
                    .disabled(running)
                Spacer()
                Button("Umsätze abrufen") { start() }.buttonStyle(.borderedProminent).disabled(!ready || running)
            }.padding(12)
        }
        .webPageDialogs(uiDelegate)
        .task {
            browser.webView.uiDelegate = uiDelegate
            while !Task.isCancelled {
                let seen = (try? await browser.webView.evaluateJavaScript(AirPlusScripts.probe)) as? Bool ?? false
                let ok = browser.onPortal && seen
                if ready != ok { ready = ok }
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
        .onDisappear { task?.cancel(); browser.teardown() }
        .interactiveDismissDisabled(running)
    }

    private func collect() async throws -> String {
        try Task.checkCancellation()
        let guardScript = "if (location.origin !== '\(AirPlusScripts.origin)' || !window.__gcAirPlus) throw new Error('AirPlus: Bitte erneut anmelden.');\n"
        let result = try await browser.webView.callAsyncJavaScript(guardScript + "return await window.__gcAirPlus.collect();",
            arguments: [:], in: nil, contentWorld: .page)
        try Task.checkCancellation()
        guard let string = result as? String else { throw AirPlusError.unreadable }
        return string
    }

    private func start() {
        running = true
        notice = nil
        task = Task {
            guard let pin = model.pinVault() else { running = false; return }
            let connectionId = connection.id
            do {
                progress = String(localized: "Umsätze abrufen …")
                let snapshot = try await collect()
                let summary = try await pin.run { try $0.importAirplusSnapshot(connectionId: connectionId, snapshotJson: snapshot) }
                guard model.isCurrent(pin) else { running = false; return }
                model.toast = String(localized: "AirPlus: \(summary.accountsUpdated) Konten aktualisiert, \(summary.transactionsImported) neue Buchungen.")
                close()
            } catch is CancellationError {
            } catch { notice = Self.message(error) }
            await pin.checkpoint()
            running = false
            model.reloadAll()
            // accounts another login already delivers wait for a choice (#346)
            model.checkPendingAccountSelection()
        }
    }

    private func close() { model.webLoginConnection = nil; dismiss() }

    /// WebKit reports a thrown page error only as "A JavaScript exception occurred"; the text the
    /// script threw (our own messages or the browser's, e.g. a failed request) is in the user info.
    static func message(_ error: Error) -> String {
        let info = (error as NSError).userInfo
        if let text = info["WKJavaScriptExceptionMessage"] as? String, !text.isEmpty { return text }
        return describe(error)
    }
}

private enum AirPlusError: LocalizedError {
    case unreadable
    var errorDescription: String? { String(localized: "AirPlus: Die Portalantwort konnte nicht gelesen werden.") }
}

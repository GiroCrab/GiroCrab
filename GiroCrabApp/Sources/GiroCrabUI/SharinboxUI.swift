import SwiftUI
import WebKit
import GiroCrabCore

/// Sharinbox (Société Générale employee share plans). Design page: `docs/sharinbox-connector.md`.
///
/// The user signs in through SG Connect (e-mail, password and authenticator app) in the visible
/// web view. The start page is server-rendered HTML in the signed-in session, so the page script
/// reads it with a same-origin request; no token or cookie ever reaches Swift, logs or the vault.
/// Only the selected share data (ISIN, name, quantity, price, value) crosses the bridge.
enum SharinboxScripts {
    static let hook = #"""
    (() => {
      const pageOrigin = 'https://sharinbox.sgmarkets.com';
      if (location.origin !== pageOrigin || window.__gcSharinbox) return;
      const HOME = '/SharinBox/connexion.do';
      const fail = message => { throw new Error('Sharinbox: ' + message); };
      // "735.330,60" -> 735330.6; "16.290" -> 16290; "-" (nothing held) -> 0
      const amount = raw => {
        const s = String(raw || '').replace(/[\s ]/g, '');
        if (s === '' || s === '-') return 0;
        if (!/^\d{1,3}(\.\d{3})*(,\d+)?$|^\d+(,\d+)?$/.test(s)) fail('Unbekanntes Zahlenformat.');
        return Number(s.replace(/\./g, '').replace(',', '.'));
      };
      const strip = html => String(html || '').replace(/<[^>]*>/g, ' ').replace(/&nbsp;/g, ' ').replace(/\s+/g, ' ').trim();
      // the available and the blocked balance each show "Menge" and "Importe"/"Bewertung"
      const panel = (block, cls) => {
        const m = new RegExp(cls + '[\\s\\S]*?<span class="text-muted">[^<]*</span>\\s*(?:<br\\s*/?>)?\\s*([^<]*?)\\s*</div>\\s*<div class="col-6">\\s*<span class="text-muted">[^<]*</span>\\s*(?:<br\\s*/?>)?\\s*([^<]*?)\\s*</div>').exec(block);
        if (!m) fail('Unbekanntes Seitenformat.');
        const value = /^([\d.,\s-]*?)\s*([A-Z]{3})?$/.exec(m[2].trim());
        if (!value) fail('Unbekanntes Seitenformat.');
        return {quantity: amount(m[1]), value: amount(value[1]), currency: value[2] || ''};
      };
      const parse = html => {
        const items = [];
        const marker = /id="myChart-assets-([A-Z]{2}[A-Z0-9]{9}\d)"/g;
        let m;
        while ((m = marker.exec(html))) {
          const isin = m[1];
          const head = html.lastIndexOf('<h5', m.index);
          const end = html.indexOf('id="myChart-detail-' + isin + '"', m.index);
          if (head < 0 || end < 0) fail('Unbekanntes Seitenformat.');
          const block = html.slice(head, end);
          const name = strip((/<h5[^>]*>([\s\S]*?)<\/h5>/.exec(block) || [])[1]).replace(/^ACTION\s+/i, '');
          const price = /class="stockprice"[\s\S]*?<em>\s*([\d.,]+)\s*\(([A-Z]{3})\)\s*<\/em>/.exec(block);
          const when = /class="stockprice"[\s\S]*?(\d{2})\.(\d{2})\.(\d{2,4})/.exec(block);
          if (!price || !when) fail('Kurs nicht gefunden.');
          const year = when[3].length === 2 ? '20' + when[3] : when[3];
          const free = panel(block, 'panelChartBorderBlue');
          const blocked = panel(block, 'panelChartBorderLightBlue');
          const currency = free.currency || blocked.currency || price[2];
          if ((free.currency && blocked.currency && free.currency !== blocked.currency) || currency !== price[2])
            fail('Uneinheitliche Währung.');
          items.push({isin, name: name || isin, quantity: free.quantity + blocked.quantity, price: amount(price[1]),
            priceDate: year + '-' + when[2] + '-' + when[1], currency, value: free.value + blocked.value});
        }
        // every security block must have been read; otherwise a layout change would look like an empty depot
        const blocks = (html.match(/<canvas[^>]*myChart-assets-/g) || []).length;
        if (blocks !== items.length) fail('Unbekanntes Seitenformat.');
        return items;
      };
      // Share rights: the start page shows only their number ("Menge"); none listed = no section
      const parseRights = html => {
        const at = html.indexOf('id="myChart-droitsActions"');
        if (at < 0) return null;
        // look only inside the rights panel: up to the next panel or chart, never into a later one
        const rest = html.slice(at);
        const from = rest.indexOf('panelChartBorderBordeaux');
        const next = from < 0 ? -1 : rest.slice(from + 24).search(/panelChartBorder|<canvas/);
        const panel = from < 0 ? '' : next < 0 ? rest.slice(from) : rest.slice(from, from + 24 + next);
        const m = /panelChartBorderBordeaux[\s\S]*?<span\s+class="text-muted">[^<]*<\/span>\s*(?:<br\s*\/?>)?\s*([^<]*?)\s*<\/div>/.exec(panel);
        // an empty number must not read as 0 and remove an existing position
        if (!m || !m[1].trim()) fail('Aktienrechte nicht erkannt.');
        return {quantity: amount(m[1])};
      };
      const MARKERS = ['myChart-assets-', 'myChart-droitsActions'];
      const depotOf = html => { const rights = parseRights(html); return rights ? {items: parse(html), rights} : {items: parse(html)}; };
      const hasOverview = html => MARKERS.some(m => html.includes(m));
      // The overview the user sees: this document and same-origin frames (the portal may embed it).
      const shown = () => {
        const out = [];
        const add = doc => { try { if (doc?.documentElement) out.push(doc.documentElement.outerHTML); } catch {} };
        add(document);
        try { for (const f of document.querySelectorAll('iframe')) add(f.contentDocument); } catch {}
        return out.filter(hasOverview);
      };
      const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
      const DOCS = '/SharinBox/documentationAcces.do';
      const DOWNLOAD = /^\/SharinBox\/sgdocs\/download\.do\?idSgdocs=([0-9a-f-]{36})$/i;
      // Same-origin GET with the session; the deadline holds until the body is read.
      const request = async (path, accept, read) => {
        if (location.origin !== pageOrigin) fail('Bitte erneut anmelden.');
        if (path !== HOME && path !== DOCS && !(DOWNLOAD.test(path) && UUID.test(DOWNLOAD.exec(path)[1])))
          fail('Ungültiger Abruf.');
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 30000);
        try {
          const r = await fetch(path, {method: 'GET', credentials: 'same-origin', mode: 'same-origin',
            headers: {'Accept': accept}, signal: controller.signal});
          if (!r.ok) throw {status: r.status};
          return await read(r);
        } finally { clearTimeout(timeout); }
      };
      const page = async path => {
        try { return {html: await request(path, 'text/html', r => r.text())}; }
        catch (e) { return {status: e && e.status ? e.status : 0}; }
      };
      const base64 = bytes => {
        let binary = '';
        for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
        return btoa(binary);
      };
      // one entry per document: title, date and ID (titles only; nothing else of the page)
      const parseDocuments = html => {
        const docs = new Map();
        const problems = new Map();
        const problem = reason => problems.set(reason, (problems.get(reason) || 0) + 1);
        const entry = /<li>\s*<div class="doc"[\s\S]*?<\/li>/g;
        let m;
        let found = 0;
        while ((m = entry.exec(html))) {
          found++;
          const id = /download\.do\?idSgdocs=([0-9a-fA-F-]{36})/.exec(m[0]);
          const date = /<div class="doc-date">\s*<em>\s*(\d{2})\.(\d{2})\.(\d{4})\s*<\/em>/.exec(m[0]);
          const label = /doc-label">([\s\S]*?)<\/div>/.exec(m[0]);
          if (!id || !UUID.test(id[1])) { problem('ohne gültige ID'); continue; }
          if (!date) { problem('ohne Datum'); continue; }
          const key = id[1].toLowerCase();
          if (!docs.has(key)) docs.set(key, {id: key, sentDate: date[3] + '-' + date[2] + '-' + date[1],
            subject: strip(label && label[1]).slice(0, 200), path: '/SharinBox/sgdocs/download.do?idSgdocs=' + key});
        }
        // an entry the pattern above skips (e.g. one with an extra attribute) is an error, not a gap
        const listed = (html.match(/<div\b[^>]*\bclass=["']doc["']/g) || []).length;
        if (listed !== found) problem('nicht erkannt');
        if (problems.size > 0)
          fail(('Unbekanntes Dokumentformat: ' + [...problems].map(([r, n]) => n + '× ' + r).join(', ')).slice(0, 150));
        return [...docs.values()];
      };
      window.__gcSharinbox = {
        // signed in: the overview is on screen
        ready: () => location.origin === pageOrigin && shown().length > 0,
        collect: async () => {
          if (location.origin !== pageOrigin) fail('Bitte erneut anmelden.');
          // a fresh copy of the start page; otherwise what is on screen
          const got = await page(HOME);
          const html = got.html && hasOverview(got.html) ? got.html : shown()[0];
          if (!html) fail(got.status ? 'Abruf fehlgeschlagen (HTTP ' + got.status + '). Bitte erneut anmelden.' : 'Keine Bestandsübersicht erkannt. Bitte erneut anmelden.');
          return JSON.stringify({version: 1, asOf: new Date().toISOString().slice(0, 10), depot: depotOf(html)});
        },
        documents: async () => {
          const got = await page(DOCS);
          if (!got.html) fail(got.status ? 'Dokumentenliste nicht abrufbar (HTTP ' + got.status + ').' : 'Dokumentenliste nicht abrufbar.');
          if (!got.html.includes('documents-documentation')) fail('Keine Dokumentenliste erkannt. Bitte erneut anmelden.');
          return JSON.stringify(parseDocuments(got.html));
        },
        pdf: async path => {
          if (typeof path !== 'string' || !DOWNLOAD.test(path)) fail('Ungültiger Dokumentpfad.');
          let bytes;
          try { bytes = new Uint8Array(await request(path, 'application/pdf, */*', r => r.arrayBuffer())); }
          catch (e) { fail(e && e.status ? 'Download fehlgeschlagen (HTTP ' + e.status + ').' : 'Download fehlgeschlagen.'); }
          if (bytes.length < 5 || String.fromCharCode(...bytes.subarray(0, 5)) !== '%PDF-') fail('Der Download ist kein PDF.');
          return base64(bytes);
        }
      };
    })();
    """#

    static let probe = #"""
    location.origin === 'https://sharinbox.sgmarkets.com' && !!window.__gcSharinbox?.ready()
    """#
}

struct SharinboxWebSheet: View {
    @Environment(\.reportRedaction) private var redaction
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    let connection: BankConnection
    @State private var browser = PortalBrowser(url: URL(string: "https://sharinbox.sgmarkets.com/home")!,
        domains: PortalDomains(kind: .sharinbox), captureTraffic: false, userScript: SharinboxScripts.hook)
    @State private var uiDelegate = AmazonUIDelegate()
    @State private var ready = false
    @State private var running = false
    @State private var includeDocuments = true
    /// "Nicht zugeteilte Aktienrechte abrufen": stored with the connection, also in its settings.
    @State private var rightsOn = true
    /// "Dokumente abrufen" of the connection (bank connection dialog): off = no list, no PDFs.
    private var documentsOn: Bool { !model.documentsOffConnectionIDs.contains(connection.id) }
    @State private var progress = ""
    @State private var notice: String?
    @State private var task: Task<Void, Never>?

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                VStack(alignment: .leading, spacing: 3) {
                    Text("\(redaction.masking(connection.displayName)) – Sharinbox").appFont(.headline)
                    if running { Text(progress).appFont(.caption) }
                    else if let notice { Text(notice).appFont(.caption).foregroundStyle(Theme.warning) }
                    else { Text(ready ? String(localized: "Angemeldet – der Bestand kann abgerufen werden.") : String(localized: "Melden Sie sich bei Sharinbox an (E-Mail, Passwort und Authenticator-App).")).appFont(.caption) }
                }
                Spacer()
                if running || browser.loading { ProgressView().controlSize(.small) }
            }.padding(14)
            Divider()
            PortalWebView(browser: browser).macFrame(minWidth: 760, minHeight: 620)
            Divider()
            HStack {
                Button("Schließen", role: .cancel) { task?.cancel(); close() }.keyboardShortcut(.cancelAction)
                if documentsOn { Toggle("PDF-Postbox abrufen", isOn: $includeDocuments).disabled(running) }
                Toggle("Nicht zugeteilte Aktienrechte abrufen", isOn: Binding(
                    get: { rightsOn },
                    // save first, then show what is stored: a failed save keeps the old value
                    set: {
                        model.setSharinboxRights(connection.id, $0)
                        rightsOn = model.sharinboxRightsEnabled(connection.id)
                    }
                )).disabled(running)
                Spacer()
                Button("Bestand abrufen") { start() }.buttonStyle(.borderedProminent).disabled(!ready || running)
            }.padding(12)
        }
        .webPageDialogs(uiDelegate)
        .task {
            browser.webView.uiDelegate = uiDelegate
            rightsOn = model.sharinboxRightsEnabled(connection.id)
            while !Task.isCancelled {
                let authenticated = (try? await browser.webView.evaluateJavaScript(SharinboxScripts.probe)) as? Bool ?? false
                let ok = browser.onPortal && authenticated
                if ready != ok { ready = ok }
                try? await Task.sleep(nanoseconds: 1_000_000_000)
            }
        }
        .onDisappear { task?.cancel(); browser.teardown() }
        .interactiveDismissDisabled(running)
    }

    private func run(_ script: String, arguments: [String: Any] = [:]) async throws -> String {
        try Task.checkCancellation()
        let guardScript = "if (location.origin !== 'https://sharinbox.sgmarkets.com' || !window.__gcSharinbox) throw new Error('Sharinbox: Bitte erneut anmelden.');\n"
        let result: Any?
        do {
            result = try await browser.webView.callAsyncJavaScript(guardScript + script, arguments: arguments, in: nil, contentWorld: .page)
        } catch {
            try Task.checkCancellation()
            // The page script's own message; WebKit's generic text says nothing. Text that
            // carries a URL is never shown (it can hold tokens).
            let message = ((error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String) ?? ""
            if !message.isEmpty, message.count <= 160, !message.contains("://") {
                throw SharinboxError.portal(message)
            }
            throw SharinboxError.unreadable
        }
        try Task.checkCancellation()
        guard let string = result as? String else { throw SharinboxError.unreadable }
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
                let snapshot = try await run("return await window.__gcSharinbox.collect();")
                let summary = try await pin.run { try $0.importSharinboxSnapshot(connectionId: connectionId, snapshotJson: snapshot) }
                var downloaded = 0
                var failed = 0
                if includeDocuments && documentsOn {
                    progress = String(localized: "Postbox auflisten …")
                    let list = try await run("return await window.__gcSharinbox.documents();")
                    let pending = try await pin.run { try $0.pendingSharinboxDocuments(connectionId: connectionId, documentsJson: list) }
                    var streak = QuirionFailureStreak()
                    for (index, metadata) in pending.enumerated() {
                        try Task.checkCancellation()
                        progress = String(localized: "PDF \(index + 1) von \(pending.count) …")
                        do {
                            guard let data = metadata.data(using: .utf8),
                                  let doc = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                                  let path = doc["path"] as? String else { throw SharinboxError.unreadable }
                            let encoded = try await run("return await window.__gcSharinbox.pdf(path);", arguments: ["path": path])
                            guard let bytes = Data(base64Encoded: encoded) else { throw SharinboxError.unreadable }
                            if try await pin.run({ try $0.importSharinboxDocument(connectionId: connectionId, documentJson: metadata, bytes: bytes) }) { downloaded += 1 }
                            streak.succeeded()
                        } catch is CancellationError { throw CancellationError() }
                        catch {
                            failed += 1
                            if streak.failed() { failed += pending.count - index - 1; break }
                        }
                    }
                }
                let message = String(localized: "Sharinbox: \(summary.accountsUpdated) Depot aktualisiert, \(downloaded) neue PDFs.")
                if model.isCurrent(pin) {
                    model.toast = message
                    if failed == 0 { close() }
                    else { notice = message + " " + String(localized: "\(failed) PDFs nicht abrufbar. Ein erneuter Abruf lädt nur fehlende Dokumente.") }
                }
            } catch is CancellationError {
            } catch { notice = describe(error) }
            await pin.checkpoint()
            running = false
            model.documentsVersion += 1
            model.reloadAll()
            // accounts another login already delivers wait for a choice (#346)
            model.checkPendingAccountSelection()
        }
    }

    private func close() { model.webLoginConnection = nil; dismiss() }
}

private enum SharinboxError: LocalizedError {
    case unreadable
    case portal(String)
    var errorDescription: String? {
        switch self {
        case .unreadable: return String(localized: "Sharinbox: Die Portalantwort konnte nicht gelesen werden.")
        case .portal(let message): return message
        }
    }
}

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// SchwaebischHallUI.swift › SchwaebischHallWebSheet (#290): the user signs in to
// "Mein Konto" of Bausparkasse Schwäbisch Hall (Azure AD B2C at login.schwaebisch-hall.de).
// The portal's own JSON calls, run inside the signed-in page, read the contracts, their
// bookings and the postbox PDFs for the core (`import_schwaebisch_hall_snapshot`,
// `pending_schwaebisch_hall_documents`, `import_schwaebisch_hall_document`).
// The scripts run in the application world (`appWorld`, like WKContentWorld.defaultClient):
// the portal's third-party scripts cannot hook their `fetch`; the page's cookies still apply.
// The scripts are the ones of SchwaebischHallScripts in the Swift file, copied by
// scripts/gen-schwaebisch-hall-qml.py; scripts/test-schwaebisch-hall-portal.cjs checks
// both are identical. Never edit the generated block by hand.
WebLoginFlow {
    id: flow
    title: connection ? qsTr("%1 – Schwäbisch Hall").arg(connection.display_name) : ""
    busy: running
    infoWarning: !running && (notice !== "" || browser.blockedHost !== "")
    info: {
        if (running) return progress
        if (notice !== "") return notice
        if (browser.blockedHost !== "")
            return qsTr("Unsichere Seite blockiert (%1) – nur HTTPS-Seiten werden geöffnet.").arg(browser.blockedHost)
        if (ready) return qsTr("Angemeldet – Bausparverträge, Umsätze und Postfach können abgerufen werden.")
        return qsTr("Melden Sie sich bei „Mein Konto“ von Schwäbisch Hall an.")
    }

    property bool ready: false
    property bool running: false
    property bool probing: false
    property string progress: ""
    property string notice: ""
    property int generation: 0
    /// Contracts another login already delivers: asked before the documents are downloaded (#346).
    property bool askedInFlow: false
    property var choiceDone: null
    readonly property string unreadable: qsTr("Schwäbisch Hall: Die Portalantwort konnte nicht gelesen werden.")
    readonly property var cancelled: ({ schwaebischHall: "cancelled" })

    // BEGIN generated: SchwaebischHallScripts (SchwaebischHallUI.swift)
    readonly property string prelude: `if (location.origin !== 'https://www.schwaebisch-hall.de') throw new Error('Schwäbisch Hall: Bitte erneut anmelden.');
const origin = 'https://www.schwaebisch-hall.de';
const get = async (path, accept = 'application/json') => {
  if (!path.startsWith('/') || path.startsWith('//')) throw new Error('Schwäbisch Hall: Ungültiger Abruf.');
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 30000);
  try {
    const r = await fetch(origin + path, {credentials:'same-origin', redirect:'error',
      headers:{'Accept':accept}, signal:controller.signal});
    if (!r.ok) throw new Error('Schwäbisch Hall: Abruf fehlgeschlagen (HTTP ' + r.status + '). Bitte erneut anmelden.');
    return r;
  } finally { clearTimeout(timer); }
};
const json = async path => {
  const data = await (await get(path)).json();
  if (data && data.status === 'error') throw new Error('Schwäbisch Hall: Sitzung abgelaufen. Bitte erneut anmelden.');
  return data;
};
const contractList = async () => {
  const list = await json('/bin/bshweb/accountcontracts');
  const contracts = list?.data?.contracts;
  if (list?.status !== 'success' || !Array.isArray(contracts)) throw new Error('Schwäbisch Hall: Keine Verträge erkannt. Bitte erneut anmelden.');
  for (const c of contracts)
    if (typeof c?.vtn !== 'string' || !/^\\d{6,24}$/.test(c.vtn)) throw new Error('Schwäbisch Hall: Vertrag ohne gültige Kennung.');
  return contracts;
};
const pick = (o, keys) => Object.fromEntries(keys.filter(k => o?.[k] !== undefined).map(k => [k, o[k]]));`
    readonly property string probe: `try {
  const r = await fetch(origin + '/bin/bshweb/accountcontracts', {credentials:'same-origin', redirect:'error',
    headers:{'Accept':'application/json'}});
  if (!r.ok) return 'false';
  const data = await r.json();
  return String(data?.status === 'success' && Array.isArray(data?.data?.contracts));
} catch { return 'false'; }`
    readonly property string snapshot: `const details = '/content/bshweb/startseite/mein-konto/vertraege/_jcr_content/root/container/par/container/myaccount_accounts.account_details.json?vtn=';
const out = [];
for (const c of await contractList()) {
  let d = await json(details + encodeURIComponent(c.vtn));
  // Some portal answers wrap the payload in \`data\` like the contract list.
  if (d?.data && typeof d.data === 'object' && !Array.isArray(d.data)
      && ('transactions' in d.data || 'contracttype' in d.data)) d = d.data;
  if (!d || typeof d !== 'object' || Array.isArray(d)) throw new Error('Schwäbisch Hall: Unvollständige Vertragsdaten.');
  out.push({...pick(c, ['type','bsn','accountBalance','coreid','contractletter','contractnumber','vtn']),
    details:pick(d, ['contracttype','contractphase','transactions'])});
}
return JSON.stringify({version:1, contracts:out});`
    readonly property string documents: `const contracts = await contractList();
const data = await json('/content/bshweb/startseite/mein-konto/nachrichten.bin.postboxmessage?command=getmessages');
const messages = [data, data?.messages, data?.data, data?.data?.messages].find(Array.isArray);
if (!messages) throw new Error('Schwäbisch Hall: Postfach nicht lesbar.');
const decode = s => String(s).replace(/&amp;/g, '&').replace(/&quot;/g, '"').replace(/&#39;/g, "'");
const attr = (tag, name) => {
  const m = tag.match(new RegExp('\\\\b' + name + '\\\\s*=\\\\s*(?:"([^"]*)"|\\'([^\\']*)\\'|([^\\\\s>]+))', 'i'));
  return m ? decode(m[1] ?? m[2] ?? m[3]) : null;
};
const squash = s => String(s ?? '').replace(/\\s+/g, '');
const out = [], seen = new Set();
for (const m of messages) {
  if (!m || typeof m !== 'object') continue;
  const html = typeof m.text === 'string' ? m.text : '';
  const ids = [];
  for (const f of html.matchAll(/<form\\b([^>]*)>([\\s\\S]*?)<\\/form>/gi)) {
    if (!(attr(f[1], 'action') || '').includes('/bin/bshweb/postbox/attachment')) continue;
    for (const i of f[2].matchAll(/<input\\b[^>]*>/gi))
      if ((attr(i[0], 'name') || '').toLowerCase() === 'id') ids.push(attr(i[0], 'value'));
  }
  for (const a of html.matchAll(/\\/bin\\/bshweb\\/postbox\\/attachment\\?([^"'\\s<>]+)/gi))
    ids.push(new URLSearchParams(decode(a[1])).get('id'));
  if (Array.isArray(m.attachments))
    for (const a of m.attachments) ids.push(typeof a === 'string' ? a : a?.id ?? a?.attachmentId);
  if (m.attachmentId !== undefined) ids.push(m.attachmentId);
  const plain = squash((m.title ?? '') + ' ' + html.replace(/<[^>]*>/g, ' '));
  const named = contracts.filter(c => plain.includes(c.vtn) || (c.bsn && plain.includes(squash(c.bsn))));
  for (const raw of ids) {
    const id = raw === null || raw === undefined ? '' : String(raw).trim();
    if (!id || id.length > 256 || seen.has(id)) continue;
    seen.add(id);
    out.push({id, messageId:String(m.id ?? ''), date:m.date ?? null,
      title:String(m.title || m.typeName || ''), ...(named.length === 1 ? {vtn:named[0].vtn} : {})});
  }
}
return JSON.stringify(out);`
    readonly property string pdf: `if (typeof id !== 'string' || !id) throw new Error('Schwäbisch Hall: Ungültige Dokument-ID.');
const r = await get('/bin/bshweb/postbox/attachment?id=' + encodeURIComponent(id), 'application/pdf,*/*');
const length = Number(r.headers.get('content-length') || 0);
if (length > 25 * 1024 * 1024) throw new Error('Schwäbisch Hall: Dokument zu groß.');
const buf = new Uint8Array(await r.arrayBuffer());
if (buf.length > 25 * 1024 * 1024 || String.fromCharCode(...buf.subarray(0, 5)) !== '%PDF-')
  throw new Error('Schwäbisch Hall: Der Download ist kein PDF.');
let bin = '';
for (let i = 0; i < buf.length; i += 0x8000) bin += String.fromCharCode.apply(null, buf.subarray(i, i + 0x8000));
return btoa(bin);`
    // END generated

    function onSite() {
        try { return new URL(browser.currentURL).hostname === "www.schwaebisch-hall.de" } catch (e) { return false }
    }

    /// Signed in: the contract list answers (`probe`). Rarely once signed in; the next
    /// fetch reports an expired session anyway.
    Timer {
        interval: flow.ready ? 10000 : 2000
        repeat: true
        running: true
        onTriggered: {
            if (flow.running || flow.probing) return
            if (!flow.onSite()) { if (flow.ready) flow.ready = false; return }
            flow.probing = true
            flow.browser.runAsync(flow.prelude + "\n" + flow.probe, {}, flow.browser.appWorld).then(r => {
                flow.probing = false
                if (flow.ready !== (r === "true")) flow.ready = r === "true"
            }, () => {
                flow.probing = false
                if (flow.ready) flow.ready = false
            })
        }
    }

    function close(success) {
        generation++
        endAccountChoice()
        if (success) AppModel.closeWebLogin()
        else AppModel.cancelWebLogin()
    }
    function cancel() { if (!running) close(false) }

    function run(script, args, gen) {
        if (gen !== generation) return Promise.reject(cancelled)
        return browser.runAsync(prelude + "\n" + script, args || {}, browser.appWorld).then(r => {
            if (gen !== generation) throw cancelled
            if (typeof r !== "string") throw new Error(unreadable)
            return r
        }, e => { throw gen !== generation ? cancelled : new Error(e === null ? qsTr("Schwäbisch Hall: Bitte erneut anmelden.") : e) })
    }

    function start() {
        running = true
        notice = ""
        const gen = ++generation
        const id = connection.id
        let summary = null, downloaded = 0, failed = 0
        progress = qsTr("Bausparverträge und Umsätze abrufen …")
        run(snapshot, {}, gen)
            .then(json => AppModel.corePromise("import_schwaebisch_hall_snapshot", { connection_id: id, snapshot_json: json }))
            .then(s => {
                summary = s
                // Held-back contracts: ask now, so their documents are fetched (or skipped) accordingly.
                return AppModel.corePromise("pending_account_selection_connections", {})
            })
            .then(open => {
                if (gen !== generation) throw cancelled
                if (open.indexOf(id) < 0) return null
                askedInFlow = true
                progress = qsTr("Auswahl der Konten …")
                choosingAccounts = true
                return new Promise(resolve => { choiceDone = resolve })
            })
            .then(() => {
                if (gen !== generation) throw cancelled
                if (!includeDocuments.checked) return null
                progress = qsTr("Postfach auflisten …")
                return run(documents, {}, gen)
                    .then(list => AppModel.corePromise("pending_schwaebisch_hall_documents", { connection_id: id, documents_json: list }))
                    .then(pending => {
                        // QuirionFailureStreak: abort after 3 failures in a row (expired session, portal change).
                        let streak = 0
                        const next = i => {
                            if (gen !== generation) return Promise.reject(cancelled)
                            if (i >= pending.length) return null
                            progress = qsTr("PDF %1 von %2 …").arg(i + 1).arg(pending.length)
                            const metadata = pending[i]
                            let docId = ""
                            try { docId = JSON.parse(metadata).id } catch (e) {}
                            const step = typeof docId === "string" && docId
                                ? run(pdf, { id: docId }, gen)
                                    .then(b64 => AppModel.corePromise("import_schwaebisch_hall_document", { connection_id: id, document_json: metadata, bytes_base64: b64 }))
                                : Promise.reject(new Error(unreadable))
                            return step.then(stored => {
                                if (stored) downloaded++
                                streak = 0
                                return next(i + 1)
                            }, e => {
                                if (e === cancelled) throw e
                                failed++
                                if (++streak >= 3) { failed += pending.length - i - 1; return null }
                                return next(i + 1)
                            })
                        }
                        return next(0)
                    })
            })
            .then(() => {
                const message = qsTr("Schwäbisch Hall: %1 Verträge aktualisiert, %2 neue Buchungen, %3 neue PDFs.")
                    .arg(summary.accounts_updated).arg(summary.transactions_imported).arg(downloaded)
                AppModel.toast = message
                if (failed === 0) { finish(); close(true); return }
                notice = message + " " + qsTr("%1 PDFs nicht abrufbar. Ein erneuter Abruf lädt nur fehlende Dokumente.").arg(failed)
                finish()
            }, e => {
                if (e !== cancelled) {
                    notice = e && e.message ? e.message : String(e)
                    ready = false
                }
                finish()
            })
    }
    // PDFs are committed individually; flush them on success, cancellation and partial failure.
    function finish() {
        try { AppModel.core("checkpoint") } catch (e) {}
        running = false
        endAccountChoice()
        AppModel.documentsVersion++
        AppModel.reloadAll()
        // Skipped ("Später") contracts were already offered in this run.
        if (!askedInFlow) AppModel.checkPendingAccountSelection()
        askedInFlow = false
    }
    /// "Später" / "Übernehmen und fortfahren" in the panel over the browser (or cancel).
    function endAccountChoice() {
        choosingAccounts = false
        const done = choiceDone
        choiceDone = null
        if (done) done()
    }

    Button { text: qsTr("Schließen"); onClicked: flow.close(false) }
    CheckBox { id: includeDocuments; text: qsTr("PDF-Dokumente aus dem Postfach"); checked: true; enabled: !flow.running }
    Item { Layout.fillWidth: true }
    Button {
        text: qsTr("Verträge abrufen")
        highlighted: true
        enabled: flow.ready && !flow.running
        onClicked: flow.start()
    }
}

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// AdvanziaWorkflow.swift + AdvanziaBrowserSession.swift + AdvanziaUI.swift (#935): cards with
// access via mein.advanzia.com. The user signs in in the browser window (id.advanzia.com) and
// confirms there with the SMS code or in the Advanzia app; the header shows which step the
// login page is at (AdvanziaAuthenticationScripts.inspect: structural states only, never
// page text). Once the portal names exactly one card, its transactions and the bank's balance
// are read with the portal's own API inside the page (AdvanziaScripts.hook) and handed to the
// core (`import_advanzia_capture`).
//
// Deviation from the Mac (on purpose): the Mac runs this login in a hidden browser, types a
// password kept in the keychain, picks the release method and submits the SMS code typed into
// its own dialog. Linux fills nothing into the bank's pages and stores no password: the
// window shows the bank's page, and the user enters everything there.
//
// The scripts are the ones of the Swift files, copied by scripts/gen-advanzia-qml.py;
// scripts/test-advanzia-linux.cjs checks both are identical. Never edit the generated block by hand.
WebLoginFlow {
    id: flow
    title: qsTr("Advanzia – Anmeldung und Abruf")
    symbol: "creditcard.fill"
    startURL: "https://mein.advanzia.com/"
    // PortalDomains(["mein.advanzia.com"], includeBaseDomains: false)
    domains: ["mein.advanzia.com"]
    userScript: hook
    readyToLoad: profile !== null
    busy: phase === "importing" || phase === "statements"
    infoWarning: error !== "" || browser.blockedHost !== "" || authentication === "rejected"
    info: {
        if (error !== "") return error
        if (browser.blockedHost !== "")
            return qsTr("Unsichere Seite blockiert (%1) – nur HTTPS-Seiten werden geöffnet.").arg(browser.blockedHost)
        if (phase === "finished") return qsTr("Advanzia-Abruf abgeschlossen.")
        if (phase === "statements") return qsTr("Kontoauszüge werden geladen …")
        if (phase === "importing") return qsTr("Advanzia-Umsätze und Bankkontostand werden übernommen …")
        if (needsAccountChoice && ready) return qsTr("Bitte einmalig das Zielkonto auswählen. Danach beginnt die Übernahme automatisch.")
        if (onLogin) {
            if (authentication === "sms") return qsTr("Bitte den SMS-Code eingeben. Anschließend läuft der Abruf automatisch weiter.")
            if (authentication === "app") return qsTr("Bitte die Anmeldung in der Advanzia-App bestätigen. Danach läuft der Abruf automatisch weiter.")
            if (authentication === "rejected") return qsTr("Advanzia hat die Anmeldung abgelehnt. Bitte das Passwort prüfen und erneut bestätigen.")
            return qsTr("Melden Sie sich im Fenster bei Advanzia an und bestätigen Sie die Freigabe per SMS-Code oder in der Advanzia-App.")
        }
        if (notice !== "") return notice
        return qsTr("Advanzia meldet sich an …")
    }

    // MARK: workflow state (AdvanziaWorkflow)
    /// preparing | signingIn | importing | failed | finished
    property string phase: "preparing"
    property var statementRequest: null
    property var profile: null              // AdvanziaProfile
    property string error: ""
    /// Hint that does not stop the flow (card not discovered yet).
    property string notice: ""
    /// -1 = not chosen, 0 = new credit card account, else an account id
    property int target: -1
    /// AdvanziaAutomaticImport: exactly one automatic attempt; a failure needs "Erneut versuchen".
    property string automatic: "waiting"    // waiting | running | finished | failed
    property bool ticking: false
    property int generation: 0

    // MARK: session state (AdvanziaBrowserSession)
    property string accountID: ""
    property bool ready: false
    property bool accountConflict: false
    property bool openedTransactions: false
    property int stablePortalTicks: 0
    /// AdvanziaAuthentication: loading | credentials | method | sms | app | rejected | unknown
    property string authentication: "loading"

    // AdvanziaSecurity.isPortalURL / isLoginURL: https, exact host, default port, no user info
    function allowed(url, host) {
        const u = browser.urlParts(url)
        return !!u && u.protocol === "https:" && u.hostname === host && (u.port === "" || u.port === "443")
            && u.username === "" && u.password === ""
    }
    readonly property bool onPortalPage: allowed(browser.currentURL, "mein.advanzia.com")
    readonly property bool onLogin: allowed(browser.currentURL, "id.advanzia.com")
    /// AdvanziaSecurity.account: a UUID, lower-cased
    function account(value) {
        return typeof value === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value) ? value.toLowerCase() : ""
    }
    /// AdvanziaSecurity.errorMessage
    function errorMessage(code) {
        switch (code) {
        case "AUTH": return qsTr("Advanzia-Sitzung abgelaufen. Bitte erneut anmelden und die gewählte Freigabe bestätigen.")
        case "TIMEOUT": return qsTr("Advanzia hat nicht rechtzeitig geantwortet. Bitte den Abruf erneut starten.")
        case "ACCOUNT": return qsTr("Advanzia meldet eine andere Karte. Der Import wurde gestoppt.")
        case "DISCOVERY": return qsTr("Die Advanzia-Karte wurde noch nicht erkannt. Öffnen Sie im Bankfenster die Umsatzübersicht; GiroCrab übernimmt die Daten anschließend automatisch.")
        case "HTTP": return qsTr("Die Advanzia-API hat den Abruf abgelehnt. Bitte erneut anmelden oder den Abruf wiederholen.")
        case "SCHEMA": return qsTr("Die Advanzia-Antwort hat ein unerwartetes Format. Es wurden keine unvollständigen Daten importiert.")
        case "SIZE": return qsTr("Die Advanzia-Antwort ist zu groß. Es wurden keine unvollständigen Daten importiert.")
        case "BALANCE": return qsTr("Der Advanzia-Bankkontostand konnte nicht bestätigt werden. Es wurden keine Umsätze importiert.")
        default: return qsTr("Advanzia konnte nicht sicher abgerufen werden. Bitte die Umsatzübersicht öffnen und erneut versuchen.")
        }
    }

    /// AdvanziaWorkflow.candidates: the legacy account of this login, else unconnected
    /// active EUR credit card accounts, by name.
    readonly property var candidates: {
        if (!profile || !connection) return []
        const legacy = profile.legacy_account_id
        return AppModel.allAccounts.filter(a => legacy !== null && legacy !== undefined
                ? a.id === legacy && a.connection_id === connection.id
                : a.kind === "CreditCard" && a.connection_id === null && a.currency === "EUR" && a.archived_unix === null)
            .sort((a, b) => a.name.localeCompare(b.name))
    }
    readonly property bool hasLegacy: !!profile && profile.legacy_account_id !== null && profile.legacy_account_id !== undefined
    readonly property bool needsAccountChoice: !statementRequest && !!profile && !profile.has_account && target < 0

    Component.onCompleted: loadProfile()
    /// AdvanziaWorkflow.run: the login's profile first (random per-login namespace, no bank access).
    function loadProfile() {
        const id = connection.id
        AppModel.corePromise("advanzia_profile", { connection_id: id }).then(p => {
            if (!connection || connection.id !== id) return
            profile = p
            if (candidates.length === 0 && !hasLegacy) target = 0
            phase = "signingIn"
        }, () => fail(qsTr("Advanzia konnte nicht vorbereitet werden. Bitte den Abruf erneut starten.")))
    }

    // BEGIN generated: Advanzia scripts (AdvanziaScripts.swift, AdvanziaAuthentication.swift, AdvanziaBrowserSession.swift)
    readonly property string hook: `(() => {
  if (location.origin !== 'https://mein.advanzia.com' || window.__gcAdvanzia) return;
  const nativeFetch = window.fetch.bind(window);
  const history = new Map();
  const accounts = new Set();
  const controllers = new Set();
  const path = value => {
    try {
      const u = new URL(value, location.href);
      if (u.origin !== 'https://mein.advanzia.com') return null;
      const m = u.pathname.match(/^\\/api\\/accounts\\/([0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12})\\/(transactions(?:\\/\\d{4}\\/\\d{1,2})?|details)$/);
      return m ? {url:u.pathname,id:m[1].toLowerCase(),transactions:m[2].startsWith('transactions')} : null;
    } catch { return null; }
  };
  const project = j => {
    if (!j || !Array.isArray(j.transactions) || j.transactions.length > 1000) throw new Error('SCHEMA');
    const keys = ['amount','currency','transactionDate','merchantName','text','status','reference','uniqueReference'];
    return JSON.stringify({transactions:j.transactions.map(t => Object.fromEntries(keys.filter(k => t?.[k] !== undefined).map(k => [k,t[k]])))});
  };
  const remember = (url, text) => {
    const p = path(url);
    if (!p || typeof text !== 'string' || text.length > 3000000) return;
    accounts.add(p.id);
    if (!p.transactions) return;
    try {
      history.set(p.url, project(JSON.parse(text)));
      while (history.size > 36) history.delete(history.keys().next().value);
    } catch { /* Parser and explicit fresh fetch report invalid responses. */ }
  };
  window.fetch = function(input) {
    const url = input instanceof Request ? input.url : String(input);
    const p = path(url);
    if (p) accounts.add(p.id);
    const response = nativeFetch(...arguments);
    if (p?.transactions) response.then(r => { if (r.ok) r.clone().text().then(t => remember(url,t)).catch(() => {}); }).catch(() => {});
    return response;
  };
  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function(method,url) {
    const p = path(String(url));
    if (p) {
      accounts.add(p.id);
      this.addEventListener('load', () => {
        if (p.transactions && this.status >= 200 && this.status < 300 && (this.responseType === '' || this.responseType === 'text')) remember(String(url),this.responseText);
      }, {once:true});
    }
    return open.apply(this,arguments);
  };
  const read = async (url, signal) => {
    const r = await nativeFetch(url,{method:'GET',credentials:'same-origin',cache:'no-store',redirect:'error',headers:{Accept:'application/json'},signal});
    if (!r.ok) throw new Error(r.status === 401 || r.status === 403 ? 'AUTH' : 'HTTP');
    if (!(r.headers.get('content-type') || '').toLowerCase().includes('json')) throw new Error('SCHEMA');
    const reader = r.body.getReader();
    const decoder = new TextDecoder();
    let size=0, text='';
    while (true) {
      const {done,value} = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > 3000000) { await reader.cancel(); throw new Error('SIZE'); }
      text += decoder.decode(value,{stream:true});
    }
    return JSON.parse(text + decoder.decode());
  };
  const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
  const sameAccount = id => location.origin === 'https://mein.advanzia.com' &&
    typeof id === 'string' && uuid.test(id) && id === id.toLowerCase() &&
    (accounts.size === 0 || (accounts.size === 1 && accounts.has(id)));
  const statement = async (id, document) => {
    if (!sameAccount(id) || (document !== null && (typeof document !== 'string' || !uuid.test(document))))
      return {error:'ACCOUNT'};
    const c = new AbortController(); controllers.add(c);
    const timeout = setTimeout(() => c.abort(),30000);
    try {
      const listing = document === null;
      const mime = listing ? 'application/json' : 'application/pdf';
      const limit = listing ? 512 * 1024 : 8 * 1024 * 1024;
      const url = '/api/accounts/' + id + '/statements' + (listing ? '' : '/' + document);
      const r = await nativeFetch(url,{method:'GET',credentials:'same-origin',cache:'no-store',redirect:'error',signal:c.signal});
      if (r.status === 401 || r.status === 403) throw new Error('AUTH');
      if (r.status !== 200 || r.redirected) throw new Error('HTTP');
      if ((r.headers.get('content-type') || '').split(';')[0].trim().toLowerCase() !== mime || !r.body)
        throw new Error('SCHEMA');
      if (Number(r.headers.get('content-length')) > limit) throw new Error('SIZE');
      const reader = r.body.getReader();
      let size = 0;
      const chunks = [];
      while (true) {
        const {done,value} = await reader.read();
        if (done) break;
        size += value.byteLength;
        if (size > limit) throw new Error('SIZE');
        chunks.push(value);
      }
      if (!sameAccount(id)) throw new Error('ACCOUNT');
      const bytes = new Uint8Array(size);
      let offset = 0;
      for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
      if (listing) {
        const list = JSON.parse(new TextDecoder('utf-8', {fatal:true}).decode(bytes));
        if (!Array.isArray(list) || list.length > 600) throw new Error('SCHEMA');
        return {listing:JSON.stringify(list.map(item => {
          if (!item || typeof item.statementId !== 'string' || !uuid.test(item.statementId) ||
              typeof item.fileReference !== 'string' || item.fileReference.length > 512 ||
              !Number.isInteger(item.forYear) || !Number.isInteger(item.forMonth))
            throw new Error('SCHEMA');
          return {statementId:item.statementId,fileReference:item.fileReference,forYear:item.forYear,forMonth:item.forMonth};
        }))};
      }
      if (String.fromCharCode(...bytes.subarray(0,5)) !== '%PDF-') throw new Error('SCHEMA');
      const chunks64 = [];
      // Multiples of three allow independent encoding without interior padding.
      for (let i = 0; i < size; i += 24576)
        chunks64.push(btoa(String.fromCharCode(...bytes.subarray(i,i+24576))));
      return {pdf:chunks64.join('')};
    } catch(e) {
      const safe = ['AUTH','HTTP','SCHEMA','SIZE','ACCOUNT'];
      return {error:c.signal.aborted?'TIMEOUT':safe.includes(e.message)?e.message:'NETWORK'};
    } finally { c.abort(); clearTimeout(timeout); controllers.delete(c); }
  };
  window.__gcAdvanzia = {
    discover: () => {
      for (const r of performance.getEntriesByType('resource')) { const p=path(r.name); if(p) accounts.add(p.id); }
      return Array.from(accounts);
    },
    openTransactions: () => {
      const labels=['umsätze','umsatzübersicht','transaktionen'];
      const candidates=Array.from(document.querySelectorAll('a,button,[role="tab"]')).filter(e =>
        labels.includes((e.textContent || '').replace(/\\s+/g,' ').trim().toLowerCase()) &&
        !e.disabled && e.getBoundingClientRect().width>0);
      if(candidates.length!==1) return false;
      const link=candidates[0];
      if(link.href && new URL(link.href,location.href).origin!==location.origin) return false;
      link.click();return true;
    },
    statement,
    cancel: () => { for (const c of controllers) c.abort(); },
    collect: async id => {
      if (!sameAccount(id)) return {error:'ACCOUNT'};
      const c = new AbortController(); controllers.add(c);
      const timeout = setTimeout(() => c.abort(),30000);
      try {
        const base = '/api/accounts/' + id;
        const current = project(await read(base + '/transactions',c.signal));
        const details = await read(base + '/details',c.signal);
        if (typeof details?.accountId !== 'string' || details.accountId.toLowerCase() !== id || details.currency !== 'EUR' ||
            typeof details.balance !== 'number' || !Number.isFinite(details.balance)) throw new Error('BALANCE');
        const bodies = Array.from(history.entries()).filter(([url]) => url.startsWith(base + '/transactions/')).map(([,body])=>body);
        bodies.push(current);
        if (bodies.reduce((n,b)=>n+b.length,0)>12000000) throw new Error('SIZE');
        return {bodies,balance:JSON.stringify(details.balance)};
      } catch(e) {
        const safe = ['AUTH','HTTP','SCHEMA','SIZE','BALANCE','ACCOUNT'];
        return {error:e.name==='AbortError'?'TIMEOUT':safe.includes(e.message)?e.message:'NETWORK'};
      } finally {clearTimeout(timeout);controllers.delete(c);}
    }
  };
})();`
    readonly property string inspect: `if (location.origin !== 'https://id.advanzia.com') return 'unknown';
const visible = e => !e.disabled && e.getBoundingClientRect().width > 0 && e.getBoundingClientRect().height > 0;
const inputs = Array.from(document.querySelectorAll('input')).filter(visible);
const codes = inputs.filter(e => ['text','tel','number','password'].includes(e.type) &&
  (e.autocomplete === 'one-time-code' || /(^|[_-])(otp|sms|tan|code|verification)([_-]|$)/i.test((e.name || '') + '_' + (e.id || ''))));
const rejected = Array.from(document.querySelectorAll('[role="alert"],.alert-danger,.error-message'))
  .some(e => visible(e) && (e.textContent || '').trim().length > 0);
if (rejected) return 'rejected';
if (codes.length === 1) return 'sms';
if (codes.length > 1) return 'unknown';
if (inputs.some(e => e.type === 'password')) return 'credentials';
const labels = Array.from(document.querySelectorAll('button,a,[role="button"],label,span,div,p')).filter(visible)
  .map(e => (e.textContent || '').replace(/\\s+/g,' ').trim().toLowerCase());
if (labels.some(t => ['sms-versand','sms','authentifizierung über app','authentifizierung ueber app','advanzia app','advanzia-app','freigabe per app','app-freigabe'].includes(t))) return 'method';
if (labels.some(t => t.length < 300 && /(?:bitte|bestätigen|freigabe|anmeldung)/.test(t) && /(?:advanzia.?app|in der app|über die app)/.test(t))) return 'app';
return 'unknown';`
    readonly property string discover: `if(location.origin !== 'https://mein.advanzia.com') return []; return window.__gcAdvanzia?.discover() ?? [];`
    readonly property string openTransactions: `return window.__gcAdvanzia?.openTransactions() ?? false;`
    readonly property string collect: `if(location.origin !== 'https://mein.advanzia.com' || !window.__gcAdvanzia) return {error:'AUTH'}; return await window.__gcAdvanzia.collect(account);`
    readonly property string statementScript: `if(location.origin !== 'https://mein.advanzia.com' || !window.__gcAdvanzia) return {error:'AUTH'}; return await window.__gcAdvanzia.statement(account, statementID || null);`
    readonly property string cancelScript: `window.__gcAdvanzia?.cancel()`
    // END generated

    // AdvanziaWorkflow.run: one tick per second
    Timer {
        interval: 1000
        repeat: true
        running: flow.profile !== null && flow.phase !== "failed" && flow.phase !== "finished"
        onTriggered: flow.tick()
    }
    function tick() {
        if (ticking || busy) return
        ticking = true
        const gen = generation
        const done = () => { if (gen === generation) ticking = false }
        // `ready` changes only once a poll knows better: cleared at the start of every poll, it
        // unloaded the cover with the account picker each second and reset the choice.
        if (onLogin) {
            ready = false
            stablePortalTicks = 0
            browser.runAsync(inspect, {}, browser.mainWorld).then(s => {
                if (gen !== generation) return
                authentication = ["loading", "credentials", "method", "sms", "app", "rejected", "unknown"].indexOf(s) >= 0 ? s : "unknown"
                done()
            }, () => { if (gen === generation) authentication = "unknown"; done() })
            return
        }
        if (!browser.loading) authentication = onPortalPage ? "loading" : "unknown"
        if (!onPortalPage || browser.loading || browser.onLoginPage || accountConflict) { ready = false; done(); return }
        // also after a portal redirect / late page start: the hook is idempotent
        browser.evaluate(hook, browser.mainWorld, () => {
            if (gen !== generation) return
            browser.runAsync(discover, {}, browser.mainWorld).then(ids => {
                if (gen !== generation) return
                if (!Array.isArray(ids)) { fail(errorMessage("SCHEMA")); done(); return }
                const found = {}
                for (const i of ids) { const a = account(i); if (a) found[a] = true }
                const list = Object.keys(found)
                const bound = profile.bank_account_id || ""
                if (list.length > 1 || list.some(a => bound !== "" && a !== bound)) {
                    accountID = ""; accountConflict = true; fail(errorMessage("ACCOUNT")); done(); return
                }
                if (list.length === 1) accountID = list[0]
                else if (accountID === "" && bound !== "") accountID = bound
                stablePortalTicks++
                const next = () => {
                    ready = accountID !== "" && stablePortalTicks >= 2
                    if (accountID === "" && stablePortalTicks >= 10) notice = errorMessage("DISCOVERY")
                    if (ready) notice = ""
                    if (ready && !needsAccountChoice && automatic === "waiting") { automatic = "running"; ticking = false
                        if (statementRequest) finishStatements()
                        else importCapture()
                        return }
                    done()
                }
                if (accountID === "" && !openedTransactions) {
                    browser.runAsync(openTransactions, {}, browser.mainWorld).then(r => {
                        if (gen !== generation) return
                        openedTransactions = r === true
                        next()
                    }, () => { if (gen === generation) next() })
                } else next()
            }, () => { if (gen === generation) fail(errorMessage("SCHEMA")); done() })
        })
    }

    /// AdvanziaWorkflow.importCapture
    function importCapture() {
        phase = "importing"; error = ""
        const gen = generation
        const id = connection.id
        const bank = accountID
        browser.runAsync(collect, { account: bank }, browser.mainWorld).then(fields => {
            if (gen !== generation) return
            if (!fields || typeof fields !== "object") { fail(errorMessage("SCHEMA")); return }
            if (typeof fields.error === "string") { fail(errorMessage(fields.error)); return }
            if (!Array.isArray(fields.bodies) || typeof fields.balance !== "string") { fail(errorMessage("SCHEMA")); return }
            const selected = !profile.has_account && target > 0 ? target : null
            // coreAsync, not corePromise: the rejection keeps the error kind (InvalidInput: the core's own text)
            AppModel.coreAsync("import_advanzia_capture", { connection_id: id, bank_account_id: bank, profile_id: profile.profile_id,
                                                            json_bodies: fields.bodies, balance_json_number: fields.balance,
                                                            target_account_id: selected }, s => {
                if (gen !== generation) return
                const note = qsTr("Advanzia: %1 Konten aktualisiert, %2 neue Buchungen.").arg(s.accounts_updated).arg(s.transactions_imported)
                AppModel.saveWebCookies(connection)   // signed in: keep the session
                AppModel.reloadAll()
                finishStatements(note)
            }, err => {
                if (gen !== generation) return
                const e = AppModel.parseCoreError(err)
                fail(e.kind === "InvalidInput" && e.message ? e.message
                     : qsTr("Advanzia-Import abgebrochen. Bitte Konto-Zuordnung und Umsatzdaten prüfen; es wurden keine unvollständigen Daten übernommen."))
            })
        }, () => { if (gen === generation) fail(errorMessage("NETWORK")) })
    }
    function finishStatements(note) {
        phase = "statements"
        const gen = generation
        const vault = AppModel.vaultGeneration
        const current = () => gen === generation && vault === AppModel.vaultGeneration
        const account = accountID
        const args = { connection_id: connection.id, bank_account_id: account, profile_id: profile.profile_id,
                       target_account_id: statementRequest ? statementRequest.account : null }
        let added = 0, failed = 0
        const download = (pending, index) => {
            if (!current() || index >= pending.length) return
            const statement = pending[index]
            return browser.runAsync(statementScript, { account: account, statementID: statement.id }, browser.mainWorld)
                .then(file => {
                    if (!current()) return
                    if (["AUTH", "ACCOUNT", "TIMEOUT"].includes(file?.error)) {
                        failed += pending.length - index
                        return
                    }
                    if (typeof file?.pdf !== "string") { failed++; return download(pending, index + 1) }
                    return AppModel.corePromise("import_advanzia_statement", Object.assign({}, args, { statement: statement, pdf: file.pdf }))
                        .then(stored => {
                            if (current() && stored) { added++; AppModel.documentsVersion++ }
                        }, () => failed++)
                        .then(() => download(pending, index + 1))
                }, () => { failed++; return download(pending, index + 1) })
        }
        return AppModel.corePromise("advanzia_statements_enabled", args).then(enabled => {
            if (!enabled || !current()) return
            return browser.runAsync(statementScript, { account: account, statementID: "" }, browser.mainWorld)
                .then(list => {
                    if (!current()) return
                    if (typeof list?.listing !== "string") throw Error()
                    return AppModel.corePromise("pending_advanzia_statements",
                        Object.assign({}, args, { listing: list.listing, months: statementRequest ? statementRequest.months : 24 }))
                        .then(pending => download(pending, 0))
                })
        }).catch(() => failed++).then(() => {
            if (!current()) return
            const messages = note ? [note] : []
            if (added) messages.push(qsTr("%1 neue Kontoauszüge").arg(added))
            if (failed) messages.push(qsTr("%1 Kontoauszüge konnten nicht geladen werden – beim nächsten Abruf erneut versuchen.").arg(failed))
            if (!messages.length) messages.push(qsTr("Keine neuen Kontoauszüge"))
            AppModel.toast = messages.join(" · ")
            automatic = "finished"; phase = "finished"
            close(true)
        })
    }
    function fail(message) {
        phase = "failed"; error = message; ready = false
        if (automatic === "running") automatic = "failed"
    }
    /// „Erneut versuchen“ (AdvanziaWorkflow.restart): a fresh attempt on the same page.
    function retry() {
        generation++
        ticking = false
        error = ""; notice = ""
        accountConflict = false; accountID = ""; stablePortalTicks = 0; openedTransactions = false; ready = false
        automatic = "waiting"
        phase = profile ? "signingIn" : "preparing"
        if (!profile) loadProfile()
    }

    function close(success) {
        generation++
        browser.evaluate(cancelScript, browser.mainWorld)
        if (success) AppModel.closeWebLogin()
        else AppModel.cancelWebLogin()
    }
    function cancel() { close(false) }

    // AdvanziaPrompt: the target account, asked once over the page
    covering: needsAccountChoice && ready
    cover: Component {
        Rectangle {
            color: Theme.card
            ColumnLayout {
                anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                anchors.margins: 22
                spacing: 16
                RowLayout {
                    spacing: 8
                    SFIcon { symbol: "creditcard.fill"; size: 16; color: Theme.accent }
                    Text { text: qsTr("Advanzia – Anmeldung und Abruf"); font.pixelSize: Theme.body; font.weight: Font.DemiBold; color: Theme.text }
                }
                Text { Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.callout; color: Theme.text
                       text: qsTr("Bitte einmalig das Zielkonto auswählen. Danach beginnt die Übernahme automatisch.") }
                Text { visible: flow.hasLegacy; Layout.fillWidth: true; wrapMode: Text.WordWrap; font.pixelSize: Theme.caption; color: Theme.secondary
                       text: qsTr("Dieses Konto stammt aus einer früheren Version. Wählen Sie es einmalig aus, um es mit seinen vorhandenen Umsätzen weiterzuverwenden.") }
                RowLayout {
                    spacing: 10
                    Text { text: qsTr("Kreditkartenkonto zuordnen"); font.pixelSize: Theme.body; color: Theme.text }
                    AppComboBox {
                        id: picker
                        Layout.preferredWidth: 320
                        readonly property var choices: [{ id: -1, title: qsTr("Bitte auswählen") }]
                            .concat(flow.hasLegacy ? [] : [{ id: 0, title: qsTr("Neues Kreditkartenkonto anlegen") }])
                            .concat(flow.candidates.map(a => ({ id: a.id, title: a.name })))
                        model: choices
                        textRole: "title"
                        currentIndex: 0
                        onActivated: i => { if (choices[i].id >= 0) flow.target = choices[i].id }
                        Sensitive {}
                    }
                }
            }
        }
    }

    Button { text: qsTr("Abbrechen"); onClicked: flow.cancel() }
    Item { Layout.fillWidth: true }
    Button { visible: flow.phase === "failed"; text: qsTr("Erneut versuchen"); highlighted: true; onClicked: flow.retry() }
}

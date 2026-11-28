// The Linux Advanzia flow (GiroCrabLinux/qml/AdvanziaWebLoginFlow.qml) runs exactly the
// navigation and capture scripts of the Swift files, and they work when run as the Linux
// browser sheet runs them (page world, `runAsync` = an async function body). It carries no
// script that fills the bank's forms or submits a code. Synthetic data only; the full script
// tests are scripts/test-advanzia-portal.cjs.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');

const read = p => readFileSync(new URL(p, `file://${__filename}`), 'utf8');
const ui = '../GiroCrabApp/Sources/GiroCrabUI/';
const qml = read('../GiroCrabLinux/qml/AdvanziaWebLoginFlow.qml');
const linux = Object.fromEntries([...qml.matchAll(/readonly property string (\w+): `((?:\\[\s\S]|[^`\\])*)`/g)]
  .map(m => [m[1], vm.runInNewContext('`' + m[2] + '`')]));
const dedent = (s, n) => s.split('\n').map(l => l.startsWith(' '.repeat(n)) ? l.slice(n) : l.trimStart()).join('\n');
const raw = (file, name) => dedent(read(ui + file).match(new RegExp('static let ' + name + ' = #"""\\n([\\s\\S]*?)\\n    """#'))[1], 4);

test('the Linux flow runs exactly the scripts of the Swift files', () => {
  const session = read(ui + 'AdvanziaBrowserSession.swift');
  const expected = {
    hook: raw('AdvanziaScripts.swift', 'hook'),
    inspect: raw('AdvanziaAuthentication.swift', 'inspect'),
    discover: session.match(/let script = "([^"\n]*discover\(\)[^"\n]*)"/)[1],
    openTransactions: session.match(/callAsyncJavaScript\("([^"\n]*openTransactions\(\)[^"\n]*)"/)[1],
    collect: session.match(/let script = "([^"\n]*\.collect\(account\);)"/)[1],
    statementScript: session.match(/static let statementScript = "([^"\n]*)"/)[1],
    cancelScript: session.match(/evaluateJavaScript\("([^"\n]*cancel\(\))"/)[1],
  };
  assert.deepEqual(Object.keys(linux).sort(), Object.keys(expected).sort());
  for (const name of Object.keys(expected))
    assert.equal(linux[name], expected[name], `${name} differs: run python3 -I scripts/gen-advanzia-qml.py`);
});

test('nothing is typed into or submitted on the bank pages', () => {
  for (const name of ['credentials', 'method']) assert(!Object.values(linux).includes(raw('AdvanziaLoginAutomation.swift', name)));
  assert(!Object.values(linux).includes(raw('AdvanziaAuthentication.swift', 'submit')));
  for (const s of Object.values(linux)) assert(!/HTMLInputElement\.prototype/.test(s), 'no script sets input values');
});

const id = '00000000-0000-4000-8000-000000000001';
const base = '/api/accounts/' + id;
function portal(origin = 'https://mein.advanzia.com') {
  const calls = [];
  const context = vm.createContext({location: {origin, href: origin + '/'}, URL, Request, Response, TextDecoder, AbortController,
    setTimeout, clearTimeout, XMLHttpRequest: class { open() {} },
    performance: {getEntriesByType: () => [{name: 'https://mein.advanzia.com' + base + '/transactions'}]},
    document: {querySelectorAll: () => []},
    fetch: async url => {
      calls.push(url);
      const data = String(url).endsWith('/details') ? {balance: -12.34, accountId: id, currency: 'EUR', privateField: 'do not export'} : {transactions: []};
      return new Response(JSON.stringify(data), {headers: {'content-type': 'application/json'}});
    }});
  vm.runInContext('window = globalThis', context);
  // WebLoginSheet.addScript: user script at document creation in the main (page) world
  vm.runInContext(linux.hook, context);
  // WebLoginSheet.runAsync: the body runs as an async function with the arguments as parameters
  const runAsync = (body, args = {}) => vm.runInContext(
    `(async function(${Object.keys(args).join(',')}){\n${body}\n})(${Object.values(args).map(v => JSON.stringify(v)).join(',')})`, context);
  return {runAsync, calls};
}

test('discovery and capture as the Linux flow calls them', async () => {
  const {runAsync, calls} = portal();
  assert.deepEqual(Array.from(await runAsync(linux.discover)), [id]);
  assert.equal(await runAsync(linux.openTransactions), false, 'no unique "Umsätze" link on the fake page');
  const fields = await runAsync(linux.collect, {account: id});
  assert.equal(fields.balance, '-12.34');
  assert.equal(fields.bodies.length, 1);
  assert(!JSON.stringify(fields).includes('do not export'));
  assert.deepEqual(calls, [base + '/transactions', base + '/details']);
  assert.equal(vm.runInContext(linux.cancelScript, vm.createContext({window: {}})), undefined);
});

test('the hook stays out of other origins, and inspect reports only a state', async () => {
  for (const origin of ['https://id.advanzia.com', 'https://mein.advanzia.com.evil.invalid']) {
    const {runAsync, calls} = portal(origin);
    assert.deepEqual(Array.from(await runAsync(linux.discover)), []);
    assert.equal((await runAsync(linux.collect, {account: id})).error, 'AUTH');
    assert.equal(calls.length, 0);
  }
  const field = {type: 'text', autocomplete: 'one-time-code', name: 'otp', id: '', disabled: false, getBoundingClientRect: () => ({width: 100, height: 25})};
  const inspect = origin => vm.runInNewContext(`(function(){${linux.inspect}})()`,
    {location: {origin}, document: {querySelectorAll: s => s === 'input' ? [field] : []}});
  assert.equal(inspect('https://id.advanzia.com'), 'sms');
  assert.equal(inspect('https://evil.invalid'), 'unknown');
});

test('PDF failures do not replace a successful capture or prevent saving the next PDF', async () => {
  const recorded = [];
  const ctx = {
    generation: 1, phase: 'importing', accountID: id,
    connection: {id: 42}, profile: {profile_id: 'fictional'},
    statementRequest: null, statementScript: linux.statementScript,
    qsTr: s => ({arg: n => s.replace('%1', n), toString: () => s}),
    close: success => recorded.push(['close', success]),
    browser: {
      async runAsync(script, args) {
        recorded.push(['browser', args.account, args.statementID]);
        return args.statementID === 'bad' ? {error:'SCHEMA'}
          : args.statementID === '' ? {listing:'fictional list'} : {pdf:'fictional PDF'};
      }
    },
    AppModel: {
      documentsVersion: 0,
      logSync: (...args) => recorded.push(['log', ...args]),
      async corePromise(method, args) {
        assert.equal(args.connection_id, 42);
        assert.equal(args.bank_account_id, id);
        recorded.push([method, args.months ?? args.statement?.id]);
        if (method === 'pending_advanzia_statements') return [{id:'bad'}, {id:'good'}];
        return true;
      }
    }
  };
  const finish = qml.slice(qml.indexOf('    function finishStatements('), qml.indexOf('    function fail(message)'));
  await vm.runInNewContext(finish + '\nfinishStatements("Umsätze übernommen")', ctx);
  assert.equal(ctx.AppModel.documentsVersion, 1);
  assert.match(ctx.AppModel.toast, /^Umsätze übernommen .*1 neue Kontoauszüge .*1 Kontoauszüge konnten nicht geladen werden/);
  assert.deepEqual(recorded.filter(r => r[0] !== 'browser'), [
    ['advanzia_statements_enabled', undefined], ['pending_advanzia_statements', 24],
    ['import_advanzia_statement', 'good'],
    ['log', 42, 'Partial', '1 neue Kontoauszüge · 1 Kontoauszüge konnten nicht geladen werden – beim nächsten Abruf erneut versuchen.', 0, 'Statements'],
    ['close', true],
  ]);
});

test('manual Advanzia statements check the refreshed banking policy and vault before opening the portal', () => {
  const app = read('../GiroCrabLinux/qml/AppModel.qml');
  const fetch = app.slice(app.indexOf('    function fetchStatements('), app.indexOf('    function autoStatements('));
  const account = {id: 42, connection_id: 7};
  const connection = {id: 7, connector: 'Advanzia'};
  for (const scenario of ['blocked', 'vault replaced', 'allowed']) {
    let afterRefresh;
    const gate = {};
    const ctx = {
      account, connections: [connection], accountsByID: {42: account},
      vaultGeneration: 1, isUnlocked: true, syncingConnectionID: null, webLoginConnection: null,
      usesPortalLogin: () => true,
      withFreshBankingPolicy: callback => { afterRefresh = callback; },
      bankingGate: () => scenario === 'blocked' ? gate : null,
      showBankingGate: value => { ctx.shownGate = value; },
    };
    vm.runInNewContext(fetch + '\nfetchStatements(account, 12)', ctx);
    assert.equal(ctx.webLoginConnection, null);
    if (scenario === 'vault replaced') ctx.vaultGeneration++;
    afterRefresh();
    assert.equal(ctx.webLoginConnection, scenario === 'allowed' ? connection : null);
    assert.equal(ctx.shownGate, scenario === 'blocked' ? gate : undefined);
    assert.equal(ctx.advanziaStatementRequest?.months, scenario === 'allowed' ? 12 : undefined);
  }
});

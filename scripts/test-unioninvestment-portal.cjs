// Run the actual scripts embedded in UnionInvestmentUI.swift (synthetic data only), as for quirion.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const source = readFileSync(new URL('../GiroCrabApp/Sources/GiroCrabUI/UnionInvestmentUI.swift', `file://${__filename}`), 'utf8');
const hook = source.match(/static let hook = #"""\n([\s\S]*?)\n    """#/)[1];
const api = 'https://service.meindepot.union-investment.de';
const pageOrigin = 'https://anlegen.meindepot.union-investment.de';
const response = (data, status=200) => ({ok: status >= 200 && status < 300, status, json: async () => data,
  arrayBuffer: async () => Buffer.from(data).buffer.slice(0)});
function page(fetch, origin=pageOrigin) {
  class XHR { open() {} setRequestHeader() {} addEventListener() {} }
  const window = {fetch};
  const context = vm.createContext({window, location:{origin, href:origin+'/'}, Request, Headers, URL,
    XMLHttpRequest:XHR, AbortController, setTimeout, clearTimeout, Date, Uint8Array, btoa, String});
  vm.runInContext(hook, context);
  return window;
}
async function authenticate(window) {
  await window.fetch(api+'/customer-functional-service/customer', {headers:{Authorization:'Bearer synthetic-test-token'}});
  await Promise.resolve();
}
const goal = (isin, name, shares, price, value, gain) => ({
  name:{vergeben:name, gewaehlt:'DO NOT EXPORT'}, betrag:{aktuellerWert:value},
  gewinnUndVerlust:{realisierbarerGewinnUndVerlust:gain}, bild:{dateiname:'DO NOT EXPORT'},
  produkt:{einzelfonds:{isin, stueckanzahl:shares, ruecknahmePreis:{value:price, date:'2026-11-19', currency:'EUR'}}}});
function server(calls, masters=1) {
  return async (url, options={}) => {
    calls.push({url, options});
    const u = new URL(url), path = u.pathname;
    if (path.endsWith('/depot/list/master-deposits')) return response({masterDeposits:Array.from({length:masters}, () => ({
      daskaStammdepotNr:'123456789', depotOwnerDisplayName:'Test Owner', depotTypeDisplayName:'UnionDepot',
      depotValue:{currency:'EUR', value:250}, links:{baseUrl:'DO NOT EXPORT'}}))});
    if (path.endsWith('/depot/anlageziel')) return response([
      goal('DE0000000001', 'Test Fonds A', 2.5, 40, 100, 10), goal('DE0000000002', 'Test Fonds B', 10, 15, 150, -3)]);
    if (path === '/postbox/depots') return response([{daskaStammdepotNr:'123456789', depotIndex:0, anzeigeName:'DO NOT EXPORT'}]);
    if (path === '/postbox') {
      const offset = Number(u.searchParams.get('offset'));
      const message = n => ({id:String(1000000000 + n), depotIndex:0, attachmentId:'1', downloadable:true, customerUpload:false, seen:true,
        sentDate:'2026-10-01T08:30:00', subject:'Test statement ' + n, attachmentHref:'/postbox/depot/0/message/' + (1000000000 + n) + '/attachment/1', href:'DO NOT EXPORT'});
      const all = Array.from({length:45}, (_, n) => message(n));
      all[3].customerUpload = true; all[4].downloadable = false;
      return response({folder:'INBOX', offset, total:all.length, status:'OK', messages:all.slice(offset, offset + 20)});
    }
    if (/^\/postbox\/depot\/0\/message\/\d+\/attachment\/1$/.test(path)) return response(path.endsWith('9/attachment/1') ? '<html>login</html>' : '%PDF-1.7 synthetic');
    if (path.endsWith('/customer')) return response({email:'DO NOT EXPORT'});
    throw new Error('Unexpected request ' + url);
  };
}

test('collects depot and funds; tokens and unrelated fields stay inside the page', async () => {
  const calls = [], w = page(server(calls));
  assert.equal(w.__gcUnion.ready(), false);
  await authenticate(w);
  assert.equal(w.__gcUnion.ready(), true);
  calls.length = 0;
  const snapshot = await w.__gcUnion.collect(), data = JSON.parse(snapshot);
  assert.equal(data.version, 1);
  assert.match(data.asOf, /^\d{4}-\d{2}-\d{2}$/);
  assert.equal(data.depots.length, 1);
  assert.equal(data.depots[0].number, '123456789');
  assert.equal(data.depots[0].value, 250);
  assert.deepEqual(data.depots[0].items.map(i => [i.isin, i.name, i.quantity, i.price, i.value, i.unrealized, i.currency]),
    [['DE0000000001', 'Test Fonds A', 2.5, 40, 100, 10, 'EUR'], ['DE0000000002', 'Test Fonds B', 10, 15, 150, -3, 'EUR']]);
  assert(!snapshot.includes('DO NOT EXPORT'));
  assert(!snapshot.includes('synthetic-test-token'));
  for (const c of calls) {
    assert.equal(c.options.method, 'GET');
    assert.equal(c.options.credentials, 'omit');
    assert.equal(c.options.redirect, 'error');
    assert.equal(c.options.headers.Authorization, 'Bearer synthetic-test-token');
  }
  // the goals endpoint is versioned through its media type (plain application/json answers 406)
  const goals = calls.find(c => c.url.endsWith('/depot/anlageziel'));
  assert.match(goals.options.headers.Accept, /^application\/vv\.anlageziel\.v3\+json/);
});

test('only successful bearer requests to the real API establish authorization', async () => {
  const w = page(async () => response({}, 200));
  await w.fetch('https://service.meindepot.union-investment.de.evil.example/customer-functional-service/customer', {headers:{authorization:'Bearer bad'}});
  assert.equal(w.__gcUnion.ready(), false);
  // the Keycloak endpoints on the same host never count
  await w.fetch(api + '/auth/realms/union-investment/protocol/openid-connect/token', {headers:{authorization:'Bearer bad'}});
  assert.equal(w.__gcUnion.ready(), false);
  // a non-bearer scheme is ignored
  await w.fetch(api + '/customer-functional-service/customer', {headers:{authorization:'Basic abc'}});
  assert.equal(w.__gcUnion.ready(), false);
  const failed = page(async () => response({}, 401));
  await authenticate(failed);
  assert.equal(failed.__gcUnion.ready(), false);
  assert.equal(page(server([]), 'https://evil.example').__gcUnion, undefined);
});

test('expired sessions, several depots and unsupported products cannot produce a snapshot', async () => {
  let fail = false;
  const base = server([]);
  const w = page(async (url, options) => fail ? response({}, 401) : base(url, options));
  await authenticate(w); fail = true;
  await assert.rejects(w.__gcUnion.collect(), /HTTP 401/);
  assert.equal(w.__gcUnion.ready(), false);
  const several = page(server([], 2));
  await authenticate(several);
  await assert.rejects(several.__gcUnion.collect(), /Mehrere Depots/);
  const portfolio = page(async (url, options) => url.endsWith('/depot/anlageziel') ? response([{produkt:{}}]) : base(url, options));
  await authenticate(portfolio);
  await assert.rejects(portfolio.__gcUnion.collect(), /ohne Einzelfonds/);
});

test('mailbox: pages through the inbox, skips uploads, and only returns real PDFs', async () => {
  const calls = [], w = page(server(calls));
  await authenticate(w);
  const docs = JSON.parse(await w.__gcUnion.documents());
  assert.equal(docs.length, 43); // 45 messages, one own upload and one not downloadable
  assert.deepEqual(Object.keys(docs[0]).sort(), ['attachmentId', 'depotNumber', 'id', 'path', 'sentDate', 'subject']);
  assert.equal(docs[0].depotNumber, '123456789');
  const pages = calls.filter(c => new URL(c.url).pathname === '/postbox').map(c => new URL(c.url).searchParams);
  assert.deepEqual(pages.map(p => [p.get('offset'), p.get('refresh'), p.get('folder'), p.get('maxResults')]),
    [['0','true','INBOX','20'], ['20','false','INBOX','20'], ['40','false','INBOX','20']]);
  assert(!JSON.stringify(docs).includes('DO NOT EXPORT'));
  const pdf = await w.__gcUnion.pdf(docs[0].path);
  assert(Buffer.from(pdf, 'base64').toString().startsWith('%PDF-'));
  await assert.rejects(w.__gcUnion.pdf('/postbox/depot/0/message/1000000009/attachment/1'), /kein PDF/);
  await assert.rejects(w.__gcUnion.pdf('/customer-functional-service/customer'), /Ungültiger Dokumentpfad/);
  await assert.rejects(w.__gcUnion.pdf('/postbox/depot/0/message/1/attachment/1/../../x'), /Ungültiger Dokumentpfad/);
});

test('mailbox: unexpected messages are summarized by shape, without ids or depot numbers', async () => {
  const base = server([]);
  const odd = (n, extra) => ({id:String(2000000000 + n), depotIndex:0, attachmentId:'1', downloadable:true, customerUpload:false,
    sentDate:'2026-01-01T00:00:00', subject:'DO NOT EXPORT', attachmentHref:'/postbox/depot/0/message/' + (2000000000 + n) + '/attachment/1', ...extra});
  const w = page(async (url, options) => {
    const u = new URL(url);
    if (u.pathname === '/postbox')
      return response({folder:'INBOX', offset:0, total:4, status:'OK', messages:[odd(1, {attachmentHref:undefined}), odd(2, {depotIndex:7,
        attachmentHref:'/postbox/depot/7/message/2000000002/attachment/1'}), odd(3, {attachmentHref:'/postbox/archive/2000000003/pdf'}), odd(4)]});
    return base(url, options);
  });
  await authenticate(w);
  await assert.rejects(w.__gcUnion.documents(), e => {
    assert.match(e.message, /Unbekanntes Postbox-Format/);
    assert.match(e.message, /1× ohne Anhangspfad/);
    assert.match(e.message, /1× Depotindex 7 unbekannt/);
    assert.match(e.message, /1× Pfad \/postbox\/archive\/#\/pdf/);
    assert(!/\d{6,}|DO NOT EXPORT/.test(e.message));
    return true;
  });
});

test('mailbox: letters to the depot owner are listed with their own scope and can be downloaded', async () => {
  const base = server([]);
  const path = '/postbox/user/depotowner/1/message/2000000001/attachment/1';
  const w = page(async (url, options) => {
    const u = new URL(url);
    if (u.pathname === '/postbox')
      return response({folder:'INBOX', offset:0, total:1, status:'OK', messages:[{id:'2000000001', depotIndex:0, attachmentId:'1', downloadable:true,
        customerUpload:false, sentDate:'2026-01-01T00:00:00', subject:'Test letter', attachmentHref:path}]});
    if (u.pathname === path) return response('%PDF-1.7 synthetic');
    return base(url, options);
  });
  await authenticate(w);
  const docs = JSON.parse(await w.__gcUnion.documents());
  assert.deepEqual(docs, [{ownerIndex:'1', id:'2000000001', attachmentId:'1', sentDate:'2026-01-01T00:00:00', subject:'Test letter', path}]);
  assert(Buffer.from(await w.__gcUnion.pdf(path), 'base64').toString().startsWith('%PDF-'));
});

test('mailbox: messages without subject are kept; messages without a valid date are reported', async () => {
  const base = server([]);
  const msg = (n, extra) => ({id:String(3000000000 + n), depotIndex:0, attachmentId:'1', downloadable:true, customerUpload:false,
    sentDate:'2026-01-01T00:00:00', subject:'x', attachmentHref:'/postbox/depot/0/message/' + (3000000000 + n) + '/attachment/1', ...extra});
  const make = messages => page(async (url, options) => new URL(url).pathname === '/postbox'
    ? response({folder:'INBOX', offset:0, total:messages.length, status:'OK', messages}) : base(url, options));
  const w = make([msg(1, {subject:null}), msg(2, {subject:undefined}), msg(3, {subject:''})]);
  await authenticate(w);
  const docs = JSON.parse(await w.__gcUnion.documents());
  assert.deepEqual(docs.map(d => d.subject), ['', '', '']);
  const bad = make([msg(4, {sentDate:null}), msg(5, {sentDate:'gestern'})]);
  await authenticate(bad);
  await assert.rejects(bad.__gcUnion.documents(), /2× ohne gültiges Datum/);
});

test('mailbox: a list that stops before the announced total is an error, not a silent cut-off', async () => {
  const base = server([]);
  const msg = n => ({id:String(4000000000 + n), depotIndex:0, attachmentId:'1', downloadable:true, customerUpload:false, sentDate:'2026-01-01T00:00:00',
    subject:'x', attachmentHref:'/postbox/depot/0/message/' + (4000000000 + n) + '/attachment/1'});
  const w = page(async (url, options) => {
    const u = new URL(url);
    if (u.pathname !== '/postbox') return base(url, options);
    const offset = Number(u.searchParams.get('offset'));
    // announces 150 messages but never serves more than 100
    return response({folder:'INBOX', offset, total:150, status:'OK', messages:offset < 100 ? Array.from({length:20}, (_, i) => msg(offset + i)) : []});
  });
  await authenticate(w);
  await assert.rejects(w.__gcUnion.documents(), /Postbox unvollständig \(100 von 150 Nachrichten\)/);
});

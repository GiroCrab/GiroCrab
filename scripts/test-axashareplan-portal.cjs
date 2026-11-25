// Run the actual script embedded in AxaShareplanUI.swift against synthetic portal answers (no real data).
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const source = readFileSync(new URL('../GiroCrabApp/Sources/GiroCrabUI/AxaShareplanUI.swift', `file://${__filename}`), 'utf8');
const hook = source.match(/static let hook = #"""\n([\s\S]*?)\n    """#/)[1];
const pageOrigin = 'https://espace-salarie.axa-mon-epargne-entreprises.com';
const API = '/1.230.0/api/b2b';
const answer = (body, status = 200) => ({ok: status >= 200 && status < 300, status, json: async () => body, text: async () => JSON.stringify(body),
  clone() { return answer(body, status); }});

// amounts are whole cents, like the portal reports them
const eur = value => ({value, currency: 'EUR'});
const position = (id, {shares = 100, price = 5000, avg = 2000, invested, value, ticker = 'QS0000000001'} = {}) => ({
  instrument_id: id, ticker, market_price: price, shares_owned: shares, can_sell: true,
  total_invested: eur(invested ?? Math.round(shares * avg)), total_value: eur(value ?? Math.round(shares * price)),
  avg_price: eur(avg), total_gain: eur(1), pl_delta_percent: '+1.00%', gain_loss: eur(1)});
const portfolio = (...positions) => ({portfolio: {positions, total_value: eur(1)}, metrics: {}});
const instrument = (id, name = 'Testplan 2022', isin = 'QS0000000001') => ({instrument_id: id, name, ticker: isin, isin, image_url: 'x'});

class FakeXhr {
  constructor() { this.listeners = {}; }
  open(method, url) { this.url = url; }
  setRequestHeader() {}
  addEventListener(type, fn) { (this.listeners[type] ||= []).push(fn); }
}

// the portal calls its profile interface once the user is signed in
async function page(fetch, {origin = pageOrigin, signIn = true, path = `${API}/profile/me`} = {}) {
  const window = {fetch};
  const context = vm.createContext({window, location: {origin, href: origin + '/'}, fetch, AbortController, setTimeout, clearTimeout,
    Date, Headers, Request, URL, XMLHttpRequest: FakeXhr});
  vm.runInContext(hook, context);
  if (signIn) {
    await window.fetch(path, {headers: {authorization: 'Bearer synthetic', deviceid: 'device-1', platform: 'web'}});
    await new Promise(resolve => setTimeout(resolve, 5));
  }
  return window;
}
const portal = (pf, ins) => async (url, options) => {
  if (/\/profile\/(me|affiliations)$/.test(url)) return answer({});
  if (url.endsWith('/profile/portfolio')) return typeof pf === 'function' ? pf(options) : answer(pf);
  if (url.endsWith('/instruments')) return answer(ins ?? {instruments: []});
  return answer({}, 404);
};

test('reads one position per fund with cents, shares and the portal names', async () => {
  const calls = [];
  const base = portal(portfolio(position(11, {shares: 150.4321, price: 5000, avg: 1800, invested: 270778, value: 752161}), position(12, {ticker: 'T12'})),
    {instruments: [instrument(11, ' Testplan  2022 Global '), instrument(12, 'Testplan 2023 Global', 'QS0000000002')]});
  const w = await page(async (url, options) => { calls.push({url, options}); return base(url, options); });
  assert.equal(w.__gcAxaShareplan.ready(), true);
  const data = JSON.parse(await w.__gcAxaShareplan.collect());
  assert.equal(data.version, 1);
  assert.match(data.asOf, /^\d{4}-\d{2}-\d{2}$/);
  assert.deepEqual(data.depot.items, [
    {id: '11', isin: 'QS0000000001', name: 'Testplan 2022 Global', quantity: 150.4321, priceCents: 5000, averagePriceCents: 1800,
      investedCents: 270778, valueCents: 752161},
    {id: '12', isin: 'QS0000000002', name: 'Testplan 2023 Global', quantity: 100, priceCents: 5000, averagePriceCents: 2000,
      investedCents: 200000, valueCents: 500000}]);
  // two read queries with fixed bodies, the portal's own headers, the observed interface version
  const sent = calls.slice(1);
  assert.deepEqual(sent.map(c => c.url), [`${API}/profile/portfolio`, `${API}/instruments`]);
  assert.deepEqual(JSON.parse(sent[0].options.body), {type: 'mutual-fund', offset: 0, limit: 500});
  assert.deepEqual(JSON.parse(sent[1].options.body), {instrument_ids: [11, 12]});
  for (const c of sent) {
    assert.equal(c.options.method, 'POST');
    assert.equal(c.options.credentials, 'same-origin');
    assert.equal(c.options.redirect, 'error');
    assert.equal(c.options.headers.authorization, 'Bearer synthetic');
    assert.equal(c.options.headers.deviceid, 'device-1');
    assert.equal(c.options.headers['content-type'], 'application/json');
  }
});

test('nothing but the selected fields crosses the bridge', async () => {
  const noisy = position(11);
  noisy.owner = 'DO NOT EXPORT';
  const ins = instrument(11);
  ins.email = 'DO NOT EXPORT';
  const w = await page(portal(portfolio(noisy), {instruments: [ins]}));
  const text = await w.__gcAxaShareplan.collect();
  assert(!text.includes('DO NOT EXPORT'));
  assert.deepEqual(Object.keys(JSON.parse(text).depot.items[0]).sort(),
    ['averagePriceCents', 'id', 'investedCents', 'isin', 'name', 'priceCents', 'quantity', 'valueCents']);
});

test('the interface version follows the portal and the sign-in requests do not count', async () => {
  const calls = [];
  const w = await page(async (url, options) => { calls.push(url); return portal(portfolio(position(1)), {instruments: [instrument(1)]})(url, options); },
    {path: '/1.231.4/api/b2b/profile/affiliations'});
  await w.__gcAxaShareplan.collect();
  assert.deepEqual(calls.slice(1), ['/1.231.4/api/b2b/profile/portfolio', '/1.231.4/api/b2b/instruments']);
  // sign-in, analytics and other calls before a profile answer: still not signed in
  for (const path of ['/api/auth/two-factor/verify-otp', `${API}/analytics/bulk_events`, `${API}/auth/realtime_token`]) {
    const early = await page(async () => answer({}), {path});
    assert.equal(early.__gcAxaShareplan.ready(), false, path);
    await assert.rejects(early.__gcAxaShareplan.collect(), /erneut anmelden/);
  }
  // a failed profile call is not a sign-in either
  const denied = await page(async () => answer({}, 401));
  assert.equal(denied.__gcAxaShareplan.ready(), false);
});

test('the portal using XMLHttpRequest also signs in and hands over its headers', async () => {
  const calls = [];
  const window = {fetch: async (url, options) => { calls.push({url, options}); return portal(portfolio(position(1)), {instruments: [instrument(1)]})(url, options); }};
  const context = vm.createContext({window, location: {origin: pageOrigin, href: pageOrigin + '/'}, fetch: window.fetch, AbortController,
    setTimeout, clearTimeout, Date, Headers, Request, URL, XMLHttpRequest: FakeXhr});
  vm.runInContext(hook, context);
  const xhr = new context.XMLHttpRequest();
  xhr.open('GET', `${API}/profile/me`);
  xhr.setRequestHeader('Authorization', 'Bearer from-xhr');
  xhr.status = 200;
  for (const fn of xhr.listeners.load) fn.call(xhr);
  assert.equal(window.__gcAxaShareplan.ready(), true);
  await window.__gcAxaShareplan.collect();
  assert.equal(calls[0].options.headers.authorization, 'Bearer from-xhr');
});

test('sold-out positions are left out, an empty plan is a valid empty depot', async () => {
  const w = await page(portal(portfolio(position(1, {shares: 0, value: 0, invested: 0}), position(2)), {instruments: [instrument(2)]}));
  assert.deepEqual(JSON.parse(await w.__gcAxaShareplan.collect()).depot.items.map(i => i.id), ['2']);
  const calls = [];
  const empty = await page(async (url, options) => { calls.push(url); return portal(portfolio())(url, options); });
  assert.deepEqual(JSON.parse(await empty.__gcAxaShareplan.collect()).depot.items, []);
  // no instruments query without positions
  assert.deepEqual(calls.slice(1), [`${API}/profile/portfolio`]);
});

test('a missing name falls back to the ticker, an odd identifier is dropped', async () => {
  const odd = instrument(1, '', 'not-an-isin');
  const w = await page(portal(portfolio(position(1, {ticker: 'FALLBACK'})), {instruments: [odd]}));
  const item = JSON.parse(await w.__gcAxaShareplan.collect()).depot.items[0];
  assert.equal(item.name, 'FALLBACK');
  assert.equal(item.isin, null);
});

test('unreadable amounts, units or answers stop the fetch instead of importing a part', async () => {
  const bad = async (pf, ins, expected) => {
    const w = await page(portal(pf, ins));
    await assert.rejects(w.__gcAxaShareplan.collect(), expected);
  };
  const wrongCurrency = position(1); wrongCurrency.total_value = {value: 100, currency: 'USD'};
  await bad(portfolio(wrongCurrency), {instruments: [instrument(1)]}, /Unbekanntes Depotformat/);
  const fractional = position(1); fractional.avg_price = eur(20.5);
  await bad(portfolio(fractional), {instruments: [instrument(1)]}, /Einstand/);
  const noPrice = position(1); noPrice.market_price = null;
  await bad(portfolio(noPrice), {instruments: [instrument(1)]}, /Kurs/);
  const negative = position(1); negative.shares_owned = -1;
  await bad(portfolio(negative), {instruments: [instrument(1)]}, /Stückzahl/);
  const noId = position(1); noId.instrument_id = 'x';
  await bad(portfolio(noId), {instruments: []}, /Kennung/);
  await bad({something: 'else'}, {}, /Keine Depotdaten/);
  await bad({errors: ['x']}, {}, /Keine Depotdaten/);
  // one good and one bad position: nothing is imported
  await bad(portfolio(position(1), wrongCurrency), {instruments: [instrument(1)]}, /Unbekanntes Depotformat/);
  // a full page of positions may be cut off
  await bad(portfolio(...Array.from({length: 500}, (_, i) => position(i + 1))), {instruments: []}, /Mehr Positionen/);
});

test('HTTP errors, an expired session and a hanging request end with a message', async () => {
  const w = await page(portal(() => answer({}, 500)));
  await assert.rejects(w.__gcAxaShareplan.collect(), /HTTP 500/);
  const expired = await page(portal(() => answer({}, 401)));
  await assert.rejects(expired.__gcAxaShareplan.collect(), /HTTP 401/);
  assert.equal(expired.__gcAxaShareplan.ready(), false);
  const broken = await page(portal(() => { throw new Error('network'); }));
  await assert.rejects(broken.__gcAxaShareplan.collect(), /Abruf fehlgeschlagen/);
});

test('the script does nothing on another origin', async () => {
  const w = await page(async () => answer({}), {origin: 'https://example.org', signIn: false});
  assert.equal(w.__gcAxaShareplan, undefined);
});

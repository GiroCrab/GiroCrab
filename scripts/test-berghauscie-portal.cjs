// Run the actual script embedded in BerghausCieUI.swift against synthetic GraphQL answers (no real data).
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const source = readFileSync(new URL('../GiroCrabApp/Sources/GiroCrabUI/BerghausCieUI.swift', `file://${__filename}`), 'utf8');
const hook = source.match(/static let hook = #"""\n([\s\S]*?)\n    """#/)[1];
const pageOrigin = 'https://my.berghauscie.de';
const API = 'https://bc-cms.herokuapp.com/graphql';
const SRC = 'WINE_SEARCHER_EUROPE_AVERAGE_EURO';
const answer = body => ({ok: true, status: 200, text: async () => JSON.stringify(body), json: async () => body,
  clone() { return answer(body); }});
const failure = status => ({ok: false, status, text: async () => '', json: async () => ({}), clone() { return failure(status); }});

// one portal entry: `amount` cases of `cask` bottles
const entry = (id, {amount=2, cask=6, buy=150, size={title:'0,75l', factorForLiterPrices:0.75}, valuations, status='PURCHASED', title='Testgut  Cuvée 2020'} = {}) => ({
  id, amount, caskOf: cask, buyPriceBtl: buy, bottleSize: size,
  vintage: {title, year: 2020, wine: {lwin: 1100000 + Number(id)},
    valuationRel: valuations || [{sourceOfValuation: SRC, datetimeCreated: '2026-07-01T00:00:00+00:00', value: 200}]},
  depotPositionStatusRel: [{status, datetimeStatusUpdate: '2026-03-15T10:00:00+00:00'}]});
const depot = (...entries) => ({data: {clientsDepot: entries}});

class FakeXhr {
  constructor() { this.listeners = {}; }
  open() {}
  setRequestHeader() {}
  addEventListener(type, fn) { this.listeners[type] = fn; }
}

// `signedIn`: the portal's own request has been answered with a user
async function page(fetch, {origin=pageOrigin, signedIn=true} = {}) {
  const window = {fetch};
  const context = vm.createContext({window, location:{origin, href:origin + '/'}, fetch, AbortController, setTimeout, clearTimeout,
    Date, Headers, Request, URL, XMLHttpRequest: FakeXhr});
  vm.runInContext(hook, context);
  if (signedIn) {
    await window.fetch(API, {method: 'POST', headers: {authorization: 'JWT synthetic'}, body: '{}'});
    await new Promise(resolve => setTimeout(resolve, 5));
  }
  return window;
}
const portal = depotAnswer => async (url, options) => {
  if (options && options.body === '{}') return answer({data: {me: {}}});
  return answer(depotAnswer);
};

test('cases times bottles, valuation by size, purchase price per bottle', async () => {
  const calls = [];
  const w = await page(async (url, options) => { calls.push({url, options}); return portal(depot(
    entry('1', {amount: 2, cask: 6, buy: 150, valuations: [
      {sourceOfValuation: SRC, datetimeCreated: '2025-07-01T00:00:00+00:00', value: 180},
      {sourceOfValuation: SRC, datetimeCreated: '2026-07-01T00:00:00+00:00', value: 200}]}),
    entry('2', {amount: 2, cask: 1, buy: 1500, size: {title: '1,5l', factorForLiterPrices: 1.5},
      valuations: [{sourceOfValuation: SRC, datetimeCreated: '2026-07-01T00:00:00+00:00', value: 800}]})))(url, options); });
  assert.equal(w.__gcBerghausCie.ready(), true);
  const data = JSON.parse(await w.__gcBerghausCie.collect());
  assert.equal(data.version, 1);
  assert.match(data.asOf, /^\d{4}-\d{2}-\d{2}$/);
  assert.deepEqual(data.depot.items, [
    {id:'1', lwin:'1100001', name:'Testgut Cuvée 2020', bottles:12, price:200, purchasePrice:150, valueDate:'2026-07-01', value:2400},
    {id:'2', lwin:'1100002', name:'Testgut Cuvée 2020 (1,5l)', bottles:2, price:1600, purchasePrice:1500, valueDate:'2026-07-01', value:3200}]);
  // one request to the service, with the portal's own authorization and a fixed query
  const last = calls[calls.length - 1];
  assert.equal(last.url, API);
  assert.equal(last.options.credentials, 'include');
  assert.equal(last.options.redirect, 'error');
  const sent = JSON.parse(last.options.body);
  assert.deepEqual(Object.keys(sent), ['query']);
  assert.doesNotMatch(sent.query, /email|phone|street|invoice|password|firstName|lastName/i);
});

test('nothing but the selected fields crosses the bridge', async () => {
  const noisy = entry('1');
  noisy.vintage.wine.secret = 'DO NOT EXPORT';
  noisy.owner = 'DO NOT EXPORT';
  const w = await page(portal(depot(noisy)));
  const text = await w.__gcBerghausCie.collect();
  assert(!text.includes('DO NOT EXPORT') && !text.includes('synthetic'));
});

test('positions that left the depot are left out, an unknown status is kept', async () => {
  const w = await page(portal(depot(entry('1'), entry('2', {status: 'SOLD'}), entry('3', {status: 'DELIVERED'}), entry('4', {status: 'IN_STORAGE'}))));
  assert.deepEqual(JSON.parse(await w.__gcBerghausCie.collect()).depot.items.map(i => i.id), ['1', '4']);
});

test('an empty depot is a valid, empty list', async () => {
  const w = await page(portal(depot()));
  assert.deepEqual(JSON.parse(await w.__gcBerghausCie.collect()).depot.items, []);
});

test('a position that cannot be read stops the import; it never leaves positions out', async () => {
  for (const [broken, reason] of [
    [entry('1', {valuations: [{sourceOfValuation: 'OTHER', datetimeCreated: '2026-07-01T00:00:00+00:00', value: 5}]}), /Bewertung/],
    // the newest valuation is unusable: the older one must not be used instead
    [entry('1', {valuations: [{sourceOfValuation: SRC, datetimeCreated: '2025-07-01T00:00:00+00:00', value: 100},
      {sourceOfValuation: SRC, datetimeCreated: '2026-07-01T00:00:00+00:00', value: null}]}), /Bewertung/],
    [entry('1', {valuations: [{sourceOfValuation: SRC, datetimeCreated: '2026-07-01T00:00:00+00:00', value: -5}]}), /Bewertung/],
    [entry('1', {amount: 0}), /Menge/],
    [entry('1', {cask: 1.5}), /Menge/],
    [entry('1', {buy: 'x'}), /Einstandspreis/],
    [entry('1', {size: {title: 'x', factorForLiterPrices: null}}), /Flaschengröße/]]) {
    const w = await page(portal(depot(entry('2'), broken)));
    await assert.rejects(w.__gcBerghausCie.collect(), reason);
  }
});

test('errors, other pages and an expired session', async () => {
  await assert.rejects((await page(portal({errors: [{message: 'x'}], data: null}))).__gcBerghausCie.collect(), /Keine Depotdaten/);
  await assert.rejects((await page(portal({data: {clientsDepot: null}}))).__gcBerghausCie.collect(), /Keine Depotdaten/);
  const expired = await page(async (url, options) => options.body === '{}' ? answer({data: {me: {}}}) : failure(401));
  await assert.rejects(expired.__gcBerghausCie.collect(), /HTTP 401/);
  assert.equal(expired.__gcBerghausCie.ready(), false);
  await assert.rejects(expired.__gcBerghausCie.collect(), /erneut anmelden/);
});

test('not ready before the portal has answered with a user', async () => {
  const w = await page(portal(depot()), {signedIn: false});
  assert.equal(w.__gcBerghausCie.ready(), false);
  await assert.rejects(w.__gcBerghausCie.collect(), /erneut anmelden/);
  // a user-less answer (signed out) does not count
  const out = await page(async () => answer({data: {me: null}}));
  assert.equal(out.__gcBerghausCie.ready(), false);
});

test('the script stays inactive outside the portal origin', async () => {
  const w = await page(portal(depot()), {origin: 'https://evil.example', signedIn: false});
  assert.equal(w.__gcBerghausCie, undefined);
});

test('the sign-in answer counts, its token is never kept in the result', async () => {
  const w = await page(async () => answer({data: {tokenAuth: {token: 'synthetic.token.value'}}}), {signedIn: false});
  await w.fetch(API, {method: 'POST', body: '{}'});
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal(w.__gcBerghausCie.ready(), true);
});

// Run the actual script embedded in SharinboxUI.swift against a synthetic start page (no real data).
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const source = readFileSync(new URL('../GiroCrabApp/Sources/GiroCrabUI/SharinboxUI.swift', `file://${__filename}`), 'utf8');
const hook = source.match(/static let hook = #"""\n([\s\S]*?)\n    """#/)[1];
const pageOrigin = 'https://sharinbox.sgmarkets.com';
const response = (text, status=200) => ({ok: status >= 200 && status < 300, status, text: async () => text,
  arrayBuffer: async () => Uint8Array.from(Buffer.from(text)).buffer});

// Same structure as the portal's start page: one block per security, then the share rights.
const block = (isin, name, price, free, blocked) => `
<div class="ml-3 mt-3">
  <h5 class="display-5">ACTION ${name}</h5> (${isin}) <br/>
  <span class="stockprice"> Kurs 20.11.26, 17:35:00 MEZ : <em>${price}</em> </span>
  <div class="row mt-3"><div class="col-md-6"><div><canvas id="myChart-assets-${isin}"></canvas></div>
    <script type="text/javascript"> let valeurDisponible${isin} = '1'; </script></div>
    <div class="col-md-6 my-auto">
      <div class="panelChartBorderBlue"><div class="ml-2"><span class="font-weight-bold">Verfügbares guthaben</span>
        <div class="row"><div class="col-6"><span class="text-muted">Menge</span> <br/> ${free[0]} </div>
          <div class="col-6"><span class="text-muted">Importe</span> <br/> ${free[1]} </div></div></div></div>
      <div class="panelChartBorderLightBlue"><div class="ml-2 mt-2"><span class="font-weight-bold">Nicht verfügbares guthaben</span>
        <div class="row"><div class="col-6"><span class="text-muted">Menge</span><br/> ${blocked[0]} </div>
          <div class="col-6"><span class="text-muted">Bewertung</span> <br/> ${blocked[1]} </div></div></div></div>
    </div></div>
  <div class="row"><div class="col-md-6"><canvas id="myChart-detail-${isin}"></canvas></div></div>
</div>`;
const rights = '<div class="ml-3 mt-3"><canvas id="myChart-droitsActions"></canvas> let valeurTotal = \'2.085\';</div>';
const home = (...blocks) => '<html><body><h4>Wertpapiere</h4>' + blocks.join('') + rights + '</body></html>';
const standard = home(block('FR0000000001', 'Test AG', '45,140 (EUR)', ['16.290', '735.330,60 EUR'], ['-', '-']));

// `shown` is the HTML the user currently sees (the portal may render it in a frame).
// The document list: one <li> per document with title, category badge and date.
const ID_A = '0a1b2c3d-1111-4222-8333-444455556666', ID_B = '0a1b2c3d-1111-4222-8333-444455557777';
const entry = (id, label, date) => `<li> <div class="doc" data-category="" data-filter="x"> <div class="doc-icon">
  <a href="/SharinBox/sgdocs/download.do?idSgdocs=${id}"><em class="fa fa-download fa-3x"></em></a></div>
  <div class="doc-info"> <div class="doc-label"> ${label} </div> <div class="doc-badges"><span class="badge">Legal</span></div> </div>
  <div class="doc-date"> <em>${date}</em> </div></div> </li>`;
const list = (...entries) => '<html><ul id="documents-documentation" class="list-doc">' + entries.join('') + '</ul></html>';
const downloadPath = id => '/SharinBox/sgdocs/download.do?idSgdocs=' + id;

function page(fetch, {origin=pageOrigin, shown=''} = {}) {
  const window = {};
  const document = {documentElement:{outerHTML:shown}, querySelectorAll: () => []};
  const context = vm.createContext({window, document, location:{origin}, fetch, AbortController, setTimeout, clearTimeout, Date, btoa});
  vm.runInContext(hook, context);
  return window;
}

test('collects the share, its price and value; nothing else crosses', async () => {
  const calls = [];
  const w = page(async (url, options) => { calls.push({url, options}); return response(standard); }, {shown: standard});
  assert.equal(w.__gcSharinbox.ready(), true);
  const snapshot = await w.__gcSharinbox.collect(), data = JSON.parse(snapshot);
  assert.equal(data.version, 1);
  assert.match(data.asOf, /^\d{4}-\d{2}-\d{2}$/);
  assert.deepEqual(data.depot.items, [{isin:'FR0000000001', name:'Test AG', quantity:16290, price:45.14,
    priceDate:'2026-11-20', currency:'EUR', value:735330.6}]);
  assert(!snapshot.includes('2.085'));
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, '/SharinBox/connexion.do');
  assert.equal(calls[0].options.credentials, 'same-origin');
  assert.equal(calls[0].options.mode, 'same-origin');
  assert.equal(calls[0].options.method, 'GET');
});

test('available and blocked balances are added up; several securities are listed', async () => {
  const html = home(
    block('FR0000000001', 'Test AG', '10,000 (EUR)', ['100', '1.000,00 EUR'], ['50', '500,00 EUR']),
    block('DE0000000002', 'Zweite AG', '2,500 (EUR)', ['4,5', '11,25 EUR'], ['-', '-']));
  const w = page(async () => response(html));
  const items = JSON.parse(await w.__gcSharinbox.collect()).depot.items;
  assert.deepEqual(items.map(i => [i.isin, i.quantity, i.value, i.price]),
    [['FR0000000001', 150, 1500, 10], ['DE0000000002', 4.5, 11.25, 2.5]]);
});

test('ready only while the overview is shown; the shown page is the fallback for the fetch', async () => {
  assert.equal(page(async () => response(''), {shown: '<html>Login</html>'}).__gcSharinbox.ready(), false);
  const w = page(async () => response('', 500), {shown: standard});
  assert.equal(w.__gcSharinbox.ready(), true);
  assert.equal(JSON.parse(await w.__gcSharinbox.collect()).depot.items[0].isin, 'FR0000000001');
});

test('nothing held is a valid, empty list; other pages and failures are errors', async () => {
  const w = page(async () => response(home()));
  assert.deepEqual(JSON.parse(await w.__gcSharinbox.collect()).depot.items, []);
  await assert.rejects(page(async () => response('<html>Login</html>')).__gcSharinbox.collect(), /Keine Bestandsübersicht/);
  await assert.rejects(page(async () => response('', 401)).__gcSharinbox.collect(), /HTTP 401/);
  await assert.rejects(page(async () => response(home(block('FR0000000001', 'X', 'abc', ['1', '1 EUR'], ['-', '-']))))
    .__gcSharinbox.collect(), /Kurs nicht gefunden/);
});

test('a changed page layout is an error, never an empty depot', async () => {
  // same page, but the security block is marked with single quotes: nothing would match
  const changed = home(block('FR0000000001', 'Test AG', '45,140 (EUR)', ['100', '4.514,00 EUR'], ['-', '-']))
    .replace('id="myChart-assets-FR0000000001"', "id='myChart-assets-FR0000000001'");
  assert(changed.includes("myChart-assets-FR0000000001'") && changed.includes('myChart-droitsActions'));
  await assert.rejects(page(async () => response(changed)).__gcSharinbox.collect(), /Seitenformat/);
  // the rights block alone (nothing held) is still a valid, empty list
  assert.deepEqual(JSON.parse(await page(async () => response(home())).__gcSharinbox.collect()).depot.items, []);
});

test('unknown number formats and mixed currencies are rejected', async () => {
  const bad = home(block('FR0000000001', 'X', '1,000 (EUR)', ['1,2,3', '1 EUR'], ['-', '-']));
  await assert.rejects(page(async () => response(bad)).__gcSharinbox.collect(), /Zahlenformat/);
  const mixed = home(block('FR0000000001', 'X', '1,000 (EUR)', ['1', '1,00 EUR'], ['1', '1,00 USD']));
  await assert.rejects(page(async () => response(mixed)).__gcSharinbox.collect(), /Währung/);
});

test('the script stays inactive outside the portal origin', async () => {
  const w = page(async () => response(standard), {origin:'https://example.org'});
  assert.equal(w.__gcSharinbox, undefined);
});

test('lists the documents with title, date and download path', async () => {
  const calls = [];
  const w = page(async (url, options) => { calls.push({url, options});
    return response(list(entry(ID_A, 'Auszug Gratisaktien', '15.01.2026'), entry(ID_B.toUpperCase(), 'Portfolioauszug', '03.02.2026'),
      entry(ID_A, 'Auszug Gratisaktien', '15.01.2026'))); });
  const docs = JSON.parse(await w.__gcSharinbox.documents());
  assert.deepEqual(docs, [
    {id:ID_A, sentDate:'2026-01-15', subject:'Auszug Gratisaktien', path:downloadPath(ID_A)},
    {id:ID_B, sentDate:'2026-02-03', subject:'Portfolioauszug', path:downloadPath(ID_B)}]);
  assert.equal(calls[0].url, '/SharinBox/documentationAcces.do');
  assert.equal(calls[0].options.credentials, 'same-origin');
  assert.equal(calls[0].options.mode, 'same-origin');
});

test('odd list entries and other pages are errors that name shapes only', async () => {
  const odd = list(entry(ID_A, 'Ok', '15.01.2026'), entry('not-an-id', 'Geheim', '15.01.2026'), entry(ID_B, 'Ohne', 'bald'));
  await assert.rejects(page(async () => response(odd)).__gcSharinbox.documents(),
    err => /ohne gültige ID/.test(err.message) && /ohne Datum/.test(err.message) && !/Geheim|not-an-id/.test(err.message));
  await assert.rejects(page(async () => response('<html>Login</html>')).__gcSharinbox.documents(), /Keine Dokumentenliste/);
  await assert.rejects(page(async () => response('', 500)).__gcSharinbox.documents(), /HTTP 500/);
});

test('a list entry the parser does not recognise stops the import', async () => {
  // same entry, but with another attribute in front of the class: it would be skipped silently
  const changed = entry(ID_B, 'Neu', '03.02.2026').replace('<div class="doc"', '<div id="x" class="doc"');
  const html = list(entry(ID_A, 'Ok', '15.01.2026'), changed);
  await assert.rejects(page(async () => response(html)).__gcSharinbox.documents(),
    err => /nicht erkannt/.test(err.message) && !/Neu/.test(err.message));
});

test('downloads a PDF only for a valid path and only if it is one', async () => {
  const calls = [];
  const w = page(async (url, options) => { calls.push(url); return response(url.includes(ID_B) ? '<html>login</html>' : '%PDF-1.7 synthetic'); });
  assert.equal(Buffer.from(await w.__gcSharinbox.pdf(downloadPath(ID_A)), 'base64').toString(), '%PDF-1.7 synthetic');
  await assert.rejects(w.__gcSharinbox.pdf(downloadPath(ID_B)), /kein PDF/);
  for (const bad of ['/SharinBox/connexion.do', '/SharinBox/sgdocs/download.do?idSgdocs=../x', 'https://example.org/x.pdf', 42])
    await assert.rejects(w.__gcSharinbox.pdf(bad), /Ungültiger Dokumentpfad/);
  assert.deepEqual(calls, [downloadPath(ID_A), downloadPath(ID_B)]);
});

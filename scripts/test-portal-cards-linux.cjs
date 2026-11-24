// Amex cards on Linux (#243): the capture hook of GiroCrabLinux/qml/WebLoginSheet.qml tags each
// response with PortalBrowser.cardKey (SHA-256 of the card token, first 8 bytes, hex) without
// handing the token out, and the card switcher scripts of PortalWebLoginFlow.qml read and pick
// the cards. Synthetic page only.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {readFileSync} = require('node:fs');
const vm = require('node:vm');
const {webcrypto, createHash} = require('node:crypto');

const read = p => readFileSync(new URL(p, `file://${__filename}`), 'utf8');
const literal = (src, name) => vm.runInNewContext('`' + src.match(new RegExp('readonly property string ' + name + ': `((?:\\\\[\\s\\S]|[^`\\\\])*)`'))[1] + '`');
const hook = literal(read('../GiroCrabLinux/qml/WebLoginSheet.qml'), 'captureHook');
const flowSrc = read('../GiroCrabLinux/qml/PortalWebLoginFlow.qml');
const switcher = literal(flowSrc, 'amexSwitcherJS');
// amexCardsJS / selectAmexCardJS are `amexSwitcherJS + \`…\``: take the second literal
const tail = name => vm.runInNewContext('`' + flowSrc.match(new RegExp('readonly property string ' + name + ': amexSwitcherJS \\+ `((?:\\\\[\\s\\S]|[^`\\\\])*)`'))[1] + '`');

test('responses carry the short card key, never the token', async () => {
  const token = 'invented-account-token';
  const context = vm.createContext({crypto: webcrypto, TextEncoder, Headers, Response,
    XMLHttpRequest: class { open() {} setRequestHeader() {} addEventListener() {} },
    fetch: async () => new Response('{"transactions":[]}')});
  vm.runInContext('window = globalThis', context);
  vm.runInContext(hook, context);
  await vm.runInContext(`fetch('https://global.americanexpress.com/api/transactions', {headers: {'account_token': '${token}'}})`, context);
  await vm.runInContext('fetch("https://global.americanexpress.com/api/profile")', context);
  for (let i = 0; i < 20 && (context.__gcBodies || []).length < 2; i++) await new Promise(r => setTimeout(r, 5));
  const q = context.__gcBodies;
  assert.equal(q.length, 2);
  const key = createHash('sha256').update(token).digest('hex').slice(0, 16);
  assert.deepEqual(Array.from(q, e => e.card).sort(), ['', key].sort());
  assert(!JSON.stringify(q).includes(token));
});

function page(current, cards) {
  let open = false;
  const clicked = [];
  const button = (text, onClick) => ({innerText: text, getBoundingClientRect: () => ({width: 50, height: 20}),
    click() { clicked.push(text); onClick(); }});
  const sw = button('-' + current, () => { open = !open; });
  const entries = cards.map(c => button(`${c.name}••••${c.number}`, () => { open = false; }));
  const context = vm.createContext({setTimeout: (f) => f(), document: {querySelectorAll: () => open ? [sw, ...entries] : [sw]}});
  const run = (body, args = {}) => vm.runInContext(
    `(async function(${Object.keys(args).join(',')}){\n${body}\n})(${Object.values(args).map(v => JSON.stringify(v)).join(',')})`, context);
  return {run, clicked};
}

test('the switcher lists the cards and opens the one asked for', async () => {
  const cards = [{name: 'American Express® Card', number: '11111'}, {name: 'Amazon Business American Express® Card', number: '22222'}];
  const {run, clicked} = page('11111', cards);
  const list = JSON.parse(await run(switcher + tail('amexCardsJS')));
  assert.equal(list.current, '11111');
  assert.deepEqual(list.cards, cards);
  assert.equal(await run(switcher + tail('selectAmexCardJS'), {number: '22222'}), true);
  assert.equal(clicked.at(-1), 'Amazon Business American Express® Card••••22222');
  assert.equal(await run(switcher + tail('selectAmexCardJS'), {number: '99999'}), false);
});

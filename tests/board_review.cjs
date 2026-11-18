// Focused UI regression checks using the same DOM model as tests/board.rs.
const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const html = fs.readFileSync('src/board.html', 'utf8');
const script = html.slice(html.indexOf('<script>') + 8, html.lastIndexOf('</script>'));
const dom = fs.readFileSync('tests/support/board_dom.js', 'utf8').replace('const fetch =', 'let fetch =');
async function main() {
  const result = await vm.runInNewContext(dom + '\n' + script + '\n' + `
(async () => {
  const check = (ok, msg) => { if (!ok) throw new Error(msg); };
  connected = true;
  const c = { id: 'a1', at: '2026-01-01', ticket: 't1', handle: 'm1', kind: 'gate', text: 'Ready', options: ['yes'] };
  const card = makeCard(c); $('cards').append(card); patchCard(card, c);
  const input = part(card, 'input');
  input.value = 'my draft'; input.setSelectionRange(2, 4); input.dispatch('input');
  patchCard(card, { ...c, text: 'Updated' });
  check(input.value === 'my draft' && input.selectionStart === 2 && input.selectionEnd === 4, 'same instance loses draft/cursor');
  patchCard(card, { ...c, at: '2026-02-01' });
  check(input.value === '', 'reused ID inherits draft');
  // Control the request completions to test an old response arriving last.
  const requests = [];
  fetch = () => new Promise((resolve, reject) => requests.push({ resolve, reject }));
  patchCard(card, c); toggleFull(card);
  patchCard(card, { ...c, handle: 'm2' });
  check(part(card, 'full').hidden && !card._loaded, 'handle change keeps expansion/cache');
  toggleFull(card);
  requests[1].resolve({ ok: true, json: async () => ({ body: 'new body' }) });
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  requests[0].resolve({ ok: true, json: async () => ({ body: 'old body' }) });
  await Promise.resolve(); await Promise.resolve(); await Promise.resolve();
  check(part(card, 'full').firstChild.textContent === 'new body', 'old response overwrites new body');
  patchCard(card, { ...c, handle: 'm2', text: 'refresh' });
  toggleFull(card); toggleFull(card);
  check(requests.length === 2, 'same handle refetches cached body');
  const t = { label: 'widgets#1', status: 'running', workers: [], joined: false };
  firstState = true;
  state = { asks: [] };
  syncTickets([t], []);
  const row = $('open').firstChild;
  syncTickets([{ ...t, thread: 't-new', joined: true }], []);
  check($('open').firstChild === row && row.dataset.thread === 't-new', 'joining replaces pending row or stale thread');
  const it = { kind: 't', t: { label: 'widgets#1', thread: 't-new' } };
  const settled = makeSettled(it); patchSettled(settled, it);
  const restore = part(settled, 'restore');
  restore.setAttribute('aria-disabled', 'true'); patchSettled(settled, it);
  check(restore.getAttribute('aria-disabled') !== 'true', 'restore stays stale');
  restore._busy = true; patchSettled(settled, it);
  check(restore.getAttribute('aria-disabled') === 'true', 'busy restore enabled');
  $('keys').open = true;
  const focus = document.activeElement;
  for (const f of document.listeners.keydown) f({ key: 'l', target: $('keys'), preventDefault() { throw new Error('dialog shortcut intercepted'); } });
  check(document.activeElement === focus, 'dialog shortcut moved focus');
  const button = h('button'); $('composer').append(button);
  fetch = async () => { throw new Error('composer failure'); };
  await act(button, '/api/asks/x/settle');
  check(!$('composer-err').hidden && $('composer-err-text').textContent === 'composer failure', 'composer error invisible');
  return 'draft, handle race/cache, pending row, restore, dialog, composer errors';
})()
`);
  assert.ok(result);
  process.stdout.write(`PASS: ${result}\n`);
}
main().catch(e => { console.error(e); process.exitCode = 1; });

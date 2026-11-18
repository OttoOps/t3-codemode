const fs = require('node:fs');
const vm = require('node:vm');
const assert = require('node:assert/strict');
const base = 'docs/design/board-ux/harness/';
const holds = fs.readFileSync(base + 'holds.js', 'utf8');
const wrapper = holds.slice(holds.indexOf('const W ='), holds.indexOf('Object.defineProperty(W'));
function notification(events) {
  const context = { window: { __notes: [] }, RN: class {
    constructor() { this.handlers = {}; }
    addEventListener(name, fn) { this.handlers[name] = fn; }
  }, setTimeout: fn => { context.timeout = fn; return 1; }, clearTimeout() {} };
  vm.createContext(context);
  vm.runInContext(wrapper + ';window.note = new W("test", {});', context);
  for (const event of events) event === 'timeout' ? context.timeout() : context.window.note.handlers[event]();
  return context.window.__notes.length;
}
assert.equal(notification([]), 0);
assert.equal(notification(['show']), 1);
assert.equal(notification(['show', 'show']), 1);
assert.equal(notification(['error', 'show']), 0);
assert.equal(notification(['timeout', 'show']), 0);
const gates = fs.readFileSync(base + 'gates.js', 'utf8');
const within = new Function('f', 'b', 'return ' + gates.match(/within: (.*?), cls:/)[1]);
const good = { top: 0, bottom: 32, width: 100, height: 32 };
assert.ok(within(good, good));
for (const bad of [{...good, width:0}, {...good, height:0}, {...good, top:-1}, {...good, bottom:461}]) {
  assert.equal(within(bad, good), false);
  assert.equal(within(good, bad), false);
}
const stability = fs.readFileSync(base + 'stability.js', 'utf8');
const start = stability.indexOf('const el = document.elementFromPoint');
const end = stability.indexOf('}, [x, y]);', start);
const hover = new Function('document', 'window', 'px', 'py', stability.slice(start, end));
const rect = {top:10,left:10,width:100,height:32};
let current = {...rect};
const item = { getBoundingClientRect: () => current };
const el = { getBoundingClientRect: () => current, closest: () => null };
const doc = { elementFromPoint: () => el };
const win = { __hov:el, __hovRect:rect, __hovItem:item, __hovItemRect:rect };
assert.equal(hover(doc, win, 20, 20), true);
current = {...rect, top:12};
assert.equal(hover(doc, win, 20, 20), false);
doc.elementFromPoint = () => ({closest: () => item});
assert.equal(hover(doc, win, 20, 20), false);
current = {...rect};
assert.equal(hover(doc, win, 20, 20), true);
current = {...rect, width:120};
assert.equal(hover(doc, win, 20, 20), false);
console.log('PASS: notification delivery, keyboard bounds, hover and placeholder geometry');

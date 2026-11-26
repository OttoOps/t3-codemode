"use strict";
class Node {}
class Text extends Node {
  constructor(t) { super(); this.nodeType = 3; this.parentNode = null; this.data = t; }
  get textContent() { return this.data; }
}
class Element extends Node {
  constructor(tag) {
    super();
    this.nodeType = 1;
    this.tagName = tag.toUpperCase();
    this.attrs = new Map();
    this.childNodes = [];
    this.parentNode = null;
    this.listeners = {};
    this.dirty = false;
    this._value = "";
    this.sel = null;
    this.scrollTop = 0;
    this.style = {};
  }
  get children() { return this.childNodes.filter((n) => n.nodeType === 1); }
  get firstChild() { return this.childNodes[0] || null; }
  get previousElementSibling() { const p = this.parentNode; if (!p) return null; const k = p.children; return k[k.indexOf(this) - 1] || null; }
  getBoundingClientRect() { return this.rect || { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 }; }
  prepend(n) { this.insertBefore(n, this.firstChild); }
  insertAdjacentHTML() {}
  set innerHTML(_) {}
  setPointerCapture() {}
  releasePointerCapture() {}
  blur() { if (document.activeElement === this) document.activeElement = document.body; }
  get nextSibling() { const p = this.parentNode; return p ? p.childNodes[p.childNodes.indexOf(this) + 1] || null : null; }
  get className() { return this.attrs.get("class") || ""; }
  set className(v) { this.attrs.set("class", v); }
  get classList() {
    const get = () => this.className.split(/\s+/).filter(Boolean);
    const set = (l) => (this.className = l.join(" "));
    return {
      contains: (c) => get().includes(c),
      add: (c) => { if (!get().includes(c)) set([...get(), c]); },
      remove: (c) => set(get().filter((x) => x !== c)),
      toggle: (c, on) => { const want = on === undefined ? !get().includes(c) : !!on; want ? this.classList.add(c) : this.classList.remove(c); return want; },
    };
  }
  get dataset() {
    const d = {};
    for (const [k, v] of this.attrs) if (k.startsWith("data-")) d[k.slice(5).replace(/-(\w)/g, (_, c) => c.toUpperCase())] = v;
    return d;
  }
  get id() { return this.attrs.get("id") || ""; }
  get href() { return this.attrs.get("href") || ""; }
  setAttribute(k, v) { this.attrs.set(k, String(v)); }
  getAttribute(k) { return this.attrs.has(k) ? this.attrs.get(k) : null; }
  hasAttribute(k) { return this.attrs.has(k); }
  removeAttribute(k) { this.attrs.delete(k); }
  get disabled() { return this.attrs.has("disabled"); }
  set disabled(v) { v ? this.attrs.set("disabled", "") : this.attrs.delete("disabled"); }
  get hidden() { return this.attrs.has("hidden"); }
  set hidden(v) { v ? this.attrs.set("hidden", "") : this.attrs.delete("hidden"); }
  get value() { return this.dirty ? this._value : (this.getAttribute("value") || ""); }
  set value(v) { this.dirty = true; this._value = String(v); this.sel = [this._value.length, this._value.length, "none"]; }
  get selectionStart() { return (this.sel || [0])[0]; }
  get selectionEnd() { return (this.sel || [0, 0])[1]; }
  get selectionDirection() { return (this.sel || [0, 0, "none"])[2]; }
  setSelectionRange(a, b, dir) {
    const n = this.value.length;
    const s = Math.min(a, n), e = Math.max(s, Math.min(b, n));
    this.sel = [s, e, dir || "none"];
  }
  get textContent() { return this.childNodes.map((n) => n.textContent).join(""); }
  set textContent(t) { this.replaceChildren(document.createTextNode(String(t))); }
  get isConnected() { let n = this; while (n.parentNode) n = n.parentNode; return n === document.documentElement; }
  contains(n) { for (; n; n = n.parentNode) if (n === this) return true; return false; }
  detach(n) {
    if (!n.parentNode) return;
    const p = n.parentNode;
    p.childNodes.splice(p.childNodes.indexOf(n), 1);
    n.parentNode = null;
    if (n.nodeType === 1 && n.contains(document.activeElement)) document.activeElement = document.body;
  }
  insertBefore(n, ref) {
    if (n.parentNode) n.parentNode.detach(n);
    const i = ref ? this.childNodes.indexOf(ref) : this.childNodes.length;
    this.childNodes.splice(i < 0 ? this.childNodes.length : i, 0, n);
    n.parentNode = this;
    return n;
  }
  append(...kids) { for (const k of kids) this.insertBefore(k instanceof Node ? k : document.createTextNode(String(k)), null); }
  replaceChildren(...kids) { for (const c of [...this.childNodes]) this.detach(c); this.append(...kids); }
  remove() { if (this.parentNode) this.parentNode.detach(this); }
  addEventListener(t, f) { (this.listeners[t] ||= []).push(f); }
  dispatch(type, extra) {
    const ev = Object.assign({ type, target: this, defaultPrevented: false, preventDefault() { this.defaultPrevented = true; } }, extra);
    for (let n = this; n; n = n.parentNode) {
      for (const f of n.listeners[type] || []) f(Object.assign(ev, { currentTarget: n }));
      if (typeof n["on" + type] === "function") n["on" + type](Object.assign(ev, { currentTarget: n }));
    }
    return ev;
  }
  focus() { if (this.isConnected && !this.disabled) document.activeElement = this; }
  click() { if (!this.disabled) this.dispatch("click", { button: 0 }); }
  matches(sel) { return sel.split(",").some((s) => matchChain(this, s.trim().split(/\s+/))); }
  closest(sel) { for (let n = this; n && n.nodeType === 1; n = n.parentNode) if (n.matches(sel)) return n; return null; }
  querySelectorAll(sel) {
    const out = [];
    const walk = (n) => { for (const c of n.children) { if (c.matches(sel)) out.push(c); walk(c); } };
    walk(this);
    return out;
  }
  querySelector(sel) { return this.querySelectorAll(sel)[0] || null; }
}

function matchOne(el, simple) {
  const re = /([#.]?[\w-]+)|\[([\w-]+)(?:="([^"]*)")?\]/g;
  let m;
  while ((m = re.exec(simple))) {
    if (m[1]) {
      if (m[1][0] === ".") { if (!el.classList.contains(m[1].slice(1))) return false; }
      else if (m[1][0] === "#") { if (el.id !== m[1].slice(1)) return false; }
      else if (el.tagName !== m[1].toUpperCase()) return false;
    } else if (m[3] === undefined ? !el.hasAttribute(m[2]) : el.getAttribute(m[2]) !== m[3]) return false;
  }
  return true;
}

function matchChain(el, parts) {
  if (!matchOne(el, parts[parts.length - 1])) return false;
  if (parts.length === 1) return true;
  for (let p = el.parentNode; p && p.nodeType === 1; p = p.parentNode) if (matchChain(p, parts.slice(0, -1))) return true;
  return false;
}

const document = {
  documentElement: new Element("html"),
  hidden: false,
  title: "",
  listeners: {},
  createElement: (t) => new Element(t),
  createElementNS: (_, t) => new Element(t),
  createTextNode: (t) => new Text(t),
  addEventListener(t, f) { (this.listeners[t] ||= []).push(f); },
  getElementById(id) {
    let el = this.body.querySelector("#" + id);
    if (!el) {
      el = new Element(/filter|question/.test(id) ? "input" : id === "issues" ? "textarea" : "div");
      el.setAttribute("id", id);
      this.body.append(el);
    }
    return el;
  },
  querySelector(sel) { return this.documentElement.querySelector(sel); },
  querySelectorAll(sel) { return this.documentElement.querySelectorAll(sel); },
};
document.body = new Element("body");
document.documentElement.append(document.body);
document.activeElement = document.body;

const storage = new Map(Object.entries(globalThis.__storage || {}));
const localStorage = {
  getItem: (k) => storage.has(k) ? storage.get(k) : null,
  setItem: (k, v) => { storage.set(k, String(v)); },
  removeItem: (k) => { storage.delete(k); },
};

const window = globalThis;
window.addEventListener = () => {};
const location = globalThis.__location || { hostname: "127.0.0.1", hash: "", pathname: "/", origin: "http://127.0.0.1:47731", reload() {} };
const AbortSignal = globalThis.AbortSignal || { timeout: () => ({}) };
const console = { error: (e) => { throw e; }, log: () => {}, warn: () => {} };
const CSS = { escape: (s) => String(s).replace(/["\\]/g, "\\$&") };
const intervals = [];
const setInterval = (f) => intervals.push(f);
const requestAnimationFrame = (f) => setTimeout(f, 16);
let fakeNow = 0;
let timerSeq = 0;
const timers = [];
const setTimeout = (f, ms) => { timers.push({ f, at: fakeNow + (ms || 0), id: ++timerSeq }); return timerSeq; };
const clearTimeout = (id) => { const i = timers.findIndex((t) => t.id === id); if (i >= 0) timers.splice(i, 1); };
function advance(ms) {
  const end = fakeNow + ms;
  for (;;) {
    timers.sort((a, b) => a.at - b.at || a.id - b.id);
    if (!timers.length || timers[0].at > end) break;
    const t = timers.shift();
    fakeNow = t.at;
    t.f();
  }
  fakeNow = end;
}
const fetches = [];
let fetchReply = globalThis.__fetchReply || null;
const fetch = (url, opts) => { fetches.push({ url, method: (opts && opts.method) || "GET", body: opts && opts.body }); return fetchReply ? fetchReply(url, opts) : new Promise(() => {}); };
let eventSource = null;
const eventSources = [];
class EventSource {
  constructor(url) { this.url = url; this.listeners = {}; this.closed = false; eventSource = this; eventSources.push(this); }
  addEventListener(t, f) { (this.listeners[t] ||= []).push(f); }
  close() { this.closed = true; }
}

function emitState(s) {
  for (const f of eventSource.listeners.state || []) f({ data: JSON.stringify(s) });
}

function typeKeys(text) {
  for (const ch of text) {
    const el = document.activeElement;
    if (el.tagName !== "INPUT") continue;
    const v = el.value, s = el.selectionStart, e = el.selectionEnd;
    el.value = v.slice(0, s) + ch + v.slice(e);
    el.setSelectionRange(s + 1, s + 1);
    el.dispatch("input");
  }
}

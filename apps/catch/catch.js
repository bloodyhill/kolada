// Catcher for Kolada's glue: the core (catch.wasm, crates/kolada-catch) does the
// reading and the protocol; this file only moves bytes between it and the
// browser: the page, pictures, sockets and requests, and the keys it keeps.

const api = globalThis.browser ?? globalThis.chrome;
const enc = new TextEncoder();
const dec = new TextDecoder();

// Frames, as kolada-sync writes them: a u16 header length, `key=value`
// lines, then the body.
function frame(fields, body = new Uint8Array()) {
  const head = enc.encode(Object.entries(fields).map(([k, v]) => `${k}=${String(v).replace(/[\r\n]/g, ' ')}`).join('\n'));
  const out = new Uint8Array(2 + head.length + body.length);
  out[0] = head.length >> 8;
  out[1] = head.length & 255;
  out.set(head, 2);
  out.set(body, 2 + head.length);
  return out;
}

function parse(bytes) {
  const n = (bytes[0] << 8) | bytes[1];
  const f = { body: bytes.slice(2 + n), h: {} };
  for (const line of dec.decode(bytes.slice(2, 2 + n)).split('\n')) {
    const at = line.indexOf('=');
    if (at > 0) f.h[line.slice(0, at)] = line.slice(at + 1);
  }
  f.t = f.h.t;
  return f;
}

function unpack(bytes) {
  const out = [];
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let at = 0; at + 4 <= bytes.length;) {
    const n = view.getUint32(at);
    out.push(parse(bytes.slice(at + 4, at + 4 + n)));
    at += 4 + n;
  }
  return out;
}

const concat = (...parts) => {
  const out = new Uint8Array(parts.reduce((s, p) => s + p.length, 0));
  let at = 0;
  for (const p of parts) { out.set(p, at); at += p.length; }
  return out;
};

const b64url = bytes => btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
const fromB64 = s => Uint8Array.from(atob(s), c => c.charCodeAt(0));
const toB64 = bytes => { let s = ''; for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000)); return btoa(s); };

/** What this browser calls itself, for the other devices' lists. */
export function browserName() {
  const ua = navigator.userAgent;
  if (/Edg\//.test(ua)) return 'Edge';
  if (/Firefox\//.test(ua)) return 'Firefox';
  if (/OPR\//.test(ua)) return 'Opera';
  if (/Chrome\//.test(ua)) return 'Chrome';
  return 'Browser';
}

let module;

/** The core, opened from the keys kept (paired or not). */
export class Core {
  static async open() {
    module ??= await WebAssembly.compileStreaming(fetch(api.runtime.getURL('catch.wasm')));
    const core = new Core();
    core.inst = await WebAssembly.instantiate(module, { kolada: { random: (p, n) => crypto.getRandomValues(new Uint8Array(core.inst.exports.memory.buffer, p, n)) } });
    const kept = await api.storage.local.get(['keys', 'base', 'devices']);
    core.base = kept.base;
    // The other devices as last known: the engine learns them only on the
    // group's socket, which a catch never opens.
    core.known = kept.devices || 0;
    core.call({ t: 'open', name: browserName(), ...(kept.base ? { base: kept.base } : {}) }, kept.keys ? fromB64(kept.keys) : undefined);
    return core;
  }

  /** One request; what it says back, the keys kept as they change. */
  call(fields, body) {
    const req = frame({ ...fields, now: Date.now() }, body);
    const x = this.inst.exports;
    const p = x.alloc(req.length);
    new Uint8Array(x.memory.buffer, p, req.length).set(req);
    const o = x.call(p, req.length);
    const n = x.out_len();
    const out = unpack(new Uint8Array(x.memory.buffer, o, n).slice());
    x.free(p, req.length);
    x.free(o, n);
    for (const f of out) {
      if (f.t === 'keys') api.storage.local.set({ keys: toB64(f.body) });
      if (f.t === 'state') {
        const devices = Number(f.h.devices) || 0;
        if (devices > 0 && devices !== this.known) {
          this.known = devices;
          api.storage.local.set({ devices });
        }
        this.state = { paired: f.h.paired === '1', devices: devices || (f.h.paired === '1' ? this.known : 0), wake: Number(f.h.wake) || 0 };
      }
    }
    this.onFrames?.(out);
    return out;
  }
}

/**
 * The sockets the core asks for (pairing; the group, once, so the other
 * devices know this one), driven until `done` says so. A browser's socket
 * carries no headers: the group's signed ones ride in its subprotocol.
 */
export function drive(core, done) {
  const sockets = {};
  let timer;
  const handle = frames => {
    for (const f of frames) {
      if (f.t === 'connect' && !sockets[f.h.ch]) open(f);
      if (f.t === 'out') sockets[f.h.ch]?.readyState === 1 && sockets[f.h.ch].send(f.body);
    }
    clearTimeout(timer);
    const wake = core.state?.wake;
    if (wake) timer = setTimeout(() => core.call({ t: 'tick' }), Math.max(50, wake - Date.now()));
    if (done(frames)) stop();
  };
  const open = f => {
    const headers = Object.entries(f.h).filter(([k]) => k.startsWith('h:')).map(([k, v]) => `${k.slice(2)}=${v}`);
    const ws = headers.length ? new WebSocket(f.h.url, [`kolada.${b64url(enc.encode(headers.join('\n')))}`]) : new WebSocket(f.h.url);
    ws.binaryType = 'arraybuffer';
    sockets[f.h.ch] = ws;
    ws.onopen = () => core.call({ t: 'opened', ch: f.h.ch });
    ws.onmessage = e => typeof e.data !== 'string' && core.call({ t: 'frame', ch: f.h.ch }, new Uint8Array(e.data));
    ws.onclose = () => { if (sockets[f.h.ch] === ws) { delete sockets[f.h.ch]; core.call({ t: 'closed', ch: f.h.ch }); } };
  };
  const stop = () => {
    clearTimeout(timer);
    core.onFrames = null;
    for (const ws of Object.values(sockets)) ws.close();
  };
  core.onFrames = handle;
  return stop;
}

/** The page in `tabId` as the extension reads it (activeTab). */
export async function readTab(tabId) {
  const [r] = await api.scripting.executeScript({
    target: { tabId },
    func: () => {
      const sel = getSelection();
      let selHtml = '';
      let selText = '';
      if (sel && sel.rangeCount && !sel.isCollapsed) {
        const div = document.createElement('div');
        for (let i = 0; i < sel.rangeCount; i++) div.appendChild(sel.getRangeAt(i).cloneContents());
        selHtml = div.innerHTML;
        selText = sel.toString();
      }
      return { url: location.href, title: document.title, html: document.documentElement.outerHTML, selHtml, selText };
    },
  });
  return r.result;
}

/** The page read as a note, as the core reads it, for `mode`. */
export function extract(core, page, mode, link) {
  const url = link?.url ?? page.url;
  const html = mode === 'selection' ? page.selHtml : mode === 'page' ? page.html : '';
  const out = core.call({ t: 'extract', mode, url, title: link?.title ?? page.title }, concat(enc.encode(html), new Uint8Array([0]), enc.encode(mode === 'selection' ? page.selText : '')));
  const clipped = out.find(f => f.t === 'clipped');
  return {
    url,
    title: clipped.h.title,
    markdown: dec.decode(clipped.body),
    fetch: out.filter(f => f.t === 'fetch').map(f => ({ name: f.h.name, url: f.h.url })),
    inside: out.filter(f => f.t === 'inside').map(f => ({ name: f.h.name, bytes: f.body })),
  };
}

/** Whether pictures may be read from `url`'s site (the page's own, or once allowed, any). */
async function mayFetch(url, pageUrl) {
  try {
    const origin = new URL(url).origin;
    if (origin === new URL(pageUrl).origin) return true;
    return await api.permissions.contains({ origins: [`${origin}/*`] });
  } catch {
    return false;
  }
}

/** A picture's size and a small JPEG of it (at most 96 KB, as Beam carries). */
async function look(bytes, mime) {
  try {
    const bitmap = await createImageBitmap(new Blob([bytes], { type: mime }));
    const { width, height } = bitmap;
    for (const edge of [320, 200, 120]) {
      const s = Math.min(1, edge / Math.max(width, height));
      const c = new OffscreenCanvas(Math.max(1, Math.round(width * s)), Math.max(1, Math.round(height * s)));
      c.getContext('2d').drawImage(bitmap, 0, 0, c.width, c.height);
      const thumb = new Uint8Array(await (await c.convertToBlob({ type: 'image/jpeg', quality: 0.72 })).arrayBuffer());
      if (thumb.length <= 96 * 1024) return { width, height, thumb };
    }
    return { width, height, thumb: new Uint8Array() };
  } catch {
    return null;
  }
}

const MIMES = { jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml', bmp: 'image/bmp' };
export const mimeOf = name => MIMES[name.split('.').pop().toLowerCase()] ?? 'image/png';

/** A request the core made, as the browser sends it: its status and body. */
async function send(f, method = 'POST') {
  const headers = {};
  for (const [k, v] of Object.entries(f.h)) if (k.startsWith('h:')) headers[k.slice(2)] = v;
  let res;
  try {
    res = await fetch(f.h.url, { method, headers, body: f.body, credentials: 'omit' });
  } catch {
    // Offline, or the server out of reach: said plainly, never the browser's words.
    throw new Error("Kolada can't be reached. Try again.");
  }
  return { status: res.status, body: new Uint8Array(await res.arrayBuffer()) };
}

/**
 * Catches what `clip` (from `extract`) holds, with the user's `line`: its
 * pictures that may be read and fit Beam ride with it, the rest stay
 * links; then it goes, one request for the note and three a picture.
 * Resolves with the note's path in Caught, or throws a short reason.
 */
export async function catchClip(core, clip, line, pageUrl) {
  if (!core.state?.paired) throw new Error('Pair with Kolada first.');
  const have = [];
  for (const p of clip.inside) have.push({ name: p.name, bytes: p.bytes, mime: mimeOf(p.name) });
  const links = [];
  for (const p of clip.fetch) {
    if (have.length < 16 && (await mayFetch(p.url, pageUrl))) {
      try {
        const res = await fetch(p.url, { credentials: 'omit' });
        if (res.ok) { have.push({ name: p.name, bytes: new Uint8Array(await res.arrayBuffer()), mime: res.headers.get('content-type')?.split(';')[0] || mimeOf(p.name), url: p.url }); continue; }
      } catch { /* kept as its link */ }
    }
    links.push(p);
  }
  for (const p of have) p.look = await look(p.bytes, p.mime);
  const usable = have.filter(p => p.look);
  const riding = core.call({ t: 'riding', names: usable.map(p => `${p.name}:${p.bytes.length}`).join(',') }).find(f => f.t === 'riding').h.names.split(',').filter(Boolean);
  for (const p of have) if (!riding.includes(p.name) && p.url) links.push(p);
  const now = new Date();
  const caught = new Date(now - now.getTimezoneOffset() * 60000).toISOString().slice(0, 16);
  const composed = core.call({ t: 'compose', title: clip.title, url: clip.url, line, caught, rides: riding.join(','), links: links.map(p => `${p.name}>${p.url}`).join(',') }, enc.encode(clip.markdown)).find(f => f.t === 'composed');
  const renames = Object.fromEntries((composed.h.renames || '').split(',').filter(Boolean).map(r => r.split('>')));
  const sent = core.call({ t: 'send', title: clip.title, url: clip.url }, composed.body).find(f => f.t === 'post' || f.t === 'error');
  if (sent.t === 'error') throw new Error(sent.h.why === 'size' ? 'That page is too long to catch whole.' : 'Pair with Kolada first.');
  const posted = await send(sent);
  if (posted.status !== 200) throw new Error(posted.status === 429 ? 'Kolada is resting until tomorrow.' : 'Not sent. Try again.');
  for (const name of riding) {
    const p = have.find(h => h.name === name);
    const ask = core.call({ t: 'picture', note: sent.h.id, name: renames[name] ?? name, mime: p.mime, width: p.look.width, height: p.look.height, thumb: p.look.thumb.length }, concat(p.look.thumb, p.bytes)).find(f => f.t === 'post');
    if (!ask) continue;
    const ticket = await send(ask);
    if (ticket.status !== 200) continue;
    for (const put of core.call({ t: 'answer' }, ticket.body).filter(f => f.t === 'put')) {
      const up = await send(put, 'PUT');
      const item = core.call({ t: 'put_done', blob: put.h.blob, status: up.status }).find(f => f.t === 'post');
      if (item) await send(item);
    }
  }
  return `Caught / ${clip.title}`;
}

export { api };

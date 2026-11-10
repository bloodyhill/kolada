// Catcher for Kolada end to end, the browser's half: the extension's own glue
// (catch.js) and core (catch.wasm) under Node, with the few browser APIs it
// needs stood in for (storage, permissions, image decoding), against a
// local server. The other half is kolada-sync's e2e test
// `a_browser_catches_a_page_into_this_pcs_caught_folder`, which shows the
// code, allows this device and saves the clip.
//
//   node apps/catch/test/e2e.mjs http://127.0.0.1:8788 <KOLADA_CATCH_DIR>

import { readFile } from 'node:fs/promises';
import { existsSync, readFileSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { deflateSync } from 'node:zlib';

const [base, dir] = process.argv.slice(2);
if (!base || !dir) throw new Error('usage: e2e.mjs <server> <dir>');

// The browser, as far as catch.js needs it.
const kept = { base };
globalThis.chrome = {
  runtime: { getURL: p => new URL(`../${p}`, import.meta.url).href },
  storage: { local: { get: async keys => Object.fromEntries([].concat(keys).filter(k => k in kept).map(k => [k, kept[k]])), set: async o => Object.assign(kept, o) } },
  permissions: { contains: async () => true },
};
// A browser's socket from an extension's page says so (its Origin), as
// the server needs before it reads headers from the subprotocol.
const NodeSocket = globalThis.WebSocket;
globalThis.WebSocket = class extends NodeSocket {
  constructor(url, protocols) {
    super(url, { protocols: protocols ? [].concat(protocols) : undefined, headers: { origin: 'chrome-extension://abcdefghijklmnopabcdefghijklmnop' } });
  }
};
const realFetch = globalThis.fetch;
globalThis.fetch = async (url, init) => {
  const u = String(url);
  if (u.startsWith('file:')) return new Response(await readFile(new URL(u)), { headers: { 'content-type': u.endsWith('.wasm') ? 'application/wasm' : 'application/octet-stream' } });
  return realFetch(url, init);
};
globalThis.createImageBitmap = async blob => {
  const b = new Uint8Array(await blob.arrayBuffer());
  const v = new DataView(b.buffer);
  return { width: v.getUint32(16), height: v.getUint32(20), close() {} };
};
globalThis.OffscreenCanvas = class {
  constructor(w, h) { this.width = w; this.height = h; }
  getContext() { return { drawImage() {} }; }
  async convertToBlob() { return new Blob([new Uint8Array([0xff, 0xd8, 0xff, 0xd9])], { type: 'image/jpeg' }); }
};

const { Core, catchClip, drive, extract } = await import('../catch.js');

// A generated page and its picture (a real PNG, 64 by 48, one colour).
function png(w, h) {
  const crc = buf => { let c = ~0; for (const x of buf) { c ^= x; for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xEDB88320 & -(c & 1)); } return ~c >>> 0; };
  const chunk = (type, data) => { const t = Buffer.from(type); const out = Buffer.alloc(12 + data.length); out.writeUInt32BE(data.length); t.copy(out, 4); data.copy(out, 8); out.writeUInt32BE(crc(Buffer.concat([t, data])), 8 + data.length); return out; };
  const ihdr = Buffer.alloc(13); ihdr.writeUInt32BE(w); ihdr.writeUInt32BE(h, 4); ihdr[8] = 8; ihdr[9] = 2;
  const rows = Buffer.concat(Array.from({ length: h }, () => Buffer.concat([Buffer.from([0]), Buffer.alloc(w * 3, 0x55)])));
  return Buffer.concat([Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk('IHDR', ihdr), chunk('IDAT', deflateSync(rows)), chunk('IEND', Buffer.alloc(0))]);
}
const picture = png(64, 48);
const html = '<html><head><title>Walking | Test</title></head><body><nav><a href="/">Home</a></nav><article><h1>The quiet power of walking</h1><p>An hour a day on foot changes how we think.</p><img src="/walk.png" alt="A path"></article><footer>Footer</footer></body></html>';
const server = createServer((req, res) => {
  if (req.url === '/walk.png') { res.writeHead(200, { 'content-type': 'image/png' }); res.end(picture); } else { res.writeHead(200, { 'content-type': 'text/html' }); res.end(html); }
});
await new Promise(r => server.listen(0, '127.0.0.1', r));
const site = `http://127.0.0.1:${server.address().port}`;

const wait = async (what, ok, secs = 60) => {
  const start = Date.now();
  while (!ok()) {
    if (Date.now() - start > secs * 1000) throw new Error(`timed out waiting for ${what}`);
    await new Promise(r => setTimeout(r, 100));
  }
};

const core = await Core.open();
await wait('the code', () => existsSync(`${dir}/code`), 900);
const code = readFileSync(`${dir}/code`, 'utf8').trim();
let paired = false;
let refused = null;
const stop = drive(core, frames => {
  for (const f of frames) {
    if (f.t === 'event' && f.h.what === 'paired') paired = true;
    if (f.t === 'event' && f.h.what === 'refused') refused = f.h.why;
  }
  return false;
});
core.call({ t: 'pair', code });
await wait('pairing', () => refused || (paired && core.state?.devices > 0));
if (refused) throw new Error(`refused: ${refused}`);
console.log(`paired; ${core.state.devices} device(s) in the group`);
// A moment for the group to hear this device's name, then the sockets go:
// the extension catches by requests alone.
await new Promise(r => setTimeout(r, 1500));
stop();
// What the extension keeps (its keys, the server), for a look at the popup
// as a paired one: a throwaway pairing with a local server.
if (process.env.CATCH_KEEP) writeFileSync(`${dir}/storage.json`, JSON.stringify(kept));

const page = { url: `${site}/walking`, title: 'Walking | Test', html, selHtml: '', selText: '' };
const clip = extract(core, page, 'page');
console.log(`clip: "${clip.title}", ${clip.markdown.length} chars, ${clip.fetch.length} picture to fetch`);
const where = await catchClip(core, clip, 'For Thursday', page.url);
console.log(`sent: ${where}`);
await wait('the PC to save it', () => existsSync(`${dir}/done`), 120);
console.log('the PC saved it in Caught');
server.close();
process.exit(0);

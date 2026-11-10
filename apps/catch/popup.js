// The popup: pair once with a code, then catch the page, what is selected,
// or the link, with a line of your own.

import { api, catchClip, Core, drive, extract, mimeOf, readTab } from './catch.js';

// The apps' own filled icons (Fluent UI System Icons, MIT; THIRD-PARTY.md).
const ICONS = {
  page: 'M12 8V2H6C4.89543 2 4 2.89543 4 4V20C4 21.1046 4.89543 22 6 22H18C19.1046 22 20 21.1046 20 20V10H14C12.8954 10 12 9.10457 12 8ZM8.25 11.5H15.75C16.1642 11.5 16.5 11.8358 16.5 12.25C16.5 12.6642 16.1642 13 15.75 13H8.25C7.83579 13 7.5 12.6642 7.5 12.25C7.5 11.8358 7.83579 11.5 8.25 11.5ZM8.25 14.25H15.75C16.1642 14.25 16.5 14.5858 16.5 15C16.5 15.4142 16.1642 15.75 15.75 15.75H8.25C7.83579 15.75 7.5 15.4142 7.5 15C7.5 14.5858 7.83579 14.25 8.25 14.25ZM8.25 17H15.75C16.1642 17 16.5 17.3358 16.5 17.75C16.5 18.1642 16.1642 18.5 15.75 18.5H8.25C7.83579 18.5 7.5 18.1642 7.5 17.75C7.5 17.3358 7.83579 17 8.25 17ZM13.5 8V2.5L19.5 8.5H14C13.7239 8.5 13.5 8.27614 13.5 8Z',
  selection: 'M5.25 2C4.00736 2 3 3.00736 3 4.25V7.25C3 8.49264 4.00736 9.5 5.25 9.5H18.75C19.9926 9.5 21 8.49264 21 7.25V4.25C21 3.00736 19.9926 2 18.75 2H5.25ZM5 11.75V11H19V11.75C19 12.9926 17.9926 14 16.75 14H7.25C6.00736 14 5 12.9926 5 11.75ZM7.50294 15.5H16.5013L16.5017 16.7881C16.5017 17.6031 16.0616 18.3494 15.36 18.7463L15.2057 18.8259L8.57101 21.9321C8.10478 22.1504 7.57405 21.8451 7.50953 21.3536L7.503 21.2529L7.50294 15.5Z',
  link: 'M9 7C9.55228 7 10 7.44772 10 8C10 8.51284 9.61396 8.93551 9.11662 8.99327L9 9H7C5.34315 9 4 10.3431 4 12C4 13.5977 5.24892 14.9037 6.82373 14.9949L7 15H9C9.55228 15 10 15.4477 10 16C10 16.5128 9.61396 16.9355 9.11662 16.9933L9 17H7C4.23858 17 2 14.7614 2 12C2 9.31125 4.12231 7.11818 6.78311 7.00462L7 7H9ZM17 7C19.7614 7 22 9.23858 22 12C22 14.6888 19.8777 16.8818 17.2169 16.9954L17 17H15C14.4477 17 14 16.5523 14 16C14 15.4872 14.386 15.0645 14.8834 15.0067L15 15H17C18.6569 15 20 13.6569 20 12C20 10.4023 18.7511 9.09634 17.1763 9.00509L17 9H15C14.4477 9 14 8.55228 14 8C14 7.48716 14.386 7.06449 14.8834 7.00673L15 7H17ZM7 11H17C17.5523 11 18 11.4477 18 12C18 12.5128 17.614 12.9355 17.1166 12.9933L17 13H7C6.44772 13 6 12.5523 6 12C6 11.4872 6.38604 11.0645 6.88338 11.0067L7 11H17H7Z',
  folder: 'M2 8V6.25C2 4.45507 3.45507 3 5.25 3H8.12868C8.72542 3 9.29771 3.23705 9.71967 3.65901L11.25 5.18934L8.65901 7.78033C8.51836 7.92098 8.32759 8 8.12868 8H2ZM2 9.5V17.75C2 19.5449 3.45507 21 5.25 21H18.75C20.5449 21 22 19.5449 22 17.75V8.75C22 6.95507 20.5449 5.5 18.75 5.5H13.0607L9.71967 8.84099C9.29771 9.26295 8.72542 9.5 8.12868 9.5H2Z',
  check: 'M8.5 16.5858L4.70711 12.7929C4.31658 12.4024 3.68342 12.4024 3.29289 12.7929C2.90237 13.1834 2.90237 13.8166 3.29289 14.2071L7.79289 18.7071C8.18342 19.0976 8.81658 19.0976 9.20711 18.7071L20.2071 7.70711C20.5976 7.31658 20.5976 6.68342 20.2071 6.29289C19.8166 5.90237 19.1834 5.90237 18.7929 6.29289L8.5 16.5858Z',
};
const icon = (name, el = document.createElementNS('http://www.w3.org/2000/svg', 'svg')) => {
  el.setAttribute('viewBox', '0 0 24 24');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', ICONS[name]);
  el.replaceChildren(path);
  return el;
};
for (const el of document.querySelectorAll('svg[data-icon]')) icon(el.dataset.icon, el);

const $ = id => document.getElementById(id);
const WORDS = { page: 'Catch this page', selection: 'Catch selection', link: 'Catch link' };
const REFUSED = {
  NoSuchCode: 'That code isn\'t showing anywhere.', Expired: 'That code ran out. Show a new one.', Burned: 'That code was tried too often. Show a new one.',
  Country: 'Both devices need to be in the same country.', Busy: 'Someone else is trying that code.', Slow: 'Too many tries. Wait a minute.',
  Quota: 'Pairing is resting until tomorrow.', WrongCode: 'That code didn\'t match.', Denied: 'Your device said no.', Full: 'That group is full.', Network: 'Kolada can\'t be reached.',
};

const core = await Core.open();
if (core.state?.paired) ready(); else pair();

// Pairing: the code Kolada shows under Devices, typed into eight cells.
function pair() {
  $('pair').hidden = false;
  const input = $('code');
  const cells = $('cells');
  const show = () => {
    const typed = input.value.toUpperCase().replace(/[^0-9A-Z]/g, '').slice(0, 8);
    cells.replaceChildren();
    for (let i = 0; i < 8; i++) {
      if (i === 4) cells.append(document.createElement('i'));
      const s = document.createElement('span');
      s.textContent = typed[i] ?? '';
      if (i === typed.length) s.className = 'cur';
      cells.append(s);
    }
    $('go-pair').disabled = typed.length !== 8;
  };
  cells.onclick = () => input.focus();
  input.oninput = show;
  input.onkeydown = e => e.key === 'Enter' && !$('go-pair').disabled && $('go-pair').click();
  show();
  input.focus();
  $('go-pair').onclick = () => {
    const said = $('pair-said');
    said.className = 'said';
    said.textContent = 'Waiting for your device';
    $('go-pair').disabled = true;
    let paired = false;
    drive(core, frames => {
      for (const f of frames) {
        if (f.t === 'event' && f.h.what === 'refused') {
          said.className = 'said error';
          said.textContent = REFUSED[f.h.why] ?? 'That didn\'t pair. Try again.';
          $('go-pair').disabled = false;
          return true;
        }
        if (f.t === 'event' && f.h.what === 'paired') paired = true;
      }
      // Paired, and known to the group (its socket is open and has the list).
      if (paired && core.state?.devices > 0) {
        setTimeout(() => location.reload(), 600);
        return true;
      }
      return false;
    });
    core.call({ t: 'pair', code: input.value });
  };
}

async function ready() {
  $('ready').hidden = false;
  $('devices').textContent = core.state.devices === 1 ? '1 device' : `${core.state.devices} devices`;
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  $('title').textContent = tab?.title || tab?.url || '';
  try { $('site').textContent = new URL(tab.url).hostname.replace(/^www\./, ''); } catch { /* none */ }
  $('fav').append(tab?.favIconUrl?.startsWith('http') ? Object.assign(document.createElement('img'), { src: tab.favIconUrl, alt: '' }) : icon('page'));
  let { askedPictures } = await api.storage.local.get('askedPictures');
  let page;
  try { page = await readTab(tab.id); } catch { page = { url: tab.url, title: tab.title, html: '', selHtml: '', selText: '' }; }
  const tabs = [...document.querySelectorAll('.seg button')];
  const readable = !!page.html;
  tabs.find(b => b.dataset.mode === 'page').disabled = !readable;
  tabs.find(b => b.dataset.mode === 'selection').disabled = !page.selText;
  let mode = page.selText ? 'selection' : readable ? 'page' : 'link';
  let clip;
  const choose = m => {
    mode = m;
    for (const b of tabs) b.setAttribute('aria-selected', String(b.dataset.mode === m));
    $('go-word').textContent = WORDS[m];
    clip = extract(core, page, m);
    preview(clip, m);
  };
  for (const b of tabs) b.onclick = () => !b.disabled && choose(b.dataset.mode);
  choose(mode);

  const go = async () => {
    if ($('go').disabled) return;
    // Pictures from other sites: asked once, the first time a page has
    // some (in the click itself, as the browser needs); declined, they
    // stay links.
    const outside = clip.fetch.some(p => { try { return new URL(p.url).origin !== new URL(page.url).origin; } catch { return false; } });
    if (outside && !askedPictures) {
      askedPictures = true;
      const asking = api.permissions.request({ origins: ['https://*/*', 'http://*/*'] });
      api.storage.local.set({ askedPictures: true });
      try { await asking; } catch { /* declined */ }
    }
    $('go').disabled = true;
    try {
      const where = await catchClip(core, clip, $('line').value, page.url);
      $('go').className = 'go done';
      $('go').replaceChildren(icon('check'), Object.assign(document.createElement('span'), { textContent: 'Caught' }));
      $('into').textContent = where;
      $('devices').textContent = '';
      setTimeout(() => window.close(), 1200);
    } catch (e) {
      $('go').disabled = false;
      $('into').className = 'error';
      $('into').textContent = e.message || 'Not sent. Try again.';
    }
  };
  $('go').onclick = go;
  $('line').onkeydown = e => e.key === 'Enter' && go();
  // The shortcut again, with the popup open: catch at once.
  document.addEventListener('keydown', e => { if (e.altKey && e.shiftKey && e.code === 'KeyC') { e.preventDefault(); go(); } });
}

/** The clip's first lines, plain, and its pictures. */
function preview(clip, mode) {
  const prev = $('prev');
  prev.replaceChildren();
  const lines = clip.markdown.split('\n').map(l => l.replace(/!\[\[[^\]]*\]\]|!\[[^\]]*\]\([^)]*\)/g, '').replace(/^#+\s*|[*_`>]/g, '').replace(/\[([^\]]*)\]\([^)]*\)/g, '$1').trim()).filter(Boolean);
  if (mode === 'selection') {
    prev.append(Object.assign(document.createElement('div'), { className: 'quote', textContent: lines.join(' ').slice(0, 280) }));
  } else {
    prev.append(Object.assign(document.createElement('b'), { textContent: clip.title }));
    const first = lines[0] === clip.title ? lines.slice(1) : lines;
    prev.append(Object.assign(document.createElement('p'), { textContent: first.join(' ').slice(0, 220) }));
  }
  const pics = [...clip.inside.map(p => URL.createObjectURL(new Blob([p.bytes], { type: mimeOf(p.name) }))), ...clip.fetch.map(p => p.url)];
  if (pics.length) {
    const row = document.createElement('div');
    row.className = 'pics';
    for (const src of pics.slice(0, 2)) row.append(Object.assign(document.createElement('div'), { style: `background-image:url("${src.replace(/"/g, '%22')}")` }));
    if (pics.length > 2) row.append(Object.assign(document.createElement('div'), { textContent: `+${pics.length - 2}` }));
    prev.append(row);
  }
  prev.append(Object.assign(document.createElement('div'), { className: 'fade' }));
}

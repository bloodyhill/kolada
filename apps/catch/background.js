// Catching without the popup: right-click a page, a selection or a link.
// The toolbar bird says how it went: a teal check, or red for an error.

import { api, catchClip, Core, extract, readTab } from './catch.js';

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.removeAll(() => {
    api.contextMenus.create({ id: 'page', title: 'Catch this page', contexts: ['page'] });
    api.contextMenus.create({ id: 'selection', title: 'Catch selection', contexts: ['selection'] });
    api.contextMenus.create({ id: 'link', title: 'Catch link', contexts: ['link'] });
  });
});

async function mark(tabId, ok) {
  await api.action.setBadgeBackgroundColor({ color: ok ? '#0E9594' : '#C4312B', tabId });
  await api.action.setBadgeText({ text: ok ? '✓' : '!', tabId });
  setTimeout(() => api.action.setBadgeText({ text: '', tabId }), 2000);
}

api.contextMenus.onClicked.addListener(async (info, tab) => {
  try {
    const core = await Core.open();
    if (!core.state?.paired) throw new Error('unpaired');
    let clip;
    let pageUrl = tab?.url ?? '';
    if (info.menuItemId === 'link') {
      clip = extract(core, { url: info.linkUrl, title: '', html: '', selHtml: '', selText: '' }, 'link', { url: info.linkUrl, title: info.selectionText || info.linkUrl });
      pageUrl = info.linkUrl;
    } else {
      const page = await readTab(tab.id);
      pageUrl = page.url;
      clip = extract(core, page, info.menuItemId === 'selection' && page.selText ? 'selection' : 'page');
    }
    await catchClip(core, clip, '', pageUrl);
    await mark(tab.id, true);
  } catch {
    if (tab?.id != null) await mark(tab.id, false);
  }
});

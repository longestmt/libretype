// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreType contributors
// Explicit, popup-initiated setup only. Never reads or edits document content.
(() => {
  let running = false;
  const text = element => element?.textContent?.trim() || '';
  const visible = element => element?.isConnected && element.getBoundingClientRect().width > 0
    && element.getBoundingClientRect().height > 0 && getComputedStyle(element).visibility !== 'hidden';
  const unique = (root, selector, matches) => {
    const found = [...root.querySelectorAll(selector)].filter(element => visible(element) && matches(element));
    return found.length === 1 ? found[0] : null;
  };
  const dialogs = () => [...document.querySelectorAll('[role="dialog"]')].filter(visible);
  const settings = () => unique(document, '[role="dialog"]', element => {
    const heading = document.getElementById(element.getAttribute('aria-labelledby'));
    return text(heading) === 'Accessibility settings';
  });
  const checkbox = (dialog, label) => unique(dialog, 'input[type="checkbox"]', element =>
    [...(element.labels || [])].some(item => text(item) === label));
  const readerLabel = 'Turn on screen reader support';
  const brailleLabel = 'Turn on braille support';

  async function setup() {
    const page = location.href;
    const deadline = Date.now() + 8000;
    async function guard() {
      if (Date.now() >= deadline || document.visibilityState !== 'visible' || location.href !== page)
        throw new Error('Setup interrupted');
      const reply = await chrome.runtime.sendMessage({type:'setupGuard'});
      if (!reply?.allowed || Date.now() >= deadline || document.visibilityState !== 'visible'
        || location.href !== page) throw new Error('Setup interrupted');
    }
    async function waitFor(read) {
      while (true) {
        await guard();
        const value = read();
        if (value) return value;
        await new Promise(resolve => setTimeout(resolve, 50));
      }
    }
    async function click(element) {
      await guard();
      if (!visible(element) || element.disabled || element.getAttribute('aria-disabled') === 'true')
        throw new Error('Control unavailable');
      // Menu controls may compare pointer positions when distinguishing a menu
      // opening click from a selection. Do not send every action at (0, 0).
      const rect=element.getBoundingClientRect();
      const clientX=rect.left+rect.width/2,clientY=rect.top+rect.height/2;
      if (!Number.isFinite(clientX) || !Number.isFinite(clientY) || clientX<0 || clientY<0
        || clientX>=window.innerWidth || clientY>=window.innerHeight)
        throw new Error('Control outside viewport');
      const options={bubbles:true,cancelable:true,composed:true,button:0,buttons:0,
        clientX,clientY,view:window,detail:1};
      for (const type of ['mouseover','mousemove','mousedown','mouseup','click'])
        element.dispatchEvent(new MouseEvent(type,{...options,buttons:type==='mousedown'?1:0}));
    }
    await guard();
    // Do not save a dialog the user may already have edited.
    if (dialogs().length) return {result:'review'};
    const tools = document.getElementById('docs-tools-menu');
    if (!visible(tools)) return {result:'unsupported'};
    await click(tools);
    const menuLabel = await waitFor(() => unique(document, '.goog-menuitem-label',
      element => text(element) === 'Accessibility'));
    const item = menuLabel.closest('[role="menuitem"]');
    if (!item) return {result:'unsupported'};
    await click(item);
    const dialog = await waitFor(settings);
    const reader = checkbox(dialog, readerLabel), braille = checkbox(dialog, brailleLabel);
    const ok = unique(dialog, 'button,[role="button"]', element => text(element) === 'OK');
    if (!reader || !braille || !ok || dialogs().length !== 1) return {result:'review'};
    const others = [...dialog.querySelectorAll('input[type="checkbox"]')]
      .filter(element => element !== reader && element !== braille)
      .map(element => ({element,checked:element.checked}));
    const unchanged = () => settings() === dialog && dialogs().length === 1
      && checkbox(dialog, readerLabel) === reader && checkbox(dialog, brailleLabel) === braille
      && dialog.querySelectorAll('input[type="checkbox"]').length === others.length + 2
      && others.every(item => item.element.isConnected && item.element.checked === item.checked);
    if (!reader.checked) await click(reader);
    await waitFor(() => reader.checked && !braille.disabled);
    if (!unchanged()) return {result:'review'};
    if (!braille.checked) await click(braille);
    await waitFor(() => reader.checked && braille.checked);
    if (!unchanged()) return {result:'review'};
    await click(ok);
    await waitFor(() => !visible(dialog));
    return {result:'saved'};
  }
  chrome.runtime.onMessage.addListener((message, sender, respond) => {
    if (sender.id !== chrome.runtime.id || sender.tab || message?.type !== 'setupAccessibility'
      || Object.keys(message).length !== 1) return false;
    if (running) { respond({result:'busy'}); return false; }
    running = true;
    setup().then(respond, () => respond({result:'review'})).finally(() => { running = false; });
    return true;
  });
})();

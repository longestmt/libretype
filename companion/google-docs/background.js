// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
const HOST = 'org.libretype.docs';
let port = null;
let pending = null;
let sourceTab = null;
let needsClear = false;
let focusRevision = 0;
let setupRun = null;
const unavailable = () => ({ enabled: false, accepted: false });
function disconnect() {
  const old = port; port = null;
  if (pending) { clearTimeout(pending.timer); pending.resolve(unavailable()); pending = null; }
  if (old) old.disconnect();
}
function native(request) {
  if (pending) return Promise.resolve(unavailable());
  return new Promise(resolve => {
    try {
      if (!port) {
        port = chrome.runtime.connectNative(HOST);
        const connected = port;
        port.onMessage.addListener(reply => {
          if (port !== connected || !pending) return;
          const item = pending; pending = null; clearTimeout(item.timer);
          item.resolve({ enabled: reply?.enabled === true, accepted: reply?.accepted === true });
          if (needsClear) { needsClear=false; native({type:'clear'}); }
        });
        port.onDisconnect.addListener(() => { void chrome.runtime.lastError; if (port === connected) disconnect(); });
      }
      pending = { resolve, timer: setTimeout(disconnect, 700) };
      port.postMessage(request);
    } catch { disconnect(); resolve(unavailable()); }
  });
}
function docsURL(value) {
  try { const u = new URL(value); return u.origin === 'https://docs.google.com'
    && /^\/document\/d\/[A-Za-z0-9_-]{10,200}\/edit$/.test(u.pathname); }
  catch { return false; }
}
async function popupStatus() {
  const deadline = Date.now()+1000;
  while (pending && Date.now()<deadline) await new Promise(resolve=>setTimeout(resolve,25));
  return pending ? unavailable() : native({type:'ping'});
}
async function setupScope(tabId, revision) {
  const tab = await chrome.tabs.get(tabId);
  const window = await chrome.windows.get(tab.windowId);
  return revision === focusRevision && tab.active && window.focused && docsURL(tab.url);
}
async function setupFromPopup() {
  if (setupRun) return {result:'busy'};
  const revision = focusRevision;
  const tabs = await chrome.tabs.query({active:true,lastFocusedWindow:true});
  if (tabs.length !== 1 || !docsURL(tabs[0].url)
    || !await setupScope(tabs[0].id, revision)) return {result:'not_docs'};
  if (setupRun) return {result:'busy'};
  // Drop old geometry and prevent normal content ticks from racing setup.
  clear();
  const lease = {tabId:tabs[0].id,revision:focusRevision,expires:Date.now()+10000};
  setupRun = lease;
  try {
    while (pending && Date.now() < lease.expires)
      await new Promise(resolve => setTimeout(resolve, 25));
    if (Date.now() >= lease.expires || !await setupScope(lease.tabId, lease.revision))
      return {result:'review'};
    if (!(await native({type:'ping'})).enabled) return {result:'off'};
    if (!await setupScope(lease.tabId, lease.revision)) return {result:'review'};
    let timer;
    const reply = await Promise.race([
      chrome.tabs.sendMessage(lease.tabId,{type:'setupAccessibility'},{frameId:0}),
      new Promise(resolve => { timer=setTimeout(()=>resolve({result:'review'}),10000); }),
    ]).finally(()=>clearTimeout(timer));
    return {result:['saved','review','unsupported','busy'].includes(reply?.result) ? reply.result : 'review'};
  } catch { return {result:'review'}; }
  finally { if (setupRun === lease) setupRun=null; clear(); }
}
async function route(message, sender) {
  if (message?.type === 'status' && sender.url === chrome.runtime.getURL('popup.html')) return popupStatus();
  if (message?.type === 'setup' && sender.url === chrome.runtime.getURL('popup.html')
    && Object.keys(message).length === 1) return setupFromPopup();
  if (!sender.tab || sender.frameId !== 0 || !docsURL(sender.url)) return unavailable();
  if (message?.type === 'setupGuard') {
    const lease = setupRun;
    return {allowed:Object.keys(message).length === 1 && !!lease && lease.tabId === sender.tab.id
      && Date.now() < lease.expires && await setupScope(lease.tabId,lease.revision)
      && setupRun === lease && (await native({type:'ping'})).enabled
      && setupRun === lease && Date.now() < lease.expires && lease.revision === focusRevision};
  }
  if (setupRun) return unavailable();
  const revision = focusRevision;
  const tab = await chrome.tabs.get(sender.tab.id);
  const window = await chrome.windows.get(tab.windowId);
  if (revision !== focusRevision) return unavailable();
  if (!tab.active || !window.focused || !docsURL(tab.url)) {
    if (sourceTab === tab.id) { sourceTab = null; await native({type:'clear'}); }
    return unavailable();
  }
  if (!['ping','observe','clear'].includes(message?.type)) return unavailable();
  if (sourceTab !== tab.id) { sourceTab = tab.id; await native({type:'clear'}); }
  if (revision !== focusRevision || sourceTab !== tab.id) return unavailable();
  if (message.type === 'observe') {
    const s=message.snapshot;
    if (!s || Object.keys(message).some(k=>!['type','snapshot'].includes(k))
      || Object.keys(s).sort().join(',')!=='caret,field,page_key,proxy_caret,selection_utf16,text_hash,text_selection_utf16,viewport'
      || !/^[a-f0-9]{64}$/.test(s.page_key) || !/^[a-f0-9]{64}$/.test(s.text_hash)
      || !Number.isInteger(s.selection_utf16) || s.selection_utf16<0 || s.selection_utf16>32768
      || !Number.isInteger(s.text_selection_utf16) || s.text_selection_utf16<s.selection_utf16
      || s.text_selection_utf16>32768 || s.text_selection_utf16-s.selection_utf16>255
      || ![s.caret,s.field,s.proxy_caret].every(r=>Array.isArray(r)&&r.length===4&&r.every(Number.isFinite))
      || !Array.isArray(s.viewport) || s.viewport.length!==2 || !s.viewport.every(Number.isFinite)) return unavailable();
    return native({type:'observe',snapshot:s});
  }
  if (Object.keys(message).length!==1) return unavailable();
  return native({type:message.type});
}
chrome.runtime.onMessage.addListener((message,sender,respond) => {
  route(message,sender).then(respond,()=>respond(unavailable())); return true;
});
function clear() { focusRevision++; sourceTab=null; if (pending) needsClear=true; else native({type:'clear'}); }
chrome.tabs.onActivated.addListener(clear);
chrome.windows.onFocusChanged.addListener(clear);
chrome.tabs.onUpdated.addListener((id,change)=>{
  if (change.url || change.status==='loading') {
    if (id===sourceTab) clear(); else focusRevision++;
  }
});
chrome.tabs.onRemoved.addListener(id=>{ if (id===sourceTab) clear(); else focusRevision++; });

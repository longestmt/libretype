// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
const HOST = 'org.libretype.docs';
let port = null;
let pending = null;
let sourceTab = null;
let needsClear = false;
let focusRevision = 0;
let setupRun = null;
let setupResult = null;
let setupReason = null;
// Only fixed, text-free outcomes may leave the setup flow or survive in memory.
const setupReasons = new Set(['focus_changed','off','expired','guard_denied','connection_lost',
  'menu_timeout','dialog_timeout','options_timeout','save_timeout','existing_dialog',
  'controls_unavailable','controls_changed','offscreen','unexpected']);
const stopped = reason => ({result:'review',reason});
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
  const reply = pending ? unavailable() : await native({type:'ping'});
  return {...reply,setupResult,setupReason};
}
async function setupScopeIssue(tabId, revision) {
  try {
    const tab = await chrome.tabs.get(tabId);
    const window = await chrome.windows.get(tab.windowId);
    if (revision !== focusRevision) return 'focus_changed';
    if (setupRun?.tabId===tabId
      && (tab.windowId!==setupRun.windowId || tab.url!==setupRun.url)) return 'focus_changed';
    if (!tab.active) return 'inactive_tab';
    if (!window.focused) return 'window_unfocused';
    if (typeof tab.url !== 'string') return 'unavailable_url';
    return docsURL(tab.url) ? null : 'not_docs';
  } catch { return 'tab_unavailable'; }
}
async function setupScope(tabId, revision) {
  return (await setupScopeIssue(tabId, revision)) === null;
}
async function setupFromPopup(tabId) {
  if (setupRun) return {result:'busy'};
  let tab;
  try { tab = await chrome.tabs.get(tabId); } catch { return {result:'tab_unavailable'}; }
  if (!tab.active) return {result:'inactive_tab'};
  if (typeof tab.url !== 'string') return {result:'unavailable_url'};
  if (!docsURL(tab.url)) return {result:'not_docs'};
  if (setupRun) return {result:'busy'};
  // The action popup can temporarily take focus from its owning Chrome window.
  // Acknowledge first so the popup can close; no page action occurs until focus returns.
  const lease = {tabId,windowId:tab.windowId,url:tab.url,preparing:true,cancelled:false,
    expires:Date.now()+3000,revision:null};
  setupRun=lease; setupResult='starting'; setupReason=null;
  lease.completion=runSetup(lease).catch(()=>stopped('connection_lost')).then(reply=>{
    setupResult=reply.result;
    setupReason=setupReasons.has(reply.reason) ? reply.reason : null;
    return reply;
  }).finally(()=>{if(setupRun===lease)setupRun=null;clear();});
  return {result:'starting'};
}
async function runSetup(lease) {
  // Give the acknowledged popup an opportunity to close, then inspect real focus.
  await new Promise(resolve=>setTimeout(resolve,75));
  while (true) {
    if (lease.cancelled || setupRun!==lease) return {result:'focus_changed'};
    if (Date.now()>=lease.expires) return {result:'window_unfocused'};
    let tab,window;
    try { tab=await chrome.tabs.get(lease.tabId);window=await chrome.windows.get(lease.windowId); }
    catch { return {result:'tab_unavailable'}; }
    if (lease.cancelled || setupRun!==lease || !tab.active || tab.windowId!==lease.windowId
      || tab.url!==lease.url) return {result:'focus_changed'};
    if (Date.now()>=lease.expires) return {result:'window_unfocused'};
    if (window.focused) break;
    await new Promise(resolve=>setTimeout(resolve,25));
  }
  clear();
  lease.preparing=false;lease.revision=focusRevision;lease.expires=Date.now()+10000;
  while (pending && Date.now() < lease.expires)
    await new Promise(resolve => setTimeout(resolve, 25));
  if (Date.now() >= lease.expires) return stopped('expired');
  if (!await setupScope(lease.tabId, lease.revision)) return stopped('focus_changed');
  if (!(await native({type:'ping'})).enabled) return {result:'off'};
  if (!await setupScope(lease.tabId, lease.revision)) return stopped('focus_changed');
  let timer;
  const reply = await Promise.race([
    chrome.tabs.sendMessage(lease.tabId,{type:'setupAccessibility'},{frameId:0}),
    new Promise(resolve => { timer=setTimeout(()=>resolve(stopped('expired')),10000); }),
  ]).finally(()=>clearTimeout(timer));
  const result = ['saved','review','unsupported','busy'].includes(reply?.result) ? reply.result : 'review';
  return {result,...(result==='review' && setupReasons.has(reply?.reason) ? {reason:reply.reason} : {})};
}
async function route(message, sender) {
  if (message?.type === 'status' && sender.url === chrome.runtime.getURL('popup.html')) return popupStatus();
  if (message?.type === 'setup' && sender.url === chrome.runtime.getURL('popup.html')
    && Object.keys(message).sort().join(',') === 'tabId,type'
    && Number.isInteger(message.tabId) && message.tabId >= 0) return setupFromPopup(message.tabId);
  if (!sender.tab || sender.frameId !== 0 || !docsURL(sender.url)) return unavailable();
  if (message?.type === 'setupGuard') {
    const lease = setupRun;
    const denied = reason => ({allowed:false,reason});
    if (Object.keys(message).length !== 1 || !lease || lease.tabId !== sender.tab.id || lease.preparing)
      return denied('guard_denied');
    if (Date.now() >= lease.expires) return denied('expired');
    if (!await setupScope(lease.tabId,lease.revision) || setupRun !== lease)
      return denied('focus_changed');
    const reply = await native({type:'ping'});
    if (setupRun !== lease || lease.revision !== focusRevision) return denied('focus_changed');
    if (Date.now() >= lease.expires) return denied('expired');
    if (!reply.enabled) return denied('off');
    return {allowed:true};
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
function cancelPreparing() { if(setupRun?.preparing)setupRun.cancelled=true; }
chrome.tabs.onActivated.addListener(()=>{cancelPreparing();clear();});
chrome.windows.onFocusChanged.addListener(id=>{
  if(setupRun?.preparing && id>=0 && id!==setupRun.windowId)cancelPreparing();
  clear();
});
chrome.tabs.onUpdated.addListener((id,change)=>{
  if (change.url || change.status==='loading') {
    if(id===setupRun?.tabId)cancelPreparing();
    if (id===sourceTab) clear(); else focusRevision++;
  }
});
chrome.tabs.onRemoved.addListener(id=>{ if(id===setupRun?.tabId)cancelPreparing(); if (id===sourceTab) clear(); else focusRevision++; });

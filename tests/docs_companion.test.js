// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
const test=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const crypto=require('node:crypto');
const root=path.resolve(__dirname,'../companion/google-docs');
const url='https://docs.google.com/document/d/synthetic_123/edit';
function harness({automaticReplies=true}={}) {
  const messages=[];let active=true,focused=true;
  const event=()=>({listeners:[],addListener(fn){this.listeners.push(fn)}});
  const ports=[];
  const chrome={runtime:{getURL:file=>'chrome-extension://test/'+file,onMessage:event(),connectNative(name){
    assert.equal(name,'org.libretype.docs');
    const port={onMessage:event(),onDisconnect:event(),disconnect(){},postMessage(message){
      messages.push(JSON.parse(JSON.stringify(message)));
      if(automaticReplies) queueMicrotask(()=>port.onMessage.listeners.forEach(fn=>fn({enabled:true,accepted:true})));
    }};ports.push(port);return port;
  }},tabs:{get:async id=>({id,windowId:2,active,url}),onActivated:event(),onUpdated:event(),onRemoved:event()},
    windows:{get:async()=>({focused}),onFocusChanged:event()}};
  const context=vm.createContext({chrome,setTimeout,clearTimeout,URL,console});
  vm.runInContext(fs.readFileSync(path.join(root,'background.js'),'utf8'),context);
  return {context,chrome,messages,ports,setActive(value){active=value},setFocused(value){focused=value}};
}
const sender=()=>({frameId:0,url,tab:{id:1,windowId:2}});
const sample=()=>({type:'observe',snapshot:{page_key:'a'.repeat(64),text_hash:'b'.repeat(64),selection_utf16:19,text_selection_utf16:19,viewport:[1000,800],field:[50,120,900,680],caret:[300,200,2,18],proxy_caret:[150,-9990,0,18]}});
test('extension identity is derived from its public key and permissions stay Docs-only',()=>{
  const manifest=JSON.parse(fs.readFileSync(path.join(root,'manifest.json')));
  const id=crypto.createHash('sha256').update(Buffer.from(manifest.key,'base64')).digest('hex').slice(0,32).split('').map(c=>String.fromCharCode(97+parseInt(c,16))).join('');
  assert.equal(id,fs.readFileSync(path.join(root,'extension-id.txt'),'utf8').trim());
  assert.deepEqual(manifest.permissions,['nativeMessaging']);
  assert.deepEqual(manifest.host_permissions,['https://docs.google.com/document/*']);
  assert.deepEqual(manifest.content_scripts[0].matches,manifest.host_permissions);
  assert.equal(manifest.content_scripts[0].all_frames,undefined);
});
test('only an active focused top-level Docs tab can send geometry',async()=>{
  const h=harness();
  for(const bad of [{...sender(),url:'https://example.invalid/'},{...sender(),frameId:3}]) {
    assert.equal((await h.context.route(sample(),bad)).accepted,false);
  }
  h.setActive(false);assert.equal((await h.context.route(sample(),sender())).accepted,false);
  h.setActive(true);h.setFocused(false);assert.equal((await h.context.route(sample(),sender())).accepted,false);
  assert.equal(h.messages.length,0);
  h.setFocused(true);assert.equal((await h.context.route(sample(),sender())).accepted,true);
  assert.deepEqual(h.messages.map(m=>m.type),['clear','observe']);
});
test('unexpected text payloads and malformed fingerprints never reach the native host',async()=>{
  const h=harness();await h.context.route({type:'ping'},sender());h.messages.length=0;
  const extra=sample();extra.snapshot.text='SYNTHETIC TEXT MUST NOT TRAVEL';
  const invalid=sample();invalid.snapshot.text_hash='not-a-fingerprint';
  for(const message of [extra,invalid,{type:'ping',text:'SYNTHETIC TEXT MUST NOT TRAVEL'}]) {
    assert.equal((await h.context.route(message,sender())).accepted,false);
  }
  for(const offset of [-1,0.5,32769,null]) {
    const message=sample();message.snapshot.selection_utf16=offset;
    assert.equal((await h.context.route(message,sender())).accepted,false);
  }
  for(const offset of [18,19.5,275,32769,null]) {
    const message=sample();message.snapshot.text_selection_utf16=offset;
    assert.equal((await h.context.route(message,sender())).accepted,false);
  }
  assert.equal(h.messages.length,0);
});
test('a stale port disconnect cannot disconnect a newer native connection',async()=>{
  const h=harness();await h.context.native({type:'ping'});const first=h.ports[0];
  h.context.disconnect();await h.context.native({type:'ping'});
  first.onDisconnect.listeners.forEach(fn=>fn());
  await h.context.native({type:'ping'});
  assert.equal(h.ports.length,2);
});
test('a delayed reply from an old port cannot answer a newer request',async()=>{
  const h=harness({automaticReplies:false});
  const firstRequest=h.context.native({type:'ping'}),first=h.ports[0];
  h.context.disconnect();await firstRequest;
  const secondRequest=h.context.native({type:'ping'}),second=h.ports[1];
  first.onMessage.listeners.forEach(fn=>fn({enabled:true,accepted:true}));
  second.onMessage.listeners.forEach(fn=>fn({enabled:false,accepted:false}));
  assert.equal((await secondRequest).enabled,false);
});
test('switching tabs during focus queries cannot restore cleared geometry',async()=>{
  const h=harness();await h.context.route({type:'ping'},sender());h.messages.length=0;
  let answer;
  h.chrome.windows.get=()=>new Promise(resolve=>{answer=resolve});
  const inFlight=h.context.route(sample(),sender());
  await new Promise(resolve=>setImmediate(resolve));
  h.chrome.tabs.onActivated.listeners.forEach(fn=>fn({tabId:2}));
  await new Promise(resolve=>setImmediate(resolve));
  answer({focused:true});
  assert.equal((await inFlight).accepted,false);
  assert.equal(h.messages.some(m=>m.type==='observe'),false);
});

// Execute the shipped adapter against a disposable DOM fixture. These checks
// cover asynchronous observation, not compatibility with Google's live DOM.
function contentHarness() {
  const state={enabled:true,focused:true,visible:true,collapsed:true,text:'A synthetic paragraph.\nThank you for your\u00a0',reads:0};
  const messages=[],listeners={};let tick;
  const box=(x,y,width,height)=>({x,y,width,height});
  state.caret=box(300,200,2,18);
  state.proxy=box(150,10,0,18);
  const anchor={};
  const parts=()=>state.paragraphTexts || [state.text];
  const paragraphs=()=>parts().map((text,index)=>({tagName:'P',innerText:text,textContent:text,
    contains:node=>node===anchor && index===(state.paragraphIndex??parts().length-1)}));
  const editor={getAttribute:name=>name==='role'?'textbox':null,isContentEditable:true,
    contains:node=>node===anchor,get children(){return paragraphs()},get textContent(){state.reads++;return state.text}};
  const selection={get isCollapsed(){return state.collapsed},rangeCount:1,anchorNode:anchor,
    get anchorOffset(){return state.anchorOffset??parts()[state.paragraphIndex??parts().length-1].length},
    getRangeAt:()=>({getBoundingClientRect:()=>({...state.proxy})})};
  const frame={contentDocument:{activeElement:editor,getSelection:()=>selection,
    createRange:()=>{let paragraph,end;return {selectNodeContents:p=>{paragraph=p},setEnd:(_node,offset)=>{end=offset},toString:()=>paragraph.textContent.slice(0,end)}}},
    getBoundingClientRect:()=>box(0,-10000,1000,1)};
  const cursor={getBoundingClientRect:()=>({...state.caret})};
  const field={getBoundingClientRect:()=>box(50,120,900,680)};
  const document={get visibilityState(){return state.visible?'visible':'hidden'},
    hasFocus:()=>state.focused,activeElement:frame,
    querySelector:selector=>({'iframe.docs-texteventtarget-iframe':frame,
      '#kix-current-user-cursor-caret':cursor,'.kix-appview-editor':field})[selector],
    addEventListener:(name,fn)=>{listeners[name]=fn}};
  const context=vm.createContext({document,window:{addEventListener:(name,fn)=>{listeners[name]=fn}},
    innerWidth:1000,innerHeight:800,location:new URL(url),TextEncoder,Uint8Array,
    getComputedStyle:()=>({display:'block'}),setInterval:fn=>{tick=fn},
    crypto:{subtle:{digest:async(...args)=>{state.onHash?.();return crypto.webcrypto.subtle.digest(...args)}}},
    chrome:{runtime:{sendMessage:async message=>{
      messages.push(JSON.parse(JSON.stringify(message)));return {enabled:state.enabled,accepted:true};
    }}}});
  vm.runInContext(fs.readFileSync(path.join(root,'content.js'),'utf8'),context);
  return {state,messages,document,frame,selection,listeners,tick:()=>tick()};
}
test('content adapter sends matching geometry and fingerprints without plaintext',async()=>{
  const h=contentHarness();await h.tick();
  assert.deepEqual(h.messages.map(m=>m.type),['ping','observe']);
  const snapshot=h.messages[1].snapshot;
  assert.equal(snapshot.text_hash,crypto.createHash('sha256').update(h.state.text).digest('hex'));
  assert.equal(snapshot.page_key,crypto.createHash('sha256').update(url).digest('hex'));
  assert.deepEqual(snapshot.proxy_caret,[150,-9990,0,18]);
  assert.deepEqual(snapshot.caret,[300,200,2,18]);
  assert.equal(snapshot.selection_utf16,h.state.text.length);
  assert.equal(JSON.stringify(h.messages).includes('synthetic paragraph'),false);
});
test('disabled or unfocused companion does not read editor text',async()=>{
  for(const change of [s=>s.enabled=false,s=>s.focused=false,s=>s.visible=false]) {
    const h=contentHarness();change(h.state);await h.tick();
    assert.equal(h.state.reads,0);
    assert.equal(h.messages.some(m=>m.type==='observe'),false);
  }
});
test('missing editor, selected text, or clipped caret clears prior geometry',async()=>{
  for(const change of [h=>h.document.activeElement=null,h=>h.frame.contentDocument=null,
    h=>h.state.collapsed=false,h=>h.selection.anchorNode={},
    h=>h.state.caret.y=900,h=>h.state.proxy.height=0]) {
    const h=contentHarness();await h.tick();h.messages.length=0;change(h);await h.tick();
    assert.deepEqual(h.messages.map(m=>m.type),['ping','clear']);
  }
});
test('text, caret, or focus changes during hashing invalidate the observation',async()=>{
  for(const change of [s=>s.text+='changed',s=>s.caret.x+=10,
    s=>s.proxy.x+=10,s=>s.focused=false]) {
    const h=contentHarness();h.state.onHash=()=>{delete h.state.onHash;change(h.state)};
    await h.tick();assert.deepEqual(h.messages.map(m=>m.type),['ping','clear']);
  }
});
test('focus, visibility, and page exit events immediately clear prior geometry',async()=>{
  const h=contentHarness();await h.tick();h.messages.length=0;
  for(const event of ['blur','visibilitychange','pagehide']) h.listeners[event]();
  assert.deepEqual(h.messages.map(m=>m.type),['clear','clear','clear']);
});
test('paragraph-window offsets match native text nodes, UTF-16 emoji, and mid-line positions',async()=>{
  const h=contentHarness();
  h.state.paragraphTexts=['Earlier 👩‍💻 paragraph.','','We should tomorrow.'];
  h.state.text=h.state.paragraphTexts.join('');
  h.state.anchorOffset='We should '.length;
  await h.tick();
  assert.equal(h.messages[1].snapshot.selection_utf16,'Earlier 👩‍💻 paragraph.We should '.length);
  assert.equal(h.messages[1].snapshot.text_selection_utf16,'Earlier 👩‍💻 paragraph.\n\nWe should '.length);
  assert.equal(h.messages[1].snapshot.text_hash,crypto.createHash('sha256').update(h.state.paragraphTexts.join('\n')).digest('hex'));
  h.messages.length=0;
  h.state.text=h.state.paragraphTexts.join('\n\n');
  await h.tick();
  assert.deepEqual(h.messages.map(m=>m.type),['ping','clear']);
});
test('a caret-only change during hashing invalidates otherwise identical geometry',async()=>{
  const h=contentHarness();h.state.onHash=()=>{delete h.state.onHash;h.state.anchorOffset=2};
  await h.tick();assert.deepEqual(h.messages.map(m=>m.type),['ping','clear']);
});
test('the observed Docs window maps raw caret 1020 into its 1037-unit native value',async()=>{
  const h=contentHarness();
  h.state.paragraphTexts=Array.from({length:17},(_,i)=>`Synthetic paragraph ${44+i} for the LibreType scrolling check.`);
  h.state.paragraphTexts.push('The agenda for tomorrow’s project meeting includes\u00a0');
  h.state.text=h.state.paragraphTexts.join('');
  assert.equal(h.state.text.length,1020);
  assert.equal(h.state.paragraphTexts.join('\n\n').length,1054);
  await h.tick();
  const snapshot=h.messages.find(m=>m.type==='observe').snapshot;
  assert.equal(snapshot.selection_utf16,1020);
  const nativeValue=h.state.paragraphTexts.join('\n');
  assert.equal(nativeValue.length,1037);
  assert.equal(snapshot.text_selection_utf16,1037);
  assert.equal(snapshot.text_hash,crypto.createHash('sha256').update(nativeValue).digest('hex'));
});

const popupSender=()=>({url:'chrome-extension://test/popup.html'});
async function requestSetup(h,message={type:'setup',tabId:1},from=popupSender()) {
  const reply=await h.context.route(message,from);
  if(reply.result!=='starting')return reply;
  return vm.runInContext('setupRun.completion',h.context);
}
test('closing the popup restores focus before setup and the result survives reopening',async()=>{
  const h=harness();h.setFocused(false);let sends=0;
  h.chrome.tabs.sendMessage=async()=>{sends++;return {result:'saved'}};
  const reply=await h.context.route({type:'setup',tabId:1},popupSender());
  assert.equal(reply.result,'starting');
  const completion=vm.runInContext('setupRun.completion',h.context);
  await new Promise(resolve=>setTimeout(resolve,100));
  assert.equal(sends,0);
  assert.equal((await h.context.route({type:'setupGuard'},sender())).allowed,false);
  h.setFocused(true);
  h.chrome.windows.onFocusChanged.listeners.forEach(fn=>fn(2));
  assert.equal((await completion).result,'saved');assert.equal(sends,1);
  const status=await h.context.route({type:'status'},popupSender());
  assert.equal(status.setupResult,'saved');
  assert.equal(JSON.stringify(status).includes('synthetic_123'),false);
  const nodes={setup:{addEventListener(){}},status:{},result:{}};
  const context=vm.createContext({document:{getElementById:id=>nodes[id]},
    chrome:{runtime:{sendMessage:async()=>status}}});
  vm.runInContext(fs.readFileSync(path.join(root,'popup.js'),'utf8'),context);
  await Promise.resolve();
  assert.match(nodes.result.textContent,/Last setup: Settings saved/);
});
test('tab, URL, or other-window changes cancel setup while popup focus is returning',async()=>{
  for(const disrupt of [
    h=>h.chrome.tabs.onActivated.listeners.forEach(fn=>fn({tabId:3})),
    h=>h.chrome.windows.onFocusChanged.listeners.forEach(fn=>fn(3)),
    h=>h.chrome.tabs.onUpdated.listeners.forEach(fn=>fn(1,{url:url+'?changed'})),
    h=>{h.chrome.tabs.get=async id=>({id,windowId:2,active:true,url:url+'?changed'})},
    h=>{h.chrome.tabs.get=async id=>({id,windowId:3,active:true,url})},
  ]) {
    const h=harness();h.setFocused(false);let sends=0;
    h.chrome.tabs.sendMessage=async()=>{sends++;return {result:'saved'}};
    assert.equal((await h.context.route({type:'setup',tabId:1},popupSender())).result,'starting');
    const completion=vm.runInContext('setupRun.completion',h.context);
    disrupt(h);h.setFocused(true);
    assert.equal((await completion).result,'focus_changed');assert.equal(sends,0);
  }
});
test('popup binds setup to its own active tab, not the worker last-focused window',async()=>{
  let click,closed=false;const requests=[];
  const button={disabled:true,addEventListener:(_type,fn)=>{click=fn}};
  const nodes={setup:button,status:{},result:{}};
  const context=vm.createContext({window:{close:()=>{closed=true}},document:{getElementById:id=>nodes[id]},chrome:{
    tabs:{query:async query=>{assert.equal(JSON.stringify(query),JSON.stringify({active:true,currentWindow:true}));return [{id:37}]}},
    runtime:{sendMessage:async message=>{requests.push(JSON.parse(JSON.stringify(message)));return message.type==='status'?{enabled:true}:{result:'starting'}}},
  }});
  vm.runInContext(fs.readFileSync(path.join(root,'popup.js'),'utf8'),context);
  await Promise.resolve();await click();
  assert.deepEqual(requests,[{type:'status'},{type:'setup',tabId:37}]);
  assert.equal(closed,true);assert.match(nodes.result.textContent,/regain focus/);
  const h=harness();
  h.chrome.tabs.query=async()=>{throw new Error('Must not look up a different window')};
  h.chrome.tabs.get=async id=>{assert.equal(id,37);return {id,windowId:2,active:true,url}};
  h.chrome.tabs.sendMessage=async id=>{assert.equal(id,37);return {result:'saved'}};
  assert.equal((await requestSetup(h,requests[1])).result,'saved');
});
test('setup reports scope failures without exposing addresses or touching the document',async()=>{
  for(const [patch,expected] of [
    [{active:false},'inactive_tab'],[{url:undefined},'unavailable_url'],
    [{url:'https://example.invalid/private-document'},'not_docs'],
  ]) {
    const h=harness();let sends=0;
    h.chrome.tabs.get=async id=>({id,windowId:2,active:true,url,...patch});
    h.chrome.tabs.sendMessage=async()=>{sends++;return {result:'saved'}};
    const reply=await requestSetup(h);
    assert.equal(JSON.stringify(reply),JSON.stringify({result:expected}));
    assert.equal(sends,0);assert.equal(h.messages.length,0);
  }
  const h=harness();
  h.chrome.tabs.get=async()=>{throw new Error('closed tab')};
  assert.equal((await requestSetup(h)).result,'tab_unavailable');
  h.chrome.tabs.get=async id=>{
    h.chrome.tabs.onActivated.listeners.forEach(fn=>fn({tabId:2}));
    return {id,windowId:2,active:true,url};
  };
  assert.equal((await requestSetup(h)).result,'focus_changed');
  for(const tabId of [-1,1.2,'1',null,undefined]) {
    assert.equal((await requestSetup(h,{type:'setup',tabId})).accepted,false);
  }
});
test('Docs setup requires the popup, an enabled app, and a focused Docs tab',async()=>{
  const h=harness();let sends=0;
  h.chrome.tabs.query=async()=>[{id:1,windowId:2,active:true,url}];
  h.chrome.tabs.sendMessage=async(id,message,options)=>{
    sends++;assert.equal(id,1);assert.equal(message.type,'setupAccessibility');assert.equal(options.frameId,0);
    assert.equal((await h.context.route({type:'setupGuard'},sender())).allowed,true);
    return {result:'saved'};
  };
  assert.equal((await h.context.route({type:'setup',tabId:1},sender())).accepted,false);
  assert.equal((await h.context.route({type:'setup',tabId:1,extra:'untrusted'},popupSender())).accepted,false);
  h.setFocused(false);
  assert.equal((await requestSetup(h)).result,'window_unfocused');
  assert.equal(sends,0);
  h.setFocused(true);
  assert.equal((await requestSetup(h)).result,'saved');
  assert.equal(sends,1);
  assert.equal((await h.context.route({type:'setupGuard'},sender())).allowed,false);
  assert.equal(h.messages.some(m=>!['ping','clear'].includes(m.type)),false);
});
test('setup leases reject changed focus, unrelated tabs, and overlapping requests',async()=>{
  const h=harness();let finish;
  h.chrome.tabs.query=async()=>[{id:1,windowId:2,active:true,url}];
  h.chrome.tabs.sendMessage=()=>new Promise(resolve=>{finish=resolve});
  const request=requestSetup(h);
  while(!finish) await new Promise(resolve=>setImmediate(resolve));
  assert.equal((await requestSetup(h)).result,'busy');
  assert.equal((await h.context.route({type:'setupGuard'},{...sender(),tab:{id:3}})).allowed,false);
  assert.equal((await h.context.route(sample(),sender())).accepted,false);
  h.chrome.tabs.onActivated.listeners.forEach(fn=>fn({tabId:3}));
  assert.equal((await h.context.route({type:'setupGuard'},sender())).allowed,false);
  finish({result:'review'});
  assert.equal((await request).result,'review');
});
test('a disabled local app cannot initiate document setup',async()=>{
  const h=harness({automaticReplies:false});let sends=0;
  h.chrome.tabs.query=async()=>[{id:1,windowId:2,active:true,url}];
  h.chrome.tabs.sendMessage=async()=>{sends++;return {result:'saved'}};
  h.context.native=async()=>({enabled:false,accepted:false});
  assert.equal((await requestSetup(h)).result,'off');
  assert.equal(sends,0);
});

function setupHarness(initial={}) {
  const state={reader:false,braille:false,collaborators:true,magnifier:false,menu:false,dialog:false,
    allowed:true,visible:true,clicks:[],...initial};
  let listener;
  const element=(name,shown,click)=>({
    name,isConnected:true,disabled:false,textContent:name,
    getBoundingClientRect:()=>({width:shown()?100:0,height:shown()?20:0}),
    getAttribute:attribute=>attribute==='aria-disabled'?'false':null,
    dispatchEvent(){},click(){state.clicks.push(name);click?.();},
  });
  const tools=element('Tools',()=>true,()=>{state.menu=true});
  const item=element('Accessibility',()=>state.menu,()=>{state.menu=false;state.dialog=true});
  const label=element('Accessibility',()=>state.menu);label.closest=()=>item;
  const heading={textContent:'Accessibility settings'};
  function input(key,label) {
    const box=element(label,()=>state.dialog,()=>{state[key]=!state[key];state.afterClick?.(key)});
    box.labels=[{textContent:label}];
    Object.defineProperty(box,'checked',{get:()=>state[key]});
    if(key==='braille') Object.defineProperty(box,'disabled',{get:()=>!state.reader});
    return box;
  }
  const reader=input('reader','Turn on screen reader support');
  const braille=input('braille','Turn on braille support');
  const collaborators=input('collaborators','Turn on collaborator announcements');
  const magnifier=input('magnifier','Turn on screen magnifier support');
  const inputs=()=>[reader,...(state.missingBraille?[]:[braille]),collaborators,magnifier];
  const ok=element('OK',()=>state.dialog,()=>{state.saved=true;state.dialog=false});
  const dialog=element('dialog',()=>state.dialog);
  dialog.getAttribute=attribute=>attribute==='aria-labelledby'?'heading':null;
  dialog.querySelectorAll=selector=>selector==='input[type="checkbox"]'?inputs():selector==='button,[role="button"]'?[ok]:[];
  const document={get visibilityState(){return state.visible?'visible':'hidden'},
    getElementById:id=>id==='docs-tools-menu'?tools:id==='heading'?heading:null,
    querySelectorAll:selector=>selector==='[role="dialog"]'?[dialog]:selector==='.goog-menuitem-label'?[label]:[]};
  const context=vm.createContext({document,window:{},location:{href:url},Date,setTimeout,
    MouseEvent:class{},getComputedStyle:()=>({visibility:'visible'}),
    chrome:{runtime:{id:'test',onMessage:{addListener:fn=>{listener=fn}},
      sendMessage:async message=>{assert.equal(message.type,'setupGuard');return {allowed:state.allowed};}}}});
  vm.runInContext(fs.readFileSync(path.join(root,'setup.js'),'utf8'),context);
  const invoke=(message={type:'setupAccessibility'},sender={id:'test'})=>new Promise(resolve=>{
    let answered=false;
    const asynchronous=listener(message,sender,result=>{answered=true;resolve(result)});
    if(!asynchronous&&!answered)resolve(undefined);
  });
  return {state,invoke};
}
test('explicit setup enables only missing required options and saves once',async()=>{
  for(const initial of [{},{reader:true},{reader:true,braille:true},{collaborators:false,magnifier:true}]) {
    const h=setupHarness(initial);
    assert.equal((await h.invoke()).result,'saved');
    assert.equal(h.state.reader,true);assert.equal(h.state.braille,true);
    assert.equal(h.state.collaborators,initial.collaborators??true);assert.equal(h.state.magnifier,initial.magnifier??false);
    assert.equal(h.state.clicks.filter(x=>x==='OK').length,1);
    assert.equal(h.state.clicks.includes('Turn on screen reader support'),!initial.reader);
    assert.equal(h.state.clicks.includes('Turn on braille support'),!initial.braille);
  }
});
test('setup refuses unrelated senders, extra payloads, pre-existing dialogs and missing controls',async()=>{
  for(const [message,sender] of [[{type:'setupAccessibility'},{id:'another'}],
    [{type:'setupAccessibility'},{id:'test',tab:{id:1}}],
    [{type:'setupAccessibility',text:'must not use'},{id:'test'}]]) {
    const h=setupHarness();assert.equal(await h.invoke(message,sender),undefined);assert.deepEqual(h.state.clicks,[]);
  }
  for(const initial of [{dialog:true},{missingBraille:true},{allowed:false},{visible:false}]) {
    const h=setupHarness(initial);assert.equal((await h.invoke()).result,'review');
    assert.equal(h.state.saved,undefined);assert.equal(h.state.reader,false);assert.equal(h.state.braille,false);
  }
});
test('setup stops without saving if focus or unrelated preferences change during the operation',async()=>{
  for(const disrupt of [state=>{state.allowed=false},state=>{state.collaborators=false}]) {
    const h=setupHarness();h.state.afterClick=key=>{if(key==='reader')disrupt(h.state)};
    assert.equal((await h.invoke()).result,'review');
    assert.equal(h.state.clicks.includes('OK'),false);
    assert.equal(h.state.braille,false);
  }
});
test('popup availability waits for an in-flight geometry request instead of reporting Off',async()=>{
  const h=harness({automaticReplies:false});
  const first=h.context.native({type:'ping'});
  const status=h.context.route({type:'status'},popupSender());
  const port=h.ports[0];port.onMessage.listeners.forEach(fn=>fn({enabled:true,accepted:false}));
  await first;
  const deadline=Date.now()+2000;
  while(h.messages.length<2) {assert.ok(Date.now()<deadline);await new Promise(resolve=>setImmediate(resolve));}
  port.onMessage.listeners.forEach(fn=>fn({enabled:true,accepted:false}));
  assert.equal((await status).enabled,true);
});
test('turning the companion off revokes an active setup lease',async()=>{
  const h=harness();
  h.chrome.tabs.query=async()=>[{id:1,windowId:2,active:true,url}];
  h.chrome.tabs.sendMessage=async()=>{
    h.context.native=async()=>({enabled:false,accepted:false});
    assert.equal((await h.context.route({type:'setupGuard'},sender())).allowed,false);
    return {result:'review'};
  };
  assert.equal((await requestSetup(h)).result,'review');
});
test('active setup rejects a changed document even before a navigation event arrives',async()=>{
  const h=harness();
  h.chrome.tabs.sendMessage=async()=>{
    h.chrome.tabs.get=async id=>({id,windowId:2,active:true,url:url+'?changed'});
    assert.equal((await h.context.route({type:'setupGuard'},sender())).allowed,false);
    return {result:'review'};
  };
  assert.equal((await requestSetup(h)).result,'review');
});

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
const sample=()=>({type:'observe',snapshot:{page_key:'a'.repeat(64),text_hash:'b'.repeat(64),selection_utf16:19,viewport:[1000,800],field:[50,120,900,680],caret:[300,200,2,18],proxy_caret:[150,-9990,0,18]}});
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
  h.messages.length=0;
  h.state.text=h.state.paragraphTexts.join('\n\n');
  await h.tick();
  assert.deepEqual(h.messages.map(m=>m.type),['ping','clear']);
});
test('a caret-only change during hashing invalidates otherwise identical geometry',async()=>{
  const h=contentHarness();h.state.onHash=()=>{delete h.state.onHash;h.state.anchorOffset=2};
  await h.tick();assert.deepEqual(h.messages.map(m=>m.type),['ping','clear']);
});
test('the observed 18-paragraph Docs window matches its 1020-unit native value',async()=>{
  const h=contentHarness();
  h.state.paragraphTexts=Array.from({length:17},(_,i)=>`Synthetic paragraph ${44+i} for the LibreType scrolling check.`);
  h.state.paragraphTexts.push('The agenda for tomorrow’s project meeting includes\u00a0');
  h.state.text=h.state.paragraphTexts.join('');
  assert.equal(h.state.text.length,1020);
  assert.equal(h.state.paragraphTexts.join('\n\n').length,1054);
  await h.tick();
  const snapshot=h.messages.find(m=>m.type==='observe').snapshot;
  assert.equal(snapshot.selection_utf16,1020);
  assert.equal(snapshot.text_hash,crypto.createHash('sha256').update(h.state.text).digest('hex'));
});

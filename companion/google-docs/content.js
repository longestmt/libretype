// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
// Read-only geometry adapter. No keys, clipboard, document edits, or text transport.
(() => {
  let busy=false;
  const rect = r => [r.x,r.y,r.width,r.height];
  const equal = (a,b) => a.every((v,i)=>Math.abs(v-b[i])<0.1);
  const hash = async text => [...new Uint8Array(await crypto.subtle.digest('SHA-256',new TextEncoder().encode(text)))].map(v=>v.toString(16).padStart(2,'0')).join('');
  const clear = () => chrome.runtime.sendMessage({type:'clear'}).catch(()=>{});
  function observe() {
    if (document.visibilityState!=='visible' || !document.hasFocus()) return null;
    const frame=document.querySelector('iframe.docs-texteventtarget-iframe');
    if (!frame || document.activeElement!==frame) return null;
    const doc=frame.contentDocument;
    const editor=doc?.activeElement;
    if (!editor || editor.getAttribute('role')!=='textbox' || !editor.isContentEditable) return null;
    const selection=doc.getSelection();
    if (!selection?.isCollapsed || selection.rangeCount!==1 || !editor.contains(selection.anchorNode)) return null;
    const cursor=document.querySelector('#kix-current-user-cursor-caret');
    const field=document.querySelector('.kix-appview-editor');
    if (!cursor || !field || getComputedStyle(cursor).display==='none') return null;
    const caret=rect(cursor.getBoundingClientRect());
    const area=rect(field.getBoundingClientRect());
    const proxy=rect(selection.getRangeAt(0).getBoundingClientRect());
    const offset=frame.getBoundingClientRect(); proxy[0]+=offset.x; proxy[1]+=offset.y;
    const width=innerWidth, height=innerHeight;
    const left=Math.max(0,area[0]), top=Math.max(0,area[1]);
    const right=Math.min(width,area[0]+area[2]), bottom=Math.min(height,area[1]+area[3]);
    if (caret[3]<=0 || proxy[3]<=0 || proxy[1]>=-1000 || caret[0]<left || caret[1]<top
      || caret[0]+caret[2]>right || caret[1]+caret[3]>bottom) return null;
    const text=editor.innerText;
    if (typeof text!=='string' || text.length>32768) return null;
    return {text,viewport:[width,height],field:[left,top,right-left,bottom-top],caret,proxy_caret:proxy};
  }
  async function tick() {
    if (busy) return;
    busy=true;
    try {
      if (document.visibilityState!=='visible' || !document.hasFocus()) { await clear(); return; }
      const status=await chrome.runtime.sendMessage({type:'ping'});
      if (!status?.enabled) return;
      const before=observe(); if (!before) { await clear(); return; }
      const page_key=await hash(location.origin+location.pathname);
      const text_hash=await hash(before.text);
      const after=observe();
      if (!after || before.text!==after.text || !equal(before.caret,after.caret)
        || !equal(before.proxy_caret,after.proxy_caret) || !equal(before.viewport,after.viewport)) { await clear(); return; }
      await chrome.runtime.sendMessage({type:'observe',snapshot:{page_key,text_hash,viewport:after.viewport,
        field:after.field,caret:after.caret,proxy_caret:after.proxy_caret}});
    } catch { await clear(); }
    finally { busy=false; }
  }
  document.addEventListener('visibilitychange',clear);
  window.addEventListener('blur',clear);
  window.addEventListener('pagehide',clear);
  setInterval(tick,200);
})();

// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
const setupButton=document.getElementById('setup');
const result=document.getElementById('result');
chrome.runtime.sendMessage({type:'status'}).then(reply=>{
  document.getElementById('status').textContent=reply?.enabled?'Local companion enabled.':'Local companion is off or unavailable.';
  setupButton.disabled=!reply?.enabled;
},()=>{document.getElementById('status').textContent='Local companion is unavailable.';});
setupButton.addEventListener('click',async()=>{
  setupButton.disabled=true;
  result.textContent='Setting up the current document…';
  const messages={
    saved:'Settings saved. Click in the document and try typing.',
    off:'Open LibreType and enable its Google Docs companion first.',
    not_docs:'Open an editable Google Doc in this window, then try again.',
    unsupported:'These controls were not recognized. Use the manual setup steps below.',
    busy:'Setup is already running. Wait a moment before trying again.',
    review:'Setup stopped. Review or cancel any open Docs dialog, then use the manual steps below.',
  };
  try {
    const reply=await chrome.runtime.sendMessage({type:'setup'});
    result.textContent=messages[reply?.result] || messages.review;
  } catch { result.textContent=messages.review; }
  finally { setupButton.disabled=false; }
});

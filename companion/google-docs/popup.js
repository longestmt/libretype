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
    not_docs:'The selected tab is not a supported Google Doc. Open the document, then reopen this popup.',
    no_tab:'No active tab was found in this popup’s window. Reopen it from the Docs window’s toolbar.',
    inactive_tab:'The document tab is no longer active. Reopen this popup from that tab.',
    window_unfocused:'Chrome reports the document window is not focused. Close this popup, click the document, and reopen it from that window’s toolbar.',
    focus_changed:'The active tab or window changed during setup. Reopen this popup from the document and retry.',
    unavailable_url:'Chrome did not expose this tab’s address. Check that this extension has site access to docs.google.com, then reload the document.',
    tab_unavailable:'Chrome could not inspect the selected tab. Reload the document and reopen this popup.',
    unsupported:'These controls were not recognized. Use the manual setup steps below.',
    busy:'Setup is already running. Wait a moment before trying again.',
    review:'Setup stopped. Review or cancel any open Docs dialog, then use the manual steps below.',
  };
  try {
    // Resolve in the popup context, not the worker's last-focused window.
    const tabs=await chrome.tabs.query({active:true,currentWindow:true});
    const reply=tabs.length===1 && Number.isInteger(tabs[0].id) && tabs[0].id>=0
      ? await chrome.runtime.sendMessage({type:'setup',tabId:tabs[0].id})
      : {result:'no_tab'};
    result.textContent=messages[reply?.result] || messages.review;
  } catch { result.textContent=messages.review; }
  finally { setupButton.disabled=false; }
});

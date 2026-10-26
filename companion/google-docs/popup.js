// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
const messages={
    saved:'Settings saved. Click in the document and try typing.',
    starting:'Setup is waiting for the document to regain focus. Close this popup; reopen it afterward to see the result.',
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
const stoppedMessages={
  focus_changed:'The active document or window changed during setup. Return to the document, review or cancel any open dialog, then retry.',
  off:'The local companion became unavailable during setup. Check LibreType, then review or cancel any open Docs dialog before retrying.',
  expired:'Setup took too long. Review or cancel any open Docs dialog, then retry.',
  guard_denied:'Setup could not confirm the active document. Reload the document, review any open dialog, then retry.',
  connection_lost:'The setup connection was lost. Reload the document after reloading the extension, then retry.',
  menu_timeout:'The Tools menu did not expose the expected Accessibility option. Use the manual setup steps below.',
  dialog_timeout:'The Accessibility settings dialog did not open. Use the manual setup steps below.',
  options_timeout:'The required accessibility options did not become available. Review or cancel the open dialog, then use the manual steps below.',
  save_timeout:'The settings dialog did not close after Save. Check the open dialog; saving is not confirmed.',
  existing_dialog:'A Docs dialog was already open. Finish or cancel it before running setup.',
  controls_unavailable:'The expected accessibility controls were unavailable. Review or cancel any open dialog, then use the manual steps below.',
  controls_changed:'The accessibility dialog or another option changed during setup. Nothing was saved automatically; review or cancel the dialog.',
  offscreen:'A required setup control is outside the visible window. Review or cancel any open dialog, enlarge the window, then retry.',
  unexpected:'Setup could not finish. Review or cancel any open dialog, then use the manual steps below.',
};
function setupMessage(reply) {
  return reply?.result==='review' && Object.hasOwn(stoppedMessages,reply.reason)
    ? stoppedMessages[reply.reason] : (Object.hasOwn(messages,reply?.result) ? messages[reply.result] : messages.review);
}
const setupButton=document.getElementById('setup');
const result=document.getElementById('result');
chrome.runtime.sendMessage({type:'status'}).then(reply=>{
  document.getElementById('status').textContent=reply?.enabled?'Local companion enabled.':'Local companion is off or unavailable.';
  setupButton.disabled=!reply?.enabled || reply?.setupResult==='starting';
  if(Object.hasOwn(messages,reply?.setupResult))result.textContent='Last setup: '+setupMessage({result:reply.setupResult,reason:reply.setupReason});
},()=>{document.getElementById('status').textContent='Local companion is unavailable.';});
setupButton.addEventListener('click',async()=>{
  setupButton.disabled=true;
  result.textContent='Setting up the current document…';

  try {
    // Resolve in the popup context, not the worker's last-focused window.
    const tabs=await chrome.tabs.query({active:true,currentWindow:true});
    const reply=tabs.length===1 && Number.isInteger(tabs[0].id) && tabs[0].id>=0
      ? await chrome.runtime.sendMessage({type:'setup',tabId:tabs[0].id})
      : {result:'no_tab'};
    result.textContent=setupMessage(reply);
    if(reply?.result==='starting')window.close();
  } catch { result.textContent=messages.review; }
  finally { setupButton.disabled=false; }
});

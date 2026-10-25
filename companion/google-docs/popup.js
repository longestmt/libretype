// SPDX-License-Identifier: GPL-3.0-only
// Copyright (c) 2026 LibreTypist contributors
chrome.runtime.sendMessage({type:'status'}).then(reply=>{
  document.getElementById('status').textContent=reply?.enabled?'Local companion enabled.':'Local companion is off or unavailable.';
},()=>{document.getElementById('status').textContent='Local companion is unavailable.';});

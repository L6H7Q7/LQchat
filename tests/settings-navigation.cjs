// Disposable browser fixture: exercises the real UI without changing user settings.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawn } = require('node:child_process');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const root = path.resolve(__dirname, '..');
const artifacts = path.join(root, 'artifacts/settings-navigation');
async function poll(fn) {
  for (let i = 0; i < 200; i++) {
    try { const result = await fn(); if (result) return result; } catch (_) {}
    await sleep(100);
  }
  throw Error('Timed out waiting for test state');
}
async function main() {
  fs.mkdirSync(artifacts, { recursive: true });
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'lanchat-settings-qa-'));
  const server = spawn(process.execPath, ['tests/notification-preview.cjs'], { cwd: root, windowsHide: true, stdio: 'ignore' });
  let browser, ws;
  try {
    await poll(async () => (await fetch('http://127.0.0.1:18764/')).ok);
    browser = spawn('C:/Program Files/Google/Chrome/Application/chrome.exe', [
      '--headless=new', '--no-first-run', '--no-default-browser-check',
      '--remote-debugging-port=19338', `--user-data-dir=${profile}`,
      'http://127.0.0.1:18764/?android-preview=1',
    ], { windowsHide: true, stdio: 'ignore' });
    const page = await poll(async () => (await (await fetch('http://127.0.0.1:19338/json/list')).json())
      .find(p => p.url.includes('android-preview=1')));
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
    let sequence = 0;
    const pending = new Map();
    ws.onmessage = event => {
      const m = JSON.parse(event.data), entry = pending.get(m.id);
      if (!entry) return;
      pending.delete(m.id);
      if (m.error || m.result?.exceptionDetails) entry.reject(m.error || m.result.exceptionDetails);
      else entry.resolve(m.result);
    };
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++sequence;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params }));
    });
    const evaluate = async expression => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value;
    await send('Emulation.setDeviceMetricsOverride', { width: 450, height: 800, deviceScaleFactor: 2, mobile: true });
    await poll(() => evaluate('!!document.querySelector(".ns-app-picker-entry")'));
    await evaluate(`(() => {
      const original = window.__TAURI__.core.invoke;
      window.qa = {writes: 0, fail: '', delay: 0, permission:'granted', permissionRequests:0, permissionMode:'deny', background: {keep_running:false,start_on_boot:false,exclude_from_recents:false,battery_alert_enabled:false,battery_alert_interval_seconds:5,battery_alert_repeat_count:3,battery_alert_levels:[50,100]}};
      qa.requestPermission = async () => {
        qa.permissionRequests++;
        if (qa.permissionMode === 'hang') return new Promise(resolve => {qa.resolvePermission=resolve;});
        if (qa.permissionMode === 'throw') throw Error('QA permission error');
        if (qa.permissionMode === 'grant') qa.permission='granted';
        return qa.permission;
      };
      window.__TAURI__.core.invoke = async (command, args) => {
        if (command === 'notification_settings' && !args?.settings) {
          qa.accessReads=(qa.accessReads||0)+1;
          if(qa.accessDelay) await new Promise(resolve=>setTimeout(resolve,qa.accessDelay));
          if(qa.accessFail) throw Error('QA access read failure');
          const result=await original(command,args);
          return {...result,access:qa.access??result.access};
        }
        if (command === 'notification_settings' && args?.settings) {
          qa.notificationWrites=(qa.notificationWrites||0)+1;
          return original(command,args);
        }
        if (command === 'request_storage_permission') {qa.directoryRequests=(qa.directoryRequests||0)+1;return;}
        if (command === 'stop_background_receive_and_exit') {qa.stopRequests=(qa.stopRequests||0)+1;if(qa.stopFail)throw Error('QA stop error');return;}
        if (command === 'get_background_receive_state') return {state:qa.runtimeState||'RUNNING',last_error_message:null};
        if (command === 'get_notification_permission_state') return qa.permission;
        if (command === 'plugin:notification|request_permission') return qa.requestPermission();
        if (command === 'get_background_runtime_settings') return {...qa.background};
        if (command === 'update_settings' || command === 'set_background_runtime_settings' ||
            (command === 'notification_action' && args.action === 'replace_allowed')) {
          qa.writes++;
          if (qa.delay) await new Promise(r => setTimeout(r, qa.delay));
          if (qa.fail) throw qa.fail;
        }
        if (command === 'set_background_runtime_settings') {qa.background = {...args.settings}; return qa.background;}
        return original(command, args);
      };
    })()`);
    const click = async selector => {
      assert(await evaluate(`!!document.querySelector(${JSON.stringify(selector)})`), selector);
      await evaluate(`(() => {const node=document.querySelector(${JSON.stringify(selector)});if(node.click)node.click();else node.dispatchEvent(new MouseEvent('click',{bubbles:true}));})()`);
    };
    const state = () => evaluate(`({hash:location.hash, settings:document.getElementById('settings-panel').style.display === 'block',
      permissions:document.getElementById('permissions-panel').classList.contains('is-open'),
      apps:document.getElementById('android-push-apps-panel').classList.contains('is-open'),
      lqPush:document.getElementById('android-lq-push-panel').classList.contains('is-open'),
      notifications:!document.querySelector('.ns-detail').hidden,
      receiveSettings:document.getElementById('android-receive-settings-panel').style.display === 'block',
      toast:document.querySelector('.message-action-toast')?.textContent || '', writes:qa.writes})`);
    const expectPage = async (hash, permissions = false, apps = false) => {
      const s = await poll(async () => { const s = await state(); return s.hash === hash && s.permissions === permissions && s.apps === apps && s; });
      assert.equal(s.settings, ['#settings', '#permissions', '#push-apps', '#lq-push'].includes(hash));
    };
    const openSettings = async () => {
      await click('#android-settings-btn');
      await expectPage('#settings');
      await poll(() => evaluate('!document.getElementById("save-settings-btn").disabled'));
    };
    const back = async () => { await evaluate('history.back()'); await sleep(100); };
    const screenshot = async name => {
      await sleep(80);
      const result = await send('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
      fs.writeFileSync(path.join(artifacts, name + '.png'), Buffer.from(result.data, 'base64'));
    };
    const passed = [];
    for (const width of [450, 360]) {
      await send('Emulation.setDeviceMetricsOverride', {width,height:800,deviceScaleFactor:2,mobile:true});
      const layout=await evaluate(`(() => {
        const inspect=card=>{const count=card.querySelector('.android-count-label').getBoundingClientRect(),refresh=card.querySelector('.android-icon-btn').getBoundingClientRect(),add=card.querySelector('.android-add-btn')?.getBoundingClientRect();return{sameRow:Math.abs((count.top+count.height/2)-(refresh.top+refresh.height/2))<1,addBelow:!add||add.top>=Math.min(count.bottom,refresh.bottom)-1,hasAdd:!!add};};
        return{width:innerWidth,scrollWidth:document.documentElement.scrollWidth,chat:inspect(document.querySelector('.android-chat-card')),receive:inspect(document.querySelector('.android-receive-card'))};
      })()`);
      assert.equal(layout.scrollWidth,width);
      assert(layout.chat.sameRow && layout.receive.sameRow);
      if(width<=420) assert(layout.chat.addBelow);
      assert.equal(layout.receive.hasAdd, false);
      await screenshot('home-toolbar-'+width);
    }
    await send('Emulation.setDeviceMetricsOverride', {width:450,height:800,deviceScaleFactor:2,mobile:true});
    await click('#android-receive-list .ns-device');
    await poll(async()=>{const s=await state();return s.hash==='#notifications'&&s.notifications;});
    assert.equal(await evaluate(`document.querySelectorAll('.ns-detail-header > button').length`),1);
    assert.equal(await evaluate(`document.querySelector('.ns-detail-header').textContent.includes('设置')`),false);
    await click('.ns-detail-header > button:first-child');
    await expectPage('');
    passed.push('Receive detail omits its former settings action and returns home');
    passed.push('Device counts stay aligned with refresh actions at regular and narrow Android widths');
    await openSettings();
    for (const width of [450, 360]) {
      await send('Emulation.setDeviceMetricsOverride', {width,height:800,deviceScaleFactor:2,mobile:true});
      const layout=await evaluate(`(() => {
        const row=document.getElementById('android-download-location-btn'),value=document.getElementById('android-download-value');
        const r=row.getBoundingClientRect(),v=value.getBoundingClientRect(),arrow=row.querySelector('svg').getBoundingClientRect();
        return {width:innerWidth,scrollWidth:document.documentElement.scrollWidth,align:getComputedStyle(value).textAlign,
          right:r.right,valueRight:v.right,arrowRight:arrow.right,arrowWidth:arrow.width,stroke:getComputedStyle(row.querySelector('svg')).stroke,
          legacyHidden:getComputedStyle(document.querySelector('.desktop-download-setting')).display==='none',
          textFits:value.scrollWidth<=value.clientWidth};
      })()`);
      assert.equal(layout.scrollWidth,width);
      assert.equal(layout.align,'right');
      assert.equal(layout.arrowWidth,18);
      assert(Math.abs(layout.arrowRight-layout.right)<1);
      assert(layout.legacyHidden && layout.textFits);
      await screenshot('download-row-'+width);
    }
    await click('.android-download-label');
    await click('#android-download-value');
    await click('#android-download-location-btn svg');
    assert.equal(await evaluate('qa.directoryRequests'),3);
    await evaluate(`window.dispatchEvent(new CustomEvent('android-download-directory-selected',{detail:{uri:'content://qa/tree/example',label:'A very long folder name '.repeat(8)}}))`);
    assert(await evaluate(`document.getElementById('android-download-value').scrollWidth>document.getElementById('android-download-value').clientWidth`));
    assert.equal(await evaluate('document.documentElement.scrollWidth'),360);
    await screenshot('download-row-long-name');
    await back(); await expectPage('');
    assert.equal((await state()).writes,0);
    await openSettings();
    await click('#android-permissions-btn');
    await expectPage('#permissions',true);
    assert.equal(await evaluate(`document.querySelectorAll('#permissions-panel .android-permission-copy small').length`),0);
    assert(await evaluate(`!!document.getElementById('android-receive-toggle').closest('.toggle-switch')`));
    assert(await evaluate(`!!document.getElementById('auto-download-toggle').closest('.toggle-switch')`));
    assert(await evaluate(`!!document.getElementById('battery-alert-toggle').closest('.toggle-switch')`));
    assert.equal(await evaluate(`document.querySelectorAll('#battery-alert-level-list .battery-alert-level-chip').length`),2);
    assert(await evaluate(`document.getElementById('battery-alert-controls').disabled`));
    assert(await evaluate(`!!document.querySelector('.ns-push-panel > .ns-toggle-row .toggle-switch')`));
    await poll(()=>evaluate(`document.getElementById('android-notification-access-btn').textContent!=='检查'`));
    await evaluate(`(async()=>{qa.access=false;await NotificationUI.refreshSettings();qa.savedName=document.getElementById('settings-device-name-input').value;document.getElementById('settings-device-name-input').value='Unsaved QA name';qa.oldKeep=document.getElementById('background-keep-running-toggle').checked;document.getElementById('background-keep-running-toggle').checked=!qa.oldKeep;qa.pushNode=document.querySelector('#android-notification-settings input');qa.nameNode=document.getElementById('settings-device-name-input');})()`);
    const accessStyle=()=>evaluate(`(()=>{const n=document.getElementById('android-notification-access-btn'),s=getComputedStyle(n),peer=getComputedStyle(document.querySelector('.android-status-ok'));return {text:n.textContent,color:s.color,background:s.backgroundColor,peerColor:peer.color,font:s.fontSize,peerFont:peer.fontSize,authorized:n.classList.contains('is-authorized')};})()`);
    assert.equal((await accessStyle()).authorized,false);
    await evaluate(`qa.access=true;qa.accessDelay=150;qa.readsBefore=qa.accessReads;Object.defineProperty(document,'visibilityState',{configurable:true,value:'visible'});document.dispatchEvent(new Event('visibilitychange'));window.dispatchEvent(new Event('focus'));`);
    await poll(async()=>(await accessStyle()).authorized);
    const authorized=await accessStyle();
    assert.equal(authorized.text,'已授权');
    assert.equal(authorized.color,authorized.peerColor);
    assert.equal(authorized.font,authorized.peerFont);
    assert.equal(authorized.background,'rgba(0, 0, 0, 0)');
    assert.equal(await evaluate('qa.accessReads-qa.readsBefore'),1);
    assert(await evaluate(`qa.nameNode===document.getElementById('settings-device-name-input') && qa.pushNode===document.querySelector('#android-notification-settings input') && qa.nameNode.value==='Unsaved QA name' && document.getElementById('background-keep-running-toggle').checked===!qa.oldKeep`));
    assert.equal((await state()).writes,0);
    await screenshot('notification-access-authorized');
    await evaluate(`qa.access=false;qa.accessFail=true;document.dispatchEvent(new Event('visibilitychange'));`);
    await sleep(250);
    assert.equal((await accessStyle()).authorized,true);
    await evaluate(`qa.accessFail=false;document.dispatchEvent(new Event('visibilitychange'));`);
    await poll(async()=>!(await accessStyle()).authorized);
    assert.equal((await accessStyle()).text,'去授权');
    assert.notEqual((await accessStyle()).background,'rgba(0, 0, 0, 0)');
    await screenshot('notification-access-not-authorized');
    await evaluate(`delete document.visibilityState;qa.accessDelay=0;qa.nameNode.value=qa.savedName;document.getElementById('background-keep-running-toggle').checked=qa.oldKeep;`);
    passed.push('Access status refreshes on visibility without focus, matches green status styling, deduplicates events, preserves drafts, and retries failed reads');
    assert.equal(await evaluate(`!!document.getElementById('stop-background-service-btn')`),false);
    assert.equal(await evaluate(`getComputedStyle(document.querySelector('.background-receive-actions')).display`),'none');
    await screenshot('running-status-action');
    await evaluate(`window.qaOriginalConfirm=window.confirm;window.confirm=()=>{qa.confirmations=(qa.confirmations||0)+1;return false;}`);
    await click('#background-receive-status');
    assert.equal(await evaluate('qa.confirmations'),1);
    assert.equal(await evaluate('qa.stopRequests||0'),0);
    await evaluate('window.confirm=()=>true');
    await click('#background-receive-status');
    assert.equal(await evaluate('qa.stopRequests'),1);
    await evaluate('qa.stopFail=true');
    await click('#background-receive-status');
    await poll(async()=>(await state()).toast.includes('QA stop error'));
    assert.equal(await evaluate(`document.getElementById('background-receive-status').disabled`),false);
    await evaluate(`window.confirm=window.qaOriginalConfirm;delete window.qaOriginalConfirm;qa.stopFail=false;qa.runtimeState='ERROR'`);
    await back(); await click('#android-permissions-btn');
    await poll(()=>evaluate(`document.getElementById('background-receive-status').disabled && !document.querySelector('.background-receive-actions').hidden`));
    await evaluate(`qa.runtimeState='RUNNING'`);
    await back(); await back(); await expectPage('');
    await send('Emulation.setDeviceMetricsOverride',{width:450,height:800,deviceScaleFactor:2,mobile:true});
    passed.push('Download row alignment, whole-row action, long-path ellipsis, and cancel without saving');
    passed.push('Running status confirms exit, cancel is safe, errors recover, and retry remains available');
    for (let i = 0; i < 3; i++) {
      await openSettings();
      await click('#android-permissions-btn');
      await expectPage('#permissions', true);
      await back(); await expectPage('#settings');
      await click('.ns-app-picker-entry');
      await expectPage('#push-apps', false, true);
      await back(); await expectPage('#settings');
      await click('#android-lq-push-settings-btn');
      await expectPage('#lq-push');
      assert.equal((await state()).lqPush, true);
      await back(); await expectPage('#settings');
      await back(); await expectPage('');
    }
    passed.push('Repeated settings / permissions / app-picker back navigation');
    await openSettings();
    await click('#android-permissions-btn');
    await click('#android-permissions-back-btn'); await expectPage('#settings');
    await click('.ns-app-picker-entry');
    await click('#android-push-apps-back-btn'); await expectPage('#settings');
    await click('#android-lq-push-settings-btn');
    await click('#android-lq-push-back-btn'); await expectPage('#settings');
    await click('#android-settings-back-btn'); await expectPage('');
    assert.equal((await state()).writes, 0);
    passed.push('Toolbar back matches system history, no implicit writes');
    await openSettings();
    await click('#save-settings-btn');
    await expectPage('');
    assert.equal((await state()).toast, '保存成功');
    await screenshot('settings-save-success');
    passed.push('Unchanged settings still show success after save');
    await openSettings();
    await click('#android-permissions-btn');
    await evaluate('document.getElementById("background-start-on-boot-toggle").checked = true');
    await evaluate('document.getElementById("battery-alert-toggle").checked = true');
    await evaluate(`document.getElementById('battery-alert-toggle').dispatchEvent(new Event('change'));document.getElementById('battery-alert-interval-input').value='12';document.getElementById('battery-alert-repeat-input').value='4';`);
    await evaluate(`window.qaOriginalPrompt=window.prompt;window.prompt=()=> '25';`);
    await click('#battery-alert-add-level-btn');
    await click('#battery-alert-level-list [data-level="50"]');
    await evaluate(`window.prompt=window.qaOriginalPrompt;delete window.qaOriginalPrompt;`);
    await click('#save-permissions-btn'); await expectPage('#settings');
    assert.equal((await state()).toast, '保存成功');
    assert.equal(await evaluate('qa.background.start_on_boot'), true);
    assert.equal(await evaluate('qa.background.battery_alert_enabled'), true);
    assert.equal(await evaluate('qa.background.battery_alert_interval_seconds'), 12);
    assert.equal(await evaluate('qa.background.battery_alert_repeat_count'), 4);
    assert.deepEqual(await evaluate('qa.background.battery_alert_levels'), [25,100]);
    await screenshot('permissions-save-success');
    passed.push('Background and battery-alert changes save with visible success feedback');
    await click('.ns-app-picker-entry');
    await poll(() => evaluate('!document.getElementById("android-push-apps-save-btn").disabled'));
    await click('#android-push-apps-save-btn'); await expectPage('#settings');
    assert.equal((await state()).toast, '保存成功');
    passed.push('App selection save returns to settings with success feedback');
    await click('#android-lq-push-settings-btn');
    await expectPage('#lq-push');
    assert.equal(await evaluate(`document.querySelector('#android-lq-push-panel input').checked`), false);
    await click('#android-lq-push-panel input');
    await poll(() => evaluate(`document.querySelector('#android-lq-push-panel .ns-save-status').textContent==='已保存'`));
    assert.equal(await evaluate('qa.notificationWrites > 0'), true);
    assert.equal(await evaluate(`document.querySelector('#android-lq-push-panel input').checked`), true);
    await back(); await expectPage('#settings');
    assert.equal(await evaluate(`document.querySelector('#android-lq-push-settings-btn .ns-app-picker-summary').textContent`), '电量提醒已开启');
    passed.push('LQ push management page persists the battery forwarding switch');
    await click('#android-permissions-btn');
    await evaluate('qa.fail = "QA persistence failure"');
    await click('#save-permissions-btn');
    await poll(async () => (await state()).toast.includes('QA persistence failure'));
    await expectPage('#permissions', true);
    assert(!(await state()).toast.includes('保存成功'));
    await screenshot('permissions-save-failure');
    await evaluate('qa.fail = ""');
    await back(); await expectPage('#settings');
    await click('.ns-app-picker-entry');
    await poll(() => evaluate('!document.getElementById("android-push-apps-save-btn").disabled'));
    await evaluate('qa.fail = "QA app selection failure"');
    await click('#android-push-apps-save-btn');
    await poll(async () => (await state()).toast.includes('QA app selection failure'));
    await expectPage('#push-apps', false, true);
    await evaluate('qa.fail = ""');
    await back(); await expectPage('#settings');
    passed.push('String errors stay visible; failed saves do not navigate');
    await evaluate('document.getElementById("port-input").value = "70000"');
    const writesBefore = (await state()).writes;
    await click('#save-settings-btn');
    await expectPage('#settings');
    assert.equal((await state()).writes, writesBefore);
    assert((await state()).toast && !(await state()).toast.includes('保存成功'));
    await evaluate('document.getElementById("port-input").value = "8888"; qa.delay = 200');
    await click('#save-settings-btn');
    await click('#save-settings-btn');
    await back(); await expectPage('');
    await poll(() => evaluate('!document.getElementById("save-settings-btn").disabled'));
    await expectPage('');
    assert.equal((await state()).writes - writesBefore, 2);
    passed.push('Validation, duplicate-save prevention, and back during pending save');
    await openSettings(); await back(); await expectPage('');
    assert.equal(await evaluate('qa.permissionRequests'), 0);
    passed.push('Already-granted permission never reaches the buggy request API');
    await openSettings();
    await click('#android-permissions-btn');
    await evaluate('qa.delay=0; qa.permission="denied"; qa.permissionMode="deny"');
    const beforePermissionTests = (await state()).writes;
    await click('#save-permissions-btn');
    await poll(async () => (await state()).toast.includes('系统通知权限未获允许'));
    await expectPage('#permissions', true);
    assert.equal((await state()).writes, beforePermissionTests);
    await evaluate('qa.permissionMode="throw"');
    await click('#save-permissions-btn');
    await poll(async () => (await state()).toast.includes('QA permission error'));
    assert.equal((await state()).writes, beforePermissionTests);
    passed.push('Permission denial and request errors cause no configuration writes');
    await evaluate('qa.permissionMode="hang"; window.__TAURI__.notification={requestPermission:qa.requestPermission}');
    await click('#save-permissions-btn');
    assert.equal(await evaluate('document.getElementById("save-permissions-btn").textContent'), '正在保存…');
    assert.equal(await evaluate('document.getElementById("save-permissions-btn").disabled'), true);
    await screenshot('permission-waiting');
    const timeoutStarted = Date.now();
    await poll(async () => (await state()).toast.includes('通知授权等待超时'));
    assert(Date.now()-timeoutStarted >= 14000);
    await expectPage('#permissions', true);
    assert.equal(await evaluate('document.getElementById("save-permissions-btn").disabled'), false);
    assert.equal(await evaluate('document.getElementById("save-permissions-btn").textContent'), '保存权限设置');
    assert.equal((await state()).writes, beforePermissionTests);
    await screenshot('permission-timeout');
    await evaluate('qa.permission="granted"; qa.resolvePermission("granted")');
    await sleep(200);
    assert.equal((await state()).writes, beforePermissionTests);
    assert(!(await state()).toast.includes('保存成功'));
    passed.push('15-second timeout unlocks buttons; late permission reply cannot resume saving');
    await click('#save-permissions-btn'); await expectPage('#settings');
    assert.equal((await state()).toast, '保存成功');
    await back(); await expectPage('');
    passed.push('Retry after permission recovery saves successfully');
    const report = { passed, final: await state() };
    fs.writeFileSync(path.join(artifacts, 'preview-results.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
    await send('Browser.close');
  } finally {
    ws?.close();
    browser?.kill();
    server.kill();
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

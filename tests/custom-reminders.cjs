// Isolated real-browser verification. Uses a disposable adapter; never reads phone settings or sends notifications.
const fs=require('node:fs'),path=require('node:path'),os=require('node:os');
const assert=require('node:assert/strict');const {spawn}=require('node:child_process');
const root=path.resolve(__dirname,'..'), out=path.join(root,'artifacts/custom-reminders');
const sleep=ms=>new Promise(r=>setTimeout(r,ms));
async function poll(fn){for(let i=0;i<150;i++){try{const value=await fn();if(value)return value;}catch{}await sleep(100);}throw Error('Timed out');}
async function main(){
  fs.mkdirSync(out,{recursive:true});
  const profile=fs.mkdtempSync(path.join(os.tmpdir(),'lq-custom-reminders-'));
  const debugPort=20000+Math.floor(Math.random()*10000);
  const server=spawn(process.execPath,['tests/notification-preview.cjs'],{cwd:root,windowsHide:true,stdio:'ignore'});
  let browser,ws;
  try{
    await poll(async()=> (await fetch('http://127.0.0.1:18764/')).ok);
    browser=spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--disable-extensions','--no-first-run','--no-default-browser-check','--remote-debugging-port='+debugPort,'--user-data-dir='+profile,'http://127.0.0.1:18764/?android-preview=1'],{windowsHide:true,stdio:'ignore'});
    const page=await poll(async()=> (await (await fetch('http://127.0.0.1:'+debugPort+'/json/list',{signal:AbortSignal.timeout(3000)})).json()).find(p=>p.url.includes('android-preview')));
    ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
    let sequence=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),entry=pending.get(m.id);if(!entry)return;pending.delete(m.id);if(m.error||m.result?.exceptionDetails)entry.reject(m.error||m.result.exceptionDetails);else entry.resolve(m.result);};
    const send=(method,params={})=>new Promise((resolve,reject)=>{const id=++sequence;const timeout=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout: '+method));},5000);pending.set(id,{resolve:v=>{clearTimeout(timeout);resolve(v);},reject:e=>{clearTimeout(timeout);reject(e);}});ws.send(JSON.stringify({id,method,params}));});
    const run=async expression=>(await send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true})).result.value;
    const click=selector=>run(`document.querySelector(${JSON.stringify(selector)}).click()`);
    await send('Emulation.setDeviceMetricsOverride',{width:390,height:844,deviceScaleFactor:1,mobile:true});
    await poll(()=>run('document.querySelector("#cr-add") && !document.querySelector("#cr-add").disabled'));
    await click('#settings-btn');
    await poll(()=>run('document.querySelector("#battery-alert-repeat-input").value==="1"'));
    assert.equal(await run('document.querySelector("#battery-alert-interval-input").value'),'');
    assert.equal(await run('document.querySelector("#battery-alert-interval-input").disabled'),true);
    assert.equal(await run('document.querySelector(".android-permissions-card").lastElementChild.classList.contains("shared-reminder-settings")'),true);
    assert.equal(await run('document.querySelector(".shared-reminder-settings strong, .shared-reminder-settings small")'),null);
    await run(`document.querySelector('#battery-alert-repeat-input').value='2';document.querySelector('#battery-alert-repeat-input').dispatchEvent(new Event('change'));`);
    await poll(()=>run('document.querySelector("#shared-reminder-settings-status").textContent.includes("请设置")'));
    assert.equal(await run('document.querySelector("#battery-alert-repeat-input").value'),'2');
    assert.equal(await run('document.querySelector("#battery-alert-interval-input").disabled'),false);
    assert.equal(await run('window.__customReminderFixture.backgroundWrites.length'),0);
    await run(`document.querySelector('#battery-alert-repeat-input').value='4';document.querySelector('#battery-alert-repeat-input').dispatchEvent(new Event('change'));document.querySelector('#battery-alert-interval-input').value='7';document.querySelector('#battery-alert-interval-input').dispatchEvent(new Event('change'));`);
    await poll(()=>run('window.__customReminderFixture.backgroundWrites.at(-1)?.battery_alert_repeat_count===4'));
    assert.equal(await run('window.__customReminderFixture.backgroundWrites.at(-1).battery_alert_interval_seconds'),7);
    await poll(()=>run('document.querySelector("#cr-next-rule").textContent.includes("1/4")'));
    assert.equal(await run('document.querySelector("#battery-alert-toggle").checked'),false);
    await run(`window.__customReminderFixture.failBackground=true;document.querySelector('#battery-alert-repeat-input').value='6';document.querySelector('#battery-alert-repeat-input').dispatchEvent(new Event('change'));`);
    await poll(()=>run('document.querySelector("#shared-reminder-settings-status").textContent.includes("模拟统一配置保存失败")'));
    assert.equal(await run('document.querySelector("#battery-alert-repeat-input").value'),'4');
    assert.equal(await run('window.__customReminderFixture.backgroundWrites.at(-1).battery_alert_repeat_count'),4);
    await run(`window.__customReminderFixture.failBackground=false;document.querySelector('#battery-alert-repeat-input').value='1';document.querySelector('#battery-alert-repeat-input').dispatchEvent(new Event('change'));`);
    await poll(()=>run('window.__customReminderFixture.backgroundWrites.at(-1)?.battery_alert_repeat_count===1'));
    assert.equal(await run('window.__customReminderFixture.backgroundWrites.at(-1).battery_alert_interval_seconds'),0);
    for(const width of [320,390]) {
      await send('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:true});
      assert.equal(await run('document.querySelector(".shared-reminder-settings").scrollWidth <= document.querySelector(".shared-reminder-settings").clientWidth'),true);
      assert.equal(await run('Math.abs(document.querySelector("#battery-alert-repeat-input").getBoundingClientRect().top-document.querySelector("#battery-alert-interval-input").getBoundingClientRect().top)<1'),true);
      const shot=await send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});fs.writeFileSync(path.join(out,'shared-settings-'+width+'.png'),Buffer.from(shot.data,'base64'));
    }
    await click('#android-notification-options-btn');
    await poll(()=>run('location.hash==="#notification-options"'));
    assert.deepEqual(await run('[...document.querySelectorAll("#cr-list .cr-row")].map(b=>b.textContent)'),['18:00今日收菜 · 今日提醒›']);
    await click('#cr-days button:nth-child(2)');assert.match(await run('document.querySelector("#cr-list").textContent'),/明日浇花/);
    assert.equal(await run('document.querySelector("#cr-next-title").textContent'),'今日收菜');
    await click('#cr-days button:nth-child(3)');assert.equal(await run('document.querySelector("#cr-list").textContent'),'当天暂无提醒');
    await click('#cr-add');assert.equal(await run('location.hash'),'#custom-reminder');
    assert.equal(await run('document.querySelector("#android-custom-reminder-panel .cr-next")'),null);
    await run(`document.querySelector('#cr-title').value='新增收菜';document.querySelector('#cr-content').value='新内容';document.querySelector('#cr-countdown').click();document.querySelector('#cr-near').click();`);
    assert.equal(await run('document.querySelector("#cr-near-fields").disabled'),false);
    assert.equal(await run('document.querySelector("#cr-form").checkValidity()'),true);
    for(const width of [320,390]){
      await send('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:true});
      assert.equal(await run('document.querySelector("#android-custom-reminder-panel").scrollWidth <= document.querySelector("#android-custom-reminder-panel").clientWidth'),true);
      const shot=await send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});fs.writeFileSync(path.join(out,'editor-'+width+'.png'),Buffer.from(shot.data,'base64'));
    }
    await click('#cr-save');await poll(()=>run('location.hash==="#notification-options"'));
    assert.equal(await run('document.querySelector("#cr-count").textContent'),'自定义通知（3）');
    await click('#cr-days button:nth-child(4)');await click('#cr-list .cr-row');
    await run(`document.querySelector('#cr-title').value='已编辑收菜';`);await click('#cr-save');await poll(()=>run('location.hash==="#notification-options"'));
    assert.match(await run('document.querySelector("#cr-list").textContent'),/已编辑收菜/);
    assert.equal(await run('document.querySelector("#cr-global-repeat")'),null);
    assert.match(await run('document.querySelector("#cr-next-rule").textContent'),/1\/1.*间隔无/);
    await run('window.__customReminderFixture.fail=true');await click('#cr-list .cr-row');await run(`document.querySelector('#cr-title').value='不应保存';`);await click('#cr-save');
    await poll(()=>run('document.querySelector("#cr-error").textContent.includes("模拟保存失败")'));assert.equal(await run('location.hash'),'#custom-reminder');
    await click('#cr-back');await poll(()=>run('location.hash==="#notification-options"'));assert.match(await run('document.querySelector("#cr-list").textContent'),/已编辑收菜/);
    await run('window.__customReminderFixture.fail=false;window.confirm=()=>true');await click('#cr-list .cr-row');await click('#cr-delete');await poll(()=>run('location.hash==="#notification-options"'));assert.equal(await run('document.querySelector("#cr-count").textContent'),'自定义通知（2）');
    for(const width of [320,390]){
      await send('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:true});await click('#cr-days button:first-child');
      assert.equal(await run('document.querySelector("#android-notification-options-panel").scrollWidth <= document.querySelector("#android-notification-options-panel").clientWidth'),true);
      const screen=await send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});fs.writeFileSync(path.join(out,'list-'+width+'.png'),Buffer.from(screen.data,'base64'));
    }
    await click('#android-notification-options-back-btn');await poll(()=>run('location.hash==="#settings"'));await click('#android-push-settings-btn');await poll(()=>run('location.hash==="#push-details"'));
    assert.equal(await run('document.querySelector("#android-push-details-panel h3").textContent'),'推送设置');
    assert.equal(await run('document.querySelector(".ns-app-picker-entry").textContent'),'应用信息推送（0）');
    assert.equal(await run('document.querySelector(".ns-app-picker-row")'),null);
    await click('.ns-app-picker-entry');await poll(()=>run('location.hash==="#push-apps" && document.querySelectorAll(".android-push-app-row").length===2'));
    await click('.android-push-app-row input');await click('#android-push-apps-save-btn');await poll(()=>run('location.hash==="#push-details"'));
    assert.equal(await run('document.querySelector(".ns-app-picker-entry").textContent'),'应用信息推送（1）');
    assert.equal(await run('document.querySelector("#android-push-details-panel input[aria-label=应用信息推送]").checked'),false);
    await click('.ns-app-picker-entry');await poll(()=>run('location.hash==="#push-apps" && document.querySelectorAll(".android-push-app-row").length===2'));
    await click('.android-push-app-row input');await click('#android-push-apps-save-btn');await poll(()=>run('location.hash==="#push-details"'));
    await poll(()=>run('!!document.querySelector("#android-push-details-panel input[aria-label=自定义通知推送]")'));
    assert.equal(await run('document.querySelector("#android-push-details-panel input[aria-label=自定义通知推送]").checked'),false);
    await run('[...document.querySelectorAll("#android-push-details-panel .ns-check-row")].find(n=>n.textContent.includes("IQOO")).querySelector("input").click()');
    await poll(()=>run('window.__customReminderFixture.notificationWrites.at(-1)?.target_device_ids.includes("iqoo")'));
    await click('#android-push-details-panel input[aria-label=自定义通知推送]');
    await poll(()=>run('window.__customReminderFixture.notificationWrites.at(-1)?.lq_reminder_push_enabled===true'));
    const snapshot=await run('window.__customReminderFixture.notificationWrites.at(-1)');assert.deepEqual(snapshot.allowed_packages,[]);assert.deepEqual(snapshot.target_device_ids,['iqoo']);assert.equal(snapshot.lq_battery_push_enabled,false);
    await click('#android-push-details-back-btn');await poll(()=>run('location.hash==="#settings"'));await click('#android-push-settings-btn');
    assert.equal(await run('document.querySelector("#android-push-details-panel input[aria-label=自定义通知推送]").checked'),true);
    await run('window.__customReminderFixture.failNotificationSettings=true');await click('#android-push-details-panel input[aria-label=自定义通知推送]');
    await poll(()=>run('document.querySelector("#android-push-details-panel .ns-save-status").textContent.includes("模拟推送设置保存失败")'));
    assert.equal(await run('document.querySelector("#android-push-details-panel input[aria-label=自定义通知推送]").checked'),true);
    const shot=await send('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});fs.writeFileSync(path.join(out,'reminder-forwarding.png'),Buffer.from(shot.data,'base64'));
    console.log('PASS: shared single/no-gap defaults, multi-repeat validation, shared save and failure rollback, app picker label/count with independent switch, reminder navigation and editing, 320/390px layouts, forwarding persistence and rollback.');
    await send('Browser.close');
  }finally{ws?.close();browser?.kill();server.kill();}
}
main().catch(e=>{console.error(e);process.exitCode=1;});

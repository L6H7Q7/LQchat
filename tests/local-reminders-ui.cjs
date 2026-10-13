// Visible, disposable browser verification. No real device settings or notifications are used.
const fs=require('node:fs'),path=require('node:path'),os=require('node:os'),assert=require('node:assert/strict');
const {spawn}=require('node:child_process');
const root=path.resolve(__dirname,'..'), output=process.env.LQ_TEST_OUTPUT || path.join(root,'artifacts/test-19.14');
const sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms));
async function poll(fn){for(let i=0;i<120;i++){const result=await fn().catch(()=>false);if(result)return result;await sleep(100);}throw Error('Fixture timeout');}
async function main(){
  fs.mkdirSync(output,{recursive:true});
  const port=22000+Math.floor(Math.random()*2000),debug=port+4000,profile=fs.mkdtempSync(path.join(os.tmpdir(),'lq-local-reminders-'));
  const server=spawn(process.execPath,['tests/notification-preview.cjs'],{cwd:root,env:{...process.env,LQ_NOTIFICATION_PREVIEW_PORT:String(port)},stdio:'inherit'});
  let browser,ws,passed=false;
  try{
    await poll(async()=> (await fetch(`http://127.0.0.1:${port}/`)).ok);
    browser=spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',[
      '--no-first-run','--no-default-browser-check','--disable-extensions','--user-data-dir='+profile,
      '--remote-debugging-port='+debug,`http://127.0.0.1:${port}/?desktop-preview=1`],{stdio:'ignore'});
    const page=await poll(async()=> (await (await fetch(`http://127.0.0.1:${debug}/json/list`)).json()).find(p=>p.url.includes('desktop-preview')));
    ws=new WebSocket(page.webSocketDebuggerUrl);await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j;});
    let sequence=0;const pending=new Map();
    ws.onmessage=e=>{const m=JSON.parse(e.data),entry=pending.get(m.id);if(!entry)return;pending.delete(m.id);
      if(m.error||m.result?.exceptionDetails)entry.reject(m.error||m.result.exceptionDetails);else entry.resolve(m.result);};
    const send=(method,params={})=>new Promise((resolve,reject)=>{const id=++sequence,timer=setTimeout(()=>{pending.delete(id);reject(Error('CDP timeout'));},8000);
      pending.set(id,{resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}});ws.send(JSON.stringify({id,method,params}));});
    const run=async expression=>(await send('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true})).result.value;
    const click=selector=>run(`document.querySelector(${JSON.stringify(selector)}).click()`);
    const checkLocalConnections=async selector=>{
      await run(`(async () => {const f=window.__notificationHistoryFixture,b=f.records[0];
        f.peers.forEach(p=>{p.notification_push_enabled=false;p.notification_push_target_device_ids=[];});
        window.__customReminderFixture.failNotificationSettings=false;
        const config=await window.__TAURI__.core.invoke('notification_settings');
        await window.__TAURI__.core.invoke('notification_settings',{settings:{...config.settings,target_device_ids:[]}});
        await window.NotificationUI.refreshSettings();window.NotificationUI.onPeers(f.peers);
        f.records.splice(0,f.records.length,...['连接','断开'].map((state,i)=>({...b,record_id:'local-peer-'+i,peer_id:'preview-local',view_kind:'notification_local',
          notification:{...b.notification,event_id:'local-peer-'+i,source_device_id:'preview-local',target_device_id:'preview-local',package:'com.lanchat.app.connection',
            app_name:'LQChat',title:'设备'+state,text:'安卓 B 已'+state,notification_key:'presence:android-b'}})));f.emit();})()`);
      await poll(()=>run(`document.querySelectorAll(${JSON.stringify(selector)}).length===1`));
      await click(selector);
      await poll(()=>run('document.querySelectorAll(".ns-card").length===2'));
      assert.deepEqual((await run('[...document.querySelectorAll(".ns-card .ns-body")].map(n=>n.textContent)')).sort(),['安卓 B 已断开','安卓 B 已连接'].sort());
    };
    await send('Emulation.setDeviceMetricsOverride',{width:1100,height:850,deviceScaleFactor:1,mobile:false});
    await poll(()=>run('!!document.querySelector(".desktop-local-reminder-entry") && !!document.querySelector(".ns-device[data-peer=preview-local]")'));
    assert.match(await run('document.querySelector(".ns-device[data-peer=preview-local] .ns-name").textContent'),/我的电脑.*本机/);
    await run(`(() => {const f=window.__notificationHistoryFixture,b=f.records[0];f.records.push({...b,record_id:'local-battery',peer_id:'preview-local',peer_name:'我的电脑',view_kind:'notification_local',
      notification:{...b.notification,event_id:'local-battery',source_device_id:'preview-local',target_device_id:'preview-local',package:'com.lanchat.app',app_name:'LQChat',title:'电量提醒',text:'50%',notification_key:'lq-battery-50'}});f.emit();})()`);
    await click('.ns-device[data-peer=preview-local]');
    await poll(()=>run('document.querySelector(".ns-card .ns-title")?.textContent==="电量"'));
    assert.equal(await run('document.querySelector(".ns-card .ns-body").textContent'),'50%');
    assert.equal(await run('window.__notificationHistoryFixture.queries.some(q=>q.direction==="local"&&q.device_id==="preview-local")'),true);
    await click('#settings-btn');
    await poll(()=>run('document.querySelector("#battery-alert-repeat-input").value==="1"'));
    await click('.desktop-local-reminder-entry');
    await poll(()=>run('document.querySelector("#android-notification-options-panel").classList.contains("is-open") && !document.querySelector("#cr-add").disabled'));
    assert.equal(await run('document.querySelector("#battery-alert-toggle").disabled'),false);
    assert.match(await run('document.querySelector("#cr-permission").textContent'),/托盘/);
    await run(`document.querySelector('#battery-alert-repeat-input').value='3';document.querySelector('#battery-alert-repeat-input').dispatchEvent(new Event('change'));
      document.querySelector('#battery-alert-interval-input').value='5';document.querySelector('#battery-alert-interval-input').dispatchEvent(new Event('change'));`);
    await poll(()=>run('window.__customReminderFixture.backgroundWrites.at(-1)?.battery_alert_repeat_count===3'));
    assert.equal(await run('window.__customReminderFixture.backgroundWrites.at(-1).battery_alert_interval_seconds'),5);
    await click('#cr-add');
    assert.equal(await run('location.hash'),'#custom-reminder');
    await run(`document.querySelector('#cr-title').value='电脑测试提醒';document.querySelector('#cr-content').value='本机记录与手机转发测试';
      document.querySelector('#cr-form').dispatchEvent(new Event('submit',{cancelable:true}));`);
    await poll(()=>run('location.hash==="#notification-options"'));
    assert.equal(await run('document.querySelector("#cr-count").textContent'),'自定义通知（3）');
    for(const width of [390,1100]){
      await send('Emulation.setDeviceMetricsOverride',{width,height:850,deviceScaleFactor:1,mobile:false});
      assert.equal(await run('document.querySelector("#android-notification-options-panel").scrollWidth<=document.querySelector("#android-notification-options-panel").clientWidth+1'),true);
      const shot=await send('Page.captureScreenshot',{format:'png'});fs.writeFileSync(path.join(output,`reminders-windows-${width}.png`),Buffer.from(shot.data,'base64'));
    }
    await click('#android-notification-options-back-btn');
    await run('window.NotificationUI.openSettings()');
    await poll(()=>run('document.querySelector("dialog.ns-dialog").open'));
    await run('[...document.querySelectorAll("dialog.ns-dialog button")].find(b=>b.textContent==="推送设置").click()');
    await poll(()=>run('document.querySelector("#android-push-details-panel").classList.contains("is-open")'));
    assert.equal(await run('!!document.querySelector("#android-push-details-panel input[aria-label=信息推送总开关]")'),true);
    assert.equal(await run('!!document.querySelector("#android-push-details-panel .ns-app-picker-entry")'),false);
    for(const label of ['信息推送总开关','电量推送','自定义通知推送','向 IQOO 推送通知']) await click(`#android-push-details-panel input[aria-label="${label}"]`);
    assert.equal(await run('window.__customReminderFixture.notificationWrites.at(-1).push_enabled'),true);
    assert.equal(await run('window.__customReminderFixture.notificationWrites.at(-1).lq_battery_push_enabled'),true);
    assert.equal(await run('window.__customReminderFixture.notificationWrites.at(-1).lq_reminder_push_enabled'),true);
    assert.deepEqual(await run('window.__customReminderFixture.notificationWrites.at(-1).target_device_ids'),['iqoo']);
    await run('window.__customReminderFixture.failNotificationSettings=true');
    await click('#android-push-details-panel input[aria-label="电量推送"]');
    await poll(()=>run('document.querySelector("#android-push-details-panel .ns-save-status").textContent.includes("模拟")'));
    assert.equal(await run('document.querySelector("#android-push-details-panel input[aria-label=电量推送]").checked'),true);
    await click('#android-push-details-back-btn');
    await run('window.__customReminderFixture.batteryAvailable=false;document.querySelector("#settings-btn").click();document.querySelector("#settings-btn").click()');
    await poll(()=>run('document.querySelector("#battery-alert-toggle").disabled'));
    assert.match(await run('document.querySelector("#battery-alert-validation").textContent'),/无可用电池/);
    await checkLocalConnections('.ns-device-list[data-kind=notification_device] .ns-device');
    await send('Page.navigate',{url:`http://127.0.0.1:${port}/?android-preview=1`});
    await poll(()=>run('!!document.querySelector("#android-receive-list [data-peer=preview-local]")'));
    await click('#android-receive-list [data-peer=preview-local]');
    await poll(()=>run('document.querySelector(".ns-empty")?.textContent.includes("本机")'));
    assert.match(await run('document.querySelector(".ns-detail-title").textContent'),/我的手机.*本机记录/);
    await checkLocalConnections('#android-receive-list [data-peer]');
    console.log('PASS: Windows shared editor, repeat settings, local history, push controls, save rollback, no-battery state, responsive layout; Android local entry');
    passed=true;await send('Browser.close');
  }finally{
    ws?.close();
    if(passed){server.kill();const target=path.resolve(profile);if(path.dirname(target)===path.resolve(os.tmpdir())&&path.basename(target).startsWith('lq-local-reminders-')){
      for(let i=0;i<5;i++){try{fs.rmSync(target,{recursive:true,force:true});break;}catch{await sleep(200);}}
    }}else{server.unref();browser?.unref();console.error('Inspection profile: '+profile);}
  }
}
main().catch(error=>{console.error(error);process.exitCode=1;});

// Disposable browser fixture; never reads real device data or sends notifications.
const assert = require('node:assert/strict');
const fs = require('node:fs'), path = require('node:path'), os = require('node:os');
const { spawn } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function poll(fn) {
  for (let i = 0; i < 120; i++) { const value = await fn().catch(() => false); if (value) return value; await sleep(100); }
  throw Error('Timed out waiting for fixture');
}
async function main() {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'lq-push-history-'));
  const port = 21000 + Math.floor(Math.random() * 3000), debugPort = port + 3000;
  const server = spawn(process.execPath, ['tests/notification-preview.cjs'], {
    cwd: root, env: { ...process.env, LQ_NOTIFICATION_PREVIEW_PORT: String(port) }, stdio: 'inherit',
  });
  let browser, ws, passed = false;
  try {
    await poll(async () => (await fetch(`http://127.0.0.1:${port}/`)).ok);
    browser = spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', [
      '--no-first-run', '--no-default-browser-check', '--disable-extensions',
      '--remote-debugging-port=' + debugPort, '--user-data-dir=' + profile,
      `http://127.0.0.1:${port}/?android-preview=1`,
    ], { stdio: 'ignore' });
    const page = await poll(async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(p => p.url.includes('android-preview')));
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
    let sequence = 0; const pending = new Map();
    ws.onmessage = event => {
      const result = JSON.parse(event.data), entry = pending.get(result.id);
      if (!entry) return; pending.delete(result.id);
      if (result.error || result.result?.exceptionDetails) entry.reject(result.error || result.result.exceptionDetails);
      else entry.resolve(result.result);
    };
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => { pending.delete(id); reject(Error('CDP timeout')); }, 8000);
      pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
      ws.send(JSON.stringify({ id, method, params }));
    });
    const run = async expression => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value;
    const scrollBottom = () => run('(() => {const l=document.querySelector(".ns-cards");l.scrollTop=l.scrollHeight;l.dispatchEvent(new Event("scroll"));})()');
    await send('Emulation.setDeviceMetricsOverride', { width: 390, height: 844, deviceScaleFactor: 1, mobile: true });
    await poll(() => run('!!document.querySelector("#android-receive-list .ns-device")'));
    await run(`(() => {
      const f = window.__notificationHistoryFixture, base = f.records[0];
      const make = (id, index, title, text, key) => ({...base,record_id:id+'-'+index,peer_id:id,peer_name:id==='iqoo'?'IQOO':'电脑',
        notification:{...base.notification,event_id:id+'-'+index,source_device_id:id,package:'com.lanchat.app',app_name:'LQChat',
          notification_key:key||id+'-'+index,title,text,post_time:Date.now()-index*1000}});
      f.records.splice(0,f.records.length,
        ...Array.from({length:300},(_,i)=>make('pc',i,'电脑通知','电脑内容')),
        ...Array.from({length:200},(_,i)=>make('iqoo',i,i===0?'电量提醒':i===1?'设备断开':i===2?'设备连接':i===3?'自定义提醒':'通知 '+i,
          i===0?'50%':i===1?'已从局域网断开':i===2?'已接入局域网':'IQOO 内容 '+i,i===0?'lq-battery-50-1-0':undefined)));
      const peer=f.peers.find(p=>p.id==='iqoo');peer.notification_push_enabled=false;peer.is_offline=true;
      window.NotificationUI.onPeers(f.peers);f.emit();
    })()`);
    await poll(() => run('document.querySelector("#android-receive-list [data-peer=pc]")!==null'));
    assert.equal(await run('document.querySelector("#android-receive-list [data-peer=iqoo]")!==null'), true);
    await run('document.querySelector("#android-receive-list [data-peer=iqoo]").click()');
    await poll(() => run('document.querySelectorAll(".ns-card").length===100'));
    assert.equal(await run('document.querySelector(".ns-card .ns-title").textContent'), '电量');
    assert.equal(await run('document.querySelector(".ns-card .ns-body").textContent'), '50%');
    assert.equal(await run('[...document.querySelectorAll(".ns-card .ns-title")].some(n=>n.textContent.includes("IQOO"))'), false);
    assert.equal(await run('[...document.querySelectorAll(".ns-card .ns-title")].some(n=>n.textContent==="设备断开")'), true);
    assert.equal(await run('[...document.querySelectorAll(".ns-card .ns-body")].some(n=>n.textContent==="电脑内容")'), false);
    assert.equal(await run('document.querySelector(".ns-load-more")'), null);
    await scrollBottom();
    await poll(() => run('document.querySelectorAll(".ns-card").length===200'));
    assert.equal(await run('document.querySelector(".ns-detail").dataset.hasMore'), 'false');
    // Refresh loaded pages after a new notification and retain the older page.
    await run(`(() => {const f=window.__notificationHistoryFixture, first=f.records.find(r=>r.peer_id==='iqoo');
      f.records.unshift({...first,record_id:'iqoo-new',notification:{...first.notification,event_id:'iqoo-new',notification_key:'new',title:'新提醒',text:'新内容'}});f.emit();})()`);
    await poll(() => run('document.querySelector(".ns-card .ns-title").textContent==="新提醒"'));
    assert.equal(await run('document.querySelectorAll(".ns-card").length'), 200);
    // Failures keep already loaded records and offer a retry.
    await run('window.__notificationHistoryFixture.fail=true');
    await scrollBottom();
    await poll(() => run('document.querySelector(".ns-load-state").textContent.includes("重试")'));
    assert.equal(await run('document.querySelectorAll(".ns-card").length'), 200);
    await run('window.__notificationHistoryFixture.fail=false');
    await scrollBottom();
    await poll(() => run('document.querySelectorAll(".ns-card").length===201'));
    // A late response for one device must not populate a newly opened device.
    await run('window.__notificationHistoryFixture.delay=250;window.NotificationUI.open("pc","notification_receive");window.NotificationUI.open("iqoo","notification_receive")');
    await poll(() => run('document.querySelectorAll(".ns-card").length===100'));
    assert.equal(await run('[...document.querySelectorAll(".ns-card")].every(n=>n.dataset.recordId.startsWith("iqoo"))'), true);
    await run(`window.dispatchEvent(new CustomEvent('synced-notification-tapped', {
      detail:{recordId:'iqoo-180',sourceDeviceId:'iqoo'}
    }))`);
    await poll(() => run('document.querySelector(".ns-card[data-record-id=iqoo-180]")!==null'));
    for (const width of [320,390,960]) {
      await send('Emulation.setDeviceMetricsOverride', { width, height: 844, deviceScaleFactor: 1, mobile: true });
      assert.equal(await run('document.querySelector(".ns-cards").scrollWidth<=document.querySelector(".ns-cards").clientWidth+1'), true);
    }
    assert.equal(await run('window.__notificationHistoryFixture.queries.some(q=>q.device_id==="iqoo" && !q.direction && q.page===2)'), true);
    // Both platforms share one deduplicated list and per-device mixed-direction history.
    for (const platform of ['desktop','android']) {
      await send('Emulation.setDeviceMetricsOverride', {width:platform==='desktop'?1100:390,height:844,deviceScaleFactor:1,mobile:platform==='android'});
      await send('Page.navigate',{url:`http://127.0.0.1:${port}/?${platform}-preview=1`});
      await poll(()=>run('!!window.__notificationHistoryFixture && !!document.querySelector(".ns-device")'));
      await run(`(async () => {
        const f=window.__notificationHistoryFixture,b=f.records[0];
        const make=(id,i,kind,status='success')=>({...b,record_id:id+'-'+i,peer_id:id,peer_name:id,
          view_kind:kind,status,notification:{...b.notification,event_id:id+'-'+i,notification_key:id+'-'+i,
            source_device_id:kind==='notification_push'?'preview-local':id,
            target_device_id:kind==='notification_push'?id:'preview-local',title:'通知 '+i,text:id+' 内容 '+i}});
        f.records.splice(0,f.records.length,...Array.from({length:150},(_,i)=>make('pc',i,'notification_receive')),
          make('iqoo',0,'notification_receive'),make('iqoo',1,'notification_push'),make('iqoo',2,'notification_push','offline'),
          make('archived-sender',0,'notification_push'));
        const info=await window.__TAURI__.core.invoke('notification_settings');
        await window.__TAURI__.core.invoke('notification_settings',{settings:{...info.settings,target_device_ids:['iqoo','target-only']}});
        await window.NotificationUI.refreshSettings();f.emit();
      })()`);
      const selector=platform==='android'?'#android-receive-list .ns-device':'.ns-device-list .ns-device';
      await poll(()=>run(`document.querySelectorAll(${JSON.stringify(selector)}).length===5`));
      assert.equal(await run(`document.querySelectorAll(${JSON.stringify(selector+'[data-peer=iqoo]')}).length`),1);
      assert.equal(await run(`document.querySelector(${JSON.stringify(selector)}).dataset.peer`),'preview-local');
      assert.equal(await run('!!document.querySelector(".ns-device[data-peer=archived-sender]")'),true);
      if(platform==='desktop') {
        assert.equal(await run('document.querySelectorAll(".ns-group").length'),1);
        assert.equal(await run('document.querySelector(".ns-disclosure").textContent'),'通知设备');
        assert.equal(await run('document.querySelector(".ns-device[data-peer=iqoo] .ns-name").textContent'),'IQOO（发送与接收记录）');
      } else assert.equal(await run('document.querySelector("#android-receive-count").textContent'),'通知设备 5 台');
      for (const [id, label] of [['pc','pc（接收记录）'],['iqoo','IQOO（发送与接收记录）'],['archived-sender','archived-sender（发送记录）'],['target-only','target-only（发送记录）']]) {
        assert.equal(await run(`document.querySelector('.ns-device[data-peer="${id}"] .ns-name').textContent`),label);
        await run(`window.NotificationUI.open(${JSON.stringify(id)})`);
        await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false"'));
        assert.equal(await run('document.querySelector(".ns-detail-title").textContent'),label);
        assert.equal(await run('document.querySelector(".ns-detail-status").textContent.split(" · ")[1]'),label.match(/（(.+)）/)[1]);
      }
      await run('document.querySelector(".ns-device[data-peer=iqoo]").click()');
      await poll(()=>run('document.querySelectorAll(".ns-card").length===3'));
      assert.deepEqual(await run('[...document.querySelectorAll(".ns-result")].map(n=>n.textContent)'),['已接收','已发送','发送 · 离线已丢弃']);
      assert.equal(await run('window.__notificationHistoryFixture.queries.some(q=>q.device_id==="iqoo"&&!q.direction)'),true);
      await run('window.NotificationUI.open("archived-sender")');
      await poll(()=>run('document.querySelectorAll(".ns-card").length===1'));
      assert.equal(await run('document.querySelector(".ns-result").textContent'),'已发送');
      // Bounds include old records outside page one; filters use complete local days before pagination.
      await run(`(async () => {
        const f=window.__notificationHistoryFixture,b=f.records[0];f.delay=0;
        f.records.splice(0,f.records.length,...Array.from({length:250},(_,i)=>({...b,record_id:'dates-'+i,
          peer_id:'pc',peer_name:'pc',view_kind:'notification_receive',notification:{...b.notification,event_id:'dates-'+i,
            notification_key:'dates-'+i,source_device_id:'pc',target_device_id:'preview-local',
            post_time:new Date(2026,9,i<100?13:i<200?12:7,i===100?0:23,i===100?0:59,i===100?0:59).getTime()}})));
        await window.NotificationUI.refresh();await window.NotificationUI.open('pc');
      })()`);
      await poll(()=>run('document.querySelectorAll(".ns-card").length===100'));
      assert.equal(await run('document.querySelector(".ns-date-start input").value'),'2026-10-07');
      assert.equal(await run('document.querySelector(".ns-date-end input").value'),'2026-10-13');
      assert.equal(await run('document.querySelector(".ns-date-start .ns-date-value").textContent'),'10月07日');
      {
        // Freeze today's local date so the seven-day wheel is reproducible on any run date.
        await run(`window.__wheelRealDate=Date;window.Date=class extends window.__wheelRealDate {
          constructor(...args){super(...(args.length?args:[2026,9,13,12]));}
          static now(){return new window.__wheelRealDate(2026,9,13,12).getTime();}
        };document.querySelector('.ns-date-start .ns-date-trigger').click()`);
        assert.equal(await run('document.querySelector(".ns-date-wheel-dialog").open'),true);
        assert.deepEqual(await run('[...document.querySelectorAll(".ns-wheel-day")].map(n=>n.dataset.date)'),
          ['2026-10-07','2026-10-08','2026-10-09','2026-10-10','2026-10-11','2026-10-12']);
        assert.equal(await run('document.querySelector(".ns-wheel-today strong").textContent'),'今日');
        assert.equal(await run('document.querySelector(".ns-wheel-today").dataset.date'),'2026-10-13');
        for (const width of platform === 'android' ? [320,390] : [320,1100]) {
          await send('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:platform==='android'});
          assert.equal(await run(`(() => {const d=document.querySelector('.ns-date-wheel-dialog'),r=d.getBoundingClientRect();
            return r.left>=0&&r.right<=innerWidth&&d.scrollWidth<=d.clientWidth+1&&
              [...d.querySelectorAll('.ns-wheel button')].every(b=>{const q=b.getBoundingClientRect();
                return q.left>=r.left&&q.right<=r.right&&getComputedStyle(b).animationName==='none';});})()`),true);
        }
        // Desktop-wide hover styling must not remove the wheel's centering transform.
        await send('Page.bringToFront');
        const positions=await run(`[...document.querySelectorAll('.ns-wheel button')].map(b=>{
          const r=b.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};})`);
        for (let i=0;i<7;i++) {
          const r=positions[i];
          await send('Input.dispatchMouseEvent',{type:'mouseMoved',x:r.x+r.width/2,y:r.y+r.height/2});
          await sleep(50);
          const after=await run(`[...document.querySelectorAll('.ns-wheel button')].map(b=>{
            const r=b.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height};})`);
          assert.equal(await run(`document.querySelectorAll('.ns-wheel button')[${i}].matches(':hover')`),true);
          for(let j=0;j<7;j++) for(const key of ['x','y','width','height'])
            assert(Math.abs(after[j][key]-positions[j][key])<0.5,`${platform}: wheel item ${j} ${key} shifted while hovering ${i}`);
        }
        // Escape cancels without changing either boundary, and returns focus to the trigger.
        await send('Page.bringToFront');
        await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
        await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
        await poll(()=>run('!document.querySelector(".ns-date-wheel-dialog").open'));
        assert.equal(await run('document.querySelector(".ns-date-start input").value'),'2026-10-07');
        assert.equal(await run('document.activeElement===document.querySelector(".ns-date-start .ns-date-trigger")'),true);
        await run('document.querySelector(".ns-date-end .ns-date-trigger").click();document.querySelector(".ns-wheel-today").click()');
        await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false"'));
        assert.equal(await run('document.querySelector(".ns-date-end input").value'),'2026-10-13');
        await run('document.querySelector(".ns-date-start .ns-date-trigger").click()');
        if(process.env.LQ_WHEEL_OUTPUT) {
          fs.mkdirSync(process.env.LQ_WHEEL_OUTPUT,{recursive:true});
          await run('[...document.querySelectorAll("body > div")].find(n=>n.textContent.includes("测试：新增通知")&&n.style.position==="fixed")?.remove()');
          const shot=await send('Page.captureScreenshot',{format:'png'});
          fs.writeFileSync(path.join(process.env.LQ_WHEEL_OUTPUT,`${platform}-date-wheel.png`),Buffer.from(shot.data,'base64'));
        }
        await run(`document.querySelector('.ns-wheel-day[data-date="2026-10-12"]').click()`);
        await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false"'));
        assert.equal(await run('document.querySelector(".ns-date-wheel-dialog").open'),false);
        assert.equal(await run('document.querySelector(".ns-date-start input").value'),'2026-10-12');
        assert.equal(await run('document.querySelectorAll(".ns-card").length'),100);
        await run(`document.querySelector('.ns-date-end .ns-date-trigger').click();document.querySelector('.ns-wheel-day[data-date="2026-10-12"]').click()`);
        await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false"'));
        const count=await run('window.__notificationHistoryFixture.queries.length');
        await run('document.querySelector(".ns-date-start .ns-date-trigger").click();document.querySelector(".ns-wheel-today").click()');
        assert.equal(await run('document.querySelector(".ns-range-error").hidden'),false);
        assert.equal(await run('window.__notificationHistoryFixture.queries.length'),count);
        assert.equal(await run('document.querySelector(".ns-date-start input").value'),'2026-10-12');
        await run('window.Date=window.__wheelRealDate;delete window.__wheelRealDate');
      }
      await run(`(() => {const a=document.querySelector('.ns-date-start input'),b=document.querySelector('.ns-date-end input');
        a.value='2026-10-12';a.dispatchEvent(new Event('change'));b.value='2026-10-12';b.dispatchEvent(new Event('change'));})()`);
      await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false" && [...document.querySelectorAll(".ns-card")].every(n=>Number(n.dataset.recordId.split("-")[1])>=100 && Number(n.dataset.recordId.split("-")[1])<200)'));
      assert.equal(await run('document.querySelectorAll(".ns-card").length'),100);
      assert.equal(await run('document.querySelector(".ns-detail").dataset.hasMore'),'false');
      if(process.env.LQ_TEST_OUTPUT){
        fs.mkdirSync(process.env.LQ_TEST_OUTPUT,{recursive:true});
        await run('[...document.querySelectorAll("body > div")].find(n=>n.textContent.includes("测试：新增通知")&&n.style.position==="fixed")?.remove()');
        const shot=await send('Page.captureScreenshot',{format:'png'});
        fs.writeFileSync(path.join(process.env.LQ_TEST_OUTPUT,`date-range-${platform}.png`),Buffer.from(shot.data,'base64'));
      }
      const callsBeforeInvalid=await run('window.__notificationHistoryFixture.queries.length');
      await run(`(() => {const a=document.querySelector('.ns-date-start input');a.value='2026-10-13';a.dispatchEvent(new Event('change'));})()`);
      assert.equal(await run('document.querySelector(".ns-range-error").hidden'),false);
      assert.equal(await run('window.__notificationHistoryFixture.queries.length'),callsBeforeInvalid);
      await run(`(() => {const a=document.querySelector('.ns-date-start input'),b=document.querySelector('.ns-date-end input');
        a.value='2026-10-08';b.value='2026-10-08';a.dispatchEvent(new Event('change'));})()`);
      await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false" && document.querySelectorAll(".ns-card").length===0'));
      assert.equal(await run('document.querySelector(".ns-empty").textContent'),'所选日期范围内暂无记录。');
      for(const width of [320,390,1100]) {
        await send('Emulation.setDeviceMetricsOverride',{width,height:844,deviceScaleFactor:1,mobile:platform==='android'});
        assert.equal(await run('document.querySelector(".ns-footer").scrollWidth<=document.querySelector(".ns-footer").clientWidth+1'),true);
      }
      await run('window.NotificationUI.open("target-only")');
      await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false"'));
      assert.equal(await run('document.querySelector(".ns-date-start input").value'),'');
      await run(`(() => {const a=document.querySelector('.ns-date-start input'),b=document.querySelector('.ns-date-end input');
        a.value='2026-10-07';a.dispatchEvent(new Event('change'));b.value='2026-10-13';b.dispatchEvent(new Event('change'));})()`);
      await poll(()=>run('document.querySelector(".ns-detail").dataset.loading==="false" && document.querySelector(".ns-range-error").hidden'));
      assert.equal(await run('document.querySelector(".ns-date-start input").value'),'2026-10-07');
    }
    passed = true;
    console.log('PASS: both platforms, static six-date wheel/today/cancel/selection, pagination/retry, local-day filtering, invalid/empty ranges, device-switch races and narrow layout');
    await send('Browser.close');
  } finally {
    ws?.close();
    if (passed) {
      server.kill();
      const temporaryRoot = path.resolve(os.tmpdir()), target = path.resolve(profile);
      if (path.dirname(target) === temporaryRoot && path.basename(target).startsWith('lq-push-history-')) {
        for (let attempt = 0; attempt < 5; attempt++) {
          try { fs.rmSync(target, { recursive: true, force: true }); break; }
          catch { await sleep(200); }
        }
      }
    }
    else { server.unref(); browser?.unref(); console.error('Fixture retained for inspection: ' + profile); }
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

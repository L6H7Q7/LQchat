// Disposable browser fixture only. Never connects to the phone or changes its settings.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const out = path.join(root, 'artifacts/push-app-selection-fix');
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function poll(fn) {
  for (let i = 0; i < 100; i++) { const result = await fn().catch(() => false); if (result) return result; await sleep(100); }
  throw Error('Timed out waiting for fixture');
}
async function main() {
  fs.mkdirSync(out, { recursive: true });
  const port = 23000 + Math.floor(Math.random() * 2000), debugPort = port + 2500;
  const profile = fs.mkdtempSync(path.join(out, 'browser-profile-'));
  const server = spawn(process.execPath, ['tests/notification-preview.cjs'], {
    cwd: root, env: { ...process.env, LQ_NOTIFICATION_PREVIEW_PORT: String(port) }, windowsHide: true, stdio: 'inherit',
  });
  let browser, ws;
  try {
    await poll(async () => (await fetch(`http://127.0.0.1:${port}/`)).ok);
    browser = spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', [
      '--headless=new', '--disable-extensions', '--no-first-run', '--no-default-browser-check',
      `--remote-debugging-port=${debugPort}`, `--user-data-dir=${profile}`, `http://127.0.0.1:${port}/?android-preview=1`,
    ], { windowsHide: true, stdio: 'ignore' });
    const page = await poll(async () => (await (await fetch(`http://127.0.0.1:${debugPort}/json/list`)).json()).find(p => p.url.includes('android-preview')));
    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
    let request = 0;
    const pending = new Map();
    ws.onmessage = e => {
      const result = JSON.parse(e.data), entry = pending.get(result.id);
      if (!entry) return;
      pending.delete(result.id);
      result.error || result.result?.exceptionDetails ? entry.reject(result.error || result.result.exceptionDetails) : entry.resolve(result.result);
    };
    const send = (method, params = {}) => new Promise((resolve, reject) => {
      const id = ++request, timer = setTimeout(() => { pending.delete(id); reject(Error(`CDP timeout: ${method}`)); }, 10000);
      pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
      ws.send(JSON.stringify({ id, method, params }));
    });
    const run = async expression => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value;
    const click = selector => run(`document.querySelector(${JSON.stringify(selector)}).click()`);
    await poll(() => run(`!!window.NotificationUI && !!document.querySelector('#settings-btn') && document.body.classList.contains('android-app')`));
    await run(`(async()=>{
      const original=window.__TAURI__.core.invoke;
      const base=await original('notification_settings');
      const f=window.__pushSelectionFixture={settings:{...base.settings,allowed_packages:['com.microsoft.emmx','com.yaxisvip.pubgtool.iueg','example.messages','com.android.systemui']},writes:[],fail:false};
      f.apps=[{package:'com.microsoft.emmx',name:'Edge',is_system:false},{package:'example.messages',name:'Messages',is_system:false},{package:'com.android.systemui',name:'System UI',is_system:true}];
      window.__TAURI__.core.invoke=async(command,args={})=>{
        if(command==='notification_settings')return {...base,settings:f.settings};
        if(command==='notification_action'&&args.action==='apps')return {apps:f.apps,installed_snapshot:f.apps.map(a=>a.package)};
        if(command==='notification_action'&&args.action==='replace_allowed'){
          const packages=args.payload.packages;
          if(packages.some(p=>!f.apps.some(a=>a.package===p)))throw Error('允许列表包含未安装或不可选择的应用');
          if(f.fail)throw Error('模拟磁盘保存失败');
          f.writes.push([...packages]); f.settings={...f.settings,allowed_packages:[...packages]};return {...base,settings:f.settings};
        }
        return original(command,args);
      };
      await NotificationUI.refreshSettings();
    })()`);
    await click('#settings-btn');
    await click('#android-push-settings-btn');
    const openPicker = async () => {
      await click('.ns-app-picker-entry');
      await poll(() => run(`location.hash==='#push-apps' && !document.querySelector('#android-push-apps-save-btn').disabled && document.querySelectorAll('.android-push-app-row').length===3`));
    };
    await openPicker();
    assert.equal(await run(`document.querySelector('#android-push-apps-count').textContent`), '已选 3 个');
    assert.equal(await run(`window.__pushSelectionFixture.writes.length`), 0, 'Opening the picker must not persist a cleanup');
    await run(`document.querySelector('#android-push-apps-category').value='user';document.querySelector('#android-push-apps-category').dispatchEvent(new Event('change'))`);
    assert.equal(await run(`document.querySelectorAll('.android-push-app-row').length`), 2);
    await run(`[...document.querySelectorAll('.android-push-app-row')].find(r=>r.textContent.includes('com.microsoft.emmx')).querySelector('input').click()`);
    await run(`document.querySelector('#android-push-apps-search').value='Edge';document.querySelector('#android-push-apps-search').dispatchEvent(new Event('input'))`);
    await click('#android-push-apps-save-btn');
    await poll(() => run(`location.hash==='#push-details'`));
    assert.deepEqual((await run(`window.__pushSelectionFixture.writes[0]`)).sort(), ['com.android.systemui', 'example.messages']);
    console.log('PASS: stale saved entry removed from draft; Edge deselected; hidden system/category/search selections preserved.');
    await openPicker();
    assert.equal(await run(`document.querySelector('#android-push-apps-count').textContent`), '已选 2 个');
    await run(`window.__pushSelectionFixture.fail=true;[...document.querySelectorAll('.android-push-app-row')].find(r=>r.textContent.includes('example.messages')).querySelector('input').click()`);
    await click('#android-push-apps-save-btn');
    await poll(() => run(`document.querySelector('#android-push-apps-status').textContent.includes('模拟磁盘保存失败')`));
    assert.equal(await run(`location.hash`), '#push-apps');
    assert.equal(await run(`window.__pushSelectionFixture.writes.length`), 1);
    await run(`window.__pushSelectionFixture.fail=false`);
    await click('#android-push-apps-save-btn');
    await poll(() => run(`location.hash==='#push-details'`));
    assert.deepEqual(await run(`window.__pushSelectionFixture.writes[1]`), ['com.android.systemui']);
    console.log('PASS: failed save preserves the draft and settings; retry persists the same intended selection.');
    await openPicker();
    await click('#android-push-apps-select-all');
    await click('#android-push-apps-select-all');
    await click('#android-push-apps-save-btn');
    await poll(() => run(`location.hash==='#push-details'`));
    assert.deepEqual(await run(`window.__pushSelectionFixture.writes[2]`), []);
    console.log('PASS: empty selection remains supported.');
    fs.writeFileSync(path.join(out, 'browser-results.json'), JSON.stringify({ passed: true, cases: ['stale saved selection', 'Edge deselection', 'category and search preservation', 'failure and retry', 'empty selection'] }, null, 2));
  } finally {
    ws?.close();
    for (const child of [browser, server]) {
      if (!child || child.exitCode !== null) continue;
      try { execFileSync('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' }); } catch {}
    }
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

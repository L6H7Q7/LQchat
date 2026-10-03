// Runs only on the explicitly selected MuMu instance; saves existing values unchanged.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const serial = process.env.LQCHAT_ANDROID_SERIAL || '127.0.0.1:16384';
const packageName = process.env.LQCHAT_ANDROID_PACKAGE || 'com.lanchat.app';
const packagePattern = packageName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
const adbPath = path.join(process.env.LOCALAPPDATA, 'Android/Sdk/platform-tools/adb.exe');
const adb = (...args) => execFileSync(adbPath, ['-s', serial, ...args], { encoding: 'utf8' });
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const root = path.resolve(__dirname, '..');
const output = path.join(root, 'artifacts/settings-navigation');
async function main() {
  const pid = adb('shell', 'pidof', packageName).trim();
  assert(/^\d+$/.test(pid));
  adb('forward', 'tcp:19225', `localabstract:webview_devtools_remote_${pid}`);
  const pages = await (await fetch('http://127.0.0.1:19225/json/list')).json();
  const page = pages.find(p => /^LQ\s*Chat$/i.test(p.title) && p.url.startsWith('http://tauri.localhost/'));
  assert(page);
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = e => {
    const m = JSON.parse(e.data), entry = pending.get(m.id);
    if (!entry) return;
    pending.delete(m.id);
    if (m.error || m.result.exceptionDetails) entry.reject(m.error || m.result.exceptionDetails);
    else entry.resolve(m.result);
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const requestId = ++id;
    pending.set(requestId, { resolve, reject });
    ws.send(JSON.stringify({ id: requestId, method, params }));
  });
  const evaluate = async expression => (await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true })).result.value;
  const wait = async expression => {
    for (let i = 0; i < 100; i++) {
      if (await evaluate(expression)) return;
      await sleep(100);
    }
    throw Error('Timeout: ' + expression);
  };
  const click = selector => evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const setValue = (selector, value, eventName = 'change') => evaluate(`(() => {
    const node = document.querySelector(${JSON.stringify(selector)});
    node.value = ${JSON.stringify(value)};
    node.dispatchEvent(new Event(${JSON.stringify(eventName)}, { bubbles: true }));
  })()`);
  const tap = async selector => {
    adb('shell', 'cmd', 'statusbar', 'collapse');
    await sleep(200);
    const point = await evaluate(`(() => {
      const rect = document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();
      return { x: (rect.left + rect.width / 2) * devicePixelRatio,
        y: (rect.top + rect.height / 2) * devicePixelRatio };
    })()`);
    adb('shell', 'input', 'tap', String(Math.round(point.x)), String(Math.round(point.y)));
  };
  const expectPage = async hash => {
    await wait(`location.hash === ${JSON.stringify(hash)}`);
    assert.equal(await evaluate(`document.getElementById('settings-panel').style.display === 'block'`), !!hash);
    assert.equal(await evaluate(`document.getElementById('permissions-panel').classList.contains('is-open')`), hash === '#permissions');
    assert.equal(await evaluate(`document.getElementById('android-push-apps-panel').classList.contains('is-open')`), hash === '#push-apps');
    assert.match(adb('shell', 'dumpsys', 'activity', 'activities'),
      new RegExp(`(?:mResumedActivity|ResumedActivity):.*${packagePattern}/`));
  };
  const back = () => { adb('shell', 'input', 'keyevent', '4'); };
  const open = async () => {
    await tap('#android-settings-btn'); await expectPage('#settings');
    await wait('!document.getElementById("save-settings-btn").disabled');
  };
  const capture = name => {
    fs.mkdirSync(output, { recursive: true });
    fs.writeFileSync(path.join(output, name + '.png'), execFileSync(adbPath,
      ['-s', serial, 'exec-out', 'screencap', '-p'], { maxBuffer: 20 * 1024 * 1024 }));
  };
  const readConfig = () => evaluate(`Promise.all([apiGetSettings(),
    window.__TAURI__.core.invoke('get_background_runtime_settings'),
    window.__TAURI__.core.invoke('notification_settings')]).then(([general, background, notifications]) =>
      ({general, background, notifications:notifications.settings}))`);
  try {
    for (const file of ['app.js', 'ui.js', 'notification-sync.js']) {
      const deployed = await evaluate(`fetch('js/${file}').then(r => r.text())`);
      const local = fs.readFileSync(path.join(root, 'src/js', file), 'utf8');
      const hash = text => crypto.createHash('sha256').update(text.replace(/\r\n/g, '\n')).digest('hex');
      assert.equal(hash(deployed), hash(local), 'Installed JavaScript must match latest source: ' + file);
    }
    await expectPage('');
    const before = await readConfig();
    fs.mkdirSync(output, { recursive: true });
    fs.writeFileSync(path.join(output, 'mumu-settings-before.json'), JSON.stringify(before, null, 2));
    for (let i = 0; i < 3; i++) {
      await open();
      await click('#android-permissions-btn'); await expectPage('#permissions');
      back(); await expectPage('#settings');
      await click('.ns-app-picker-entry'); await expectPage('#push-apps');
      back(); await expectPage('#settings');
      back(); await expectPage('');
    }
    capture('mumu-return-home');
    await open();
    await click('#save-settings-btn'); await expectPage('');
    await wait('document.querySelector(".message-action-toast")?.textContent === "保存成功"');
    await sleep(220); capture('mumu-settings-save-success');
    await open();
    await click('#android-permissions-btn');
    await click('#save-permissions-btn'); await expectPage('#settings');
    await wait('document.querySelector(".message-action-toast")?.textContent === "保存成功"');
    await sleep(220); capture('mumu-permissions-save-success');
    await click('.ns-app-picker-entry');
    await wait('!document.getElementById("android-push-apps-save-btn").disabled');
    await setValue('#android-push-apps-category', 'system');
    await wait('document.querySelectorAll(".android-push-app-row").length > 0');
    assert.equal(await evaluate(`Array.from(document.querySelectorAll('.android-push-app-row'))
      .every(row => row.querySelector('.android-push-app-system-tag'))`), true);
    await setValue('#android-push-apps-search', 'com.android.systemui', 'input');
    await wait('document.querySelectorAll(".android-push-app-row").length === 1');
    assert.equal(await evaluate(`document.querySelector('.android-push-app-package')
      .firstChild.textContent === 'com.android.systemui'`), true);
    capture('mumu-system-app-filter');
    await setValue('#android-push-apps-search', '', 'input');
    await setValue('#android-push-apps-category', 'user');
    await wait('document.querySelectorAll(".android-push-app-row").length > 0');
    assert.equal(await evaluate(`Array.from(document.querySelectorAll('.android-push-app-row'))
      .every(row => !row.querySelector('.android-push-app-system-tag'))`), true);
    await setValue('#android-push-apps-category', 'all');
    await click('#android-push-apps-save-btn'); await expectPage('#settings');
    await wait('document.querySelector(".message-action-toast")?.textContent === "保存成功"');
    await sleep(220); capture('mumu-apps-save-success');
    back(); await expectPage('');
    const after = await readConfig();
    assert.deepEqual(after, before, 'QA must preserve all existing configuration values');
    back();
    await sleep(300);
    assert(!new RegExp(`(?:mResumedActivity|ResumedActivity):.*${packagePattern}/`)
      .test(adb('shell', 'dumpsys', 'activity', 'activities')));
    adb('shell', 'am', 'start', '-W', '-n', `${packageName}/com.lanchat.app.MainActivity`);
    await expectPage('');
    const report = { passed: [
      'Installed assets match latest source',
      'Three repeated physical settings taps and native system-back cycles through all settings pages',
      'General, permission and app selection saves show visible success',
      'All, user and system filters work; com.android.systemui is selectable',
      'Existing configuration preserved',
      'Home system-back still leaves app; reopening shows home',
    ] };
    fs.writeFileSync(path.join(output, 'mumu-results.json'), JSON.stringify(report, null, 2));
    console.log(JSON.stringify(report, null, 2));
  } finally { ws.close(); }
}
main().catch(error => { console.error(error); process.exitCode = 1; });

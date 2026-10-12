const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.resolve(__dirname, '..');
const ui = fs.readFileSync(path.join(root, 'src/js/ui.js'), 'utf8');
const app = fs.readFileSync(path.join(root, 'src/js/app.js'), 'utf8');
const helper = ui.slice(ui.indexOf('function updatePeerPresenceIndicator'), ui.indexOf('// 添加新用户到列表'));
const setting = ui.slice(ui.indexOf('function initRefreshCountdown()'), ui.indexOf('// 初始化设置功能', ui.indexOf('function initRefreshCountdown()')));
const indicator = id => ({ dataset: { presencePeerId: id }, textContent: '', title: '' });
const indicators = [indicator('a'), indicator('a'), indicator('b')];
function entry() {
  const classes = new Set();
  const make = () => ({ textContent: '', value: '', handlers: {}, addEventListener(k, fn) { this.handlers[k] = fn; }, focus() {}, select() {} });
  const value = make(), input = make();
  return { value, input, classList: { add: k => classes.add(k), remove: k => classes.delete(k), contains: k => classes.has(k) }, querySelector: selector => selector.endsWith('-value') ? value : input };
}
const entries = [entry(), entry()];
let peers = [
  { id: 'a', name: 'A', addr: 'a:8888', is_offline: false, presence_remaining_secs: 18, presence_grace_secs: 20 },
  { id: 'b', name: 'B', addr: 'b:8888', is_offline: false, presence_remaining_secs: 9, presence_grace_secs: 20 },
];
let renders = 0, saves = [], saveError = '', toast = '';
const context = vm.createContext({
  console, Map, Set,
  window: { NotificationUI: { onPeers() { renders++; } }, setTimeout, setInterval() { throw Error('Setting must not start a countdown'); } },
  document: {
    getElementById: id => id === 'desktop-refresh-countdown' ? entries[0] : id === 'android-refresh-countdown' ? entries[1] : null,
    querySelectorAll: selector => selector === '[data-presence-peer-id]' ? indicators : [],
  },
  apiGetPeers: async () => peers,
  apiGetSettings: async () => ({ presence_grace_secs: 20 }),
  apiUpdatePresenceGraceSecs: async seconds => { if (saveError) throw Error(saveError); saves.push(seconds); },
  showMessageActionToast(message) { toast = message; }, getActionErrorMessage: String,
  addUserToList() {}, sortUserList() {},
});
vm.runInContext(helper + setting + app.slice(app.indexOf('let peerRefreshInFlight'), app.indexOf('async function startPeerPolling')), context);
const flush = () => new Promise(resolve => setImmediate(resolve));
async function main() {
  vm.runInContext('initRefreshCountdown()', context);
  await flush();
  assert.equal(entries[0].value.textContent, '20s');
  await vm.runInContext('refreshPeerListNow()', context);
  assert.deepEqual(indicators.map(n => n.textContent), ['18s', '18s', '9s']);
  const initialRenders = renders;
  peers = peers.map(peer => ({ ...peer, presence_remaining_secs: peer.presence_remaining_secs - 1 }));
  await vm.runInContext('refreshPeerListNow()', context);
  assert.deepEqual(indicators.map(n => n.textContent), ['17s', '17s', '8s']);
  assert.equal(renders, initialRenders, 'Timing updates must bypass metadata render suppression');
  peers[0].presence_remaining_secs = 0;
  await vm.runInContext('refreshPeerListNow()', context);
  assert.equal(indicators[0].textContent, '待确认');
  peers[0].presence_remaining_secs = 20;
  await vm.runInContext('refreshPeerListNow()', context);
  assert.equal(indicators[1].textContent, '20s', 'A confirmed heartbeat resets both copies');
  peers[0].is_offline = true;
  peers[0].presence_remaining_secs = null;
  await vm.runInContext('refreshPeerListNow()', context);
  assert.equal(indicators[0].textContent, '离线');
  assert.equal(entries[0].value.textContent, '20s', 'List refresh must not count down the setting');
  entries[0].value.handlers.click({ preventDefault() {}, stopPropagation() {} });
  entries[0].input.value = '30';
  entries[0].input.handlers.keydown({ key: 'Enter', preventDefault() {} });
  await flush();
  assert.deepEqual(saves, [30]);
  assert.equal(entries[0].value.textContent, '30s');
  assert.equal(entries[1].value.textContent, '30s');
  saveError = 'readonly database';
  entries[0].value.handlers.click({ preventDefault() {}, stopPropagation() {} });
  entries[0].input.value = '40';
  entries[0].input.handlers.keydown({ key: 'Enter', preventDefault() {} });
  await flush();
  assert.match(toast, /保存断开判定时长失败/);
  assert.equal(entries[0].value.textContent, '30s', 'A failed save keeps the previous setting');
  assert.equal(entries[1].value.textContent, '30s');
  assert.deepEqual(saves, [30]);
  const unknown = indicator('missing');
  context.unknownIndicator = unknown;
  vm.runInContext('initPeerPresenceIndicator(unknownIndicator, "missing")', context);
  assert.equal(unknown.textContent, '—');
  console.log('PASS: independent peer times, synchronized lists, unchanged metadata, heartbeat, pending/offline, fixed editable setting');
}
main().catch(error => { console.error(error); process.exitCode = 1; });

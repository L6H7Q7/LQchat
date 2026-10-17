const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '..', 'src', 'js', 'ui.js'), 'utf8');
const helpers = source.slice(source.indexOf('function fileTransferLabel('), source.indexOf('function createMessageElement('));
const apiSource = fs.readFileSync(path.join(__dirname, '..', 'src', 'js', 'api.js'), 'utf8');
const chunkHelper = apiSource.slice(apiSource.indexOf('function calculateOptimalChunkSize('), apiSource.indexOf('async function apiSendFile('));
const chunkSize = vm.runInNewContext(`${chunkHelper}\ncalculateOptimalChunkSize(4 * 1024 ** 3)`);
assert.equal(chunkSize, 10 * 1024 * 1024, 'large files use 10 MiB chunks');
assert.equal(Math.ceil((4 * 1024 ** 3) / chunkSize), 410, '4 GiB files report progress across 410 confirmations');

function card(direction, id, total, file = true) {
  const status = {
    textContent: '', className: '',
    classList: { add() {}, remove() {}, contains() { return false; } },
  };
  const fill = { style: {} };
  const bar = { setAttribute(name, value) { this[name] = value; } };
  const detail = { textContent: '' };
  const activeClasses = new Set();
  const progress = {
    dataset: { total: String(total) }, hidden: false,
    // 进度条区域始终占位：可见性用 is-active 类切换，绝不使用 hidden 属性。
    classList: {
      toggle(name, on) { if (on) activeClasses.add(name); else activeClasses.delete(name); },
      contains(name) { return activeClasses.has(name); },
    },
    setAttribute() {},
    querySelector(selector) {
      return { '.file-transfer-fill': fill, '.file-transfer-bar': bar,
        '.file-transfer-detail': detail }[selector];
    },
  };
  const element = {
    dataset: { senderMsgId: String(id), msgId: String(id) },
    classList: { contains(name) { return name === direction; } },
    querySelector(selector) {
      return { '.message-file': file ? {} : null,
        '.file-transfer-status': status, '.file-transfer-progress': progress }[selector];
    },
  };
  return { element, status, fill, bar, detail, progress };
}

for (const platform of ['Windows', 'Android']) {
  const sent = card('sent', 7, 1000);
  const received = card('received', 7, 1000);
  const text = card('sent', 8, 1000, false);
  const fresh = card('sent', 9, 1000);
  const document = { getElementById() { return {
    querySelectorAll() { return [received.element, sent.element, text.element, fresh.element]; },
  }; } };
  const context = vm.createContext({ document, window: {}, performance: { now: () => 1000 } });
  vm.runInContext(helpers, context);
  const update = context.updateFileTransferById;

  update(7, 'uploading', 500, 1000, 1, true);
  assert.equal(sent.fill.style.width, '50%', `${platform} send uses acknowledged bytes`);
  assert.equal(sent.status.textContent, '', '进行中只显示进度条，不再显示状态文字');
  assert.match(sent.detail.textContent, /500 B \/ 1000 B.*1\.0 MB\/s/);
  assert.equal(received.progress.classList.contains('is-active'), false, `${platform} incoming card remains independent`);
  assert.equal(sent.progress.hidden, false, 'progress keeps its box so card height never changes');
  assert.equal(sent.progress.classList.contains('is-active'), true, `${platform} active transfer shows progress`);
  update(7, 'uploading', 2000, 1000, 1, true);
  assert.equal(sent.fill.style.width, '100%', 'progress never exceeds confirmed total');
  update(7, 'uploading', 500, 1000, 1, true);
  update(7, 'uploading', 501, 1000, 1);
  assert.equal(sent.detail.textContent.includes('501 B'), false, 'same-percent updates are throttled');
  update(7, 'uploading', 510, 1000, 1);
  assert.equal(sent.fill.style.width, '100%', 'progress never drops below the highest acknowledged bytes');
  update(9, 'uploading', 100, 1000, 1, true);
  update(9, 'uploading', 510, 1000, 1);
  assert.equal(fresh.fill.style.width, '51%', 'meaningful percent change renders immediately');

  update(7, 'downloading', 250, 1000, 0.5, true, 'received');
  assert.equal(received.fill.style.width, '25%', `${platform} receive uses written bytes`);
  assert.equal(sent.fill.style.width, '100%', 'send progress is unaffected by receive updates');
  update(7, 'saving', 1000, 1000, 0, true, 'received');
  assert.equal(received.status.textContent, '', '保存中不显示状态文字');
  update(7, 'accepted', 1000, 1000, 0, true, 'received');
  assert.equal(received.status.textContent, '', '完成态不显示状态文字');
  assert.equal(received.progress.classList.contains('is-active'), false);
  assert.equal(received.progress.hidden, false, 'finished transfer keeps the same card box');

  update(7, 'retrying', 0, 1000, 0, true);
  assert.equal(sent.fill.style.width, '0%', `${platform} retry resets progress`);
  assert.equal(sent.status.textContent, '', '重试中不显示状态文字');
  update(7, 'failed', 0, 1000, 0, true);
  assert.equal(sent.status.textContent, '发送失败', '失败状态必须保留文字说明');
  update(8, 'uploading', 500, 1000, 1, true);
  assert.equal(text.status.textContent, '', 'text messages have no progress');
}
console.log('Windows/Android send and receive progress contracts passed');

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const source = fs.readFileSync(path.join(__dirname, '..', 'src/js/ui.js'), 'utf8');
const preview = source.slice(source.indexOf('function openAndroidImagePreview('), source.indexOf('async function openAndroidAttachment('));
const navigationStart = source.indexOf('window.addEventListener("popstate", function (event)');
const navigation = source.slice(navigationStart, source.indexOf('// 发送消息', navigationStart));

function setup(width, baseHash = '#chat-attachment') {
  let overlay = null;
  let closedChat = 0;
  let hiddenLoading = 0;
  const panelClasses = new Set(['open']);
  const panel = { classList: {
    contains: name => panelClasses.has(name),
    remove: name => panelClasses.delete(name),
  } };
  const entries = [{ state: {}, hash: '' }];
  if (baseHash === '#chat-attachment') {
    entries.push({ state: { chatOpen: true }, hash: '#chat' });
    entries.push({ state: { attachmentOpen: true }, hash: baseHash });
  }
  let index = entries.length - 1;
  const location = { hash: baseHash };
  const queue = [];
  let onPopState;
  const history = {
    get state() { return entries[index].state; },
    get length() { return entries.length; },
    pushState(state, _, hash) {
      entries.splice(index + 1);
      entries.push({ state, hash });
      index++;
      location.hash = hash;
    },
    replaceState(state, _, hash) {
      entries[index] = { state, hash };
      location.hash = hash;
    },
    back() {
      queue.push(() => {
        if (!index) return;
        index--;
        location.hash = entries[index].hash;
        onPopState({ state: entries[index].state });
      });
    },
  };
  const document = {
    body: { classList: { contains: name => name === 'android-app' }, appendChild: node => { overlay = node; } },
    querySelector: () => overlay,
    getElementById: id => id === 'android-attachment-panel' ? panel : {},
    createElement() {
      return {
        querySelector: () => ({}),
        addEventListener(_, listener) { this.click = listener; },
        remove() { overlay = null; },
      };
    },
  };
  const window = {
    innerWidth: width, history, location,
    currentChatPeer: { id: 'peer' },
    selectMode: { active: false, selectedMessages: new Set(['selected-image']) },
    AndroidAppLoading: { hide() { hiddenLoading++; } },
    addEventListener(_, listener) { onPopState = listener; },
  };
  const context = vm.createContext({ window, document, location, console,
    performCloseChatUI() { closedChat++; window.currentChatPeer = null; },
  });
  vm.runInContext(preview + '\n' + navigation, context);
  return { context, window, history, location,
    overlay: () => overlay, panelOpen: () => panelClasses.has('open'),
    closedChat: () => closedChat, hiddenLoading: () => hiddenLoading,
    flush() { while (queue.length) queue.shift()(); },
  };
}

for (const width of [360, 1024]) {
  for (const close of ['gesture', 'click']) {
    const app = setup(width);
    const item = { thumbnail: 'data:image/png;base64,test' };
    app.context.openAndroidImagePreview({});
    assert.equal(app.overlay(), null, 'missing image creates no preview');
    const baseline = app.history.length;
    app.context.openAndroidImagePreview(item);
    app.context.openAndroidImagePreview(item);
    assert.equal(app.history.length, baseline + 1, 'duplicate opens add no extra history');
    assert.equal(app.location.hash, '#chat-attachment', 'native Android back recognizes the preview route');
    if (close === 'gesture') app.history.back();
    else { app.overlay().click(); app.overlay().click(); }
    app.flush();
    assert.equal(app.overlay(), null, 'first back closes the preview');
    assert(app.panelOpen(), 'first back preserves the image picker');
    assert.equal(app.hiddenLoading(), 0, 'preview close does not run attachment teardown');
    assert.equal(app.closedChat(), 0, 'first back preserves the chat');
    assert.equal(app.history.state.attachmentOpen, true, 'click and gesture restore the underlying history state');
    assert(app.window.selectMode.selectedMessages.has('selected-image'), 'selection survives preview close');
    app.context.openAndroidImagePreview(item);
    app.history.back(); app.flush();
    assert.equal(app.history.length, baseline + 1, 'repeated previews do not accumulate history');
    app.history.back(); app.flush();
    assert.equal(app.panelOpen(), false, 'second back closes the image picker');
    assert.equal(app.closedChat(), 0, 'second back preserves the chat');
    assert.equal(app.location.hash, '#chat');
    if (width <= 768) {
      app.history.back(); app.flush();
      assert.equal(app.closedChat(), 1, 'third back leaves the phone chat');
    }
  }
}

const tablet = setup(1024, '');
tablet.context.openAndroidImagePreview({ thumbnail: 'data:image/png;base64,test' });
assert.equal(tablet.location.hash, '#chat-attachment', 'wide layouts also expose a native back route');
tablet.history.back(); tablet.flush();
assert.equal(tablet.overlay(), null);
assert(tablet.panelOpen());
assert.equal(tablet.location.hash, '', 'wide layouts restore the original URL');
assert.equal(tablet.closedChat(), 0);
console.log('Android image preview back navigation checks passed (phone, tablet, gesture, click, repeated opens).');

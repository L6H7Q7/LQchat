// Explicitly scoped real-phone verification. Temporary app preference changes are restored.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');
const root = path.resolve(__dirname, '..');
const probe = path.join(root, 'artifacts/phone-save-diagnosis/probe.cjs');
const output = path.join(root, 'artifacts/phone-save-fix');
const adb = path.join(process.env.LOCALAPPDATA, 'Android/Sdk/platform-tools/adb.exe');
const serial = '10AFAM1FN5005XG';
const runProbe = (...args) => execFileSync(process.execPath, [probe, ...args], {encoding:'utf8',timeout:25000,maxBuffer:4*1024*1024});
const evaluate = expression => JSON.parse(runProbe('eval', expression));
const config = () => evaluate(`(async()=>({general:await apiGetSettings(),name:await apiGetMyName(),
  background:await window.__TAURI__.core.invoke('get_background_runtime_settings'),
  notifications:await window.__TAURI__.core.invoke('get_notifications_enabled'),
  permission:await window.__TAURI__.core.invoke('get_notification_permission_state')}))()`);
const wait = expression => assert(evaluate(`(async()=>{for(let i=0;i<150;i++){if(${expression})return true;await new Promise(r=>setTimeout(r,20));}return false;})()`), expression);
function main() {
  fs.mkdirSync(output,{recursive:true});
  const baselineFile=path.join(output,'before-install.json');
  if(process.argv[2]==='baseline') {
    assert(!fs.existsSync(baselineFile),'Do not overwrite pre-install baseline');
    fs.writeFileSync(baselineFile,JSON.stringify(config(),null,2));
    console.log('Pre-install configuration recorded'); return;
  }
  const before=JSON.parse(fs.readFileSync(baselineFile,'utf8'));
  assert.deepEqual(config(),before,'Upgrade must preserve configuration');
  for(const file of ['api.js','ui.js','app.js']) {
    const deployed=evaluate(`fetch('js/${file}').then(r=>r.text())`);
    const local=fs.readFileSync(path.join(root,'src/js',file),'utf8');
    const hash=text=>crypto.createHash('sha256').update(text.replace(/\r\n/g,'\n')).digest('hex');
    assert.equal(hash(deployed),hash(local),'Installed source mismatch: '+file);
  }
  let temporaryPreference=false;
  runProbe('trace-start');
  const samples=[];
  const openSettings=()=>{
    const hash=evaluate('location.hash');
    if(hash==='')evaluate(`document.getElementById('android-settings-btn').click();true`);
    wait(`location.hash==='#settings' && !document.getElementById('save-settings-btn').disabled`);
  };
  const openPermissions=()=>{
    openSettings();evaluate(`document.getElementById('android-permissions-btn').click();true`);
    wait(`location.hash==='#permissions'`);
  };
  const save=(id,name,hash)=>{
    const start=evaluate('window.__saveProbe.events.length');
    const point=evaluate(`(()=>{const n=document.getElementById('${id}'),r=n.getBoundingClientRect();
      if(n.disabled)throw Error('Save button disabled before tap');
      if(!n.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)))throw Error('Save button covered');
      return {x:Math.round((r.x+r.width/2)*devicePixelRatio),y:Math.round((r.y+r.height/2)*devicePixelRatio)};})()`);
    execFileSync(adb,['-s',serial,'shell','input','tap',String(point.x),String(point.y)]);
    wait(`window.__saveProbe.events.slice(${start}).some(e=>e.toast==='保存成功') && location.hash===${JSON.stringify(hash)}`);
    const events=evaluate(`window.__saveProbe.events.slice(${start})`);
    const clicked=events.find(e=>e.type==='click' && e.id===id);
    const busy=events.find(e=>e.settingsDisabled && e.saveLabel==='正在保存…');
    const success=events.find(e=>e.toast==='保存成功');
    assert(clicked && busy && success,'Must show busy before success');
    assert.equal(evaluate(`document.getElementById('${id}').disabled`),false);
    samples.push({name,toBusyMs:Math.round((busy.at-clicked.at)*10)/10,toSuccessMs:Math.round((success.at-clicked.at)*10)/10});
    if(samples.length===1) {
      fs.writeFileSync(path.join(output,'phone-save-success.png'),execFileSync(adb,['-s',serial,'exec-out','screencap','-p'],{maxBuffer:20*1024*1024}));
    }
  };
  try {
    for(let i=0;i<3;i++) {
      openPermissions();save('save-permissions-btn','permissions-'+(i+1),'#settings');
    }
    openPermissions();
    temporaryPreference=true;
    evaluate(`document.getElementById('background-keep-running-toggle').checked=${!before.background.keep_running};true`);
    save('save-permissions-btn','changed-background','#settings');
    assert.equal(config().background.keep_running,!before.background.keep_running,'Background preference must really persist');
    openPermissions();
    evaluate(`document.getElementById('background-keep-running-toggle').checked=${before.background.keep_running};true`);
    save('save-permissions-btn','restore-background','#settings');
    assert.deepEqual(config(),before,'Restore original settings after temporary preference test');
    temporaryPreference=false;
    save('save-settings-btn','general-settings','');
    openPermissions();
    execFileSync(adb,['-s',serial,'shell','input','keyevent','4']);wait(`location.hash==='#settings'`);
    execFileSync(adb,['-s',serial,'shell','input','keyevent','4']);wait(`location.hash===''`);
    assert.deepEqual(config(),before);
    const report={samples,configurationPreserved:true,backgroundWriteReadback:true,systemBackPassed:true};
    fs.writeFileSync(path.join(output,'results.json'),JSON.stringify(report,null,2));
    console.log(JSON.stringify(report,null,2));
  } finally {
    if(temporaryPreference) {
      evaluate(`window.__TAURI__.core.invoke('set_background_runtime_settings',{settings:${JSON.stringify(before.background)}})`);
      assert.deepEqual(config(),before,'Emergency preference restoration');
    }
    runProbe('trace-stop','fixed-phone-trace');
  }
  // Reload without writing: verify readback and leave the phone on the permission page.
  runProbe('reload');
  wait(`!!document.querySelector('.ns-app-picker-entry')`);
  openPermissions();
  assert.deepEqual(config(),before);
}
try {main();} catch(error) {console.error(error);process.exitCode=1;}

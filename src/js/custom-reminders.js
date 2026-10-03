/* Android-local custom reminders. The native scheduler owns next-push calculations and persistence. */
(() => {
  'use strict';
  const week = ['周日','周一','周二','周三','周四','周五','周六'];
  const pad = n => String(n).padStart(2, '0');
  const dateKey = d => `${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}`;
  const addDays = (d,n) => { const copy = new Date(d); copy.setDate(copy.getDate()+n); return copy; };
  const matches = (r,d) => ({daily:()=>true,weekly:()=>d.getDay()===r.weekday,monthly:()=>d.getDate()===r.monthday,
    once:()=>dateKey(d)===r.date,weekdays:()=>r.weekdays.includes(d.getDay()),monthdays:()=>r.monthdays.includes(d.getDate())}[r.mode]?.() || false);
  function fresh() { const today = new Date(); return {id:null,title:'',content:'',mode:'once',date:dateKey(addDays(today,3)),time:'18:00',
    weekday:today.getDay(),monthday:today.getDate(),weekdays:[1,3,5],monthdays:[1,15],countdown:false,dayGap:1,dayTime:'09:00',near:false,nearStart:60,nearGap:10}; }
  const fields = {title:'title',content:'content',mode:'mode',date:'date',time:'time',weekday:'weekday',monthday:'monthday',
    countdown:'countdown',dayGap:'day-gap',dayTime:'day-time',near:'near',nearStart:'near-start',nearGap:'near-gap'};
  const numbers = new Set(['weekday','monthday','dayGap','nearStart','nearGap']);
  let state = null, draft = null, busy = false, loaded = false, selectedDate = dateKey(new Date());
  const el = id => document.getElementById('cr-'+id);
  const invoke = request => window.__TAURI__.core.invoke('custom_reminder_settings', {request});
  const copy = v => JSON.parse(JSON.stringify(v));
  function setBusy(value) {
    busy = value;
    for (const id of ['add','save','delete','alarm-settings']) el(id).disabled = value || !loaded;
  }
  function accept(result) {
    if (!result || !Array.isArray(result.reminders) || !result.globalSettings) throw Error('自定义通知数据无效');
    if (result.error) throw Error(result.error);
    state = result; loaded = true;
    render();
  }
  async function load() {
    if (busy) return;
    setBusy(true);
    try { accept(await invoke({action:'get'})); el('status').textContent = ''; }
    catch (e) { el('status').textContent = '读取失败：'+String(e.message||e); }
    finally { setBusy(false); }
  }
  async function saveSettings(settings, success) {
    if (busy || !loaded) return false;
    setBusy(true);
    try { accept(await invoke({action:'save',settings})); el('status').textContent=success; return true; }
    catch(e) { (draft ? el('error') : el('status')).textContent='保存失败：'+String(e.message||e); return false; }
    finally { setBusy(false); }
  }
  function render() {
    if (!state) return;
    el('count').textContent=`自定义通知（${state.reminders.length}）`;
    const next = state.next?.[0];
    el('next-title').textContent=next?.title || (state.enabled === false ? '通知提醒已关闭' : '暂无待推送通知');
    el('next-content').textContent=next?.content || '';
    const kind = {event:'正式提醒',countdown:'倒计时提醒',near:'临近提醒'};
    const at = next && new Date(next.at);
    el('next-time').textContent=next?`${dateKey(at)} ${pad(at.getHours())}:${pad(at.getMinutes())}:${pad(at.getSeconds())} · ${kind[next.kind]||''}`:'';
    el('next-rule').textContent=next?`第 ${next.index}/${state.globalSettings.repeat} 次 · ${state.globalSettings.repeat===1?'间隔无':`间隔 ${state.globalSettings.repeatGap} 秒`}${state.next.length>1?` · 同时还有 ${state.next.length-1} 条`:''}`:'';
    el('alarm-settings').hidden=state.exactAllowed;
    el('permission').textContent=state.scheduleError?`调度失败：${state.scheduleError}`:!state.notificationAllowed?'系统通知权限未开启，请在系统设置中允许 LQChat 通知。':!state.exactAllowed?'未允许精确提醒，系统可能延迟推送。':'精确提醒已允许；手机休眠或省电限制仍可能延迟连续提醒。';
    const today = new Date(), days=Array.from({length:7},(_,i)=>addDays(today,i));
    if (!days.some(d=>dateKey(d)===selectedDate)) selectedDate=dateKey(today);
    el('days').replaceChildren();
    days.forEach((d,i)=>{
      const b=document.createElement('button');b.type='button';b.className='cr-day';b.textContent=week[d.getDay()];
      b.setAttribute('aria-pressed',String(dateKey(d)===selectedDate));b.setAttribute('aria-label',`${i===0?'今天 ':''}${dateKey(d)} ${week[d.getDay()]}，查看当天通知`);
      if(i===0)b.setAttribute('aria-current','date');
      const number=document.createElement('span');number.textContent=`${d.getMonth()+1}/${d.getDate()}`;b.append(number);
      b.addEventListener('click',()=>{selectedDate=dateKey(d);render();});el('days').append(b);
    });
    const day=days.find(d=>dateKey(d)===selectedDate);
    const rows=state.reminders.filter(r=>matches(r,day)).sort((a,b)=>a.time.localeCompare(b.time));
    el('day-heading').textContent=`${week[day.getDay()]} ${day.getMonth()+1}/${day.getDate()} · ${rows.length} 条通知`;
    el('list').replaceChildren();
    rows.forEach(r=>{
      const b=document.createElement('button');b.type='button';b.className='cr-row';
      const time=document.createElement('time');time.textContent=r.time;
      const text=document.createElement('span');text.textContent=`${r.title} · ${r.content}`;
      b.setAttribute('aria-label',`编辑 ${r.title}，${r.time}，${r.content}`);b.append(time,text,document.createTextNode('›'));
      b.addEventListener('click',()=>open(r));el('list').append(b);
    });
    if(!rows.length)el('list').textContent='当天暂无提醒';
  }
  function readDraft() {
    for(const [key,id] of Object.entries(fields)){const node=el(id);draft[key]=node.type==='checkbox'?node.checked:numbers.has(key)?Number(node.value):node.value;}
    return draft;
  }
  function updateFields() {
    if(!draft)return;readDraft();
    for(const [id,mode] of [['date','once'],['week','weekly'],['month','monthly'],['weekdays','weekdays'],['monthdays','monthdays']])el(id+'-field').hidden=draft.mode!==mode;
    el('date').required=draft.mode==='once';el('monthday').required=draft.mode==='monthly';el('monthday').disabled=draft.mode!=='monthly';
    el('month-note').hidden=!['monthly','monthdays'].includes(draft.mode);
    el('countdown-fields').hidden=!draft.countdown;el('countdown-fields').disabled=!draft.countdown;
    el('near-fields').hidden=!draft.near;el('near-fields').disabled=!draft.countdown||!draft.near;
  }
  function chips(id,values,selected,label) {
    el(id).replaceChildren();values.forEach(value=>{
      const b=document.createElement('button');b.type='button';b.className='cr-chip';b.textContent=label(value);b.setAttribute('aria-pressed',String(selected.includes(value)));
      b.addEventListener('click',()=>{const i=selected.indexOf(value);if(i<0)selected.push(value);else selected.splice(i,1);b.setAttribute('aria-pressed',String(selected.includes(value)));});el(id).append(b);
    });
  }
  function open(r) {
    if(busy||!loaded)return;
    draft=copy(r||fresh());el('heading').textContent=draft.id?'编辑自定义通知':'添加自定义通知';el('delete').hidden=!draft.id;el('error').textContent='';
    for(const [key,id] of Object.entries(fields)){const node=el(id);if(node.type==='checkbox')node.checked=draft[key];else node.value=draft[key];}
    chips('weekdays',[1,2,3,4,5,6,0],draft.weekdays,v=>week[v]);chips('monthdays',Array.from({length:31},(_,i)=>i+1),draft.monthdays,v=>`${v}日`);
    updateFields();history.pushState({customReminder:true},'', '#custom-reminder');syncPage();
  }
  function syncPage() {
    const editing=location.hash==='#custom-reminder';
    document.getElementById('android-custom-reminder-panel').classList.toggle('is-open',editing);
    if(editing&&!draft){location.replace('#notification-options');return;}
    if(!editing)draft=null;
  }
  function back() { if(busy||location.hash!=='#custom-reminder')return;history.back(); }
  async function save(event) {
    event.preventDefault();if(!draft||busy)return;readDraft();
    if(!draft.title.trim()||!draft.content.trim()){el('error').textContent='请填写标题和内容';return;}
    if(draft.mode==='weekdays'&&!draft.weekdays.length||draft.mode==='monthdays'&&!draft.monthdays.length){el('error').textContent='至少选择一个提醒日期';return;}
    if(draft.mode==='once'&&new Date(draft.date+'T'+draft.time)<=new Date()){el('error').textContent='请选择未来的事件日期和时间';return;}
    if(draft.countdown&&draft.near&&draft.nearGap>draft.nearStart){el('error').textContent='临近提醒间隔不能大于提前开始时间';return;}
    const r=copy(draft);r.title=r.title.trim();r.content=r.content.trim();r.id=r.id||('reminder-'+crypto.randomUUID());
    const all=state.reminders.filter(v=>v.id!==r.id).concat(r);
    if(await saveSettings({reminders:all,globalSettings:state.globalSettings},'已保存'))back();
  }
  async function remove() {
    if(!draft?.id||busy)return;
    if(!window.confirm(`删除“${draft.title}”及其后续提醒？`))return;
    if(await saveSettings({reminders:state.reminders.filter(r=>r.id!==draft.id),globalSettings:state.globalSettings},'已删除'))back();
  }
  function init() {
    const android=navigator.userAgent.includes('Android')||new URLSearchParams(location.search).get('android-preview')==='1';
    if(!android||!window.__TAURI__)return;
    const host=document.querySelector('#android-notification-options-panel .settings-content');
    const section=document.createElement('section');section.className='cr-card android-only';
    section.innerHTML=`<div class="cr-header"><strong id="cr-count">自定义通知（0）</strong><button id="cr-add" type="button" class="cr-link">＋ 添加</button></div>
      <div class="cr-next" aria-live="polite"><span class="cr-hint">下一条待推送通知</span><div id="cr-next-time" class="cr-hint"></div><strong id="cr-next-title">正在读取…</strong><div id="cr-next-content"></div><div id="cr-next-rule" class="cr-hint"></div></div>
      <div id="cr-days" class="cr-days" aria-label="从今天起的七天"></div>
      <div id="cr-permission" class="cr-hint"></div><button id="cr-alarm-settings" class="cr-link" type="button" hidden>允许精确提醒</button>
      <div id="cr-day-heading" class="cr-hint"></div><div id="cr-list"></div><div id="cr-status" class="cr-error" role="status" aria-live="polite"></div>`;
    host.append(section);
    for(const day of [1,2,3,4,5,6,0]){const o=document.createElement('option');o.value=day;o.textContent=week[day];el('weekday').append(o);}
    el('add').addEventListener('click',()=>open());el('back').addEventListener('click',back);el('delete').addEventListener('click',remove);el('form').addEventListener('submit',save);
    el('form').addEventListener('input',()=>{updateFields();el('error').textContent='';});el('form').addEventListener('change',updateFields);
    window.addEventListener('shared-reminder-settings-changed',()=>void load());
    el('alarm-settings').addEventListener('click',async()=>{try{await invoke({action:'exact_settings'});}catch(e){el('status').textContent=String(e.message||e);}});
    const navigation=()=>{syncPage();if(location.hash==='#notification-options')void load();};
    window.addEventListener('popstate',navigation);window.addEventListener('hashchange',navigation);
    document.addEventListener('visibilitychange',()=>{if(!document.hidden&&location.hash==='#notification-options')void load();});
    for(const id of ['notification-toggle','notification-sound-toggle'])document.getElementById(id)?.addEventListener('change',()=>setTimeout(()=>void load(),600));
    setInterval(()=>{if(!document.hidden&&location.hash==='#notification-options'&&!draft)void load();},15000);
    syncPage();void load();
  }
  document.addEventListener('DOMContentLoaded',init);
})();

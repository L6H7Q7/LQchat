// Local-only UI fixture. Serves the real frontend with a disposable Tauri adapter.
const http = require("http"),
  fs = require("fs"),
  path = require("path");
const root = path.resolve(__dirname, "../src");
const fixtureIcon = fs.readFileSync(path.resolve(__dirname, "../src-tauri/icons/32x32.png")).toString("base64");
const fixture = `(() => {
  const android = new URLSearchParams(location.search).has('android-preview');
  const peers = [{id:'iqoo',name:'IQOO',addr:'192.168.5.10:8888',is_offline:false,notification_push_enabled:true,notification_push_target_device_ids:['preview-local']},{id:'redm',name:'REDM',addr:'192.168.5.11:8888',is_offline:false,notification_push_enabled:true,notification_push_target_device_ids:['other-device']},{id:'4060',name:'4060',addr:'192.168.5.4:8888',is_offline:true,notification_push_enabled:false,notification_push_target_device_ids:[]}];
  peers.forEach(peer=>{peer.notification_receive_enabled=!peer.is_offline;});
  let settings = {push_enabled:false,receive_enabled:true,lq_battery_push_enabled:false,lq_reminder_push_enabled:false,allowed_packages:[],target_device_ids:[]};
  const dateKey=d=>d.getFullYear()+'-'+String(d.getMonth()+1).padStart(2,'0')+'-'+String(d.getDate()).padStart(2,'0');
  const today=new Date(), tomorrow=new Date(today);tomorrow.setDate(today.getDate()+1);
  let custom={reminders:[{id:'fixture-today',title:'今日收菜',content:'今日提醒',mode:'once',date:dateKey(today),time:'18:00',weekday:today.getDay(),monthday:today.getDate(),weekdays:[1],monthdays:[1],countdown:false,dayGap:1,dayTime:'09:00',near:false,nearStart:60,nearGap:10},{id:'fixture-tomorrow',title:'明日浇花',content:'明日提醒',mode:'once',date:dateKey(tomorrow),time:'19:00',weekday:tomorrow.getDay(),monthday:tomorrow.getDate(),weekdays:[1],monthdays:[1],countdown:false,dayGap:1,dayTime:'09:00',near:false,nearStart:60,nearGap:10}],globalSettings:{repeat:3,repeatGap:5}};
  custom.globalSettings={repeat:1,repeatGap:0};
  let background={keep_running:false,start_on_boot:false,exclude_from_recents:false,battery_alert_enabled:false,battery_alert_interval_seconds:0,battery_alert_repeat_count:1,battery_alert_levels:[50,100]};
  window.__customReminderFixture={writes:0,fail:false,notificationWrites:[],failNotificationSettings:false,backgroundWrites:[],failBackground:false};
  const listeners = new Map();
  const records = [{peer_id:'iqoo',peer_name:'IQOO',view_kind:'notification_receive',status:'success',notification:{msg_type:'notification',event_id:'preview-event',source_device_id:'iqoo',target_device_id:'preview-local',package:'example.messages',app_name:'短信',title:'快递到达提醒',text:'您的包裹已到达驿站。此内容是本地界面测试数据，不是真实系统通知。',notification_key:'preview-key',post_time:Date.now()}}];
  records[0].notification.app_icon='${fixtureIcon}';
  records.push({...records[0],notification:{...records[0].notification,event_id:'without-icon',notification_key:'without-icon',app_name:'无图标示例',title:'文字仍然可读',text:'应用图标不可用时显示应用名称首字。',app_icon:null}});
  window.__TAURI__={event:{listen:async(name,fn)=>{if(!listeners.has(name))listeners.set(name,[]);listeners.get(name).push(fn);return()=>{};}},core:{invoke:async(command,args={})=>{
    switch(command){
      case 'custom_reminder_settings':{
        if(args.request.action==='save'){if(window.__customReminderFixture.fail)throw Error('模拟保存失败');custom={...JSON.parse(JSON.stringify(args.request.settings)),globalSettings:custom.globalSettings};window.__customReminderFixture.writes++;}
        const r=custom.reminders[0];return {...custom,exactAllowed:false,notificationAllowed:true,enabled:true,next:r?[{id:r.id,title:r.title,content:r.content,at:Date.now()+60000,kind:'event',index:1}]:[]};
      }
      case 'notification_settings':if(args.settings){if(window.__customReminderFixture.failNotificationSettings)throw Error('模拟推送设置保存失败');settings=args.settings;window.__customReminderFixture.notificationWrites.push(JSON.parse(JSON.stringify(settings)));}return{settings,platform:android?'android':'windows',access:false,permission:'unknown'};
      case 'notification_records':return records;
      case 'notification_action':
        if(args.action==='apps')return{apps:[{package:'example.messages',name:'短信',icon:'${fixtureIcon}'},{package:'example.chat',name:'聊天',icon:null}]};
        if(args.action==='replace_allowed'){settings={...settings,allowed_packages:[...(args.payload?.packages||[])]};return{settings,platform:android?'android':'windows',access:false,permission:'unknown'};}
        return{settings,platform:android?'android':'windows',access:false,permission:'unknown'};
      case 'notification_test':if(!settings.push_enabled)throw Error('信息推送未开启');if(!settings.target_device_ids.length)throw Error('请先选择目标设备');return null;
      case 'get_my_name':return android?'我的手机':'我的电脑';case 'get_my_id':return 'preview-local';
      case 'get_peers':return peers;case 'get_local_device_info':return{ip:'192.168.5.20',port:8888};
      case 'get_settings':return{download_path:'D:/Downloads',port:8888,auto_download:true,close_to_tray:true};
      case 'get_language':return 'zh';case 'get_current_theme':return 'default';case 'get_themes':return [];
      case 'get_default_download_path':return 'D:/Downloads';case 'get_notifications_enabled':return true;
      case 'get_unread_count':return 0;case 'get_notification_permission_state':return 'granted';
      case 'get_background_receive_state':return{state:'RUNNING',last_error_message:null};
      case 'get_background_runtime_settings':return {...background};
      case 'set_background_runtime_settings':{
        if(window.__customReminderFixture.failBackground)throw Error('模拟统一配置保存失败');
        background={...args.settings};custom.globalSettings={repeat:background.battery_alert_repeat_count,repeatGap:background.battery_alert_interval_seconds};
        window.__customReminderFixture.backgroundWrites.push({...background});return {...background};
      }
      case 'get_battery_optimization_state':return'unrestricted';
      case 'get_core_status':return{state:'RUNNING'};
      default:return [];
    }
  }}};
  document.addEventListener('DOMContentLoaded',()=>{
    const toolbar=document.createElement('div');toolbar.style='position:fixed;bottom:0;right:0;z-index:50;background:white;font-size:10px;';
    const add=document.createElement('button');add.textContent='测试：新增通知';add.onclick=()=>{records.unshift({...records[0],notification:{...records[0].notification,event_id:'fixture-'+Date.now(),notification_key:'key-'+Date.now(),title:'新增测试通知',text:'新的示例内容'}});for(const fn of listeners.get('notification-records-changed')||[])fn({payload:null});};
    const update=document.createElement('button');update.textContent='测试：更新通知';update.onclick=()=>{records[records.length-1].notification.text='已更新的测试正文。'+ '用于验证长文本展开和阅读位置。'.repeat(40);for(const fn of listeners.get('notification-records-changed')||[])fn({payload:null});};
    const fail=document.createElement('button');fail.textContent='测试：设置保存失败';fail.onclick=()=>{const original=window.__TAURI__.core.invoke;window.__TAURI__.core.invoke=(c,a)=>c==='notification_settings'&&a?.settings?Promise.reject('模拟保存失败'):original(c,a);};
    toolbar.append(add,update,fail);document.body.append(toolbar);
  });
})();`;
http
  .createServer((req, res) => {
    const pathname = new URL(req.url, "http://localhost").pathname;
    if (pathname === "/fixture.js") {
      res.setHeader("Content-Type", "text/javascript; charset=utf-8");
      return res.end(fixture);
    }
    const file = path.resolve(
      root,
      "." + decodeURIComponent(pathname === "/" ? "/index.html" : pathname),
    );
    if (!file.startsWith(root + path.sep)) {
      res.writeHead(403);
      return res.end();
    }
    try {
      let bytes = fs.readFileSync(file);
      const ext = path.extname(file);
      res.setHeader(
        "Content-Type",
        ({
          ".html": "text/html",
          ".js": "text/javascript",
          ".css": "text/css",
          ".png": "image/png",
        }[ext] || "application/octet-stream") + "; charset=utf-8",
      );
      if (ext === ".html")
        bytes = bytes
          .toString()
          .replace(
            '<script src="js/api.js">',
            '<script src="/fixture.js"></script><script src="js/api.js">',
          );
      res.end(bytes);
    } catch {
      res.writeHead(404);
      res.end();
    }
  })
  .listen(18764, "127.0.0.1", () =>
    console.log(
      "Notification UI fixture: http://127.0.0.1:18764/?desktop-preview=1",
    ),
  );

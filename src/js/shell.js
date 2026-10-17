// 首绘前确定外壳布局类。
// CSP 禁止内联脚本，因此用这个外部脚本作为 <body> 的第一个子节点同步执行：
// 浏览器解析到它时页面内容还没开始排版，此时写入 android-app / windows-app，
// 首帧就会带着最终外壳样式，不会先画出“默认文字/按钮”的骨架再跳成正式布局。
(function () {
  "use strict";
  var body = document.body;
  if (!body) return;

  var userAgent = navigator.userAgent || "";
  var params = new URLSearchParams(location.search);
  var androidPreview = params.get("android-preview") === "1";
  var desktopPreview = params.get("desktop-preview") === "1";
  var hasTauri = !!window.__TAURI__;

  // 与 app.js 的 renderPage 判定保持一致；Android WebView 的 UA 足以确定外壳。
  var android = userAgent.indexOf("Android") >= 0 || androidPreview;
  var windows = !android && (desktopPreview || (hasTauri && userAgent.indexOf("Windows") >= 0));

  if (android) {
    body.classList.add("android-app");
    document.documentElement.classList.add("android-app");
  } else if (windows) {
    body.classList.add("windows-app");
    document.documentElement.classList.add("windows-app");
  }

  // 供自动化验证读取：首个渲染机会（早于首帧绘制）时的外壳类与布局几何。
  requestAnimationFrame(function () {
    var header = document.querySelector(".header");
    var home = document.getElementById("android-home");
    var measure = function (node) {
      if (!node) return null;
      var rect = node.getBoundingClientRect();
      if (!rect.width && !rect.height) return { visible: false };
      return {
        visible: true,
        x: Math.round(rect.x * 10) / 10,
        y: Math.round(rect.y * 10) / 10,
        width: Math.round(rect.width * 10) / 10,
        height: Math.round(rect.height * 10) / 10,
      };
    };
    window.__shellFirstFrame = {
      className: body.className,
      header: measure(header),
      home: measure(home),
    };
  });
})();

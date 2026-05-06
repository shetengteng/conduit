// proxy.pac —— 局域网 VPN 共享的智能路由配置
// 由 server-app 通过 http://<server-LAN-IP>:8080/proxy.pac 对外暴露。
//
// 路由规则:
//   1. 本地 / 私网 / 链路本地地址                 → DIRECT
//   2. Zoom 内网域名（必须走 VPN 才能访问）       → PROXY（无 fallback）
//   3. 可能被 GFW 阻断、走 VPN 更稳的境外域名     → PROXY（无 fallback）
//      [背景说明：原方案是 "PROXY first, DIRECT on failure"，但浏览器
//       一旦在 A 重启等短暂窗口内 fallback 到 DIRECT，就会缓存这个决策
//       5–30 分钟；对被 GFW 阻断的目标这意味着用户要等 30s+ DIRECT 超时。
//       关掉 fallback，让代理故障立即暴露为快速失败，而不是慢挂。]
//   4. 国内大流量域名                             → DIRECT（直连本地 ISP）
//   5. 其他所有                                   → DIRECT
//
// 匹配函数:
//   shExpMatch  —— glob 风格精确匹配，例如 "git.zoom.us"
//   dnsDomainIs —— 同时覆盖裸域名和所有子域名
//                  例如 dnsDomainIs(host, "google.com") 同时匹配
//                  "google.com" / "www.google.com" / "ai.google.com"
//
// 新增域名: 直接在对应分组里加一行 `dnsDomainIs(host, "newdomain.com")`。
// server 端 /check?host=xxx 接口可以用来确认某个 host 会落到哪个分组。

function FindProxyForURL(url, host) {
    var PROXY = "PROXY __PROXY_HOST__:__PROXY_PORT__";
    var DIRECT = "DIRECT";

    host = host.toLowerCase();

    // ---------- 1. 本地 / 私网 / 链路本地地址 ----------
    if (isPlainHostName(host)
        || shExpMatch(host, "localhost")
        || dnsDomainIs(host, "local")
        || dnsDomainIs(host, "lan")
        || dnsDomainIs(host, "internal")
        || isInNet(host, "10.0.0.0",    "255.0.0.0")
        || isInNet(host, "172.16.0.0",  "255.240.0.0")
        || isInNet(host, "192.168.0.0", "255.255.0.0")
        || isInNet(host, "127.0.0.0",   "255.0.0.0")
        || isInNet(host, "169.254.0.0", "255.255.0.0")) {
        return DIRECT;
    }

    // ---------- 2. Zoom 内网域名 —— 必须走 VPN ----------
    if (dnsDomainIs(host, "zoom.us")
        || dnsDomainIs(host, "zoomdev.us")
        || dnsDomainIs(host, "corp.zoom.us")
        || dnsDomainIs(host, "ops.corp.zoom.us")
        || dnsDomainIs(host, "zoomvideo.atlassian.net")
        || dnsDomainIs(host, "eng.corp.zoom.com")
        || dnsDomainIs(host, "zoom.com")) {
        return PROXY;
    }

    // ---------- 3. 可能需要走 VPN 的境外域名 ----------
    if (dnsDomainIs(host, "google.com")
        || dnsDomainIs(host, "googleapis.com")
        || dnsDomainIs(host, "googleusercontent.com")
        || dnsDomainIs(host, "gstatic.com")
        || dnsDomainIs(host, "youtube.com")
        || dnsDomainIs(host, "ytimg.com")
        || dnsDomainIs(host, "github.com")
        || dnsDomainIs(host, "githubusercontent.com")
        || dnsDomainIs(host, "githubassets.com")
        || dnsDomainIs(host, "openai.com")
        || dnsDomainIs(host, "anthropic.com")
        || dnsDomainIs(host, "claude.ai")
        || dnsDomainIs(host, "twitter.com")
        || dnsDomainIs(host, "x.com")
        || dnsDomainIs(host, "facebook.com")
        || dnsDomainIs(host, "instagram.com")
        || dnsDomainIs(host, "medium.com")
        || dnsDomainIs(host, "notion.so")
        || dnsDomainIs(host, "wikipedia.org")) {
        return PROXY;
    }

    // ---------- 4. 国内大流量域名 —— 直连本地 ISP ----------
    if (dnsDomainIs(host, "baidu.com")
        || dnsDomainIs(host, "taobao.com")
        || dnsDomainIs(host, "tmall.com")
        || dnsDomainIs(host, "alipay.com")
        || dnsDomainIs(host, "aliyuncs.com")
        || dnsDomainIs(host, "qq.com")
        || dnsDomainIs(host, "weixin.qq.com")
        || dnsDomainIs(host, "bilibili.com")
        || dnsDomainIs(host, "bytedance.com")
        || dnsDomainIs(host, "douyin.com")
        || dnsDomainIs(host, "jd.com")
        || dnsDomainIs(host, "weibo.com")
        || dnsDomainIs(host, "163.com")
        || dnsDomainIs(host, "sina.com.cn")) {
        return DIRECT;
    }

    // ---------- 5. 兜底默认 ----------
    return DIRECT;
}

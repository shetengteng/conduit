// proxy.pac — LAN VPN sharing smart routing
// Hosted at http://<machine-A-LAN-IP>:8080/proxy.pac
//
// Behavior:
//   1. Local / private / link-local hosts        → DIRECT
//   2. Internal-only hosts (Zoom corp/dev)       → PROXY (no fallback)
//   3. Hosts that may need VPN to reach          → PROXY (no fallback)
//      [historic: was "PROXY first, DIRECT on failure" — but browsers
//       trigger that fallback whenever proxy is briefly unreachable
//       (e.g. during A's restart) and then cache the DIRECT decision
//       for 5–30 minutes. For GFW-blocked hosts that means the user
//       silently waits 30s+ on DIRECT timeouts. We disable fallback so
//       proxy outages surface as immediate errors instead of slow hangs.]
//   4. Large CN traffic                          → DIRECT (use local ISP)
//   5. Everything else                           → DIRECT
//
// shExpMatch  — glob pattern, exact host match (e.g. "git.zoom.us")
// dnsDomainIs — covers the bare domain AND every subdomain
//               (e.g. dnsDomainIs(host, "google.com") matches "google.com"
//                AND "www.google.com" AND "ai.google.com")
//
// To add a host: drop a `dnsDomainIs(host, "newdomain.com")` line into the
// matching section. Server's /check?host=xxx tells you which section a host
// would land in.

function FindProxyForURL(url, host) {
    var PROXY = "PROXY __PROXY_HOST__:__PROXY_PORT__";
    var DIRECT = "DIRECT";

    host = host.toLowerCase();

    // ---------- 1. Local / private / link-local ----------
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

    // ---------- 2. Zoom internal — must go via VPN ----------
    if (dnsDomainIs(host, "zoom.us")
        || dnsDomainIs(host, "zoomdev.us")
        || dnsDomainIs(host, "corp.zoom.us")
        || dnsDomainIs(host, "ops.corp.zoom.us")
        || dnsDomainIs(host, "zoomvideo.atlassian.net")
        || dnsDomainIs(host, "eng.corp.zoom.com")
        || dnsDomainIs(host, "zoom.com")) {
        return PROXY;
    }

    // ---------- 3. May require VPN — try proxy then DIRECT ----------
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

    // ---------- 4. Large CN traffic — direct via local ISP ----------
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

    // ---------- 5. Default ----------
    return DIRECT;
}

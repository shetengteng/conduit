/**
 * 中文(简体) — Conduit Server UI
 *
 * key 命名约定:
 *   - 顶层按"区域"分组(nav / status / dashboard / logs / settings / boot / actions / units / common)
 *   - 二级 camelCase
 *   - 插值用 {var}
 *
 * 与 en-US.ts 保持完全相同的 key 集合(测试时 i18n.fallbackLocale 会兜底)。
 */
export default {
  app: {
    name: "Conduit",
    role: "Server",
  },

  nav: {
    dashboard: "仪表盘",
    logs: "日志",
    settings: "设置",
    language: "语言",
  },

  status: {
    running: "运行中",
    stopped: "已停止",
    notStarted: "未启动",
    notReady: "未就绪",
    vpnError: "VPN 异常",
    portsPending: "端口待分配",
    sseConnected: "SSE 已订阅",
    sseDisconnected: "SSE 未连接",
  },

  topbar: {
    sub: {
      notStarted: "点击侧边栏的「设置」开始配置代理",
      stopped: "代理服务已停止,等待重新启动",
      vpnError: "VPN 接口未就绪,部分流量可能无法走代理",
      notReady: "代理正在启动中,端口尚未就绪",
      waiting: "等待客户端接入",
      passiveOnly: "{count} 个客户端已链接(待命中)",
      activeOnly: "{count} 个客户端正在传输流量",
      mixed: "共 {total} 个客户端 · {active} 传输 + {passive} 待命",
    },
    uptime: "运行 {value}",
    stopAndQuit: "停止代理并退出",
    restart: "重启代理",
    restarting: "重启中…",
    restartTitle: "重新启动 sidecar 让代理回到运行中",
    confirmStopTitle: "确认停止 Conduit Server?",
    confirmStopBody:
      "停止后代理引擎(HTTP / SOCKS5 / mDNS 广播)将全部下线,正在使用本机 VPN 的客户端会立刻断开。",
    confirmStopHint:
      "v0.1 阶段不支持在窗口里重启代理,需要重新打开 Conduit Server 应用(或在终端跑 {cmd})才能再次启动。",
    confirmStopOk: "确认停止",
    confirmStopOkLoading: "停止中…",
    cancel: "取消",
    toastStopOk: "应用即将退出",
    toastStopOkDetail:
      "代理引擎已停止,Conduit Server 进程正在清理资源…",
    toastStopFail: "停止失败",
    toastRestartTip: "正在重启应用…",
    toastRestartTipDetail: "主窗口将立即关闭并重新启动 sidecar",
    toastRestartFail: "重启失败",
    toastRestartFailHint: "请手动退出并重新打开 Conduit Server",
    tauriUnavailable: "Tauri invoke 不可用 (可能在浏览器中预览)",
  },

  dashboard: {
    proxyEngine: "代理引擎",
    kpi: {
      clients: "已链接客户端",
      clientsUnit: "个",
      down: "下行",
      up: "上行",
      uptime: "运行时长",
    },
    clientsSub: {
      waiting: "等待客户端接入",
      activeOnly: "{count} 个正在传输流量",
      passiveOnly: "{count} 个待命中(暂无流量)",
      mixed: "{active} 传输中 · {passive} 待命",
    },
    trafficDirection: {
      idle: "当前空闲",
      down: "同事 → 服务端",
      up: "服务端 → 同事",
    },
    uptimeSub: {
      stable: "稳定运行中",
      notStarted: "尚未启动",
    },
  },

  boot: {
    splashTitle: "Conduit Server",
    splashSub: "正在启动代理引擎,请稍候…",
    phase: {
      port: "分配端口",
      engine: "启动代理引擎",
      health: "健康检查",
      ready: "就绪",
    },
    failedTitle: "代理引擎启动失败",
    failedSub: "Tauri 主进程未能在超时时间内完成健康检查",
    failedHints: "可以这样做:",
    failedHint1: "点「重试」再启动一次,sidecar 冷启动有时需要更久",
    failedHint2: "如反复失败,查看日志: ~/.conduit/logs/proxy.log",
    failedHint3: "若怀疑环境问题,可重启电脑或重新安装 Conduit Server",
    retry: "重试",
    quit: "退出",
  },

  language: {
    label: "语言 / Language",
    zh: "简体中文",
    en: "English",
  },

  settings: {
    title: "设置",
    subtitle: "端口、安全策略、mDNS 广播 —— v0.1 阶段为只读占位",
    readonly: "只读",
    readonlyAlertBody:
      "v0.1 阶段所有配置项为只读展示,与代理运行状态无关。如需自定义,请通过启动参数(如 {mdns} / {port})指定。在窗口里编辑并热重启的能力将在 S4 与打包同步发布。",

    general: {
      title: "通用",
      desc: "界面语言等本地偏好,切换后立即生效。",
      languageLabel: "界面语言",
      languageHint: "已保存到 localStorage,下次启动应用时自动应用。",
    },

    ports: {
      title: "端口",
      desc: "这些端口由 Tauri 主进程在启动时通过 portpicker 动态分配,无法在窗口里修改",
      http: "HTTP 代理",
      socks5: "SOCKS5 代理",
      api: "控制 API",
      httpHint: "v0.1 阶段不支持窗口内编辑;启动 server 时用 --http-port 指定",
      socks5Hint:
        "v0.1 阶段不支持窗口内编辑;启动 server 时用 --socks5-port 指定",
      apiHint:
        "v0.1 阶段不支持窗口内编辑;启动 server 时用 --api-port 指定(loopback only)",
    },

    security: {
      title: "安全",
      desc: "仅允许下列 LAN 段 / 目标端口的连接通过代理",
      allowedCidrs: "允许接入的 LAN 段(CIDR)",
      allowedPorts: "CONNECT 允许的目标端口",
    },

    mdns: {
      title: "mDNS 广播",
      desc: "让 LAN 上的 Conduit Client 自动发现本机",
      enable: "启用 mDNS 广播",
      name: "广播名称",
      nameHint:
        "默认取系统短主机名;如需自定义请用 {cmd} 启动 server",
      nameTitle: "v0.1 不支持窗口内编辑;启动时用 {cmd} 指定",
      type: "服务类型",
    },

    about: {
      title: "关于",
      checkUpdate: "检查更新",
      checking: "检查中…",
      upToDate: "已是最新版本",
      upToDateDetail: "本地 v{local} = 远端 {latest}",
      updateAvailable: "发现新版本 {latest}",
      updateAvailableDetail: "本地 v{local},点击通知前往下载",
      noRelease: "尚无正式版本",
      noReleaseDetail: "GitHub Releases 暂未发布,请稍后再试",
      networkError: "网络异常",
      networkErrorDetail: "无法访问 GitHub,请检查代理 / VPN 状态",
      rateLimited: "GitHub 限流",
      rateLimitedDetail: "公共 IP 触发 60 次/小时限制,请稍后再试",
    },
  },

  logs: {
    title: "日志",
    subtitle: "实时订阅代理引擎的事件流,按关键词过滤,便于排查接入问题",
    eventStream: "事件流",
    searchPlaceholder: "搜索 host / IP / 关键词…",
    clear: "清空",
    autoScroll: "自动滚动到最新",
    maxKept: "最多保留 {max} 条",
    emptyMatch: "未匹配到任何日志",
    emptyAll: "尚无事件流入",
    emptyHint:
      "日志只在以下事件发生时被推送:\n· 代理引擎重启 (ready)\n· 客户端经 SOCKS5/HTTP 发起请求 (connected/disconnected)\n· 客户端心跳上报 (passive_client_seen)\n· macOS 路由表的 VPN 状态变更",
    panelMounted: "[ui] 日志面板已挂载,等待 sidecar 事件流",
    line: {
      ready: "代理引擎就绪 (v{version})",
      clientConnected:
        "客户端接入 {peer} → {target} (proto={proto}, session={session})",
      clientDisconnected:
        "客户端离开 {peer} sent={sent}B recv={recv}B duration={duration}s",
      passiveSeen: "待命客户端登记 {name} ({peer}, v{version})",
      passiveLost: "待命客户端离线 {name} ({peer}) — 心跳超时",
      vpnState: "VPN 状态变更 available={available} iface={iface}",
    },
  },

  network: {
    title: "本机网络",
    pass: "{passed} / {total} 通过",
    lanEgress: "LAN 出口",
    lanDetected: "已检测",
    lanUndetected: "未检测",
    vpnEgress: "VPN 出口",
    vpnViaVpn: "默认路由 → VPN",
    vpnNotViaVpn: "未走 VPN",
    portsListening: "端口监听",
    httpPort: "HTTP 代理端口",
    socks5Port: "SOCKS5 代理端口",
    apiPort: "管控 API 端口",
    pendingDetail: "尚未检测",
  },

  traffic: {
    title: "实时流量",
    window: "窗口 {sec}s · {n} 点",
    peak: "峰值 {value}",
    in: "下行",
    out: "上行",
    waitingTitle: "等待流量",
    waitingDesc: "客户端连入后将实时刷新上下行曲线",
  },

  clientList: {
    title: "在线客户端",
    summaryTotal: "共 {total} 个",
    summaryActive: "{n} 传输中",
    summaryPassive: "{n} 待命",
    th: {
      peer: "客户端",
      proto: "协议",
      target: "目标",
      down: "下行",
      up: "上行",
      total: "累计",
      since: "接入",
    },
    emptyTitleNoTraffic: "暂无客户端在传输流量",
    emptyTitleNoClient: "还没有客户端连进来",
    emptyDescNoTraffic: "下方「待命」客户端正等着发起请求",
    emptyDescNoClient: "把右侧 PAC URL 分享给同事即可接入",
    passiveSection: "待命客户端 · 已链接但暂无流量",
  },

  share: {
    title: "接入信息",
    subtitle: "分享给同事",
    notRunning: "代理未启动,启动后此处会显示同事可用的接入信息",
    noReachable:
      "代理监听在 0.0.0.0 但未检测到对外可达地址,同事将无法连接。请确认本机已加入有线/Wi-Fi 网络,或在启动时显式指定 {flag}",
    notLan:
      "未检测到物理网卡的私有 IP。当前接入信息可用,但同事必须能直接访问 {host}",
    pacBadge: "推荐",
    pacTitle: "PAC 自动配置",
    pacHint: "智能分流,国内不绕路",
    httpTitle: "HTTP 代理",
    httpHint: "全局走代理,国内会变慢",
    socksTitle: "SOCKS5",
    socksHint: "curl / git / SSH 命令行",
    placeholder: "启动后显示",
    toastCopied: "{label} 已复制",
    toastCopyFail: "复制失败",
    pacLabel: "PAC URL",
    httpLabel: "HTTP 代理",
    socksLabel: "SOCKS5 代理",
  },

  firstLaunch: {
    title: "首次启动确认",
    desc:
      "你即将启动一个把本机 VPN 共享给局域网的代理服务,请先确认使用场景",
    avoidTitle: "以下场景下不要启用:",
    risk1: "你的电脑由公司 IT 管理且有合规审计",
    risk2: "你在公司公共 WiFi、客户场地等不受控网络",
    risk3: "你不确定 LAN 上还有谁能访问到本机",
    recommend: "推荐场景:家庭 WiFi、私人办公室、自己掌控的 LAN",
    ack: "我已了解上述风险并自行承担",
    cancel: "取消",
    start: "启动代理",
  },

  toast: {
    error: "错误",
    warning: "警告",
    info: "提示",
    success: "成功",
    proxyConnectFail: "无法连接到代理服务",
  },

  format: {
    idleOnline: "在线",
    idleSecAgo: "{n}s 前",
    idleMinAgo: "{n} 分钟前",
    idleHourAgo: "{n} 小时前",
  },
};

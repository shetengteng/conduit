/**
 * English (US) — Conduit Server UI
 *
 * Mirrors zh-CN.ts key for key. Phrasing follows enterprise dashboard
 * conventions (Stripe / Vercel / Datadog) — short, sentence case,
 * present tense, no exclamation, no marketing fluff.
 */
export default {
  app: {
    name: "Conduit",
    role: "Server",
  },

  nav: {
    dashboard: "Dashboard",
    logs: "Logs",
    settings: "Settings",
    language: "Language",
  },

  status: {
    running: "Running",
    stopped: "Stopped",
    notStarted: "Not started",
    notReady: "Not ready",
    vpnError: "VPN issue",
    portsPending: "Ports pending",
    sseConnected: "SSE connected",
    sseDisconnected: "SSE offline",
  },

  topbar: {
    sub: {
      notStarted: "Click Settings in the sidebar to configure the proxy",
      stopped: "Proxy stopped — waiting to restart",
      vpnError: "VPN interface unavailable; some traffic may bypass the proxy",
      notReady: "Proxy is starting up; ports not yet ready",
      waiting: "Waiting for clients",
      passiveOnly: "{count} client connected (idle)",
      activeOnly: "{count} client streaming",
      mixed: "{total} clients · {active} active + {passive} idle",
    },
    uptime: "Up {value}",
    stopAndQuit: "Stop & quit",
    restart: "Restart proxy",
    restarting: "Restarting…",
    restartTitle: "Relaunch the sidecar to bring the proxy back online",
    confirmStopTitle: "Stop Conduit Server?",
    confirmStopBody:
      "The proxy engine (HTTP / SOCKS5 / mDNS) will go offline. Any client currently using this VPN will be disconnected.",
    confirmStopHint:
      "v0.1 cannot restart the proxy from this window — relaunch the Conduit Server app (or run {cmd} in a terminal) to start it again.",
    confirmStopOk: "Stop proxy",
    confirmStopOkLoading: "Stopping…",
    cancel: "Cancel",
    toastStopOk: "Quitting",
    toastStopOkDetail:
      "Proxy engine stopped — Conduit Server is cleaning up.",
    toastStopFail: "Failed to stop",
    toastRestartTip: "Restarting…",
    toastRestartTipDetail: "Main window will close and the sidecar will relaunch",
    toastRestartFail: "Restart failed",
    toastRestartFailHint: "Please quit and reopen Conduit Server manually.",
    toastRestartDevTitle: "Restart not supported in dev mode",
    toastRestartDevDetail:
      "Dev-mode restart would orphan the vite dev server. Please ctrl+c the terminal and rerun `pnpm tauri dev`. Release builds don't have this limitation.",
    tauriUnavailable: "Tauri invoke unavailable (likely browser preview).",
  },

  dashboard: {
    proxyEngine: "Proxy engine",
    kpi: {
      clients: "Active clients",
      clientsUnit: "",
      down: "Downstream",
      up: "Upstream",
      uptime: "Uptime",
    },
    clientsSub: {
      waiting: "Waiting for clients",
      activeOnly: "{count} streaming",
      passiveOnly: "{count} idle (no traffic)",
      mixed: "{active} active · {passive} idle",
    },
    trafficDirection: {
      idle: "Idle",
      down: "Peers → server",
      up: "Server → peers",
    },
    uptimeSub: {
      stable: "Stable",
      notStarted: "Not started",
    },
  },

  boot: {
    splashTitle: "Conduit Server",
    splashSub: "Starting the proxy engine…",
    phase: {
      port: "Allocate ports",
      engine: "Spawn engine",
      health: "Health check",
      ready: "Ready",
    },
    failedTitle: "Proxy engine failed to start",
    failedSub: "Tauri main process didn't pass the health check in time",
    failedHints: "What to try:",
    failedHint1: "Click Retry — sidecar cold-start can take longer than expected",
    failedHint2: "If it keeps failing, check ~/.conduit/logs/proxy.log",
    failedHint3:
      "If you suspect an environment issue, restart your Mac or reinstall Conduit Server",
    retry: "Retry",
    quit: "Quit",
  },

  language: {
    label: "Language / 语言",
    zh: "简体中文",
    en: "English",
  },

  settings: {
    title: "Settings",
    subtitle: "Ports, security, mDNS broadcast",
    readonly: "Read-only",
    readonlyAlertBody:
      "Read-only display. To customize, pass startup flags such as {mdns} or {port}.",

    general: {
      title: "General",
      desc: "Local UI preferences. Changes apply immediately.",
      languageLabel: "Interface language",
      languageHint:
        "Saved in localStorage and applied automatically next launch.",
    },

    ports: {
      title: "Ports",
      desc: "These ports are picked by the Tauri main process at startup via portpicker. They cannot be edited from the window.",
      http: "HTTP proxy",
      socks5: "SOCKS5 proxy",
      api: "Control API",
      httpHint:
        "Not editable in v0.1. Use --http-port when starting the server.",
      socks5Hint:
        "Not editable in v0.1. Use --socks5-port when starting the server.",
      apiHint:
        "Not editable in v0.1. Use --api-port when starting the server (loopback only).",
    },

    security: {
      title: "Security",
      desc: "Only allow proxy connections from the LAN segments / destination ports below.",
      allowedCidrs: "Allowed LAN segments (CIDR)",
      allowedPorts: "Allowed CONNECT ports",
    },

    mdns: {
      title: "mDNS broadcast",
      desc: "Lets Conduit Client on the same LAN auto-discover this server",
      enable: "Enable mDNS broadcast",
      name: "Broadcast name",
      nameHint: "Defaults to system hostname. Customize with {cmd}.",
      nameTitle: "Start with {cmd}.",
      type: "Service type",
    },

    about: {
      title: "About",
      checkUpdate: "Check for update",
      checking: "Checking…",
      upToDate: "You're on the latest version",
      upToDateDetail: "Local v{local} = remote {latest}",
      updateAvailable: "New version {latest} available",
      updateAvailableDetail: "Local v{local}. Click the toast to download",
      noRelease: "No release published yet",
      noReleaseDetail:
        "No release on GitHub Releases yet — please try again later",
      networkError: "Network error",
      networkErrorDetail:
        "Couldn't reach GitHub. Please check your proxy / VPN state",
      rateLimited: "GitHub rate-limited",
      rateLimitedDetail:
        "Anonymous IPs are limited to 60 req/hour — please retry later",
    },
  },

  logs: {
    title: "Logs",
    subtitle:
      "Live event stream from the proxy engine. Filter by keyword to debug connections.",
    eventStream: "Event stream",
    searchPlaceholder: "Search host / IP / keyword…",
    clear: "Clear",
    autoScroll: "Auto-scroll to latest",
    maxKept: "Keeps the last {max} entries",
    emptyMatch: "No matching log lines",
    emptyAll: "No events yet",
    emptyHint:
      "Logs are pushed only when:\n· The proxy engine restarts (ready)\n· A client connects via SOCKS5/HTTP (connected/disconnected)\n· A client sends a heartbeat (passive_client_seen)\n· macOS VPN routing table changes",
    panelMounted: "[ui] Log panel mounted, waiting for sidecar events",
    line: {
      ready: "Proxy engine ready (v{version})",
      clientConnected:
        "Client connected {peer} → {target} (proto={proto}, session={session})",
      clientDisconnected:
        "Client disconnected {peer} sent={sent}B recv={recv}B duration={duration}s",
      passiveSeen: "Passive client registered {name} ({peer}, v{version})",
      passiveLost: "Passive client lost {name} ({peer}) — heartbeat timeout",
      vpnState: "VPN state changed available={available} iface={iface}",
    },
  },

  network: {
    title: "Network",
    pass: "{passed} / {total} pass",
    lanEgress: "LAN egress",
    lanDetected: "Detected",
    lanUndetected: "Not detected",
    vpnEgress: "VPN egress",
    vpnViaVpn: "Default route → VPN",
    vpnNotViaVpn: "Not via VPN",
    portsListening: "Listening ports",
    httpPort: "HTTP proxy port",
    socks5Port: "SOCKS5 proxy port",
    apiPort: "Control API port",
    pendingDetail: "Pending check",
  },

  traffic: {
    title: "Live traffic",
    window: "Window {sec}s · {n} pts",
    peak: "Peak {value}",
    in: "Down",
    out: "Up",
    waitingTitle: "Waiting for traffic",
    waitingDesc:
      "The chart will update in real time once a client starts sending traffic",
  },

  clientList: {
    title: "Online clients",
    summaryTotal: "{total} total",
    summaryActive: "{n} active",
    summaryPassive: "{n} idle",
    th: {
      peer: "Client",
      proto: "Proto",
      target: "Target",
      down: "Down",
      up: "Up",
      total: "Total",
      since: "Since",
    },
    emptyTitleNoTraffic: "No client is moving traffic right now",
    emptyTitleNoClient: "No client connected yet",
    emptyDescNoTraffic: "Idle clients below are waiting to make a request",
    emptyDescNoClient: "Share the PAC URL on the right with your teammate",
    passiveSection: "Idle clients · connected but no traffic",
    recentSection: "Recent sessions · last {n}",
    recentDuration: "duration {d}",
  },

  share: {
    title: "Connection info",
    subtitle: "Share with teammates",
    notRunning:
      "Proxy not started; connection info will appear once it's running",
    noReachable:
      "Proxy is listening on 0.0.0.0 but no reachable address was found — teammates won't be able to connect. Make sure this machine is on Wi-Fi/Ethernet, or pass {flag} explicitly at startup",
    notLan:
      "No private IP detected on a physical interface. The info below works, but teammates must be able to reach {host} directly",
    pacBadge: "Recommended",
    pacTitle: "PAC auto-config",
    pacHint: "Smart routing, no detour for domestic sites",
    httpTitle: "HTTP proxy",
    httpHint: "Forces all traffic through the proxy",
    socksTitle: "SOCKS5",
    socksHint: "For curl / git / SSH command line",
    placeholder: "Available after start",
    toastCopied: "{label} copied",
    toastCopyFail: "Copy failed",
    pacLabel: "PAC URL",
    httpLabel: "HTTP proxy",
    socksLabel: "SOCKS5 proxy",
  },

  firstLaunch: {
    title: "First-launch confirmation",
    desc:
      "You're about to start a proxy that shares this machine's VPN with the LAN. Please confirm your use case first.",
    avoidTitle: "Do NOT enable in these scenarios:",
    risk1: "Your computer is IT-managed by the company with compliance audit",
    risk2: "You're on public Wi-Fi, a client site or other untrusted network",
    risk3: "You're not sure who else can reach this machine on the LAN",
    recommend:
      "Recommended: home Wi-Fi, private office, or any LAN under your control",
    ack: "I understand the risks and accept them",
    cancel: "Cancel",
    start: "Start proxy",
  },

  toast: {
    error: "Error",
    warning: "Warning",
    info: "Info",
    success: "Success",
    proxyConnectFail: "Unable to reach proxy service",
  },

  format: {
    idleOnline: "Live",
    idleSecAgo: "{n}s ago",
    idleMinAgo: "{n} min ago",
    idleHourAgo: "{n} h ago",
  },
};

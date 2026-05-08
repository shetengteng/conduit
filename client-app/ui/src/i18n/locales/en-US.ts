/**
 * English (US) — Conduit Client UI
 *
 * Mirrors zh-CN.ts key for key. Phrasing follows enterprise dashboard
 * conventions (Stripe / Vercel / Datadog) — short, sentence case.
 */
export default {
  app: {
    name: "Conduit",
    role: "Client",
  },

  nav: {
    discovery: "Discovery",
    connected: "Connected",
    diagnose: "Diagnose",
    settings: "Settings",
  },

  status: {
    ready: "Ready",
    notReady: "Not started",
    error: "Error",
    connFail: "Connect failed",
  },

  topbar: {
    sub: {
      apiError: "Local control API is unreachable",
      waitingHealthz: "Waiting for the local sidecar",
      selfCheckFail: "Self-check failed: {names}",
      scanning: "Scanning the LAN for Conduit servers…",
      foundServers: "{count} server(s) found — pick one in Discovery",
      selfCheckUnknown: "unknown",
    },
    portsAuto: "Ports auto-allocated by Tauri",
    uptime: "Up {value}",
    restart: "Restart",
    restarting: "Restarting…",
    restartTitle: "Restart Tauri main process; sidecar will be relaunched",
    tauriUnavailable: "Tauri invoke unavailable (browser preview?)",
    toastRestartTip: "Restarting…",
    toastRestartTipDetail:
      "Main window will close and the sidecar will relaunch",
    toastRestartFail: "Restart failed",
    toastRestartFailHint: "Please quit and reopen Conduit Client manually",
    toastRestartDevTitle: "Restart not supported in dev mode",
    toastRestartDevDetail:
      "Dev-mode restart would orphan the vite dev server. Please ctrl+c the terminal and rerun `pnpm tauri dev`. Release builds don't have this limitation.",
  },

  boot: {
    splashTitle: "Conduit Client",
    splashSub: "Starting the proxy engine…",
    phase: {
      port: "Allocate ports",
      engine: "Spawn engine",
      health: "Health check",
      ready: "Ready",
    },
    failedTitle: "Proxy engine failed to start",
    failedSub:
      "Tauri main process didn't pass the health check within 9 seconds",
    failedHints: "What to try:",
    hint1: "Check whether ports 8090 / 1080 / 8080 are taken",
    hint2:
      "Confirm a Conduit Server is running on the same LAN (mDNS / Bonjour reachable)",
    hint3:
      "Pick different API/HTTP/SOCKS5 ports under Settings → Ports",
    retry: "Retry",
    quit: "Quit",
  },

  language: {
    label: "Language / 语言",
    zh: "简体中文",
    en: "English",
  },

  proxyBanner: {
    title: "System proxy not switched · ",
    bodyWithPort:
      "Configure {code} in your browser/app manually, otherwise traffic won't go through Conduit.",
    bodyNoPort:
      "Configure SOCKS5 in your browser/app manually, otherwise traffic won't go through Conduit.",
    titleOverridden: "System proxy hijacked externally · ",
    bodyOverriddenWithPort:
      "Switch succeeded but was immediately overwritten by an enterprise proxy daemon (Zoom/Okta/MDM). Configure {code} in your browser/app to bypass.",
    bodyOverriddenNoPort:
      "Switch succeeded but was immediately overwritten by an enterprise proxy daemon (Zoom/Okta/MDM). Configure SOCKS5 in your browser/app to bypass.",
    copy: "Copy config",
    detail: "Details",
    dismissTitle: "Hide for this session",
    toastCopied: "Copied",
    toastCopyFail: "Copy failed",
    toastCopyFailHint: "Please select and copy manually",
  },

  discovery: {
    title: "Discover Conduit servers",
    sub: {
      scanning: "Scanning…",
      mdnsOff: "mDNS service disabled",
      empty: "No Conduit Server discovered yet",
      onlineCount: "{count} online",
      historyCount: "{count} previously seen",
    },
    historyHint:
      "“Previously seen” is persisted (up to {max} entries, sorted by recency). Use “Clear history” at the top right or the X button per entry to remove.",
    forgetAll: "Clear history",
    rescan: "Rescan",
    errorTitle: "Couldn't load server list",
    mdnsOffTitle: "Auto-discovery disabled",
    mdnsOffDesc:
      "Sidecar didn't load the zeroconf module, so mDNS broadcast/listen is off. Check your packaging or add a server manually (coming soon).",
    emptyTitle: "Searching the LAN for Conduit servers…",
    emptyDescLine1:
      "If this is the first launch, macOS will prompt for Local Network access — please allow it.",
    emptyDescLine2:
      "Servers on the same subnet usually appear within 5–10 seconds.",
    cardOnline: "online",
    cardSeen: "seen before",
    cardSocks: "SOCKS",
    cardApi: "Control API",
    cardVpn: "VPN",
    cardVpnOn: "on",
    cardVpnOff: "off",
    seenAtOnline: "broadcast at",
    seenAtOffline: "last seen",
    btnConnected: "Connected",
    btnConnecting: "Connecting…",
    btnConnect: "Connect",
    btnTitleHistory:
      "Historical server — wait for it to broadcast again",
    btnTitleAlreadyConnected:
      "Disconnect the current server in the Connected tab first",
    forgetTitle: "Remove from history",
    confirmForget:
      "Remove “{name}” from history?\nIt will reappear when the server broadcasts again.",
    confirmForgetAll:
      "Clear all {count} historical servers?\n(Online servers are not affected.)",
    confirmForgetTitle: "Remove this server from history?",
    confirmForgetAllTitle: "Clear all historical servers?",
    confirmOk: "Confirm",
    confirmCancel: "Cancel",
    toastRemoved: "Removed",
    toastNotFound: "Server not found",
    toastNotFoundDetail: "It may have been cleaned up already",
    toastRemoveFail: "Remove failed",
    toastClearedDetail: "{count} entries removed",
    toastClearedTitle: "History cleared",
    toastClearFail: "Clear failed",
    toastConnFail: "Connect failed",
    relTime: {
      never: "never",
      secAgo: "{n}s ago",
      minAgo: "{n} min ago",
      hourAgo: "{n} h ago",
      dayAgo: "{n} d ago",
    },
  },

  connecting: {
    title: "Connecting…",
    targetWith: "Target server: {name} ({host}:{port})",
    targetWithout:
      "Negotiating with the selected server — waiting for 5 steps to finish…",
    cancel: "Cancel",
    failedTitle: "Connect failed",
    panelTitle: "Connection progress",
    panelDesc:
      "5-step contract aligned with the backend SSE connect_progress event",
    stepWaiting: "Waiting…",
    stepRunning: "Running…",
    step: {
      probe: "Reachability probe",
      fetchPac: "Fetch PAC",
      prefillCache: "Resolve PAC and prefill cache",
      switchEndpoint: "Switch upstream server",
      startHeartbeat: "Start heartbeat & system proxy",
    },
  },

  connected: {
    titleConnectedTo: "Connected to {name}",
    subSysProxyOn:
      "System proxy is on — traffic flows through Conduit automatically",
    subSysProxyOff:
      "System proxy is off — configure SOCKS5 in your apps manually",
    btnDisconnect: "Disconnect",
    btnDisconnecting: "Disconnecting…",
    disconnectingHint: "Disconnecting, please wait…",
    heartbeat: {
      label: "Heartbeat · {state}",
      green: "healthy",
      yellow: "flapping",
      red: "lost",
      unknown: "unknown",
    },
    elapsed: "Connected for",
    socksRemote: "Remote SOCKS",
    apiRemote: "Remote control API",
    vpnRemote: "Remote VPN",
    vpnOn: "enabled",
    vpnOff: "disabled",
    elapsedSec: "{n}s",
    elapsedMin: "{m}m {s}s",
    elapsedHour: "{h}h {m}m",
    failTitle: "Lost contact with the server",
    failDesc:
      "Multiple consecutive heartbeats failed; traffic may be affected. Conduit will auto-recover when the server reappears via mDNS. If the red state persists, disconnect and reconnect manually.",
    trafficTitle: "Traffic chart",
    trafficDesc:
      "Local proxy up/down rate over the last 60 seconds (only counts established connections)",
    cacheTitle: "Route hits",
    cacheDesc:
      "Per-host direction decision and hit counts. direct = local; proxy = via server",
    notConnectedTitle: "Not connected",
    notConnectedSub:
      "Pick a Conduit Server in Discovery to connect to",
    lastErrorTitle: "Last connect attempt failed",
    emptyTitle: "No active connection",
    emptyDesc:
      "Click the button below to pick a server in Discovery",
    btnGoDiscovery: "Go to Discovery",
    toastConnected: "Connected to {name}",
    toastDisconnected: "Disconnected",
    toastDisconnectFail: "Disconnect failed",
    toastConnFail: "Connect failed",
    toastUnknownErr: "Unknown error",
  },

  traffic: {
    upRate: "Up rate",
    downRate: "Down rate",
    upTotal: "Up total",
    downTotal: "Down total",
    waiting:
      "Waiting for traffic — visit any website and the curve will update live",
    windowDesc: "60-second window · 1 sample per second",
    peak: "Peak {value}",
  },

  cache: {
    searchPlaceholder: "Search by host…",
    filterAll: "All",
    filterDirect: "Direct",
    filterProxy: "Via server",
    countLabel: "{filtered} / {total}",
    flush: "Flush",
    thHost: "Host",
    thDirection: "Direction",
    thSource: "Source",
    thHits: "Hits",
    thLastUsed: "Last used",
    emptyAll:
      "No routing decisions yet. They'll appear automatically once you browse.",
    emptyMatch: "No matching entries",
    direct: "Direct",
    proxy: "Via server",
    source: {
      pac: "PAC prefill",
      probe: "TCP probe",
      manual: "Manual",
      cache: "Cache hit",
      pattern: "Wildcard",
      private_ip: "Private IP",
      global_override: "Global override",
      self_heal: "Self-heal",
    },
    relTime: {
      none: "—",
      secAgo: "{n}s ago",
      minAgo: "{n} min ago",
      hourAgo: "{n} h ago",
      dayAgo: "{n} d ago",
    },
    toastFlushed: "Route cache flushed",
    toastFlushFail: "Flush failed",
  },

  diagnose: {
    title: "Diagnose",
    sub:
      "One-click self-check across 5 core stages. Failed items come with actionable hints.",
    copy: "Copy report",
    rerun: "Rerun",
    running: "Running…",
    statusReady: "Ready",
    statusOk: "All good",
    statusFail: "{n} item(s) need attention",
    lastRunNever: "Not run yet",
    lastRun: "Last run at {time}",
    fetchFailTitle: "Couldn't fetch diagnose result",
    fetchFailHint:
      "Usually means the sidecar isn't running. Try restarting the app.",
    listTitle: "Checks",
    listSub:
      "Failed items are highlighted in red. Follow the hints, then click Rerun.",
    reportHeader: "Conduit Client diagnose report · {time}",
    reportOverall: "Overall: {status}",
    reportRemediation: "  Hint:",
    toastRunFail: "Diagnose failed",
    toastCopied: "Diagnose report copied",
    toastCopyFail: "Copy failed",
  },

  settings: {
    title: "Settings",
    sub: "Runtime info, manual add server, cache maintenance",
    runtime: {
      title: "Runtime",
      desc:
        "These ports are picked by the Tauri main process at startup via portpicker",
      socksPort: "SOCKS5 port",
      apiPort: "Control API port",
    },
    autostart: {
      title: "Launch at login",
      desc:
        "Writes a plist into ~/Library/LaunchAgents/ via launchctl so Conduit Client starts after login",
      currentLabel: "Current:",
      enabled: "enabled",
      disabled: "disabled",
      toastEnabled: "Launch at login enabled",
      toastDisabled: "Launch at login disabled",
      toastFail: "Launch-at-login toggle failed",
    },
    general: {
      title: "General",
      desc: "Local UI preferences. Changes apply immediately.",
      languageLabel: "Interface language",
      languageHint:
        "Saved in localStorage and applied automatically next launch.",
    },
    manual: {
      title: "Manually connect to a server",
      desc:
        "When mDNS is unavailable (cross-subnet, sandbox, multicast-blocking corp WLAN), specify a server here. Note: only servers previously seen on the Discovery page can be reconnected; the full manual-register endpoint ships in M-δ.",
      name: "Name",
      namePlaceholder: "Teammate's Mac",
      host: "Host",
      hostPlaceholder: "192.168.1.14",
      httpPort: "HTTP / PAC port",
      socksPort: "SOCKS5 port",
      apiPort: "Control API port",
      btn: "Try to connect",
      btnBusy: "Connecting…",
      tip:
        "Tip: server_id is {code}. If you've never seen this server in Discovery, the connect call returns NOT_FOUND.",
      toastNeedHostPort: "Please enter host and port",
      toastTried: "Tried connecting to {id}",
      toastFail: "Manual connect failed",
    },
    cache: {
      title: "Cache maintenance",
      desc:
        "Flush the current route cache (host → direct/proxy decisions). Useful after server-side PAC rule updates.",
      currentEntries: "Current entries:",
      hitMiss: "Hits / Misses:",
      btn: "Flush route cache",
    },
    diag: {
      title: "Self-check details",
      desc:
        "The full 5-step self-check (sidecar / mDNS / server reach / PAC / system proxy) lives on its own page",
      hint:
        "Failed items come with actionable hints, and the full report can be copied with one click",
      btn: "Open Diagnose",
    },
    about: {
      title: "About",
      version: "Conduit Client v{version} · macOS only",
      tagline:
        "Smart local proxy: auto direct vs server VPN per destination",
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

  toast: {
    error: "Error",
    warning: "Warning",
    info: "Info",
    success: "Success",
  },

  format: {
    idleOnline: "Live",
    idleSecAgo: "{n}s ago",
    idleMinAgo: "{n} min ago",
    idleHourAgo: "{n} h ago",
  },
};

/**
 * 检查 GitHub Releases 上是否有比本地更新的版本。
 *
 * 设计:
 *   - 查询 https://api.github.com/repos/shetengteng/conduit/releases/latest
 *   - 把 release.tag_name (形如 v0.1.0) 与本地 status.version (形如 0.1.0) 做 semver 比对
 *   - 不需要登录:GitHub 公开 repo 的 releases endpoint 走匿名 60req/h 配额,
 *     检查更新这种低频操作完全够用
 *   - 网络错误 / 解析失败 / 限流 都视为"暂时不可用",由调用方 toast warn 即可
 *   - 返回结构化的 UpdateCheckResult,UI 决定文案,避免在这里硬编码 i18n
 *
 * 不在这里做:
 *   - 不做 toast(交给调用方,因为 toast 文案是 i18n)
 *   - 不做"打开浏览器跳转"(交给调用方)
 *   - 不做后台轮询 / 定时检查(用户主动点 button 才查)
 */
import { invoke } from "@tauri-apps/api/core";

const GITHUB_OWNER = "shetengteng";
const GITHUB_REPO = "conduit";
const RELEASES_URL = `https://api.github.com/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases/latest`;
const RELEASES_PAGE = `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}/releases/latest`;
const REQUEST_TIMEOUT_MS = 8000;

export type UpdateCheckOutcome =
  | "up-to-date"
  | "update-available"
  | "network-error"
  | "rate-limited"
  | "no-release";

export interface UpdateCheckResult {
  outcome: UpdateCheckOutcome;
  /** 本地版本(传入即原样返回) */
  local: string;
  /** 远端最新版本,只在 outcome ∈ {up-to-date, update-available} 时存在 */
  latest?: string;
  /** 远端 release 详情页 URL — UI 上"前往下载"按钮用 */
  releaseUrl: string;
  /** 错误细节,用于 toast detail */
  detail?: string;
}

/**
 * "1.2.3" / "v1.2.3" / "1.2.3-rc1" → [1, 2, 3] (忽略 pre-release 后缀,生产 v0.x 用不到)
 */
function parseSemver(raw: string): [number, number, number] | null {
  const m = raw.trim().replace(/^v/i, "").match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!m) return null;
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

/** 1 if a > b, -1 if a < b, 0 if equal */
function cmpSemver(a: [number, number, number], b: [number, number, number]): number {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) return a[i] > b[i] ? 1 : -1;
  }
  return 0;
}

interface GithubRelease {
  tag_name: string;
  html_url: string;
  draft: boolean;
  prerelease: boolean;
}

/**
 * 同步执行一次更新检查。返回 Promise<UpdateCheckResult>,不会 throw。
 *
 * @param localVersion - 当前本地版本号(形如 "0.1.0",可带 v 前缀)
 */
export async function checkForUpdate(
  localVersion: string,
): Promise<UpdateCheckResult> {
  const ctrl = new AbortController();
  const timer = window.setTimeout(() => ctrl.abort(), REQUEST_TIMEOUT_MS);
  try {
    const resp = await fetch(RELEASES_URL, {
      signal: ctrl.signal,
      headers: { Accept: "application/vnd.github+json" },
    });
    if (resp.status === 403) {
      // GitHub anonymous rate limit hit (60/h/IP)
      return {
        outcome: "rate-limited",
        local: localVersion,
        releaseUrl: RELEASES_PAGE,
        detail: `HTTP ${resp.status} ${resp.statusText}`,
      };
    }
    if (resp.status === 404) {
      // 仓库还没发布过任何 release
      return {
        outcome: "no-release",
        local: localVersion,
        releaseUrl: RELEASES_PAGE,
      };
    }
    if (!resp.ok) {
      return {
        outcome: "network-error",
        local: localVersion,
        releaseUrl: RELEASES_PAGE,
        detail: `HTTP ${resp.status} ${resp.statusText}`,
      };
    }
    const json = (await resp.json()) as GithubRelease;
    if (!json?.tag_name || json.draft) {
      return {
        outcome: "no-release",
        local: localVersion,
        releaseUrl: json?.html_url ?? RELEASES_PAGE,
      };
    }
    const remote = parseSemver(json.tag_name);
    const local = parseSemver(localVersion);
    if (!remote || !local) {
      // 解析失败也当作 up-to-date 兜底,但用 latest 透出原始 tag 让用户自己判断
      return {
        outcome: "up-to-date",
        local: localVersion,
        latest: json.tag_name,
        releaseUrl: json.html_url ?? RELEASES_PAGE,
      };
    }
    return {
      outcome: cmpSemver(remote, local) > 0 ? "update-available" : "up-to-date",
      local: localVersion,
      latest: json.tag_name,
      releaseUrl: json.html_url ?? RELEASES_PAGE,
    };
  } catch (e) {
    return {
      outcome: "network-error",
      local: localVersion,
      releaseUrl: RELEASES_PAGE,
      detail: e instanceof Error ? e.message : String(e),
    };
  } finally {
    window.clearTimeout(timer);
  }
}

/**
 * 调 Tauri Rust 端 `open_external` 命令打开系统浏览器,降级用 window.open。
 * 失败兜底返回 false,不抛错。
 */
export async function openExternal(url: string): Promise<boolean> {
  try {
    // 优先走 Tauri (v2 invoke + commands.rs::open_external) —— 系统默认浏览器,
    // 不会被 webview 拦截。失败说明在 dev 浏览器预览,降级 window.open。
    await invoke("open_external", { url });
    return true;
  } catch {
    try {
      window.open(url, "_blank", "noopener,noreferrer");
      return true;
    } catch {
      return false;
    }
  }
}

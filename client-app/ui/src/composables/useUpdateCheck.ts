/**
 * 检查 GitHub Releases 上是否有比本地更新的版本。
 *
 * 与 server-app/ui 的同名 composable 保持一致 — server / client 来自同一仓库,
 * release tag 也共享,所以同一份逻辑能复用。如果未来 client 单独发版本,只需要改
 * GITHUB_REPO 和路径 fragment 即可。
 *
 * 行为:
 *   - 查询 https://api.github.com/repos/shetengteng/conduit/releases/latest
 *   - 把 release.tag_name (形如 v0.1.0) 与 caller 提供的 localVersion 做 semver 比对
 *   - 不需要登录:GitHub 公开 repo 走匿名 60req/h 配额
 *   - 网络错误 / 解析失败 / 限流 → 由调用方 toast warn
 *   - 不在这里做 toast (文案是 i18n)
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
  local: string;
  latest?: string;
  releaseUrl: string;
  detail?: string;
}

function parseSemver(raw: string): [number, number, number] | null {
  const m = raw.trim().replace(/^v/i, "").match(/^(\d+)\.(\d+)\.(\d+)/);
  if (!m) return null;
  return [Number(m[1]), Number(m[2]), Number(m[3])];
}

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
      return {
        outcome: "rate-limited",
        local: localVersion,
        releaseUrl: RELEASES_PAGE,
        detail: `HTTP ${resp.status} ${resp.statusText}`,
      };
    }
    if (resp.status === 404) {
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

export async function openExternal(url: string): Promise<boolean> {
  try {
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

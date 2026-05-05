/**
 * 单一版本来源:由 vite.config.ts 在 build/dev 阶段从 server-app/ui/package.json 注入。
 * 改版本时只需要 `pnpm scripts/bump-version.sh 0.x.y`,这里自动跟。
 *
 * 仅用于:Sidebar 角标、SettingsView 的"包定义版本号"展示等。
 * "运行时实际跑的版本"应该走 healthz 的 status?.version (来自 Python core),
 * 二者出现 mismatch 通常说明 sidecar 没重新打包。
 */
export const APP_VERSION = __APP_VERSION__;

/** 带 v 前缀的展示版本号,例如 "v0.1.1"。 */
export const APP_VERSION_LABEL = `v${APP_VERSION}`;

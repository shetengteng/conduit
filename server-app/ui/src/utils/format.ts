/**
 * Display formatters shared by every business / view component.
 *
 * Keep this file *pure*: no Vue, no I/O, no dates from outside (tests
 * pin time via the `now` argument).  This makes the helpers trivially
 * importable from `<script setup>`, `composables/`, or unit tests.
 */

const KIB = 1024;
const MIB = 1024 * 1024;
const GIB = 1024 * 1024 * 1024;

/** Throughput, full label form. e.g. `"1.23 MB/s"`. */
export function formatBps(bps: number): string {
  if (!Number.isFinite(bps) || bps < 0) return "—";
  if (bps < KIB) return `${bps} B/s`;
  if (bps < MIB) return `${(bps / KIB).toFixed(1)} KB/s`;
  return `${(bps / MIB).toFixed(2)} MB/s`;
}

/** Throughput, number only — pair with `formatBpsUnit` to render the unit separately. */
export function formatBpsValue(bps: number): string {
  if (!Number.isFinite(bps) || bps < 0) return "—";
  if (bps < KIB) return `${bps}`;
  if (bps < MIB) return `${(bps / KIB).toFixed(1)}`;
  return `${(bps / MIB).toFixed(2)}`;
}

export function formatBpsUnit(bps: number): string {
  if (!Number.isFinite(bps) || bps < 0) return "—";
  if (bps < KIB) return "B/s";
  if (bps < MIB) return "KB/s";
  return "MB/s";
}

/** Cumulative bytes, full label. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < KIB) return `${bytes} B`;
  if (bytes < MIB) return `${(bytes / KIB).toFixed(1)} KB`;
  if (bytes < GIB) return `${(bytes / MIB).toFixed(2)} MB`;
  return `${(bytes / GIB).toFixed(2)} GB`;
}

/** Compact uptime: `"3h 12m 45s"` / `"45m 12s"` / `"45s"`. */
export function formatUptime(seconds: number): string {
  if (!seconds || seconds < 0 || !Number.isFinite(seconds)) return "—";
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h) return `${h}h ${m}m ${s}s`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

/** Two-segment uptime for compact KPI tiles: `"3h 12m"` / `"12m 45s"` / `"45s"`. */
export function formatUptimeShort(seconds: number): string {
  if (!seconds || seconds < 0 || !Number.isFinite(seconds)) return "—";
  const total = Math.floor(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

/** Localised wall-clock for log rows / tooltips. */
export function formatTime(ts: number): string {
  if (!Number.isFinite(ts) || ts <= 0) return "—";
  return new Date(ts).toLocaleTimeString();
}

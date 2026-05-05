import { computed, type ComputedRef, type Ref } from "vue";

import type { TrafficSamplePoint } from "../types/proxy";

import { trafficStore } from "../stores/traffic";

export type TrafficDirection = "in" | "out";

export interface SeriesPath {
  peer: string;
  color: string;
  /** SVG `d` attribute for the line path. */
  line: string;
  /** SVG `d` attribute for the closed area under the line. */
  area: string;
  /** Peak Bps observed on this series in the active window. */
  peakBps: number;
  /** Pixel-space points (after layout), index-aligned to `raw`. */
  pointsRel: ReadonlyArray<readonly [number, number]>;
  /** Raw samples (timestamp, sentBps, recvBps). */
  raw: TrafficSamplePoint[];
}

export interface UseTrafficSeriesOptions {
  width: number;
  height: number;
  padX: number;
  padY: number;
  direction: Ref<TrafficDirection>;
  /** Optional palette override. Falls back to the standard 6-stop ramp. */
  palette?: ReadonlyArray<string>;
}

// Palette ordered so the first 1-2 peers render in green/blue — matching the
// client-side traffic chart (uplink emerald + downlink blue). chart-1 is the
// near-black "Stripe-style" tone reserved as a last-resort fallback so a single
// connected client doesn't show up as a black line.
const DEFAULT_PALETTE: ReadonlyArray<string> = [
  "var(--chart-2)",
  "var(--chart-5)",
  "var(--chart-3)",
  "var(--chart-4)",
  "var(--status-info)",
  "var(--chart-1)",
];

/**
 * Pure layout pipeline for the traffic chart.
 *
 * Reads from `trafficStore.state.series`, projects each peer's samples into
 * a normalised `[0..maxBps]` range, then maps to pixel coordinates inside
 * `(width, height)` — leaving room for `padX` / `padY`.  All math is
 * deterministic from inputs, so the consumer (TrafficChart.vue) stays a
 * thin renderer.
 */
export function useTrafficSeries(opts: UseTrafficSeriesOptions): {
  seriesPaths: ComputedRef<SeriesPath[]>;
  sampleCount: ComputedRef<number>;
  peakBps: ComputedRef<number>;
  peerColor: (idx: number) => string;
} {
  const palette = opts.palette ?? DEFAULT_PALETTE;
  const peerColor = (idx: number): string => palette[idx % palette.length];

  const seriesPaths = computed<SeriesPath[]>(() => {
    const peers = Object.keys(trafficStore.state.series);
    if (peers.length === 0) return [];

    let maxBps = 1;
    for (const arr of Object.values(trafficStore.state.series)) {
      for (const [, sent, recv] of arr) {
        const v = opts.direction.value === "in" ? recv : sent;
        if (v > maxBps) maxBps = v;
      }
    }

    return peers.map((peer, i): SeriesPath => {
      const arr = trafficStore.state.series[peer] ?? [];
      if (arr.length === 0) {
        return {
          peer,
          color: peerColor(i),
          line: "",
          area: "",
          peakBps: 0,
          pointsRel: [],
          raw: [],
        };
      }
      const xStep = (opts.width - opts.padX * 2) / Math.max(1, arr.length - 1);
      const points = arr.map((p: TrafficSamplePoint, j: number): readonly [number, number] => {
        const v = opts.direction.value === "in" ? p[2] : p[1];
        const x = opts.padX + j * xStep;
        const y = opts.height - opts.padY - (v / maxBps) * (opts.height - opts.padY * 2);
        return [x, y];
      });
      const line = `M ${points.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" L ")}`;
      const last = points[points.length - 1]?.[0] ?? opts.padX;
      const first = points[0]?.[0] ?? opts.padX;
      const baseY = opts.height - opts.padY;
      const area = `${line} L ${last.toFixed(1)},${baseY} L ${first.toFixed(1)},${baseY} Z`;
      const peak = arr.reduce(
        (m, p) => Math.max(m, opts.direction.value === "in" ? p[2] : p[1]),
        0,
      );
      return { peer, color: peerColor(i), line, area, peakBps: peak, pointsRel: points, raw: arr };
    });
  });

  const sampleCount = computed<number>(() => {
    let n = 0;
    for (const arr of Object.values(trafficStore.state.series)) {
      if (arr.length > n) n = arr.length;
    }
    return n;
  });

  const peakBps = computed<number>(() => {
    let max = 0;
    for (const p of seriesPaths.value) if (p.peakBps > max) max = p.peakBps;
    return max;
  });

  return { seriesPaths, sampleCount, peakBps, peerColor };
}

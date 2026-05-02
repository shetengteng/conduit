import { computed, ref, type ComputedRef, type Ref } from "vue";

/**
 * Generic table-sort composable.
 *
 * Caller provides:
 *   - the keys it knows how to sort by
 *   - which keys are "string-y" (defaults to ascending) vs "numeric-y"
 *     (defaults to descending) on first activation
 *   - a `valueOf(row, key)` accessor returning a comparable primitive
 *
 * Returns `{ key, dir, sorted, set }` where `set(k)` toggles direction
 * if the key is already active, or activates the key with the right default.
 *
 * Numeric values use natural `< >` ordering; everything else falls back to
 * `localeCompare` to keep ASCII / CJK lists deterministic.
 */
export interface UseTableSortOptions<TRow, TKey extends string> {
  defaultKey: TKey;
  defaultDir?: "asc" | "desc";
  ascendingKeys?: ReadonlyArray<TKey>;
  valueOf: (row: TRow, key: TKey) => string | number;
}

export interface UseTableSortReturn<TRow, TKey extends string> {
  key: Ref<TKey>;
  dir: Ref<"asc" | "desc">;
  sorted: ComputedRef<TRow[]>;
  set: (k: TKey) => void;
}

export function useTableSort<TRow, TKey extends string>(
  source: Ref<readonly TRow[]> | (() => readonly TRow[]),
  opts: UseTableSortOptions<TRow, TKey>,
): UseTableSortReturn<TRow, TKey> {
  const key = ref(opts.defaultKey) as Ref<TKey>;
  const dir = ref(opts.defaultDir ?? "desc") as Ref<"asc" | "desc">;

  const ascending = new Set<TKey>(opts.ascendingKeys ?? []);
  const getRows = typeof source === "function" ? source : () => source.value;

  const sorted = computed<TRow[]>(() => {
    const rows = [...getRows()];
    const sign = dir.value === "asc" ? 1 : -1;
    const k = key.value;
    rows.sort((a, b) => {
      const va = opts.valueOf(a, k);
      const vb = opts.valueOf(b, k);
      if (typeof va === "number" && typeof vb === "number") return (va - vb) * sign;
      return String(va).localeCompare(String(vb)) * sign;
    });
    return rows;
  });

  function set(k: TKey): void {
    if (key.value === k) {
      dir.value = dir.value === "asc" ? "desc" : "asc";
    } else {
      key.value = k;
      dir.value = ascending.has(k) ? "asc" : "desc";
    }
  }

  return { key, dir, sorted, set };
}

/**
 * Keyed loading with stale-while-revalidate: the last good value stays visible while a new key
 * (filters, page, live tick) loads, and responses that arrive out of order are ignored.
 */
import { batch, createEffect, createSignal, on, untrack } from "solid-js";

export interface Query<T> {
  data: () => T | undefined;
  error: () => unknown;
  loading: () => boolean;
  refetch: () => void;
  mutate: (fn: (value: T | undefined) => T | undefined) => void;
}

export function useQuery<K, T>(key: () => K | null | undefined | false, fetcher: (key: K) => Promise<T>): Query<T> {
  const [data, setData] = createSignal<T | undefined>(undefined);
  const [error, setError] = createSignal<unknown>(undefined);
  const [loading, setLoading] = createSignal(false);
  let seq = 0;
  const run = (k: K) => {
    const id = ++seq;
    setLoading(true);
    fetcher(k).then(
      (value) => {
        if (id !== seq) return;
        batch(() => {
          setData(() => value);
          setError(undefined);
          setLoading(false);
        });
      },
      (err) => {
        if (id !== seq) return;
        batch(() => {
          setError(() => err ?? new Error("request failed"));
          setLoading(false);
        });
      },
    );
  };
  createEffect(
    on(key, (k) => {
      if (k !== null && k !== undefined && k !== false) run(k as K);
    }),
  );
  return {
    data,
    error,
    loading,
    refetch: () => {
      const k = untrack(key);
      if (k !== null && k !== undefined && k !== false) run(k as K);
    },
    mutate: (fn) => setData((value) => fn(value)),
  };
}

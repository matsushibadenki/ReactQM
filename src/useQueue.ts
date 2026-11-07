import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type QueueItem = { sequence: number; id: string; name: string; formula: string; status: string };
export type QueuePage = { paused: boolean; waiting: number; active: QueueItem | null; items: QueueItem[]; nextCursor: number | null };
export function useQueue() {
  const [queue, setQueue] = useState<QueuePage>({ paused: true, waiting: 0, active: null, items: [], nextCursor: null });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generation = useRef(0);
  const mounted = useRef(false);
  const refresh = useCallback(async (after: number | null = null) => {
    if (!isTauri()) return;
    const request = ++generation.current;
    setLoading(true);
    try {
      const next = await invoke<QueuePage>('queue_snapshot', { after });
      if (!mounted.current || generation.current !== request) return;
      setQueue(previous => ({ ...next, items: after === null ? next.items : [...previous.items, ...next.items] }));
      setError(null);
    } catch (cause) { if (mounted.current && generation.current === request) setError(String(cause)); }
    finally { if (mounted.current && generation.current === request) setLoading(false); }
  }, []);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    const stops: (() => void)[] = [];
    if (isTauri()) {
      void listen('job-history-changed', () => { if (!disposed) void refresh(); }).then(unlisten => {
        if (disposed) unlisten(); else { stops.push(unlisten); void refresh(); }
      }).catch(cause => { if (!disposed) setError(String(cause)); });
      void listen<string>('queue-error', event => { if (!disposed) setError(event.payload); }).then(unlisten => {
        if (disposed) unlisten(); else stops.push(unlisten);
      }).catch(cause => { if (!disposed) setError(String(cause)); });
    }
    return () => { disposed = true; mounted.current = false; generation.current++; stops.forEach(stop => stop()); };
  }, [refresh]);
  return { queue, loading, error, refresh };
}
export type QueueProjection = ReturnType<typeof useQueue>;

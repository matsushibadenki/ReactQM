import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type PanelId = 'result' | 'history';
export type DockId = 'right' | 'bottom';
export type Zone = DockId | 'floating';
export type Layout = { schemaVersion: number; revision: number; panels: { id: PanelId; zone: Zone; home: DockId }[]; rightActive: PanelId; bottomActive: PanelId; rightMode: 'tabs' | 'split'; bottomMode: 'tabs' | 'split'; rightWidth: number };
export type LayoutAction = { kind: 'move'; panel: PanelId; zone: Zone; before?: PanelId } | { kind: 'activate'; panel: PanelId } | { kind: 'mode'; dock: DockId; mode: 'tabs' | 'split' } | { kind: 'resize'; width: number } | { kind: 'reset' };
type View = { layout: Layout; error: string | null };
const initial: Layout = { schemaVersion: 1, revision: 0, panels: [{ id: 'result', zone: 'right', home: 'right' }, { id: 'history', zone: 'right', home: 'right' }], rightActive: 'result', bottomActive: 'history', rightMode: 'split', bottomMode: 'tabs', rightWidth: 42 };
export function useLayout() {
  const [view, setView] = useState<View>({ layout: initial, error: null });
  const [pending, setPending] = useState(false);
  const current = useRef(initial);
  const mounted = useRef(false);
  const accept = useCallback((next: View) => {
    if (mounted.current && next.layout.revision >= current.current.revision) { current.current = next.layout; setView(next); }
  }, []);
  const refresh = useCallback(async () => { accept(await invoke<View>('layout_snapshot')); }, [accept]);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    const stops: (() => void)[] = [];
    const fail = (cause: unknown) => { if (!disposed) setView(previous => ({ ...previous, error: String(cause) })); };
    if (isTauri()) {
      void listen<View>('layout-changed', event => { if (!disposed) accept(event.payload); }).then(unlisten => {
        if (disposed) unlisten(); else { stops.push(unlisten); void refresh().catch(fail); }
      }).catch(fail);
      void listen<string>('panel-error', event => fail(event.payload)).then(unlisten => { if (disposed) unlisten(); else stops.push(unlisten); }).catch(fail);
    }
    return () => { disposed = true; mounted.current = false; stops.forEach(stop => stop()); };
  }, [accept, refresh]);
  const update = useCallback(async (action: LayoutAction) => {
    if (!isTauri()) return;
    setPending(true);
    try { accept(await invoke<View>('update_layout', { expectedRevision: current.current.revision, action })); }
    catch (cause) { if (mounted.current) { try { await refresh(); } catch { /* Keep the command error visible if refresh also fails. */ } setView(previous => ({ ...previous, error: String(cause) })); } }
    finally { if (mounted.current) setPending(false); }
  }, [accept, refresh]);
  return { ...view, pending, update };
}
export type LayoutProjection = ReturnType<typeof useLayout>;

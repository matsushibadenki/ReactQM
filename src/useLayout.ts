import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type PanelId = 'result' | 'history';
export type DockId = 'right' | 'bottom';
export type Zone = DockId | 'floating';
export type Layout = { schemaVersion: number; revision: number; panels: { id: PanelId; zone: Zone; home: DockId }[]; rightActive: PanelId; bottomActive: PanelId; rightMode: 'tabs' | 'split'; bottomMode: 'tabs' | 'split'; rightWidth: number };
export type LayoutAction = { kind: 'move'; panel: PanelId; zone: Zone; before?: PanelId } | { kind: 'activate'; panel: PanelId } | { kind: 'mode'; dock: DockId; mode: 'tabs' | 'split' } | { kind: 'resize'; width: number } | { kind: 'reset' };
export type PanelFocus = { panel: PanelId; revision: number };
type View = { layout: Layout; error: string | null };
const initial: Layout = { schemaVersion: 1, revision: 0, panels: [{ id: 'result', zone: 'right', home: 'right' }, { id: 'history', zone: 'right', home: 'right' }], rightActive: 'result', bottomActive: 'history', rightMode: 'split', bottomMode: 'tabs', rightWidth: 42 };
export function useLayout() {
  const [view, setView] = useState<View>({ layout: initial, error: null });
  const [pending, setPending] = useState(false);
  const [focusRequest, setFocusRequest] = useState<PanelFocus | null>(null);
  const inFlight = useRef(false);
  const current = useRef(initial);
  const mounted = useRef(false);
  const accept = useCallback((next: View) => {
    if (mounted.current && next.layout.revision >= current.current.revision) { current.current = next.layout; setView(next); }
  }, []);
  const requestFocus = useCallback((request: PanelFocus) => {
    if (mounted.current && request.revision >= current.current.revision) {
      setFocusRequest(previous => !previous || request.revision >= previous.revision ? request : previous);
    }
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
      void listen<PanelFocus>('panel-focus', event => { if (!disposed) requestFocus(event.payload); }).then(unlisten => { if (disposed) unlisten(); else stops.push(unlisten); }).catch(fail);
      void listen<string>('panel-error', event => fail(event.payload)).then(unlisten => { if (disposed) unlisten(); else stops.push(unlisten); }).catch(fail);
    }
    return () => { disposed = true; mounted.current = false; stops.forEach(stop => stop()); };
  }, [accept, refresh, requestFocus]);
  const update = useCallback(async (action: LayoutAction) => {
    if (!isTauri() || inFlight.current) return;
    inFlight.current = true;
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setPending(true);
    try {
      const next = await invoke<View>('update_layout', { expectedRevision: current.current.revision, action });
      accept(next);
      if (mounted.current && (action.kind === 'move' || action.kind === 'activate')) {
        requestFocus({ panel: action.panel, revision: next.layout.revision });
      }
    }
    catch (cause) { if (mounted.current) { try { await refresh(); } catch { /* Keep the command error visible if refresh also fails. */ } setView(previous => ({ ...previous, error: String(cause) })); } }
    finally {
      inFlight.current = false;
      if (mounted.current) {
        setPending(false);
        requestAnimationFrame(() => {
          if (mounted.current && document.activeElement === document.body && previousFocus?.isConnected) previousFocus.focus();
        });
      }
    }
  }, [accept, refresh, requestFocus]);
  return { ...view, pending, update, focusRequest };
}
export type LayoutProjection = ReturnType<typeof useLayout>;

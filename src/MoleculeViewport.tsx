import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { messages, type Locale } from './i18n';

type Action = { kind: 'orbit'; dx: number; dy: number } | { kind: 'zoom'; delta: number } | { kind: 'pick'; x: number; y: number } | { kind: 'fit' | 'bonds' | 'select_next' };
type View = { ready: boolean; documentRevision: number; selected: { index: number; symbol: string; position: number[] } | null; showBonds: boolean; bondCount: number; pixelSize: number[]; frameCount: number };
const icons = { fit: 'M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5M8 12h8m-4-4v8', plus: 'M12 5v14M5 12h14', minus: 'M5 12h14', bonds: 'm7 7 10 10M9 5l10 10M7 4a3 3 0 1 0 0 6 3 3 0 0 0 0-6m10 10a3 3 0 1 0 0 6 3 3 0 0 0 0-6', atom: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18m0 6v6m-3-3h6' };
function Glyph({ name }: { name: keyof typeof icons }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={icons[name]} /></svg>; }

export function MoleculeViewport({ locale, documentRevision, atomCount }: { locale: Locale; documentRevision: number; atomCount: number }) {
  const t = messages[locale];
  const element = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View | null>(null);
  const [error, setError] = useState<string | null>(null);
  const schedule = useRef<(action?: Action) => void>(() => {});
  const current = useRef({ documentRevision, atomCount });
  const drag = useRef<{ id: number; x: number; y: number; startX: number; startY: number; moved: boolean } | null>(null);

  useEffect(() => {
    current.current = { documentRevision, atomCount };
    schedule.current();
  }, [documentRevision, atomCount]);
  useEffect(() => {
    const node = element.current;
    if (!isTauri() || !node) return;
    const client = crypto.randomUUID();
    const theme = matchMedia('(prefers-color-scheme: dark)');
    let disposed = false, inFlight = false, dirty = true, attached = false, frame = 0;
    let actions: Action[] = [];
    async function flush() {
      frame = 0;
      if (disposed || inFlight || !dirty) return;
      dirty = false; inFlight = true;
      const rect = node!.getBoundingClientRect();
      const batch = actions; actions = [];
      try {
        const next = await invoke<View>('update_viewport', { request: {
          client, attach: !attached, document_revision: current.current.documentRevision,
          bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height, window_width: innerWidth, window_height: innerHeight, visible: !document.hidden && current.current.atomCount > 0, dark: theme.matches },
          actions: batch,
        } });
        attached = true;
        if (!disposed) { setView(next); setError(null); }
      } catch (cause) { if (!disposed) setError(String(cause)); }
      finally {
        inFlight = false;
        if (disposed) { void invoke('detach_viewport', { client }).catch(() => {}); }
        else if (dirty) queue();
      }
    }
    function queue(action?: Action) {
      if (disposed) return;
      if (action) {
        const last = actions.at(-1);
        if (action.kind === 'orbit' && last?.kind === 'orbit') { last.dx += action.dx; last.dy += action.dy; }
        else if (action.kind === 'zoom' && last?.kind === 'zoom') last.delta += action.delta;
        else if (actions.length < 128) actions.push(action);
      }
      dirty = true;
      if (!frame && !inFlight) frame = requestAnimationFrame(() => void flush());
    }
    schedule.current = queue;
    const changed = () => queue();
    const observer = new ResizeObserver(changed);
    observer.observe(node); observer.observe(document.body);
    window.addEventListener('resize', changed);
    window.addEventListener('scroll', changed, true);
    document.addEventListener('visibilitychange', changed);
    theme.addEventListener('change', changed);
    const nativeWindow = getCurrentWindow();
    const nativeListeners = [nativeWindow.onScaleChanged(changed), nativeWindow.onFocusChanged(changed)];
    for (const listener of nativeListeners) void listener.catch(cause => { if (!disposed) setError(String(cause)); });
    const wheel = (event: WheelEvent) => {
      if (!current.current.atomCount) return;
      event.preventDefault();
      queue({ kind: 'zoom', delta: event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? node.clientHeight : 1) });
    };
    node.addEventListener('wheel', wheel, { passive: false });
    queue();
    return () => {
      disposed = true; cancelAnimationFrame(frame); observer.disconnect();
      window.removeEventListener('resize', changed); window.removeEventListener('scroll', changed, true);
      document.removeEventListener('visibilitychange', changed); theme.removeEventListener('change', changed);
      for (const listener of nativeListeners) void listener.then(unlisten => unlisten()).catch(() => {});
      node.removeEventListener('wheel', wheel); schedule.current = () => {};
      if (!inFlight) void invoke('detach_viewport', { client }).catch(() => {});
    };
  }, []);
  const enabled = isTauri() && atomCount > 0;
  const selected = view?.documentRevision === documentRevision ? view.selected : null;
  const button = (icon: keyof typeof icons, label: string, action: Action, pressed?: boolean) => <button type="button" title={label} aria-label={label} aria-pressed={pressed} disabled={!enabled} onClick={() => schedule.current(action)}><Glyph name={icon} /></button>;
  return <div className="molecular-view">
    <div ref={element} className="native-viewport" tabIndex={0} role="group" aria-label={t.viewport} aria-describedby="viewport-help"
      onPointerDown={event => { if (!enabled || event.button !== 0) return; event.currentTarget.focus({ preventScroll: true }); event.currentTarget.setPointerCapture(event.pointerId); drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, moved: false }; }}
      onPointerMove={event => { const previous = drag.current; if (!previous || previous.id !== event.pointerId) return;
        if (Math.hypot(event.clientX - previous.startX, event.clientY - previous.startY) > 3) previous.moved = true;
        if (previous.moved) schedule.current({ kind: 'orbit', dx: event.clientX - previous.x, dy: event.clientY - previous.y });
        previous.x = event.clientX; previous.y = event.clientY;
      }}
      onPointerUp={event => { const previous = drag.current; drag.current = null; if (!previous || previous.id !== event.pointerId) return;
        if (!previous.moved) { const rect = event.currentTarget.getBoundingClientRect(); schedule.current({ kind: 'pick', x: (event.clientX - rect.left) / rect.width, y: (event.clientY - rect.top) / rect.height }); }
        event.currentTarget.releasePointerCapture(event.pointerId);
      }}
      onPointerCancel={() => { drag.current = null; }} onLostPointerCapture={() => { drag.current = null; }}
      onKeyDown={event => {
        if (!enabled) return;
        let action: Action | undefined;
        if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key)) action = { kind: 'orbit', dx: event.key === 'ArrowLeft' ? -15 : event.key === 'ArrowRight' ? 15 : 0, dy: event.key === 'ArrowUp' ? -15 : event.key === 'ArrowDown' ? 15 : 0 };
        else if (event.key === '+' || event.key === '=') action = { kind: 'zoom', delta: -100 };
        else if (event.key === '-') action = { kind: 'zoom', delta: 100 };
        else if (event.key.toLowerCase() === 'f' || event.key === 'Home') action = { kind: 'fit' };
        else if (event.key === 'Enter') action = { kind: 'select_next' };
        if (action) { event.preventDefault(); schedule.current(action); }
      }}>
      <span>{!isTauri() ? t.viewportNative : !atomCount ? t.empty : view?.ready ? t.viewport : t.connecting}</span>
    </div>
    <div className="viewport-tools" aria-label={t.viewport}>
      {button('fit', t.fit, { kind: 'fit' })}{button('plus', t.zoomIn, { kind: 'zoom', delta: -100 })}{button('minus', t.zoomOut, { kind: 'zoom', delta: 100 })}
      {button('bonds', t.bonds, { kind: 'bonds' }, view?.showBonds ?? true)}{button('atom', t.selectAtom, { kind: 'select_next' })}
      <button type="button" className="info" aria-label={t.viewportHelp}>ⓘ<span role="tooltip" id="viewport-help">{t.viewportHelp}</span></button>
    </div>
    <div className="viewport-status" role="status">
      {selected ? <><strong>{selected.symbol} · {selected.index}</strong><span>{selected.position.map(value => value.toFixed(3)).join(', ')} Å</span></> : <span>{t.selectionEmpty}</span>}
      <small title={t.bonds}>{view?.documentRevision === documentRevision ? view.bondCount : 0} {t.bondHints}</small>
    </div>
    {error ? <div className="viewport-error" role="alert">{t.viewportError}<button type="button" onClick={() => schedule.current()}>{t.retry}</button><details><summary>{t.details}</summary><code>{error}</code></details></div> : null}
  </div>;
}

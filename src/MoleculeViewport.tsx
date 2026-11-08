import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { messages, type Locale } from './i18n';

type Action = { kind: 'orbit'; dx: number; dy: number } | { kind: 'zoom'; delta: number } | { kind: 'pick'; x: number; y: number } | { kind: 'fit' | 'bonds' | 'select_next' };
type Valence = { documentRevision: number; index: number; status: 'inferred' | 'unsupported' | 'failed'; reason: string | null; engineVersion: string; valence: number | null; formalCharge: number | null; neighbors: number | null };
type View = { ready: boolean; documentRevision: number; selected: { index: number; symbol: string; position: number[]; inferredNeighbors: number } | null; showBonds: boolean; bondCount: number; bondSource: 'rdkit' | 'geometry'; bondStatus: string; bondEngineVersion: string | null; doubleBondCount: number; tripleBondCount: number; pixelSize: number[]; frameCount: number };
const icons = { fit: 'M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5M8 12h8m-4-4v8', plus: 'M12 5v14M5 12h14', minus: 'M5 12h14', bonds: 'm7 7 10 10M9 5l10 10M7 4a3 3 0 1 0 0 6 3 3 0 0 0 0-6m10 10a3 3 0 1 0 0 6 3 3 0 0 0 0-6', atom: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18m0 6v6m-3-3h6' };
function Glyph({ name }: { name: keyof typeof icons }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={icons[name]} /></svg>; }

export function MoleculeViewport({ locale, documentRevision, atomCount }: { locale: Locale; documentRevision: number; atomCount: number }) {
  const t = messages[locale];
  const element = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View | null>(null);
  const [valence, setValence] = useState<Valence | null>(null);
  const [valenceError, setValenceError] = useState<string | null>(null);
  const [analysisAttempt, setAnalysisAttempt] = useState(0);
  const lastAnalysisAttempt = useRef(0);
  const [bondAttempt, setBondAttempt] = useState(0);
  const [bondLoading, setBondLoading] = useState(false);
  const [bondError, setBondError] = useState<string | null>(null);
  const lastBondAttempt = useRef(0);
  const selectedIndex = view?.documentRevision === documentRevision ? view.selected?.index : undefined;
  useEffect(() => {
    setValence(null); setValenceError(null);
    if (!isTauri() || selectedIndex === undefined) return;
    let disposed = false;
    const retry = analysisAttempt !== lastAnalysisAttempt.current;
    lastAnalysisAttempt.current = analysisAttempt;
    void invoke<Valence>('atom_valence', { expectedRevision: documentRevision, index: selectedIndex, retry }).then(result => {
      if (!disposed && result.documentRevision === documentRevision && result.index === selectedIndex) { setValence(result); schedule.current(); }
    }).catch(cause => { if (!disposed) setValenceError(String(cause)); });
    return () => { disposed = true; };
  }, [documentRevision, selectedIndex, analysisAttempt]);
  const [error, setError] = useState<string | null>(null);
  const schedule = useRef<(action?: Action) => void>(() => {});
  const current = useRef({ documentRevision, atomCount });
  const drag = useRef<{ id: number; x: number; y: number; startX: number; startY: number; moved: boolean } | null>(null);

  useEffect(() => {
    current.current = { documentRevision, atomCount };
    schedule.current();
  }, [documentRevision, atomCount]);
  useEffect(() => {
    setBondError(null);
    if (!isTauri() || !atomCount) { setBondLoading(false); return; }
    let disposed = false;
    const retry = bondAttempt !== lastBondAttempt.current;
    lastBondAttempt.current = bondAttempt;
    setBondLoading(true);
    void invoke<{ documentRevision: number }>('analyze_bonds', { expectedRevision: documentRevision, retry }).catch(cause => {
      if (!disposed) setBondError(String(cause));
    }).finally(() => {
      if (!disposed) { setBondLoading(false); schedule.current(); }
    });
    return () => { disposed = true; };
  }, [documentRevision, atomCount, bondAttempt]);
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
      let clipTop = Math.max(0, document.querySelector('[data-app-chrome]')?.getBoundingClientRect().bottom ?? 0);
      for (const tooltip of document.querySelectorAll('[data-app-chrome] [role="tooltip"]')) {
        const overlay = tooltip.getBoundingClientRect();
        if (overlay.width > 0 && overlay.height > 0 && overlay.right > rect.left && overlay.left < rect.right) clipTop = Math.max(clipTop, overlay.bottom);
      }
      try {
        const next = await invoke<View>('update_viewport', { request: {
          client, attach: !attached, document_revision: current.current.documentRevision,
          bounds: { x: rect.x, y: rect.y, width: rect.width, height: rect.height, window_width: innerWidth, window_height: innerHeight, clip_top: clipTop, visible: !document.hidden && current.current.atomCount > 0, dark: theme.matches },
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
    const chrome = document.querySelector('[data-app-chrome]');
    if (chrome) observer.observe(chrome);
    window.addEventListener('resize', changed);
    window.addEventListener('scroll', changed, true);
    document.addEventListener('visibilitychange', changed);
    theme.addEventListener('change', changed);
    const overlayChanged = (event: Event) => { if (event.target instanceof Element && event.target.closest('[data-app-chrome] .info')) queue(); };
    for (const event of ['pointerover', 'pointerout', 'focusin', 'focusout']) document.addEventListener(event, overlayChanged);
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
      for (const event of ['pointerover', 'pointerout', 'focusin', 'focusout']) document.removeEventListener(event, overlayChanged);
      for (const listener of nativeListeners) void listener.then(unlisten => unlisten()).catch(() => {});
      node.removeEventListener('wheel', wheel); schedule.current = () => {};
      if (!inFlight) void invoke('detach_viewport', { client }).catch(() => {});
    };
  }, []);
  const enabled = isTauri() && atomCount > 0;
  const selected = view?.documentRevision === documentRevision ? view.selected : null;
  const bondView = view?.documentRevision === documentRevision ? view : null;
  const orderedBonds = bondView?.bondSource === 'rdkit';
  const atomValence = valence?.documentRevision === documentRevision && valence.index === selected?.index ? valence : null;
  const analysisStatus = atomValence?.status === 'inferred' ? t.valenceInferred : atomValence?.status === 'unsupported' ? t.valenceUnsupported : atomValence?.status === 'failed' || valenceError ? t.valenceFailed : t.valenceAnalyzing;
  const analysisReason = valenceError === 'worker_missing' ? t.valenceWorkerMissing : valenceError?.startsWith('valence_timeout') ? t.valenceTimeout : atomValence?.reason === 'open_shell' ? t.valenceOpenShell : atomValence?.reason === 'element' ? t.valenceElement : t.valenceNoAssignment;
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
      {selected ? <><strong>{selected.symbol} · {t.atomLabel} {selected.index}</strong><span>{selected.position.map(value => value.toFixed(3)).join(', ')} Å</span></> : <span>{t.selectionEmpty}</span>}
      <small title={t.bonds}>{bondView?.bondCount ?? 0} {orderedBonds ? t.bondCountLabel : t.bondHints}</small>
    </div>
    {atomCount > 0 ? <div className="bond-source-status" role="status">
      <span>{orderedBonds ? `${t.bondSourceRdkit} · ${bondView?.bondEngineVersion ?? ''}` : t.bondSourceGeometry}</span>
      {orderedBonds ? <small>{t.doubleBonds} {bondView?.doubleBondCount ?? 0} · {t.tripleBonds} {bondView?.tripleBondCount ?? 0}</small> : <small>{bondLoading ? t.valenceAnalyzing : bondError || bondView?.bondStatus === 'failed' ? t.valenceFailed : bondView?.bondStatus === 'unsupported' ? t.valenceUnsupported : ''}</small>}
      {!orderedBonds && !bondLoading && !selected ? <button type="button" disabled={!isTauri()} onClick={() => setBondAttempt(attempt => attempt + 1)}>{t.retryValence}</button> : null}
    </div> : null}
    {selected ? <div className="atom-inspector" aria-label={t.atomProperties}>
      <div className="atom-analysis"><dl><div><dt>{t.valence}</dt><dd>{atomValence?.status === 'inferred' ? atomValence.valence : '—'} <small>{analysisStatus}</small></dd></div>{atomValence?.status === 'inferred' ? <div><dt>{t.formalCharge}</dt><dd>{atomValence.formalCharge! > 0 ? `+${atomValence.formalCharge}` : atomValence.formalCharge}</dd></div> : null}<div><dt>{t.inferredNeighbors}</dt><dd>{selected.inferredNeighbors}</dd></div></dl>
      {atomValence?.engineVersion ? <small className="valence-provenance">RDKit {atomValence.engineVersion}</small> : null}
      {valenceError || atomValence?.status === 'failed' || atomValence?.status === 'unsupported' ? <p className="valence-reason" role="status">{analysisReason}<button type="button" onClick={() => setAnalysisAttempt(attempt => attempt + 1)}>{t.retryValence}</button></p> : null}</div>
      <button type="button" className="info" aria-label={t.valenceHelp}>ⓘ<span role="tooltip">{t.valenceHelp}</span></button>
    </div> : atomCount > 0 ? <p className="atom-selection-hint">{t.selectAtomHint}</p> : null}
    {error ? <div className="viewport-error" role="alert">{t.viewportError}<button type="button" onClick={() => schedule.current()}>{t.retry}</button><details><summary>{t.details}</summary><code>{error}</code></details></div> : null}
  </div>;
}

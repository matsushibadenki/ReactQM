import { useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { messages, errorMessage, type Locale } from './i18n';
import { MoleculeEditor, type Edit, type Selection } from './MoleculeEditor';

type Action = { kind: 'orbit'; dx: number; dy: number } | { kind: 'zoom'; delta: number } | { kind: 'pick'; x: number; y: number } | { kind: 'fit' | 'bonds' | 'select_next' };
type Valence = { documentRevision: number; index: number; status: 'inferred' | 'unsupported' | 'failed'; reason: string | null; engineVersion: string; valence: number | null; formalCharge: number | null; neighbors: number | null };
export type Field = 'density' | 'homo' | 'lumo' | 'homo_alpha' | 'lumo_alpha' | 'homo_beta' | 'lumo_beta';
type NebResult = { climb: boolean; steps: number; ase_version: string; max_neb_force_ev_per_angstrom: number; images: { energy_hartree: number; max_force_ev_per_angstrom: number }[] };
export type FieldResult = { job_id: string; method: string; fields?: Field[]; neb?: NebResult };
type View = { ready: boolean; documentRevision: number; selected: Selection | null; showBonds: boolean; bondCount: number; bondSource: 'rdkit' | 'geometry' | 'manual'; bondStatus: string; bondEngineVersion: string | null; doubleBondCount: number; tripleBondCount: number; pixelSize: number[]; frameCount: number };
const icons = { fit: 'M8 3H3v5m13-5h5v5M3 16v5h5m13-5v5h-5M8 12h8m-4-4v8', plus: 'M12 5v14M5 12h14', minus: 'M5 12h14', bonds: 'm7 7 10 10M9 5l10 10M7 4a3 3 0 1 0 0 6 3 3 0 0 0 0-6m10 10a3 3 0 1 0 0 6 3 3 0 0 0 0-6', atom: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18m0 6v6m-3-3h6' };
function Glyph({ name }: { name: keyof typeof icons }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={icons[name]} /></svg>; }

export function MoleculeViewport({ locale, documentRevision, atomCount, manualBonds, editingDisabled, onEdit, result }: { locale: Locale; documentRevision: number; atomCount: number; manualBonds: boolean; editingDisabled: boolean; onEdit: (edit: Edit) => void; result: FieldResult | null }) {
  const t = messages[locale];
  const pathMode = !!result?.neb;
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
    if (!isTauri() || manualBonds || pathMode || selectedIndex === undefined) return;
    let disposed = false;
    const retry = analysisAttempt !== lastAnalysisAttempt.current;
    lastAnalysisAttempt.current = analysisAttempt;
    void invoke<Valence>('atom_valence', { expectedRevision: documentRevision, index: selectedIndex, retry }).then(result => {
      if (!disposed && result.documentRevision === documentRevision && result.index === selectedIndex) { setValence(result); schedule.current(); }
    }).catch(cause => { if (!disposed) setValenceError(String(cause)); });
    return () => { disposed = true; };
  }, [documentRevision, selectedIndex, analysisAttempt, manualBonds, pathMode]);
  const [error, setError] = useState<string | null>(null);
  const schedule = useRef<(action?: Action) => void>(() => {});
  const current = useRef({ documentRevision, atomCount });
  const drag = useRef<{ id: number; x: number; y: number; startX: number; startY: number; moved: boolean; editing: boolean; revision: number } | null>(null);

  useEffect(() => {
    current.current = { documentRevision, atomCount };
    schedule.current();
  }, [documentRevision, atomCount]);
  useEffect(() => {
    setBondError(null);
    if (!isTauri() || !atomCount || manualBonds || pathMode) { setBondLoading(false); return; }
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
  }, [documentRevision, atomCount, bondAttempt, manualBonds, pathMode]);
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
  useEffect(() => { schedule.current(); }, [result?.job_id]);

  const enabled = isTauri() && atomCount > 0;
  const selected = view?.documentRevision === documentRevision ? view.selected : null;
  const bondView = view?.documentRevision === documentRevision ? view : null;
  const orderedBonds = manualBonds || bondView?.bondSource === 'rdkit';
  const atomValence = valence?.documentRevision === documentRevision && valence.index === selected?.index ? valence : null;
  const analysisStatus = atomValence?.status === 'inferred' ? t.valenceInferred : atomValence?.status === 'unsupported' ? t.valenceUnsupported : atomValence?.status === 'failed' || valenceError ? t.valenceFailed : t.valenceAnalyzing;
  const analysisReason = valenceError === 'worker_missing' ? t.valenceWorkerMissing : valenceError?.startsWith('valence_timeout') ? t.valenceTimeout : atomValence?.reason === 'open_shell' ? t.valenceOpenShell : atomValence?.reason === 'element' ? t.valenceElement : t.valenceNoAssignment;
  const button = (icon: keyof typeof icons, label: string, action: Action, pressed?: boolean) => <button type="button" title={label} aria-label={label} aria-pressed={pressed} disabled={!enabled} onClick={() => schedule.current(action)}><Glyph name={icon} /></button>;
  return <div className="molecular-view">
    <div ref={element} className="native-viewport" tabIndex={0} role="group" aria-label={t.viewport} aria-describedby="viewport-help"
      onPointerDown={event => { if (!enabled || event.button !== 0) return; event.currentTarget.focus({ preventScroll: true }); event.currentTarget.setPointerCapture(event.pointerId); drag.current = { id: event.pointerId, x: event.clientX, y: event.clientY, startX: event.clientX, startY: event.clientY, moved: false, editing: event.shiftKey && !editingDisabled && !pathMode, revision: documentRevision }; }}
      onPointerMove={event => { const previous = drag.current; if (!previous || previous.id !== event.pointerId) return;
        if (Math.hypot(event.clientX - previous.startX, event.clientY - previous.startY) > 3) previous.moved = true;
        if (previous.moved && !previous.editing) schedule.current({ kind: 'orbit', dx: event.clientX - previous.x, dy: event.clientY - previous.y });
        previous.x = event.clientX; previous.y = event.clientY;
      }}
      onPointerUp={event => { const previous = drag.current; drag.current = null; if (!previous || previous.id !== event.pointerId) return;
        const rect = event.currentTarget.getBoundingClientRect();
        const point = (x: number, y: number) => [(x - rect.left) / rect.width, (y - rect.top) / rect.height].map(v => Math.min(1, Math.max(0, v)));
        if (!previous.moved) {
          const [x, y] = point(event.clientX, event.clientY);
          schedule.current({ kind: 'pick', x, y });
        } else if (previous.editing && !editingDisabled && previous.revision === documentRevision && rect.width && rect.height) {
          onEdit({ kind: 'drag', aspect: rect.width / rect.height, start: point(previous.startX, previous.startY), end: point(event.clientX, event.clientY) });
        }
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
    {result && !pathMode && result.method !== 'GFN2-xTB' ? <ElectronicFields key={`${documentRevision}/${result.job_id}`} result={result} revision={documentRevision} locale={locale} redraw={() => schedule.current()} /> : null}
    {result?.neb ? <NebImages key={`${documentRevision}/${result.job_id}`} result={result.neb} jobId={result.job_id} revision={documentRevision} locale={locale} redraw={() => schedule.current()} /> :
      <MoleculeEditor key={documentRevision} locale={locale} disabled={editingDisabled} selected={selected ?? null} onEdit={onEdit} />}

    <div className="viewport-status" role="status">
      {selected ? <><strong>{selected.symbol} · {t.atomLabel} {selected.index}</strong><span>{selected.position.map(value => value.toFixed(3)).join(', ')} Å</span></> : <span>{t.selectionEmpty}</span>}
      <small title={t.bonds}>{bondView?.bondCount ?? 0} {orderedBonds ? t.bondCountLabel : t.bondHints}</small>
    </div>
    {!pathMode && atomCount > 0 ? <div className="bond-source-status" role="status">
      <span>{manualBonds ? t.bondSourceManual : orderedBonds ? `${t.bondSourceRdkit} · ${bondView?.bondEngineVersion ?? ''}` : t.bondSourceGeometry}</span>
      {orderedBonds ? <small>{t.doubleBonds} {bondView?.doubleBondCount ?? 0} · {t.tripleBonds} {bondView?.tripleBondCount ?? 0}</small> : <small>{bondLoading ? t.valenceAnalyzing : bondError || bondView?.bondStatus === 'failed' ? t.valenceFailed : bondView?.bondStatus === 'unsupported' ? t.valenceUnsupported : ''}</small>}
      {!orderedBonds && !bondLoading && !selected ? <button type="button" disabled={!isTauri()} onClick={() => setBondAttempt(attempt => attempt + 1)}>{t.retryValence}</button> : null}
    </div> : null}
    {pathMode ? null : selected && manualBonds ? <div className="atom-inspector"><dl><div><dt>{t.valence}</dt><dd>{selected.bonds.reduce((sum, [, order]) => sum + order, 0)}</dd></div></dl></div> : selected ? <div className="atom-inspector" aria-label={t.atomProperties}>
      <div className="atom-analysis"><dl><div><dt>{t.valence}</dt><dd>{atomValence?.status === 'inferred' ? atomValence.valence : '—'} <small>{analysisStatus}</small></dd></div>{atomValence?.status === 'inferred' ? <div><dt>{t.formalCharge}</dt><dd>{atomValence.formalCharge! > 0 ? `+${atomValence.formalCharge}` : atomValence.formalCharge}</dd></div> : null}<div><dt>{t.inferredNeighbors}</dt><dd>{selected.inferredNeighbors}</dd></div></dl>
      {atomValence?.engineVersion ? <small className="valence-provenance">RDKit {atomValence.engineVersion}</small> : null}
      {valenceError || atomValence?.status === 'failed' || atomValence?.status === 'unsupported' ? <p className="valence-reason" role="status">{analysisReason}<button type="button" onClick={() => setAnalysisAttempt(attempt => attempt + 1)}>{t.retryValence}</button></p> : null}</div>
      <button type="button" className="info" aria-label={t.valenceHelp}>ⓘ<span role="tooltip">{t.valenceHelp}</span></button>
    </div> : atomCount > 0 ? <p className="atom-selection-hint">{t.selectAtomHint}</p> : null}
    {error ? <div className="viewport-error" role="alert">{t.viewportError}<button type="button" onClick={() => schedule.current()}>{t.retry}</button><details><summary>{t.details}</summary><code>{error}</code></details></div> : null}
  </div>;
}

function ElectronicFields({ result, revision, locale, redraw }: { result: FieldResult; revision: number; locale: Locale; redraw: () => void }) {
  const t = messages[locale];
  const [field, setField] = useState<Field | ''>('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const names: Record<Field, string> = {
    density: t.fieldDensity, homo: 'HOMO', lumo: 'LUMO',
    homo_alpha: 'HOMO α', lumo_alpha: 'LUMO α', homo_beta: 'HOMO β', lumo_beta: 'LUMO β',
  };
  async function change(value: Field | '') {
    setPending(true);
    setError(null);
    try {
      await invoke('set_electronic_field', { revision, jobId: result.job_id, field: value || null });
      setField(value);
    } catch (cause) {
      setField('');
      setError(String(cause));
    } finally {
      setPending(false);
      redraw();
    }
  }
  if (!result.fields?.length) return <small>{t.fieldUnavailable}</small>;
  return <div className="electronic-fields">
    <label>{t.fieldLabel}
      <select aria-label={t.fieldLabel} value={field} disabled={pending || !isTauri()} onChange={event => void change(event.target.value as Field | '')}>
        <option value="">{t.fieldOff}</option>
        {result.fields.map(value => <option key={value} value={value}>{names[value]}</option>)}
      </select>
    </label>
    <button type="button" className="info" aria-label={t.fieldHelp}>ⓘ<span role="tooltip">{t.fieldHelp}</span></button>
    {pending ? <small role="status">{t.fieldLoading}</small> : null}
    {field ? <small>{field === 'density' ? '0.02 e/bohr³' : `|ψ| = 0.03 bohr⁻³⁄² · ${t.fieldPhases}`}</small> : null}
    {error ? <small role="alert">{errorMessage(error, locale)}</small> : null}
  </div>;
}

function NebImages({ result, jobId, revision, locale, redraw }: { result: NebResult; jobId: string; revision: number; locale: Locale; redraw: () => void }) {
  const t = messages[locale];
  const [selected, setSelected] = useState<number | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function show(index: number) {
    setPending(true);
    setError(null);
    try {
      await invoke('show_neb_image', { expectedRevision: revision, jobId, index });
      setSelected(index);
      redraw();
    } catch (cause) { setError(String(cause)); }
    finally { setPending(false); }
  }

  return <div className="neb-results">
    <strong>{result.climb ? 'CI-NEB' : 'NEB'} · ASE {result.ase_version}</strong>
    <div>{t.optimizationStep} {result.steps} · {t.nebProjectedForce} {result.max_neb_force_ev_per_angstrom.toPrecision(3)} eV/Å</div>
    <table>
      <thead><tr><th scope="col">{t.nebImage}</th><th scope="col">E (Eh)</th><th scope="col">{t.maximumForce} (eV/Å)</th></tr></thead>
      <tbody>{result.images.map((image, index) => <tr key={index}>
        <th scope="row"><button type="button" aria-pressed={selected === index} disabled={pending} onClick={() => void show(index)} aria-label={`${t.nebImage} ${index + 1}`}>{index + 1}</button></th>
        <td>{image.energy_hartree.toFixed(8)}</td><td>{image.max_force_ev_per_angstrom.toPrecision(3)}</td>
      </tr>)}</tbody>
    </table>
    {error ? <small role="alert">{errorMessage(error, locale)}</small> : null}
  </div>;
}

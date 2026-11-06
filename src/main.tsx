import { StrictMode, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messages, errorMessage, type Locale } from './i18n';
import './style.css';
import { MoleculeViewport } from './MoleculeViewport';

type Calculation = { job_id: string; input_hash: string; method: string; basis: string; energy_hartree: number; engine_version: string; elapsed_seconds: number; converged: boolean };
type JobStatus = 'running' | 'cancelling' | 'succeeded' | 'cancelled' | 'failed' | 'timed_out';
type Snapshot = { revision: number; documentRevision: number; locale: Locale; atomCount: number; name: string; formula: string; charge: number; spin: number; rendererReady: boolean; canUndo: boolean; canRedo: boolean; dirty: boolean; hasSaved: boolean; calculation: Calculation | null; job: { id: string; status: JobStatus; cycle: number; error: string | null } | null; busy: boolean };
const initial: Snapshot = { revision: 0, documentRevision: 0, locale: 'ja', atomCount: 0, name: '', formula: '', charge: 0, spin: 0, rendererReady: false, canUndo: false, canRedo: false, dirty: false, hasSaved: false, calculation: null, job: null, busy: false };
const native = isTauri();
const hydrogen = '2\nH₂ · 0.74 Å\nH 0 0 0\nH 0 0 0.74\n';
const water = '3\nH₂O\nO 0 0 0\nH .7586 0 .5043\nH -.7586 0 .5043\n';
const paths = {
  open: 'M3 7h6l2-3h5v4H5v12h14l2-10H7L5 20',
  save: 'M5 3h12l4 4v14H3V3h2Zm2 0v6h10V3M7 21v-8h10v8',
  reload: 'M4 10a8 8 0 1 1 0 5M4 4v6h6',
  undo: 'M9 5 3 11l6 6M3 11h11a6 6 0 0 1 6 6',
  redo: 'm15 5 6 6-6 6m6-6H10a6 6 0 0 0-6 6',
  play: 'm8 4 12 8-12 8Z', stop: 'M5 5h14v14H5Z',
};
function Icon({ name }: { name: keyof typeof paths }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={paths[name]} /></svg>; }
function Info({ text }: { text: string }) { return <button type="button" className="info" aria-label={text}>ⓘ<span role="tooltip">{text}</span></button>; }

function App() {
  // Only drafts and a read-only projection live here; Rust owns the project/history/job.
  const [snapshot, setSnapshot] = useState<Snapshot>(initial);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<'saved' | 'loaded' | 'imported' | null>(null);
  const [pending, setPending] = useState(false);
  const [draft, setDraft] = useState('');
  const [charge, setCharge] = useState('0');
  const [spin, setSpin] = useState('0');
  const file = useRef<HTMLInputElement>(null);
  const t = messages[snapshot.locale];
  const locked = !connected || snapshot.busy || pending;
  const accept = (next: Snapshot) => setSnapshot(current => next.revision >= current.revision ? next : current);

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<Snapshot>('workspace-changed', event => { if (!disposed) accept(event.payload); }).then(async unlisten => {
      if (disposed) { unlisten(); return; }
      stop = unlisten;
      const next = await invoke<Snapshot>('workspace_snapshot');
      if (!disposed) { accept(next); setConnected(true); }
    }).catch(cause => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; stop?.(); };
  }, []);
  useEffect(() => { document.documentElement.lang = snapshot.locale; }, [snapshot.locale]);

  async function command(name: string, args: Record<string, unknown> = {}, notice?: 'saved' | 'loaded' | 'imported') {
    setPending(true); setError(null); setNotice(null);
    try {
      const next = await invoke<Snapshot>(name, { expectedRevision: snapshot.documentRevision, ...args });
      accept(next); setConnected(true); if (notice) setNotice(notice);
      return true;
    } catch (cause) { setError(String(cause)); return false; }
    finally { setPending(false); }
  }
  async function changeLocale(locale: Locale) {
    if (!native) { setSnapshot(current => ({ ...current, locale })); return; }
    await command('set_locale', { locale });
  }
  async function importText(text: string, sample = false) {
    const q = sample ? 0 : Number(charge), s = sample ? 0 : Number(spin);
    if (!Number.isSafeInteger(q) || q < -2147483648 || q > 2147483647 || !Number.isSafeInteger(s) || s < 0 || s > 4294967295 || (!sample && (!charge.trim() || !spin.trim()))) { setError('numeric_input'); return; }
    if (await command('import_xyz', { text, charge: q, spin: s }, 'imported')) setDraft('');
  }
  const jobError = snapshot.job?.error;
  const shownError = error ?? jobError;
  const result = snapshot.calculation;

  return <main>
    <header><strong>ReactQM <small>0.1</small></strong>
      <select aria-label={t.language} value={snapshot.locale} disabled={pending} onChange={event => void changeLocale(event.target.value as Locale)}>
        <option value="ja">日本語</option><option value="en">English</option><option value="zh-CN">简体中文</option>
      </select>
    </header>
    <div className="page-heading"><h1>{t.workspace}</h1><span className="connection">{connected ? t.native : native ? t.connecting : t.preview}</span></div>
    <nav className="toolbar" aria-label={t.molecule}>
      <button title={t.open} aria-label={t.open} disabled={locked} onClick={() => file.current?.click()}><Icon name="open" /></button>
      <button title={t.sample} aria-label={t.sample} disabled={locked} onClick={() => void importText(hydrogen, true)}>H₂</button>
      <button title={t.waterSample} aria-label={t.waterSample} disabled={locked} onClick={() => void importText(water, true)}>H₂O</button>
      <span className="separator" />
      <button title={t.undo} aria-label={t.undo} disabled={locked || !snapshot.canUndo} onClick={() => void command('history_step', { redo: false })}><Icon name="undo" /></button>
      <button title={t.redo} aria-label={t.redo} disabled={locked || !snapshot.canRedo} onClick={() => void command('history_step', { redo: true })}><Icon name="redo" /></button>
      <span className="separator" />
      <button title={t.save} aria-label={t.save} disabled={locked || !snapshot.atomCount} onClick={() => void command('save_project', {}, 'saved')}><Icon name="save" /></button>
      <button title={t.reload} aria-label={t.reload} disabled={locked || !snapshot.hasSaved} onClick={() => void command('load_project', {}, 'loaded')}><Icon name="reload" /></button>
      <Info text={t.saveHelp} />
    </nav>
    <input ref={file} className="file-input" type="file" accept=".xyz,text/plain" aria-label={t.open} onChange={async event => {
      const selected = event.target.files?.[0]; event.target.value = '';
      if (!selected) return;
      if (selected.size > 16_384) { setError('xyz_size'); return; }
      try { await importText(await selected.text()); } catch (cause) { setError(String(cause)); }
    }} />
    <div className="workspace-grid">
      <section aria-labelledby="molecule-title">
        <div className="heading"><h2 id="molecule-title">{t.molecule}</h2>{snapshot.dirty ? <span className="dirty" aria-label={t.unsaved} title={t.unsaved}>●</span> : null}</div>
        <MoleculeViewport locale={snapshot.locale} documentRevision={snapshot.documentRevision} atomCount={snapshot.atomCount} />
        <div className="molecule-summary"><strong className="formula">{snapshot.formula || '—'}</strong><span>{snapshot.atomCount ? snapshot.name || t.untitled : t.empty}</span></div>
        <dl className="metrics"><div><dt>{t.atoms}</dt><dd>{snapshot.atomCount}</dd></div><div><dt>{t.charge}</dt><dd>{snapshot.charge}</dd></div><div><dt>{t.spin}</dt><dd>{snapshot.spin}</dd></div></dl>
        <details className="xyz-editor"><summary>{t.importTitle}</summary><form onSubmit={event => { event.preventDefault(); void importText(draft); }}>
          <div className="heading"><label htmlFor="xyz">{t.xyz}</label><Info text={t.inputHelp} /></div>
          <textarea id="xyz" aria-label={t.draft} spellCheck={false} value={draft} disabled={locked} maxLength={16_384} placeholder={'2\nH₂\nH 0 0 0\nH 0 0 0.74'} onChange={event => setDraft(event.target.value)} />
          <div className="input-row"><label>{t.charge}<input type="number" step="1" value={charge} disabled={locked} onChange={event => setCharge(event.target.value)} /></label><label>{t.spin}<input type="number" min="0" step="1" value={spin} disabled={locked} onChange={event => setSpin(event.target.value)} /></label><button type="submit" disabled={locked || !draft.trim()}>{t.apply}</button></div>
        </form></details>
      </section>
      <section aria-labelledby="calculation-title">
        <div className="heading"><h2 id="calculation-title">{t.result}</h2><Info text={t.calculationHelp} /></div>
        <div className="method">RHF <span>/</span> STO-3G</div>
        <div className="calculation-actions">
          {snapshot.busy ? <button className="primary" disabled={pending || snapshot.job?.status === 'cancelling'} onClick={() => void command('cancel_job')}><Icon name="stop" />{t.cancel}</button>
            : <button className="primary" disabled={locked || !snapshot.atomCount} onClick={() => void command('start_calculation')}><Icon name="play" />{t.calculate}</button>}
          <span className={'job-status ' + (snapshot.job?.status || '')} role="status">{snapshot.job ? t[snapshot.job.status] : result ? t.succeeded : t.noResult}</span>
        </div>
        {snapshot.busy ? <div className="progress"><progress aria-label={t.running} /><span>{t.cycle} {snapshot.job?.cycle ?? 0} / 50</span></div> : null}
        {result ? <div className="result">
          <span className="eyebrow">{t.energy}</span><div className="energy"><output>{result.energy_hartree.toFixed(10)}</output><span>Eh</span></div>
          <dl><div><dt>{t.engine}</dt><dd>PySCF {result.engine_version}</dd></div><div><dt>{t.elapsed}</dt><dd>{result.elapsed_seconds.toFixed(2)} s</dd></div></dl>
          <details><summary>{t.provenance}</summary><code>{result.job_id}</code><code>SHA-256 {result.input_hash}</code></details>
        </div> : null}
      </section>
    </div>
    {shownError ? <div className="error" role="alert">{errorMessage(shownError, snapshot.locale)}<details><summary>{t.details}</summary><code>{shownError}</code>{snapshot.job ? <code>{snapshot.job.id}</code> : null}</details></div> : null}
    <footer><span role="status">{notice ? t[notice] : !native ? t.previewHelp : ''}</span>{native && !connected ? <button onClick={() => void command('workspace_snapshot')}>{t.retry}</button> : null}</footer>
  </main>;
}
createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);

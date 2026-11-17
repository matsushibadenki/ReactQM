import { RefinementComparison, RefinementQueue, refinementName, type RefinementReport } from './Refinement';
import { Thermochemistry, ThermalQueue, type ThermalReport } from './Thermochemistry';
import { ReactionEndpoints } from './ReactionEndpoints';
import { StrictMode, useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messages, errorMessage, type Locale } from './i18n';
import './style.css';
import { MoleculeViewport, type FieldResult } from './MoleculeViewport';
import { JobHistory } from './JobHistory';
import { useQueue } from './useQueue';
import { QueueToolbar } from './QueueToolbar';
import { CalculationControls, defaultSettings, conditionLabel, type Settings } from './CalculationControls';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { useLayout, type PanelId } from './useLayout';
import { DockWorkspace, FloatingPanel, LayoutTools, panelTitle } from './DockWorkspace';

type Transition = {
  source_job: string; seed_image: number; sella_version: string; steps: number; imaginary_count: number; frequencies_cm1: number[];
  branches: { converged: boolean; match: 'reactant' | 'product' | null; reactant_rmsd_angstrom: number; product_rmsd_angstrom: number; max_force_ev_per_angstrom: number; steps: number }[];
};
type Calculation = FieldResult & { input_hash: string; basis: string; energy_hartree: number; engine_version: string; elapsed_seconds: number; converged: boolean; settings?: Settings; transition?: Transition; thermochemistry?: ThermalReport; refinement?: RefinementReport; spin_squared?: number; optimization?: { steps: number; max_force_ev_per_angstrom: number; ase_version: string } };
type JobStatus = 'running' | 'cancelling' | 'succeeded' | 'cancelled' | 'failed' | 'timed_out' | 'interrupted' | 'corrupt';
type InputFormat = 'xyz' | 'smiles' | 'sdf';
type StructureSource = { format: string; method: 'etkdgv3' | 'sdf_3d'; seed: number | null; engineVersion: string; inputHash: string };
type Snapshot = { revision: number; documentRevision: number; locale: Locale; atomCount: number; manualBonds: boolean; name: string; formula: string; charge: number; spin: number; rendererReady: boolean; canUndo: boolean; canRedo: boolean; dirty: boolean; hasSaved: boolean; calculation: Calculation | null; job: { id: string; status: JobStatus; cycle: number; error: string | null } | null; busy: boolean; structureSource: StructureSource | null; settings: Settings };
const initial: Snapshot = { revision: 0, documentRevision: 0, locale: 'ja', atomCount: 0, manualBonds: false, name: '', formula: '', charge: 0, spin: 0, rendererReady: false, canUndo: false, canRedo: false, dirty: false, hasSaved: false, calculation: null, job: null, busy: false, structureSource: null, settings: defaultSettings };
const native = isTauri();
const windowPanel: PanelId | null = native && getCurrentWindow().label === 'panel-result' ? 'result' : native && getCurrentWindow().label === 'panel-history' ? 'history' : null;
const hydrogen = '2\nH₂ · 0.74 Å\nH 0 0 0\nH 0 0 0.74\n';
const water = '3\nH₂O\nO 0 0 0\nH .7586 0 .5043\nH -.7586 0 .5043\n';
const carbonDioxide = '3\nCO₂\nO -1.16 0 0\nC 0 0 0\nO 1.16 0 0\n';
const nitrogen = '2\nN₂\nN -.55 0 0\nN .55 0 0\n';
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

function TransitionDetails({ result, locale }: { result: Transition; locale: Locale }) {
  const t = messages[locale];
  const matches = new Set(result.branches.map(branch => branch.match));
  const confirmed = result.imaginary_count === 1 && matches.has('reactant') && matches.has('product');
  return <section className="transition-result">
    <strong>{t.transitionJob} · Sella {result.sella_version}</strong>
    <p role="status">{confirmed ? t.transitionConfirmed : t.transitionUnconfirmed}</p>
    <p>{t.nebImage} {result.seed_image + 1} · {t.imaginaryModes} {result.imaginary_count}</p>
    <p>{t.frequencies} (cm⁻¹) · {result.frequencies_cm1.map(value => value.toFixed(1)).join(' / ')}</p>
    {result.branches.map((branch, i) => <div key={i}>
      <strong>{i === 0 ? t.ircForward : t.ircReverse}</strong>
      <span> · {branch.match ? t[branch.match] : branch.converged ? t.ircUnmatched : t.ircNotConverged}</span>
      <small>RMSD · {t.reactant} {branch.reactant_rmsd_angstrom.toFixed(3)} Å · {t.product} {branch.product_rmsd_angstrom.toFixed(3)} Å</small>
    </div>)}
    <details><summary>{t.provenance}</summary><code>CI-NEB {result.source_job}</code></details>
  </section>;
}

function App() {
  // Only drafts and a read-only projection live here; Rust owns the project/history/job.
  const [snapshot, setSnapshot] = useState<Snapshot>(initial);
  const [connected, setConnected] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dismissedJobError, setDismissedJobError] = useState<string | null>(null);
  const [notice, setNotice] = useState<'saved' | 'loaded' | 'imported' | null>(null);
  const [pending, setPending] = useState(false);
  const [draft, setDraft] = useState('');
  const [format, setFormat] = useState<InputFormat>('smiles');
  const [seed, setSeed] = useState('42');
  const [charge, setCharge] = useState('0');
  const [multiplicity, setMultiplicity] = useState('1');
  const file = useRef<HTMLInputElement>(null);
  const queue = useQueue();
  const layout = useLayout();
  const t = messages[snapshot.locale];
  const locked = !connected || pending;
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
  async function importText(text: string, sample = false, inputFormat = format) {
    if (!sample && inputFormat !== 'xyz') {
      const value = Number(seed);
      if (!seed.trim() || !Number.isSafeInteger(value) || value < 0 || value > 2147483647) { setError('structure_seed'); return; }
      if (await command('import_structure', { format: inputFormat, text, seed: value }, 'imported')) setDraft('');
      return;
    }
    const q = sample ? 0 : Number(charge), s = sample ? 0 : Number(multiplicity) - 1;
    if (!Number.isSafeInteger(q) || q < -2147483648 || q > 2147483647 || !Number.isSafeInteger(s) || s < 0 || s > 4294967295 || (!sample && (!charge.trim() || !multiplicity.trim()))) { setError('numeric_input'); return; }
    if (await command('import_xyz', { text, charge: q, spin: s }, 'imported')) setDraft('');
  }
  const jobError = snapshot.job?.error;
  const shownError = error ?? layout.error ?? (dismissedJobError === snapshot.job?.id ? null : jobError);
  const result = snapshot.calculation;

  const moleculePanel = (      <section aria-labelledby="molecule-title">
        <div className="heading"><h2 id="molecule-title">{t.molecule}</h2>{snapshot.dirty ? <span className="dirty" aria-label={t.unsaved} title={t.unsaved}>●</span> : null}</div>
        <MoleculeViewport locale={snapshot.locale} documentRevision={snapshot.documentRevision} atomCount={snapshot.atomCount} manualBonds={snapshot.manualBonds} result={result?.transition || result?.thermochemistry || result?.refinement ? null : result} editingDisabled={locked} onEdit={edit => void command('edit_molecule', { edit })} />
        <div className="molecule-summary"><strong className="formula">{snapshot.formula || '—'}</strong><span>{snapshot.atomCount ? snapshot.name || t.untitled : t.empty}</span></div>
        <dl className="metrics"><div><dt>{t.atoms}</dt><dd>{snapshot.atomCount}</dd></div><div><dt>{t.charge} <Info text={t.chargeHelp} /></dt><dd>{snapshot.charge === 0 ? `0 · ${t.neutral}` : snapshot.charge > 0 ? `+${snapshot.charge}` : `−${Math.abs(snapshot.charge)}`}</dd></div><div><dt>{t.spin} <Info text={t.spinHelp} /></dt><dd>{snapshot.atomCount ? snapshot.spin + 1 : '—'}</dd></div></dl>
        {snapshot.atomCount ? <form key={snapshot.documentRevision} className="input-row multiplicity-editor" onSubmit={event => {
          event.preventDefault();
          const value = Number(new FormData(event.currentTarget).get('multiplicity'));
          if (value !== snapshot.spin + 1) void command('set_multiplicity', { multiplicity: value });
        }}><label>{t.spin}<input name="multiplicity" type="number" min="1" step="1" required defaultValue={snapshot.spin + 1} disabled={locked} /></label><button type="submit" disabled={locked}>{t.apply}</button></form> : null}
        {snapshot.structureSource ? <details className="structure-source"><summary>{snapshot.structureSource.method === 'etkdgv3' ? t.initialGeometry : t.preservedGeometry} · RDKit {snapshot.structureSource.engineVersion}</summary><div className="heading"><span>{snapshot.structureSource.format.toUpperCase()}</span><Info text={t.structureHelp} /></div>{snapshot.structureSource.seed !== null ? <div>{t.seed}: {snapshot.structureSource.seed} · ETKDGv3</div> : null}<code>SHA-256 {snapshot.structureSource.inputHash}</code></details> : null}
        <details className="xyz-editor"><summary>{t.importTitle}</summary><form onSubmit={event => { event.preventDefault(); void importText(draft); }}>
          <div className="heading"><label>{t.inputFormat}<select aria-label={t.inputFormat} value={format} disabled={locked} onChange={event => setFormat(event.target.value as InputFormat)}><option value="smiles">SMILES</option><option value="sdf">SDF / MOL</option><option value="xyz">XYZ</option></select></label><Info text={format === 'xyz' ? t.inputHelp : t.structureHelp} /></div>
          <textarea id="xyz" aria-label={t.draft} spellCheck={false} value={draft} disabled={locked} maxLength={16_384} placeholder={format === 'smiles' ? 'CCO' : format === 'sdf' ? t.sdfPlaceholder : '2\nH₂\nH 0 0 0\nH 0 0 0.74'} onChange={event => setDraft(event.target.value)} />
          <div className="input-row">{format === 'xyz' ? <><label><span>{t.charge}</span><input type="number" step="1" value={charge} disabled={locked} onChange={event => setCharge(event.target.value)} /></label><label>{t.spin}<input type="number" min="1" step="1" value={multiplicity} disabled={locked} onChange={event => setMultiplicity(event.target.value)} /></label></> : <label>{t.seed}<input type="number" min="0" max="2147483647" step="1" value={seed} disabled={locked} onChange={event => setSeed(event.target.value)} /></label>}<button type="submit" disabled={locked || !draft.trim()}>{t.apply}</button></div>
        </form></details>
        <ReactionEndpoints locale={snapshot.locale} documentRevision={snapshot.documentRevision} atomCount={snapshot.atomCount} locked={locked} />
      </section>);
  const panels = {
    result: <><div className="result-help"><Info text={(result?.method ?? snapshot.settings?.method) === 'GFN2-xTB' ? t.screeningHelp : t.calculationHelp} /></div>        <CalculationControls settings={snapshot.settings ?? defaultSettings} locale={snapshot.locale} spin={snapshot.spin} disabled={locked} onChange={settings => void command('set_calculation_settings', { settings })} />
        <div className="calculation-actions">
          <span className="job-status" role="status">{result ? t.succeeded : t.noResult}</span>
        </div>
        {result ? <div className="result">
          {result.transition ? <TransitionDetails result={result.transition} locale={snapshot.locale} /> : null}
          {result.transition?.imaginary_count === 1 && result.transition.branches.some(branch => branch.match === 'reactant') && result.transition.branches.some(branch => branch.match === 'product') ?
            <ThermalQueue key={result.job_id} locale={snapshot.locale} disabled={locked} onQueue={conditions => void command('start_thermochemistry', { id: result.job_id, ...conditions })} /> : null}
          {snapshot.spin === 0 && snapshot.atomCount <= 10 && result.transition?.imaginary_count === 1 && result.transition.branches.some(branch => branch.match === 'reactant') && result.transition.branches.some(branch => branch.match === 'product') ?
            <RefinementQueue key={result.job_id} locale={snapshot.locale} disabled={locked} queue={(method, basis) => void command('start_refinement', { id: result.job_id, method, basis })} /> : null}
          {result.refinement ? <RefinementComparison report={result.refinement} basis={result.basis} locale={snapshot.locale} /> : null}
          {result.thermochemistry ? <Thermochemistry report={result.thermochemistry} locale={snapshot.locale} /> : null}
          {result.optimization ? <p className="job-status">{t.optimizeStructure} · ASE BFGS {result.optimization.ase_version} · {t.optimizationStep} {result.optimization.steps} · {t.maximumForce} {result.optimization.max_force_ev_per_angstrom.toPrecision(3)} eV/Å</p> : null}
          <div className="method">{result.refinement ? `${refinementName(result.refinement.method)} / ${result.basis.toUpperCase()}` : result.settings ? conditionLabel(result.settings) : `${result.method} / ${result.basis.toUpperCase()}`}</div>
          <span className="eyebrow">{result.neb ? `${t.reactant} · ${t.energy}` : t.energy}</span><div className="energy"><output>{result.energy_hartree.toFixed(10)}</output><span>Eh</span></div>
          <dl>{result.spin_squared !== undefined ? <div><dt>⟨S²⟩ <Info text={t.spinSquaredHelp} /></dt><dd>{result.spin_squared.toFixed(4)} / {(snapshot.spin * (snapshot.spin + 2) / 4).toFixed(4)}</dd></div> : null}<div><dt>{t.engine}</dt><dd>{result.method === 'GFN2-xTB' ? 'tblite' : 'PySCF'} {result.engine_version}</dd></div><div><dt>{t.elapsed}</dt><dd>{result.elapsed_seconds.toFixed(2)} s</dd></div></dl>
          <details><summary>{t.provenance}</summary><code>{result.job_id}</code><code>SHA-256 {result.input_hash}</code>{result.method === 'GFN2-xTB' ? <code>SCC ≤ 250 · {t.screeningAccuracy} 1.0 · 300 K</code> : result.settings ? <code>SCF ≤ {result.settings.max_cycle} · {result.settings.conv_tol.toExponential()} Eh{result.settings.grid_level ? ` · Grid ${result.settings.grid_level}` : ''}</code> : null}</details>
        </div> : null}
</>,
    history: <JobHistory locale={snapshot.locale} busy={locked} queueState={queue} onAction={command} />,
  };

  return <main className={windowPanel ? 'utility-window' : undefined}>
    <div className="app-chrome" data-app-chrome>
      <header><strong className="app-brand">ReactQM <small>0.1</small></strong>
        <h1 title={snapshot.name || t.workspace}>{windowPanel ? `${panelTitle(windowPanel, snapshot.locale)} · ${snapshot.name || t.empty}` : snapshot.name || t.workspace}</h1>
        <span className={'connection-dot ' + (connected ? 'ready' : '')} title={connected ? t.native : native ? t.connecting : t.preview} aria-label={connected ? t.native : native ? t.connecting : t.preview} />
        <select aria-label={t.language} value={snapshot.locale} disabled={pending} onChange={event => void changeLocale(event.target.value as Locale)}>
          <option value="ja">日本語</option><option value="en">English</option><option value="zh-CN">简体中文</option>
        </select>
      </header>
      {!windowPanel ? <nav className="toolbar command-bar" aria-label={t.molecule}>
        <div className="toolbar-group">
          <button title={t.open} aria-label={t.open} disabled={locked} onClick={() => file.current?.click()}><Icon name="open" /></button>
          <button title={t.sample} aria-label={t.sample} disabled={locked} onClick={() => void importText(hydrogen, true)}>H₂</button>
          <button title={t.waterSample} aria-label={t.waterSample} disabled={locked} onClick={() => void importText(water, true)}>H₂O</button>
          <button title={t.carbonDioxideSample} aria-label={t.carbonDioxideSample} disabled={locked} onClick={() => void importText(carbonDioxide, true)}>CO₂</button>
          <button title={t.nitrogenSample} aria-label={t.nitrogenSample} disabled={locked} onClick={() => void importText(nitrogen, true)}>N₂</button>
        </div>
        <div className="toolbar-group">
          <button title={t.undo} aria-label={t.undo} disabled={locked || !snapshot.canUndo} onClick={() => void command('history_step', { redo: false })}><Icon name="undo" /></button>
          <button title={t.redo} aria-label={t.redo} disabled={locked || !snapshot.canRedo} onClick={() => void command('history_step', { redo: true })}><Icon name="redo" /></button>
        </div>
        <div className="toolbar-group">
          <button title={t.save} aria-label={t.save} disabled={locked || (!snapshot.atomCount && !snapshot.dirty)} onClick={() => void command('save_project', {}, 'saved')}><Icon name="save" /></button>
          <button title={t.reload} aria-label={t.reload} disabled={locked || !snapshot.hasSaved} onClick={() => void command('load_project', {}, 'loaded')}><Icon name="reload" /></button>
          <Info text={t.saveHelp} />
        </div>
        <button type="button" disabled={locked || snapshot.atomCount < 2 || snapshot.settings?.method === 'GFN2-xTB'} onClick={() => void command('start_optimization')} title={snapshot.settings?.method === 'GFN2-xTB' ? t.screeningHelp : t.optimizationHelp}>{t.enqueueOptimization}</button>
        <button className="primary enqueue-button" disabled={locked || !snapshot.atomCount} onClick={() => void command('start_calculation')}><Icon name="play" />{t.enqueue}</button>
        <LayoutTools state={layout} locale={snapshot.locale} />
      </nav> : null}
      {!windowPanel ? <QueueToolbar locale={snapshot.locale} state={queue} job={snapshot.job} disabled={locked} onAction={command} /> : null}
      {shownError ? <div className="error app-notification" role="alert"><div className="notification-heading"><span>{errorMessage(shownError, snapshot.locale)}{layout.error ? <button disabled={layout.pending} onClick={() => void layout.update({ kind: 'reset' })}>{t.resetLayout}</button> : null}</span><button type="button" aria-label={t.dismissError} title={t.dismissError} onClick={() => { setError(null); setDismissedJobError(snapshot.job?.id ?? null); }}>×</button></div><details><summary>{t.details}</summary><code>{shownError}</code>{snapshot.job ? <code>{snapshot.job.id}</code> : null}</details></div> : null}
    </div>
    <input ref={file} className="file-input" type="file" accept=".xyz,.sdf,.mol,.smi,.smiles,text/plain" aria-label={t.open} onChange={async event => {
      const selected = event.target.files?.[0]; event.target.value = '';
      if (!selected) return;
      if (selected.size > 16_384) { setError('xyz_size'); return; }
      try { const kind: InputFormat = /\.(sdf|mol)$/i.test(selected.name) ? 'sdf' : /\.(smi|smiles)$/i.test(selected.name) ? 'smiles' : 'xyz'; setFormat(kind); await importText(await selected.text(), false, kind); } catch (cause) { setError(String(cause)); }
    }} />
    {windowPanel ? <FloatingPanel panel={windowPanel} state={layout} locale={snapshot.locale}>{panels[windowPanel]}</FloatingPanel> : <DockWorkspace state={layout} locale={snapshot.locale} molecule={moleculePanel} panels={panels} />}

    <footer><span role="status">{notice ? t[notice] : !native ? t.previewHelp : ''}</span>{native && !connected ? <button onClick={() => void command('workspace_snapshot')}>{t.retry}</button> : null}</footer>
  </main>;
}
createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);

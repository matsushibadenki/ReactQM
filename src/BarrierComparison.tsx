import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { conditionLabel, type Settings } from './CalculationControls';
import { errorMessage, messages, type Locale } from './i18n';

type Barrier = {
  id: string; sourceId: string; kind: 'transition' | 'ci_neb' | 'neb';
  status: 'confirmed' | 'unconfirmed' | 'endpoint_peak'; settings: Settings;
  engineVersion: string; forward: number; reverse: number; reactionEnergy: number; peakImage: number; profile: number[];
};
type Comparison = { rows: Barrier[]; comparable: boolean | null };

export function BarrierComparison({ ids, locale, onClear }: { ids: string[]; locale: Locale; onClear: () => void }) {
  const t = messages[locale];
  const [result, setResult] = useState<Comparison | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function load() {
    setPending(true); setError(null); setResult(null);
    try { setResult(await invoke<Comparison>('compare_barriers', { ids })); }
    catch (cause) { setError(String(cause)); }
    finally { setPending(false); }
  }

  return <section className="barrier-comparison" aria-label={t.barriers}>
    <button type="button" disabled={pending} onClick={() => void load()}>{t.barriers} · {ids.length}/2</button>
    <button type="button" onClick={onClear}>{t.clearBarrierSelection}</button>
    {error ? <p role="alert">{errorMessage(error, locale)}</p> : null}
    {result ? <>
      {result.comparable !== null && <p role="status">{result.comparable ? t.barrierComparable : t.barrierIncomparable}</p>}
      <div className="barrier-cards">{result.rows.map(row => <article key={row.id}>
        <strong>{row.kind === 'transition' ? t.transitionJob : row.kind === 'ci_neb' ? 'CI-NEB' : 'NEB'}</strong>
        <small>{row.status === 'confirmed' ? t.transitionConfirmed : row.status === 'endpoint_peak' ? t.barrierEndpointPeak : t.transitionUnconfirmed}</small>
        <small>{conditionLabel(row.settings)} · PySCF {row.engineVersion}</small>
        <dl>
          <div><dt>{t.barrierForward}</dt><dd>{row.forward.toFixed(2)}</dd></div>
          <div><dt>{t.barrierReverse}</dt><dd>{row.reverse.toFixed(2)}</dd></div>
          <div><dt>{t.barrierReaction}</dt><dd>{row.reactionEnergy.toFixed(2)}</dd></div>
        </dl>
        <small>{t.barrierElectronic} (kJ/mol)</small>
        <details><summary>{t.provenance}</summary><small>{t.nebImage} {row.peakImage} · {row.profile.map(value => value.toFixed(2)).join(' / ')} kJ/mol</small><code>{row.id}</code>{row.sourceId !== row.id && <code>CI-NEB {row.sourceId}</code>}</details>
      </article>)}</div>
      <small>{t.barrierThermalUnavailable}</small>
    </> : null}
  </section>;
}

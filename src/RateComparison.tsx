import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { conditionLabel, type Settings } from './CalculationControls';
import { messages, errorMessage, type Locale } from './i18n';

type Estimate = { barrierKjMol: number; log10PerSecond: number | null };
type Pair = { forward: Estimate | null; reverse: Estimate | null };
type Conditions = {
  id: string; source: string; settings: Settings; engineVersion: string; temperatureKelvin: number;
  standardState: 'atmosphere' | 'molar'; solvent: { model: 'c_pcm' | 'ief_pcm'; medium: 'water' | 'acetonitrile' } | null;
  quasiRrho: boolean; gas: Pair; solution: Pair | null;
};
type Comparison = { rows: Conditions[]; comparable: boolean | null };

function rateText(log: number) {
  const exponent = Math.floor(log);
  return `${(10 ** (log - exponent)).toPrecision(3)}e${exponent}`;
}

export function RateComparison({ ids, locale, onClear }: { ids: string[]; locale: Locale; onClear: () => void }) {
  const t = messages[locale];
  const [result, setResult] = useState<Comparison | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function load() {
    setPending(true);
    setResult(null);
    setError(null);
    try { setResult(await invoke<Comparison>('compare_rates', { ids })); }
    catch (cause) { setError(String(cause)); }
    finally { setPending(false); }
  }
  const cell = (estimate: Estimate | null) => estimate
    ? <><strong>{estimate.log10PerSecond === null ? '—' : rateText(estimate.log10PerSecond)}</strong><small>ΔG‡ {estimate.barrierKjMol.toFixed(2)} kJ/mol</small></>
    : <span>—</span>;

  return <section className="barrier-comparison" aria-label={t.compareRates}>
    <button type="button" disabled={pending} onClick={() => void load()}>{t.compareRates} · {ids.length}/2</button>
    <button type="button" onClick={onClear}>{t.clearBarrierSelection}</button>
    {error ? <p role="alert">{errorMessage(error, locale)}</p> : null}
    {result ? <>
      {result.comparable !== null && <p role="status">{result.comparable ? t.rateComparable : t.rateIncomparable}</p>}
      <div className="barrier-cards">{result.rows.map(row => <article key={row.id}>
        <strong>{row.temperatureKelvin} K · {row.standardState === 'molar' ? '1 M' : '1 atm'}</strong>
        <small>{conditionLabel(row.settings)} · PySCF {row.engineVersion} · {row.quasiRrho ? 'Grimme quasi-RRHO' : 'RRHO'}</small>
        <table className="rate-table">
          <thead><tr><th scope="col">k (s⁻¹)</th><th scope="col">{t.rateForward}</th><th scope="col">{t.rateReverse}</th></tr></thead>
          <tbody>
            <tr><th scope="row">{t.solventGas}</th><td>{cell(row.gas.forward)}</td><td>{cell(row.gas.reverse)}</td></tr>
            {row.solution && row.solvent ? <tr>
              <th scope="row">{row.solvent.medium === 'water' ? t.solventWater : t.solventAcetonitrile}<small>{row.solvent.model === 'c_pcm' ? 'C-PCM' : 'IEF-PCM'}</small></th>
              <td>{cell(row.solution.forward)}</td><td>{cell(row.solution.reverse)}</td>
            </tr> : null}
          </tbody>
        </table>
        <details><summary>{t.provenance}</summary><code>{row.id}</code><code>{row.source}</code></details>
      </article>)}</div>
      <details><summary>{t.rateAssumptions}</summary><code>k = (k_BT/h) exp(−ΔG‡/RT) · κ = 1</code><p>{t.rateModelHelp}</p><p>{t.rateUnavailable}</p></details>
    </> : null}
  </section>;
}

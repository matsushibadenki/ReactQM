import { useState } from 'react';
import { conditionLabel, type Settings } from './CalculationControls';
import { messages, type Locale } from './i18n';

export type RefinementMethod = 'mp2' | 'ccsdt';
export type RefinementReport = {
  method: RefinementMethod;
  baseline: { source_job: string; path_job: string; settings: Settings; engine_version: string; energies_hartree: [number, number, number] };
  energies_hartree: [number, number, number];
  reference_hartree: [number, number, number];
  triples_hartree: [number, number, number];
  orbital_count: number;
};

export const refinementName = (method: RefinementMethod) => method === 'mp2' ? 'MP2' : 'CCSD(T)';

export function RefinementQueue({ locale, disabled, queue }: { locale: Locale; disabled: boolean; queue: (method: RefinementMethod, basis: string) => void }) {
  const t = messages[locale];
  const [method, setMethod] = useState<RefinementMethod>('mp2');
  const [basis, setBasis] = useState('6-31g');
  return <form className="thermal-queue" onSubmit={event => { event.preventDefault(); queue(method, basis); }}>
    <label>{t.refinementTitle} <select value={method} onChange={event => setMethod(event.target.value as RefinementMethod)}>
      <option value="mp2">MP2</option><option value="ccsdt">CCSD(T)</option>
    </select></label>
    <label>{t.basis} <select value={basis} onChange={event => setBasis(event.target.value)}>
      <option value="sto-3g">STO-3G</option><option value="6-31g">6-31G</option><option value="def2-svp">def2-SVP</option>
    </select></label>
    <button type="submit" disabled={disabled}>{t.queueRefinement}</button>
    <button type="button" className="info" aria-label={t.refinementHelp}>ⓘ<span role="tooltip">{t.refinementHelp}</span></button>
  </form>;
}

const hartreeToKjMol = 2625.499639;

export function RefinementComparison({ report, basis, locale }: { report: RefinementReport; basis: string; locale: Locale }) {
  const t = messages[locale];
  const barrier = (energies: RefinementReport['energies_hartree'], i: number) => (energies[2] - energies[i]) * hartreeToKjMol;
  const row = (label: string, value: (index: number) => number) => <tr><th scope="row">{label}</th><td>{value(0).toFixed(2)}</td><td>{value(1).toFixed(2)}</td></tr>;
  return <section className="thermal-results">
    <strong>{t.refinementTitle}</strong>
    <table>
      <thead><tr><th scope="col">{t.barrierElectronic} (kJ/mol)</th><th scope="col">{t.rateForward}</th><th scope="col">{t.rateReverse}</th></tr></thead>
      <tbody>
        {row(conditionLabel(report.baseline.settings), i => barrier(report.baseline.energies_hartree, i))}
        {row(`${refinementName(report.method)} / ${basis.toUpperCase()}`, i => barrier(report.energies_hartree, i))}
        {row(t.refinementDelta, i => barrier(report.energies_hartree, i) - barrier(report.baseline.energies_hartree, i))}
      </tbody>
    </table>
    <details><summary>{t.provenance}</summary>
      <p>{t.refinementHelp}</p>
      <p>PySCF {report.baseline.engine_version}</p>
      <code>TS {report.baseline.source_job}</code><code>CI-NEB {report.baseline.path_job}</code>
      {(['reactant', 'product', 'transitionJob'] as const).map((key, index) => <p key={key}>{t[key]} · {report.energies_hartree[index].toFixed(10)} Eh</p>)}
    </details>
  </section>;
}

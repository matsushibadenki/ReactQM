import { useState } from 'react';
import { messages, type Locale } from './i18n';

type StandardState = 'atmosphere' | 'molar';
type Solvent = { model: 'c_pcm' | 'ief_pcm'; medium: 'water' | 'acetonitrile' };
type ThermalConditions = { temperatureKelvin: number; standardState: StandardState; solvent: Solvent | null; quasiRrho: boolean };

type Species = {
  electronic_hartree: number;
  frequencies_cm1: number[];
  max_force_ev_per_angstrom: number;
  solvated_electronic_hartree?: number;
  corrections: { zpe_hartree: number; gibbs_hartree: number } | null;
};

export type ThermalReport = {
  source_job: string;
  temperature_kelvin: number;
  standard_state?: StandardState;
  standard_shift_hartree?: number;
  solvent?: Solvent;
  quasi_rrho?: boolean;
  reactant: Species;
  product: Species;
  saddle: Species;
};

export function ThermalQueue({ locale, disabled, onQueue }: { locale: Locale; disabled: boolean; onQueue: (conditions: ThermalConditions) => void }) {
  const t = messages[locale];
  const [temperature, setTemperature] = useState('298.15');
  const [standardState, setStandardState] = useState<StandardState>('atmosphere');
  const [quasiRrho, setQuasiRrho] = useState(false);
  const entropy = { en: 'Vibrational entropy', ja: '振動エントロピー', 'zh-CN': '振动熵' }[locale];
  const [model, setModel] = useState<Solvent['model'] | ''>('');
  const [medium, setMedium] = useState<Solvent['medium']>('water');
  return <form className="thermal-queue" onSubmit={event => { event.preventDefault(); onQueue({ temperatureKelvin: Number(temperature), standardState, quasiRrho, solvent: model ? { model, medium } : null }); }}>
    <label>{t.thermalTemperature} (K) <input type="number" min="1" max="2000" step="any" required value={temperature} onChange={event => setTemperature(event.target.value)} /></label>
    <label>{t.solventStandardState} <select value={standardState} onChange={event => setStandardState(event.target.value as StandardState)}>
      <option value="atmosphere">1 atm</option><option value="molar">1 M</option>
    </select></label>
    <label>{t.solventModel} <select value={model} onChange={event => setModel(event.target.value as typeof model)}>
      <option value="">{t.solventGas}</option><option value="c_pcm">C-PCM</option><option value="ief_pcm">IEF-PCM</option>
    </select></label>
    {model ? <label>{t.solventMedium} <select value={medium} onChange={event => setMedium(event.target.value as typeof medium)}>
      <option value="water">{t.solventWater}</option><option value="acetonitrile">{t.solventAcetonitrile}</option>
    </select></label> : null}
    <label>{entropy} <select value={quasiRrho ? 'quasi' : 'rrho'} onChange={event => setQuasiRrho(event.target.value === 'quasi')}>
      <option value="rrho">RRHO</option><option value="quasi">Grimme quasi-RRHO · 100 cm⁻¹</option>
    </select></label>
    <button type="submit" disabled={disabled}>{t.queueThermal}</button>
  </form>;
}

const hartreeToKjMol = 2625.499639;
function barrier(saddle: Species, endpoint: Species, kind: 'zpe' | 'gibbs' | 'solvent') {
  const a = saddle.corrections;
  const b = endpoint.corrections;
  if (!a || !b) return '—';
  const delta = kind === 'zpe'
    ? (saddle.electronic_hartree - endpoint.electronic_hartree) + (a.zpe_hartree - b.zpe_hartree)
    : a.gibbs_hartree - b.gibbs_hartree;
  if (kind === 'solvent') {
    if (saddle.solvated_electronic_hartree === undefined || endpoint.solvated_electronic_hartree === undefined) return '—';
    const change = (saddle.solvated_electronic_hartree - saddle.electronic_hartree)
      - (endpoint.solvated_electronic_hartree - endpoint.electronic_hartree);
    return ((delta + change) * hartreeToKjMol).toFixed(2);
  }
  return (delta * hartreeToKjMol).toFixed(2);
}

export function Thermochemistry({ report, locale }: { report: ThermalReport; locale: Locale }) {
  const t = messages[locale];
  return <section className="thermal-results">
    <strong>{t.thermalTitle} · {report.temperature_kelvin} K</strong>
    <p>{report.quasi_rrho ? 'Grimme quasi-RRHO · 100 cm⁻¹' : 'RRHO'} · {report.standard_state === 'molar' ? '1 M' : '1 atm'}
      {report.solvent ? ` · ${report.solvent.model === 'c_pcm' ? 'C-PCM' : 'IEF-PCM'} / ${report.solvent.medium === 'water' ? t.solventWater : t.solventAcetonitrile}` : ''}
    </p>
    <table>
      <thead><tr><th scope="col">kJ/mol</th><th scope="col">{t.barrierForward}</th><th scope="col">{t.barrierReverse}</th></tr></thead>
      <tbody>
        <tr><th scope="row">{t.thermalZpe}</th><td>{barrier(report.saddle, report.reactant, 'zpe')}</td><td>{barrier(report.saddle, report.product, 'zpe')}</td></tr>
        <tr><th scope="row">{t.thermalGibbs} · {t.solventGas}</th><td>{barrier(report.saddle, report.reactant, 'gibbs')}</td><td>{barrier(report.saddle, report.product, 'gibbs')}</td></tr>
        {report.solvent ? <tr><th scope="row">{t.thermalGibbs} · {t.solventSolution}</th><td>{barrier(report.saddle, report.reactant, 'solvent')}</td><td>{barrier(report.saddle, report.product, 'solvent')}</td></tr> : null}
      </tbody>
    </table>
    {(!report.reactant.corrections || !report.product.corrections || !report.saddle.corrections) && <p role="status">{t.thermalInvalidModes}</p>}
    {report.solvent ? <small>{t.solventApproximation}</small> : null}
    {report.standard_shift_hartree !== undefined ? <small className="thermal-standard-shift">
      {t.solventStandardShift} {(report.standard_shift_hartree * hartreeToKjMol).toFixed(3)} kJ/mol
    </small> : null}
    <details><summary>{t.frequencies}</summary>
      {(['reactant', 'product', 'saddle'] as const).map(key => <div key={key}>
        <strong>{key === 'saddle' ? t.transitionJob : t[key]}</strong>
        <p>{report[key].frequencies_cm1.map(value => value.toFixed(1)).join(' / ')} cm⁻¹</p>
        <p>{t.maximumForce} {report[key].max_force_ev_per_angstrom.toPrecision(3)} eV/Å</p>
      </div>)}
      <code>{report.source_job}</code>
    </details>
  </section>;
}

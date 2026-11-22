import { useState } from 'react';
import { messages, type Locale } from './i18n';

type Term = { job_id: string; coefficient: number };
type SavedJob = { id: string; name: string; formula: string; createdAt: number };
type Selection = { reactants: Term[]; products: Term[]; saddle: string | null; temperatureKelvin: number; quasiRrho: boolean };
type Energy = { reaction: number; forward?: number; reverse?: number };
type Summary = Term & { name: string };

export type StoichiometryReport = {
  reactants: Summary[]; products: Summary[]; saddle: string | null;
  temperature_kelvin: number; quasi_rrho: boolean; per_species_shift_kj_mol: number;
  atmosphere_kj_mol: Energy; molar_kj_mol: Energy;
};

export const stoichiometryLabels = {
  en: { title: 'Multispecies thermochemistry', coefficient: 'Coefficient', add: 'Add species', remove: 'Remove species', optional: 'None', reaction: 'Reaction ΔG', forward: 'Forward ΔG‡', reverse: 'Reverse ΔG‡', help: 'Choose completed gas-phase jobs with optimized geometries and matching electronic settings. The saddle is optional. Verify reaction connectivity separately.' },
  ja: { title: '多化学種の熱化学', coefficient: '係数', add: '化学種を追加', remove: '化学種を削除', optional: '指定なし', reaction: '反応 ΔG', forward: '正方向 ΔG‡', reverse: '逆方向 ΔG‡', help: '同じ電子状態計算条件の、最適化済み気相構造を選びます。鞍点は任意です。経路の接続は別途確認してください。' },
  'zh-CN': { title: '多物种热化学', coefficient: '系数', add: '添加物种', remove: '移除物种', optional: '未指定', reaction: '反应 ΔG', forward: '正向 ΔG‡', reverse: '逆向 ΔG‡', help: '请选择电子结构计算条件相同的已优化气相结构。鞍点可选，反应连通性需单独验证。' },
};

const newTerm = () => ({ job_id: '', coefficient: 1 });

export function StoichiometryQueue({ locale, jobs, saddles, disabled, queue }: { locale: Locale; jobs: SavedJob[]; saddles: SavedJob[]; disabled: boolean; queue: (selection: Selection) => void }) {
  const t = messages[locale];
  const labels = stoichiometryLabels[locale];
  const [reactants, setReactants] = useState([newTerm()]);
  const [products, setProducts] = useState([newTerm()]);
  const [saddle, setSaddle] = useState('');
  const options = (items: SavedJob[]) => items.map(job => <option key={job.id} value={job.id}>
    {job.name || job.formula} · {new Date(job.createdAt).toLocaleString(locale)} · {job.id.slice(0, 8)}
  </option>);
  const side = (label: string, terms: Term[], update: (terms: Term[]) => void) => <fieldset>
    <legend>{label}</legend>
    {terms.map((term, index) => <div className="stoichiometry-term" key={index}>
      <select required aria-label={`${label} ${index + 1}`} value={term.job_id} onChange={event => update(terms.map((item, i) => i === index ? { ...item, job_id: event.target.value } : item))}>
        <option value="" disabled>—</option>{options(jobs)}
      </select>
      <input required aria-label={`${labels.coefficient} ${index + 1}`} type="number" min="1" max="16" value={term.coefficient || ''}
        onChange={event => update(terms.map((item, i) => i === index ? { ...item, coefficient: Number(event.target.value) } : item))} />
      <button type="button" disabled={terms.length === 1} aria-label={labels.remove} onClick={() => update(terms.filter((_, i) => i !== index))}>−</button>
    </div>)}
    <button type="button" disabled={terms.length >= 8} aria-label={labels.add} title={labels.add} onClick={() => update([...terms, newTerm()])}>＋</button>
  </fieldset>;

  return <details>
    <summary>{labels.title} <span className="info" tabIndex={0} aria-label={labels.help}>ⓘ<span role="tooltip">{labels.help}</span></span></summary>
    <form onSubmit={event => {
      event.preventDefault();
      const data = new FormData(event.currentTarget);
      queue({ reactants, products, saddle: saddle || null, temperatureKelvin: Number(data.get('temperature')), quasiRrho: data.get('model') === 'quasi' });
    }}>
      <fieldset disabled={disabled} className="stoichiometry-form">
        {side(t.reactant, reactants, setReactants)}
        {side(t.product, products, setProducts)}
        <label>{t.transitionJob}<select value={saddle} onChange={event => setSaddle(event.target.value)}><option value="">{labels.optional}</option>{options(saddles)}</select></label>
        <div className="dynamics-fields">
          <label>{t.thermalTemperature} (K)<input required name="temperature" type="number" min="1" max="2000" step="any" defaultValue="298.15" /></label>
          <select name="model" aria-label={t.thermalTitle}><option value="rrho">RRHO</option><option value="quasi">Grimme quasi-RRHO</option></select>
          <button type="submit" disabled={!jobs.length}>{t.enqueue}</button>
        </div>
      </fieldset>
    </form>
  </details>;
}

export function StoichiometryResult({ report, locale }: { report: StoichiometryReport; locale: Locale }) {
  const t = stoichiometryLabels[locale];
  const number = new Intl.NumberFormat(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const side = (terms: Summary[]) => terms.map(term => `${term.coefficient} × ${term.name || term.job_id.slice(0, 8)}`).join(' + ');
  return <section className="thermal-results">
    <strong>{t.title} · {report.temperature_kelvin} K · {report.quasi_rrho ? 'Grimme quasi-RRHO' : 'RRHO'}</strong>
    <p>{side(report.reactants)} → {side(report.products)}</p>
    <table>
      <thead><tr><th scope="col">kJ/mol</th><th scope="col">1 atm</th><th scope="col">1 M</th></tr></thead>
      <tbody>{(['reaction', 'forward', 'reverse'] as const).map(key => <tr key={key}>
        <th scope="row">{t[key]}</th>
        {[report.atmosphere_kj_mol[key], report.molar_kj_mol[key]].map((value, index) => <td key={index}>{value == null ? '—' : number.format(value)}</td>)}
      </tr>)}</tbody>
    </table>
  </section>;
}

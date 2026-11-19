import { messages, type Locale } from './i18n';

export type Solvent = { model: 'c_pcm' | 'ief_pcm'; medium: 'water' | 'acetonitrile' };
export type SolutionReport = {
  solvent: Solvent;
  ase_version: string;
  steps: number;
  max_force_ev_per_angstrom: number;
  frequencies_cm1: number[];
  minimum_confirmed: boolean;
};

const labels = {
  en: { title: 'Solution optimization and frequencies', minimum: 'Minimum confirmed', uncertain: 'Minimum unconfirmed', help: 'Optimizes an RHF structure in PCM and computes harmonic frequencies in the same solvent. Restore the completed job from history to view the optimized structure.' },
  ja: { title: '溶液中の最適化・振動数', minimum: '極小を確認', uncertain: '極小未確認', help: 'PCM溶媒中でRHF構造を最適化し、同じ溶媒条件で調和振動数を求めます。完了したジョブを履歴から復元すると、最適化構造を表示できます。' },
  'zh-CN': { title: '溶液结构优化与频率', minimum: '已确认极小值', uncertain: '尚未确认极小值', help: '在 PCM 溶剂中优化 RHF 结构，并在相同溶剂条件下计算谐振频率。从历史记录恢复已完成的任务可查看优化结构。' },
};

export function solventLabel(solvent: Solvent, locale: Locale) {
  const t = messages[locale];
  return `${solvent.model === 'c_pcm' ? 'C-PCM' : 'IEF-PCM'} · ${solvent.medium === 'water' ? t.solventWater : t.solventAcetonitrile}`;
}

export function SolutionQueue({ locale, disabled, queue, title }: { locale: Locale; disabled: boolean; queue: (solvent: Solvent) => void; title?: string }) {
  const t = messages[locale];
  return <details>
    <summary title={title ? undefined : labels[locale].help}>{title ?? labels[locale].title}</summary>
    <form onSubmit={event => {
      event.preventDefault();
      const values = new FormData(event.currentTarget);
      queue({ model: values.get('model') as Solvent['model'], medium: values.get('medium') as Solvent['medium'] });
    }}>
      <fieldset disabled={disabled} className="dynamics-fields">
        <label>{t.solventModel}<select name="model"><option value="c_pcm">C-PCM</option><option value="ief_pcm">IEF-PCM</option></select></label>
        <label>{t.solventMedium}<select name="medium"><option value="water">{t.solventWater}</option><option value="acetonitrile">{t.solventAcetonitrile}</option></select></label>
        <button type="submit">{t.enqueue}</button>
      </fieldset>
    </form>
  </details>;
}

export function SolutionDetails({ locale, report }: { locale: Locale; report: SolutionReport }) {
  const t = messages[locale];
  const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 2 });
  return <section>
    <strong>{solventLabel(report.solvent, locale)}</strong>
    <p>{report.minimum_confirmed ? labels[locale].minimum : labels[locale].uncertain}</p>
    <small>ASE BFGS {report.ase_version} · {t.optimizationStep} {report.steps} · {t.maximumForce} {report.max_force_ev_per_angstrom.toPrecision(3)} eV/Å</small>
    <dl><dt>{t.frequencies} (cm⁻¹)</dt>
      <dd>{report.frequencies_cm1.map(value => number.format(value)).join(' · ')}</dd>
    </dl>
  </section>;
}

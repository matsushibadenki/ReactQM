import { messages, type Locale } from './i18n';

export type XtbPolicy = {
  source_job: string; source_engine: string; max_force_ev_per_angstrom: number;
  source_energy_hartree: number; budget: number; decision: 'dft' | 'out_of_scope' | 'below_threshold' | 'no_budget';
};
export type XtbReport = { policy_v1: XtbPolicy; dft_max_force_ev_per_angstrom: number | null };

const labels = {
  en: { title: 'Select DFT from GFN2-xTB', budget: 'DFT job budget', low: 'Stopped: force at or below 0.30 eV/Å', scope: 'Stopped: outside DFT policy scope', none: 'Stopped: no DFT budget', help: 'DFT requires a neutral gas-phase singlet of 2–16 H–Ne atoms and an xTB maximum force above 0.30 eV/Å. Force screens for nonstationary geometries.' },
  ja: { title: 'GFN2-xTBからDFTを選択', budget: 'DFT計算の予算', low: '停止・最大力が0.30 eV/Å以下', scope: '停止・DFT判定の適用範囲外', none: '停止・DFT予算なし', help: 'DFTへ進む条件は、気相の中性一重項、H〜Neの2〜16原子、xTB最大力0.30 eV/Å超です。力を使って停留点から外れた候補を拾います。' },
  'zh-CN': { title: '从 GFN2-xTB 选择 DFT', budget: 'DFT 任务预算', low: '停止：最大力不超过 0.30 eV/Å', scope: '停止：超出 DFT 策略范围', none: '停止：DFT 预算为零', help: '进入 DFT 需满足气相中性单重态、含 2–16 个 H–Ne 原子且 xTB 最大力超过 0.30 eV/Å。力用于筛选非驻点构型。' },
};

export function xtbDecision(policy: XtbPolicy, locale: Locale) {
  const t = labels[locale];
  return { dft: 'PBE0 / def2-SVP', out_of_scope: t.scope, below_threshold: t.low, no_budget: t.none }[policy.decision];
}

export function XtbQueue({ locale, disabled, submit }: { locale: Locale; disabled: boolean; submit: (budget: number) => void }) {
  const t = labels[locale];
  return <details><summary>{t.title}</summary>
    <form className="dynamics-fields" onSubmit={event => {
      event.preventDefault();
      submit(Number(new FormData(event.currentTarget).get('budget')));
    }}>
      <label>{t.budget}<input name="budget" type="number" min="0" max="1" required defaultValue="1" /></label>
      <button type="submit" disabled={disabled}>{messages[locale].enqueue}</button>
      <button type="button" className="info" aria-label={t.help}>ⓘ<span role="tooltip">{t.help}</span></button>
    </form>
  </details>;
}

export function XtbReason({ policy, locale }: { policy: XtbPolicy; locale: Locale }) {
  const t = messages[locale];
  return <dl>
    <div><dt>{t.maximumForce} · GFN2-xTB</dt><dd>{policy.max_force_ev_per_angstrom.toFixed(3)} eV/Å</dd></div>
    <div><dt>{labels[locale].budget}</dt><dd>{policy.decision === 'dft' ? 1 : 0}/{policy.budget}</dd></div>
    <div><dt>tblite {policy.source_engine}</dt><dd><code>{policy.source_job}</code></dd></div>
  </dl>;
}

export function XtbResult({ report, energy, locale }: { report: XtbReport; energy: number; locale: Locale }) {
  return <section className="result">
    <strong>{xtbDecision(report.policy_v1, locale)}</strong>
    {report.dft_max_force_ev_per_angstrom !== null ? <>
      <div className="energy"><output>{energy.toFixed(10)}</output><span>Eh</span></div>
      <p>{messages[locale].maximumForce} · DFT {report.dft_max_force_ev_per_angstrom.toFixed(3)} eV/Å</p>
    </> : null}
    <XtbReason policy={report.policy_v1} locale={locale} />
  </section>;
}

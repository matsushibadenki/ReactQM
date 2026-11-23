import { messages, type Locale } from './i18n';

export type Policy = {
  dft_job: string; mace_job: string; max_force_difference: number; dft_energy_hartree: number;
  budget: number; orbitals: number; decision: 'mp2' | 'ccsdt' | 'below_threshold' | 'out_of_scope' | 'no_budget';
};
export type EscalationReport = { policy_v1: Policy; rhf_hartree: number | null; triples_hartree: number | null };

const labels = {
  en: { title: 'Select correlated method', budget: 'Budget (cost units)', difference: 'Maximum MACE–DFT force difference', orbitals: 'Basis functions', rule: 'Policy v1', low: 'Stopped below threshold', scope: 'Stopped outside scope', none: 'Stopped at budget limit', help: 'MP2 costs 1 unit. CCSD(T) costs 3 and requires a difference above 2.00 eV/Å and at most 32 basis functions. A difference at or below 0.50 eV/Å stops escalation. This heuristic does not certify accuracy.' },
  ja: { title: '高精度法を選択', budget: '予算（計算単位）', difference: 'MACEとDFTの最大力差', orbitals: '基底関数数', rule: '判定ルール v1', low: '停止・力差が閾値以下', scope: '停止・適用範囲外', none: '停止・予算なし', help: 'MP2は1単位です。CCSD(T)は3単位で、力差2.00 eV/Å超かつ基底関数32以下が条件です。力差0.50 eV/Å以下では停止します。この判定で精度は保証されません。' },
  'zh-CN': { title: '选择高精度方法', budget: '预算（计算单位）', difference: 'MACE 与 DFT 的最大力差', orbitals: '基函数数', rule: '判定规则 v1', low: '停止：力差未超过阈值', scope: '停止：超出适用范围', none: '停止：预算不足', help: 'MP2 需要 1 单位。CCSD(T) 需要 3 单位、力差大于 2.00 eV/Å 且基函数不超过 32。力差不超过 0.50 eV/Å 时停止。该规则无法保证精度。' },
};

export function decisionLabel(policy: Policy, locale: Locale) {
  const t = labels[locale];
  return { mp2: 'MP2', ccsdt: 'CCSD(T)', below_threshold: t.low, out_of_scope: t.scope, no_budget: t.none }[policy.decision];
}

export function EscalationQueue({ locale, disabled, submit }: { locale: Locale; disabled: boolean; submit: (budget: number) => void }) {
  const t = labels[locale];
  return <details>
    <summary>{t.title}</summary>
    <form className="dynamics-fields" onSubmit={event => {
      event.preventDefault();
      submit(Number(new FormData(event.currentTarget).get('budget')));
    }}>
      <label>{t.budget}<input name="budget" type="number" required min="0" max="3" defaultValue="1" /></label>
      <button type="submit" disabled={disabled}>{messages[locale].enqueue}</button>
      <button type="button" className="info" aria-label={t.help}>ⓘ<span role="tooltip">{t.help}</span></button>
    </form>
  </details>;
}

export function EscalationReason({ policy, locale }: { policy: Policy; locale: Locale }) {
  const t = labels[locale];
  return <dl>
    <div><dt>{t.rule}</dt><dd>{decisionLabel(policy, locale)}</dd></div>
    <div><dt>{t.difference}</dt><dd>{policy.max_force_difference.toFixed(3)} eV/Å</dd></div>
    <div><dt>{t.orbitals}</dt><dd>{policy.orbitals}</dd></div>
    <div><dt>{t.budget}</dt><dd>{policy.decision === 'ccsdt' ? 3 : policy.decision === 'mp2' ? 1 : 0}/{policy.budget}</dd></div>
    <div><dt>DFT</dt><dd><code>{policy.dft_job}</code></dd></div>
    <div><dt>MACE</dt><dd><code>{policy.mace_job}</code></dd></div>
  </dl>;
}

export function EscalationResult({ report, energy, locale }: { report: EscalationReport; energy: number; locale: Locale }) {
  return <section className="result">
    <strong>{decisionLabel(report.policy_v1, locale)}</strong>
    {report.rhf_hartree !== null ? <div className="energy"><output>{energy.toFixed(10)}</output><span>Eh</span></div> : null}
    <EscalationReason policy={report.policy_v1} locale={locale} />
  </section>;
}

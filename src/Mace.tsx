import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage, messages, type Locale } from './i18n';

type Diagnostics = {
  schema_version: 1; minimum_ratio: number; closest_pair: [number, number];
  connectivity_ratio: number; connectivity_pair: [number, number];
  disagreement: { checkpoint_sha256: string; max_force_ev_per_angstrom: number; rms_force_ev_per_angstrom: number; atom: number } | null;
};
export type MaceReport = { energy_ev: number; checkpoint_sha256: string; torch_version: string; ase_version: string; diagnostics?: Diagnostics };
const labels = {
  en: { scope: 'Scope', ready: 'Input passed scope checks', force: 'Force by atom', help: 'Isolated, neutral singlets with 2–64 atoms. Trained on neutral closed-shell organic chemistry (SPICE v1). Geometry checks cover close contacts and connectivity.' },
  ja: { scope: '適用範囲', ready: '入力は適用条件を満たしています', force: '原子ごとの力', help: '非周期・中性・一重項の単一分子、2〜64原子。学習領域はSPICE v1の中性閉殻有機化学です。原子の極端な接近と分離を検査します。' },
  'zh-CN': { scope: '适用范围', ready: '输入符合适用条件', force: '逐原子力', help: '仅限非周期、中性、单重态的单个分子，含 2–64 个原子。训练范围为 SPICE v1 的中性闭壳层有机分子。检查原子过近和结构分离。' },
};

const diagnosticLabels = {
  en: { suspect: 'Possible extrapolation', clear: 'No warning indicators triggered', missing: 'Diagnostics unavailable for this result', close: 'Closest distance / summed covalent radii', bridge: 'Distance ratio needed for connectivity', disagreement: 'Small–medium force disagreement', unavailable: 'Medium checkpoint unavailable', help: 'Warnings trigger below 0.80 for the closest ratio, above 1.20 for the connectivity ratio, or above 0.50 eV/Å for maximum force disagreement. Model disagreement is an uncertainty indicator without calibration against prediction errors.' },
  ja: { suspect: '外挿の疑い', clear: '警告条件に該当なし', missing: 'この結果には診断データがありません', close: '最短距離／共有結合半径の和', bridge: '連結に必要な距離比', disagreement: 'small／medium間の力の差', unavailable: 'mediumモデル未設定', help: '最短距離比が0.80未満、連結距離比が1.20超、または力の最大差が0.50 eV/Å超で警告します。モデル間の差は不確かさの指標で、予測誤差に対する校正は行っていません。' },
  'zh-CN': { suspect: '疑似外推', clear: '未触发警告条件', missing: '该结果没有诊断数据', close: '最短距离／共价半径之和', bridge: '连通所需的距离比', disagreement: 'small 与 medium 的力差异', unavailable: '未配置 medium 模型', help: '最近距离比低于 0.80、连通距离比高于 1.20，或最大力差高于 0.50 eV/Å 时发出警告。模型差异是不确定性指标，尚未按预测误差校准。' },
};

function MaceDiagnostics({ value, locale }: { value?: Diagnostics; locale: Locale }) {
  const t = diagnosticLabels[locale];
  if (!value) return <p>{t.missing}</p>;
  const pair = (atoms: [number, number]) => atoms.map(index => index + 1).join('–');
  const close = value.minimum_ratio < 0.80;
  const stretched = value.connectivity_ratio > 1.20;
  const disagreement = value.disagreement;
  const forceWarning = (disagreement?.max_force_ev_per_angstrom ?? 0) > 0.50;
  return <section>
    <strong>{close || stretched || forceWarning ? t.suspect : t.clear}</strong>
    {' '}<button type="button" className="info" aria-label={t.help}>ⓘ<span role="tooltip">{t.help}</span></button>
    <dl>
      <div><dt>{t.close}</dt><dd>{close ? '⚠ ' : ''}{value.minimum_ratio.toFixed(3)} ({pair(value.closest_pair)})</dd></div>
      <div><dt>{t.bridge}</dt><dd>{stretched ? '⚠ ' : ''}{value.connectivity_ratio.toFixed(3)} ({pair(value.connectivity_pair)})</dd></div>
      <div><dt>{t.disagreement}</dt><dd>{disagreement
        ? `${forceWarning ? '⚠ ' : ''}max ${disagreement.max_force_ev_per_angstrom.toFixed(3)} (#${disagreement.atom + 1}) · RMS ${disagreement.rms_force_ev_per_angstrom.toFixed(3)} eV/Å`
        : t.unavailable}</dd></div>
    </dl>
  </section>;
}

export function MaceQueue({ locale, disabled, scopeError, queue }: { locale: Locale; disabled: boolean; scopeError?: string | null; queue: () => void }) {
  const t = messages[locale];
  const copy = labels[locale];
  return <details>
    <summary>MACE-OFF23-small</summary>
    <div>
      <strong>{copy.scope}</strong> <button type="button" className="info" aria-label={copy.help}>ⓘ<span role="tooltip">{copy.help}</span></button>
      <p>H, C, N, O, F, P, S, Cl, Br, I · q = 0 · 2S + 1 = 1</p>
      {!disabled || scopeError ? <p role="status">{scopeError ? errorMessage(scopeError, locale) : copy.ready}</p> : null}
      <button type="button" disabled={disabled || !!scopeError} onClick={queue}>{t.enqueue}</button>
    </div>
  </details>;
}

export function MaceDetails({ report, locale, jobId, atomCount, revision, engineVersion }: { report: MaceReport; locale: Locale; jobId: string; atomCount: number; revision: number; engineVersion: string }) {
  const t = messages[locale];
  const label = labels[locale].force;
  const [index, setIndex] = useState(0);
  const [force, setForce] = useState<{ index: number; vector: [number, number, number] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let current = true;
    void invoke<[number, number, number]>('mace_force', { jobId, expectedRevision: revision, index }).then(vector => {
      if (current) { setError(null); setForce({ index, vector }); }
    }).catch(cause => { if (current) { setForce(null); setError(String(cause)); } });
    return () => { current = false; };
  }, [index, revision, jobId]);

  return <section className="result">
    <strong>MACE-OFF23-small</strong>
    <div className="energy"><output>{report.energy_ev.toFixed(8)}</output><span>eV</span></div>
    <label>{label} <select value={index} onChange={event => { setError(null); setIndex(Number(event.target.value)); }}>
      {Array.from({ length: atomCount }, (_, i) => <option key={i} value={i}>{t.atomLabel} {i + 1}</option>)}
    </select></label>
    {force?.index === index ? <dl aria-label={label}>
      {force.vector.map((value, axis) => <div key={axis}><dt>{'XYZ'[axis]}</dt><dd>{value.toFixed(6)} eV/Å</dd></div>)}
    </dl> : null}
    {error ? <p role="alert">{errorMessage(error, locale)}</p> : null}
    <MaceDiagnostics value={report.diagnostics} locale={locale} />
    <details><summary>{t.provenance}</summary><code>MACE {engineVersion} · PyTorch {report.torch_version} · ASE {report.ase_version}</code><code>SHA-256 {report.checkpoint_sha256}</code>{report.diagnostics?.disagreement ? <code>Medium SHA-256 {report.diagnostics.disagreement.checkpoint_sha256}</code> : null}</details>
  </section>;
}

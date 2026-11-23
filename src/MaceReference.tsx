import { AtomForce } from './Mace';
import { messages, type Locale } from './i18n';

export type ReferenceSelection = { source_job: string; max_disagreement: number; rank: number; budget: number; warning_v1?: { minimum_ratio: number; connectivity_ratio: number } };
export const referenceLabels = {
  en: { warningsOnly: 'Only structures with warnings', minimum: 'Closest distance ratio', connectivity: 'Connectivity ratio', rule: 'Warning rule v1', title: 'MACE reference calculations', select: 'Select MACE candidate', budget: 'Calculation budget', score: 'Maximum model force difference', source: 'Source MACE job', rank: 'Rank / budget', help: 'Select completed MACE jobs. Only H–Ne structures with a medium comparison are eligible. Exact duplicate structures are skipped.' },
  ja: { warningsOnly: '警告のある構造のみ', minimum: '最短距離比', connectivity: '連結距離比', rule: '警告ルール v1', title: 'MACE参照計算', select: 'MACE候補を選択', budget: '計算件数の上限', score: 'モデル間の最大力差', source: '元のMACEジョブ', rank: '順位／予算', help: '完了済みのMACEジョブを選びます。mediumとの比較があるH〜Neの構造が対象です。同じ座標と元素順の構造は重複を除きます。' },
  'zh-CN': { warningsOnly: '仅选择有警告的结构', minimum: '最短距离比', connectivity: '连通距离比', rule: '警告规则 v1', title: 'MACE 参考计算', select: '选择 MACE 候选', budget: '计算数量上限', score: '模型间最大力差', source: '原始 MACE 任务', rank: '排名／预算', help: '选择已完成的 MACE 任务。仅限含 H–Ne 元素且具有 medium 比较结果的结构。会跳过元素顺序和坐标相同的重复结构。' },
};

export function ReferenceReason({ selection, locale }: { selection: ReferenceSelection; locale: Locale }) {
  const t = referenceLabels[locale];
  return <dl>
    <div><dt>{t.score}{selection.warning_v1 ? ' (> 0.50 eV/Å)' : ''}</dt><dd>{selection.max_disagreement.toFixed(3)} eV/Å</dd></div>
    {selection.warning_v1 ? <>
      <div><dt>{t.rule} · {t.minimum} (&lt; 0.80)</dt><dd>{selection.warning_v1.minimum_ratio.toFixed(3)}</dd></div>
      <div><dt>{t.connectivity} (&gt; 1.20)</dt><dd>{selection.warning_v1.connectivity_ratio.toFixed(3)}</dd></div>
    </> : null}
    <div><dt>{t.rank}</dt><dd>{selection.rank}/{selection.budget}</dd></div>
    <div><dt>{t.source}</dt><dd><code>{selection.source_job}</code></dd></div>
  </dl>;
}

export function ReferenceResult({ selection, locale, jobId, atomCount, revision, energy, engineVersion }: {
  selection: ReferenceSelection; locale: Locale; jobId: string; atomCount: number; revision: number; energy: number; engineVersion: string;
}) {
  return <section className="result">
    <strong>{referenceLabels[locale].title} · PBE0 / def2-SVP</strong>
    <div className="energy"><output>{energy.toFixed(8)}</output><span>Eh</span></div>
    <ReferenceReason selection={selection} locale={locale} />
    <AtomForce locale={locale} jobId={jobId} atomCount={atomCount} revision={revision} />
    <details><summary>{messages[locale].provenance}</summary><code>PySCF {engineVersion} · {jobId}</code></details>
  </section>;
}

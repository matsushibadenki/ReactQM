import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage, messages, type Locale } from './i18n';
import { solventLabel, type Solvent } from './Solution';

export type SolutionPathReport = {
  solvent: Solvent;
  baseline: { source_job: string; energies_hartree: [number, number, number] };
  saddle: { frequencies_cm1: number[] };
  endpoint_energies_hartree: [number, number];
  path_verified: boolean;
  lengths: [number, number];
};

export const solutionPathLabels = {
  en: { title: 'Solution transition state and barriers', verified: 'Both IRC endpoints verified', failed: 'Path unverified', gas: 'Gas phase (saved CI-NEB endpoints)', forward: 'Forward', reverse: 'Reverse', frame: 'IRC frame' },
  ja: { title: '溶液中の遷移状態・障壁', verified: 'IRCの両端点を確認', failed: '経路未確認', gas: '気相（保存済みCI-NEB端点）', forward: '正方向', reverse: '逆方向', frame: 'IRCフレーム' },
  'zh-CN': { title: '溶液过渡态与势垒', verified: '已验证 IRC 两端', failed: '路径尚未验证', gas: '气相（已保存的 CI-NEB 端点）', forward: '正向', reverse: '逆向', frame: 'IRC 帧' },
};

export function SolutionBarriers({ report, energy, locale }: { report: SolutionPathReport; energy: number; locale: Locale }) {
  const t = solutionPathLabels[locale];
  const gas = report.baseline.energies_hartree;
  const barrier = (saddle: number, endpoint: number) => ((saddle - endpoint) * 2625.499639).toFixed(2);
  return <section>
    <p>{report.path_verified ? t.verified : t.failed}</p>
    <table>
      <thead><tr><th scope="col">kJ/mol</th><th scope="col">{t.forward}</th><th scope="col">{t.reverse}</th></tr></thead>
      <tbody>
        <tr><th scope="row">{t.gas}</th><td>{barrier(gas[2], gas[0])}</td><td>{barrier(gas[2], gas[1])}</td></tr>
        <tr><th scope="row">{solventLabel(report.solvent, locale)}</th>
          {report.endpoint_energies_hartree.map((endpoint, index) => <td key={index}>{report.path_verified ? barrier(energy, endpoint) : '—'}</td>)}</tr>
      </tbody>
    </table>
    <p>{messages[locale].frequencies} · {report.saddle.frequencies_cm1.map(value => value.toFixed(1)).join(' · ')} cm⁻¹</p>
  </section>;
}

export function SolutionIrc({ report, jobId, revision, locale, redraw }: { report: SolutionPathReport; jobId: string; revision: number; locale: Locale; redraw: () => void }) {
  const t = messages[locale];
  const [branch, setBranch] = useState(0);
  const [frame, setFrame] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const pending = useRef(Promise.resolve());
  const length = report.lengths[branch];
  const index = (branch === 0 ? 0 : report.lengths[0]) + frame;
  useEffect(() => {
    if (!length) return;
    let current = true;
    pending.current = pending.current.then(async () => {
      if (!current) return;
      await invoke('show_frame', { jobId, expectedRevision: revision, index });
      if (current) { setError(null); redraw(); }
    }).catch(cause => { if (current) setError(String(cause)); });
    return () => { current = false; };
  }, [index, jobId, length, redraw, revision]);
  if (!length) return null;
  const label = solutionPathLabels[locale].frame;
  return <div className="dynamics-transport">
    <select aria-label="IRC" value={branch} onChange={event => { setBranch(Number(event.target.value)); setFrame(0); }}>
      <option value={0}>{t.ircForward}</option><option value={1}>{t.ircReverse}</option>
    </select>
    <input type="range" aria-label={label} min={0} max={length - 1} value={frame} onChange={event => setFrame(Number(event.target.value))} />
    <output>{frame + 1}/{length}</output>
    {error ? <small role="alert">{errorMessage(error, locale)}</small> : null}
  </div>;
}

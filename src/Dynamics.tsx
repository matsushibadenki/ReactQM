import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage, messages, type Locale } from './i18n';

export type Ensemble = 'nve' | 'langevin';
export type DynamicsInput = { ensemble: Ensemble; steps: number; timestep_fs: number; temperature_k: number; seed: number };
export type DynamicsReport = { input: DynamicsInput; frames: number; ase_version: string; mean_temperature_k: number; nve_max_energy_deviation_ev: number | null };

const labels = {
  ja: { seed: '乱数シード', title: '分子動力学', steps: 'ステップ数', timestep: '時間刻み (fs)', temperature: '温度 (K)', play: '軌跡を再生', pause: '軌跡を一時停止', frame: 'フレーム', mean: '平均温度', drift: '全エネルギー最大偏差', help: 'NVEでは初期速度の設定に温度を使用します。Langevinでは目標温度にも使用します。完了した軌跡は計算履歴から開けます。' },
  en: { seed: 'Random seed', title: 'Molecular dynamics', steps: 'Steps', timestep: 'Time step (fs)', temperature: 'Temperature (K)', play: 'Play trajectory', pause: 'Pause trajectory', frame: 'Frame', mean: 'Mean temperature', drift: 'Maximum total-energy deviation', help: 'NVE uses temperature to sample initial velocities. Langevin also uses it as the thermostat target. Open completed trajectories from calculation history.' },
  'zh-CN': { seed: '随机种子', title: '分子动力学', steps: '步数', timestep: '时间步长 (fs)', temperature: '温度 (K)', play: '播放轨迹', pause: '暂停轨迹', frame: '帧', mean: '平均温度', drift: '总能量最大偏差', help: 'NVE 使用温度采样初始速度。Langevin 还将其用作恒温目标。可从计算历史打开已完成的轨迹。' },
};
export const ensembleName = (ensemble: Ensemble) => ensemble === 'nve' ? 'NVE' : 'Langevin';

export function DynamicsQueue({ locale, disabled, queue }: { locale: Locale; disabled: boolean; queue: (input: DynamicsInput) => void }) {
  const t = labels[locale];
  const [ensemble, setEnsemble] = useState<Ensemble>('nve');
  return <details><summary>{t.title}</summary>
    <form className="dynamics-form" title={t.help} onSubmit={event => {
      event.preventDefault();
      const values = new FormData(event.currentTarget);
      queue({ ensemble, steps: Number(values.get('steps')), timestep_fs: Number(values.get('timestep')),
        temperature_k: Number(values.get('temperature')), seed: Number(values.get('seed')) });
    }}>
      <fieldset disabled={disabled} className="dynamics-fields">
        <select aria-label={t.title} value={ensemble} onChange={event => setEnsemble(event.target.value as Ensemble)}>
          <option value="nve">NVE</option><option value="langevin">Langevin</option>
        </select>
        <label>{t.steps}<input name="steps" type="number" required min="1" max="500" defaultValue="50" /></label>
        <label>{t.timestep}<input name="timestep" type="number" required min="0.05" max="2" step="any" defaultValue="0.5" /></label>
        <label>{t.temperature}<input name="temperature" type="number" required min="1" max="2000" step="any" defaultValue="300" /></label>
        <label>{t.seed}<input name="seed" type="number" required min="0" max="4294967295" defaultValue="42" /></label>
        <button type="submit">{messages[locale].enqueue}</button>
      </fieldset>
    </form>
  </details>;
}

export function DynamicsPlayer({ locale, report, jobId, revision, redraw }: { locale: Locale; report: DynamicsReport; jobId: string; revision: number; redraw: () => void }) {
  const t = labels[locale];
  const [frame, setFrame] = useState(0);
  const [playing, setPlaying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const last = report.frames - 1;
  const pending = useRef(Promise.resolve());
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    pending.current = pending.current.then(async () => {
      if (!active) return;
      await invoke('show_frame', { jobId, expectedRevision: revision, index: frame });
      if (!active) return;
      redraw();
      if (playing && frame < last) timer = setTimeout(() => setFrame(frame + 1), 85);
      else if (playing) setPlaying(false);
    }).catch(cause => {
      if (!active) return;
      setError(String(cause));
      setPlaying(false);
    });
    return () => { active = false; clearTimeout(timer); };
  }, [frame, jobId, last, playing, redraw, revision]);
  return <div className="dynamics-player">
    <strong>{ensembleName(report.input.ensemble)} · ASE {report.ase_version}</strong>
    <div className="dynamics-transport">
      <button type="button" aria-label={playing ? t.pause : t.play} title={playing ? t.pause : t.play}
        onClick={() => { setError(null); if (!playing && frame === last) setFrame(0); setPlaying(!playing); }}>{playing ? 'Ⅱ' : '▶'}</button>
      <input type="range" aria-label={t.frame} min="0" max={last} value={frame}
        onChange={event => { setPlaying(false); setError(null); setFrame(Number(event.target.value)); }} />
      <output>{frame + 1}/{report.frames} · {(frame * report.input.timestep_fs).toFixed(2)} fs</output>
    </div>
    <small>{t.mean} {report.mean_temperature_k.toFixed(1)} K
      {report.nve_max_energy_deviation_ev !== null ? ` · ${t.drift} ${report.nve_max_energy_deviation_ev.toPrecision(3)} eV` : ''}</small>
    {error ? <p role="alert">{errorMessage(error, locale)}</p> : null}
  </div>;
}

import { xtbDecision } from './XtbFollowup';
import { decisionLabel } from './Escalation';
import { referenceLabels } from './MaceReference';
import { stoichiometryLabels } from './Stoichiometry';
import { solventLabel } from './Solution';
import { ensembleName } from './Dynamics';
import { refinementName } from './Refinement';
import { conditionLabel } from './CalculationControls';
import { messages, type Locale } from './i18n';
import type { QueueProjection } from './useQueue';
const paths = { play: 'm8 4 12 8-12 8Z', pause: 'M8 4v16M16 4v16', stop: 'M5 5h14v14H5Z' };
function Glyph({ name }: { name: keyof typeof paths }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={paths[name]} /></svg>; }
export function QueueToolbar({ locale, state, job, disabled, onAction }: { locale: Locale; state: QueueProjection; job: { id: string; cycle: number; status: string } | null; disabled: boolean; onAction: (name: string, args: Record<string, unknown>) => Promise<boolean> }) {
  const t = messages[locale];
  const { queue } = state;
  async function action(name: string, args: Record<string, unknown>) { await onAction(name, args); await state.refresh(); }
  const cancelling = queue.active?.status === 'cancelling' || (job?.id === queue.active?.id && job?.status === 'cancelling');
  const transitionPhase = [t.transitionJob, t.frequencies, t.ircForward, t.ircReverse][job?.cycle ?? 0] ?? t.transitionJob;
  return <div className="queue-toolbar" role="toolbar" aria-label={t.queue}>
    <button type="button" title={queue.paused ? t.resumeQueue : t.pauseQueue} aria-label={queue.paused ? t.resumeQueue : t.pauseQueue} disabled={disabled || state.loading} onClick={() => void action('set_queue_paused', { paused: !queue.paused })}><Glyph name={queue.paused ? 'play' : 'pause'} /></button>
    <span className="queue-overview">{t.queue} <strong>{queue.waiting}</strong><span>{queue.paused ? t.queuePaused : t.queueEnabled}</span></span>
    <div className="active-job-summary" role="status">{queue.active ? <><strong title={queue.active.settings ? conditionLabel(queue.active.settings) : undefined}>{queue.active.formula}</strong><span title={queue.active.name}>{queue.active.name || t.untitled}</span><small>{cancelling ? t.cancelling : t.running} · {queue.active.xtbFollowup ? xtbDecision(queue.active.xtbFollowup, locale) : queue.active.escalation ? decisionLabel(queue.active.escalation, locale) : queue.active.maceReference ? `${referenceLabels[locale].title} #${queue.active.maceReference.rank}` : queue.active.mace ? 'MACE-OFF23-small' : queue.active.stoichiometry ? `${stoichiometryLabels[locale].title} · ${job?.id === queue.active.id ? job.cycle : 0}` : queue.active.solutionPath ? `${solventLabel(queue.active.solutionPath, locale)} · ${[t.reactant, t.product, t.transitionJob, t.frequencies, t.ircForward, t.ircReverse][job?.id === queue.active.id ? job.cycle : 0] ?? t.transitionJob}` : queue.active.solution ? `${solventLabel(queue.active.solution, locale)} · ${job?.id === queue.active.id && job.cycle === 51 ? t.frequencies : `${t.optimizationStep} ${job?.id === queue.active.id ? job.cycle : 0}`}` : queue.active.dynamics ? `${ensembleName(queue.active.dynamics.ensemble)} · ${t.optimizationStep} ${job?.id === queue.active.id ? job.cycle : 0}/${queue.active.dynamics.steps}` : queue.active.refinement ? `${refinementName(queue.active.refinement)} · ${[t.reactant, t.product, t.transitionJob][job?.cycle ?? 0] ?? ''}` : queue.active.thermochemistry ? `${t.thermalTitle} · ${[t.reactant, t.product, t.transitionJob][job?.cycle ?? 0] ?? t.thermalTitle}` : queue.active.transition ? transitionPhase : queue.active.settings?.method === 'GFN2-xTB' ? 'GFN2-xTB' : `${queue.active.neb != null ? `${queue.active.neb ? 'CI-NEB' : 'NEB'} · ${t.optimizationStep}` : queue.active.optimize ? t.optimizationStep : t.cycle} ${job?.id === queue.active.id ? job.cycle : 0}`}</small></> : null}</div>
    {queue.active ? <button type="button" title={t.cancel} aria-label={`${t.cancel}: ${queue.active.name || queue.active.formula}`} disabled={disabled || cancelling} onClick={() => void action('cancel_job', { id: queue.active!.id })}><Glyph name="stop" /></button> : null}
  </div>;
}

import { messages, type Locale } from './i18n';
import type { QueueProjection } from './useQueue';
const paths = { play: 'm8 4 12 8-12 8Z', pause: 'M8 4v16M16 4v16', stop: 'M5 5h14v14H5Z' };
function Glyph({ name }: { name: keyof typeof paths }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={paths[name]} /></svg>; }
export function QueueToolbar({ locale, state, job, disabled, onAction }: { locale: Locale; state: QueueProjection; job: { id: string; cycle: number; status: string } | null; disabled: boolean; onAction: (name: string, args: Record<string, unknown>) => Promise<boolean> }) {
  const t = messages[locale];
  const { queue } = state;
  async function action(name: string, args: Record<string, unknown>) { await onAction(name, args); await state.refresh(); }
  const cancelling = queue.active?.status === 'cancelling' || (job?.id === queue.active?.id && job?.status === 'cancelling');
  return <div className="queue-toolbar" role="toolbar" aria-label={t.queue}>
    <button type="button" title={queue.paused ? t.resumeQueue : t.pauseQueue} aria-label={queue.paused ? t.resumeQueue : t.pauseQueue} disabled={disabled || state.loading} onClick={() => void action('set_queue_paused', { paused: !queue.paused })}><Glyph name={queue.paused ? 'play' : 'pause'} /></button>
    <span className="queue-overview">{t.queue} <strong>{queue.waiting}</strong><span>{queue.paused ? t.queuePaused : t.queueEnabled}</span></span>
    <div className="active-job-summary" role="status">{queue.active ? <><strong>{queue.active.formula}</strong><span title={queue.active.name}>{queue.active.name || t.untitled}</span><small>{cancelling ? t.cancelling : t.running} · {t.cycle} {job?.id === queue.active.id ? job.cycle : 0}</small></> : null}</div>
    {queue.active ? <button type="button" title={t.cancel} aria-label={`${t.cancel}: ${queue.active.name || queue.active.formula}`} disabled={disabled || cancelling} onClick={() => void action('cancel_job', { id: queue.active!.id })}><Glyph name="stop" /></button> : null}
  </div>;
}

import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messages, errorMessage, type Locale } from './i18n';
import type { QueueProjection } from './useQueue';

type RecordStatus = 'queued' | 'running' | 'cancelling' | 'succeeded' | 'cancelled' | 'failed' | 'timed_out' | 'interrupted' | 'corrupt';
type JobRecord = { sequence: number; id: string; name: string; formula: string; status: RecordStatus; createdAt: number; error: string | null; energyHartree: number | null; retryOf: string | null; legacy: boolean };
type JobPage = { items: JobRecord[]; nextCursor: number | null };
const restoreIcon = 'M4 10a8 8 0 1 1 0 5M4 4v6h6';
const stopIcon = 'M5 5h14v14H5Z';
const retryIcon = 'm8 4 12 8-12 8Z';
function Glyph({ path }: { path: string }) { return <svg aria-hidden="true" viewBox="0 0 24 24"><path d={path} /></svg>; }

export function JobHistory({ locale, busy, queueState, onAction }: { locale: Locale; busy: boolean; queueState: QueueProjection; onAction: (name: string, args: Record<string, unknown>) => Promise<boolean> }) {
  const t = messages[locale];
  const native = isTauri();
  const [page, setPage] = useState<JobPage>({ items: [], nextCursor: null });
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [acting, setActing] = useState(false);
  const { queue, loading: queueLoading, refresh: refreshQueue } = queueState;
  const generation = useRef(0);
  const mounted = useRef(false);
  const refresh = useCallback(async (before: number | null = null) => {
    if (!native) return;
    const request = ++generation.current;
    setLoading(true);
    try {
      const next = await invoke<JobPage>('list_jobs', { before });
      if (!mounted.current || generation.current !== request) return;
      setPage(previous => ({ items: before === null ? next.items : [...previous.items, ...next.items], nextCursor: next.nextCursor }));
      setError(null);
    } catch (cause) { if (mounted.current && generation.current === request) setError(String(cause)); }
    finally { if (mounted.current && generation.current === request) setLoading(false); }
  }, [native]);
  useEffect(() => {
    mounted.current = true;
    let disposed = false;
    let stop: (() => void) | undefined;
    if (native) void listen('job-history-changed', () => { if (!disposed) { void refresh(); } }).then(unlisten => {
      if (disposed) unlisten(); else { stop = unlisten; void refresh(); }
    }).catch(cause => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; mounted.current = false; generation.current++; stop?.(); };
  }, [native, refresh]);
  async function act(command: string, args: Record<string, unknown>) {
    setActing(true);
    try { await onAction(command, args); }
    finally { setActing(false); await Promise.all([refresh(), refreshQueue()]); }
  }
  return <div className="history-panel">
    <div className="heading"><h3>{t.queue} <small>· {queue.waiting}</small></h3>
      <div className="history-tools"><button type="button" className="info" aria-label={t.queueHelp}>ⓘ<span role="tooltip">{t.queueHelp}</span></button></div>
    </div>
    {!queue.items.length ? <p className="history-empty">{queueLoading ? t.connecting : t.queueEmpty}</p> : null}
    <ol className="queue-list" aria-label={t.queue} aria-busy={queueLoading}>
      {queue.items.map(item => <li key={item.id}><div><strong>{item.formula}</strong><span>{item.name || t.untitled}</span><small>{t.queued}</small></div>
        <button type="button" title={t.cancelQueued} aria-label={`${t.cancelQueued}: ${item.name || item.formula}`} disabled={busy || acting} onClick={() => void act('cancel_queued_job', { id: item.id })}><Glyph path={stopIcon} /></button>
      </li>)}
    </ol>
    {queue.nextCursor !== null ? <button type="button" className="history-more" disabled={queueLoading} onClick={() => void refreshQueue(queue.nextCursor)}>{t.moreQueue}</button> : null}
    <div className="heading history-heading"><h3>{t.jobHistory}</h3>
      <div className="history-tools"><button type="button" title={t.refreshHistory} aria-label={t.refreshHistory} disabled={!native || loading} onClick={() => void refresh()}><Glyph path={restoreIcon} /></button>
        <button type="button" className="info" aria-label={t.historyHelp}>ⓘ<span role="tooltip">{t.historyHelp}</span></button></div>
    </div>
    {error || queueState.error ? <div className="error" role="alert">{errorMessage(error ?? queueState.error!, locale)}<details><summary>{t.details}</summary><code>{error ?? queueState.error}</code></details></div> : null}
    {!page.items.length ? <p className="history-empty" role="status">{loading ? t.connecting : t.noHistory}</p> : null}
    <ul className="job-list" aria-label={t.jobHistory} aria-busy={loading}>
      {page.items.map(job => <li key={job.id}>
        <div className="job-record">
          <div className="job-record-title"><strong>{job.formula}</strong><span>{job.name || t.untitled}</span></div>
          <div className="job-record-meta"><time dateTime={new Date(job.createdAt).toISOString()}>{new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }).format(job.createdAt)}</time><span className={'job-status ' + job.status}>{t[job.status]}</span></div>
          {job.energyHartree !== null ? <div className="history-energy">{job.energyHartree.toFixed(10)} <small>Eh</small></div> : null}
          <details><summary>{t.details}</summary><code>{job.id}</code>{job.retryOf ? <p>{t.retryOf}<code>{job.retryOf}</code></p> : null}{job.legacy ? <p>{t.legacyRecord}</p> : null}{job.error ? <p>{errorMessage(job.error, locale)}</p> : null}</details>
        </div>
        <div className="job-record-actions">
          <button type="button" title={t.restoreResult} aria-label={`${t.restoreResult}: ${job.name || job.formula}`} disabled={busy || acting || job.status !== 'succeeded'} onClick={() => void act('restore_job', { id: job.id })}><Glyph path={restoreIcon} /></button>
          <button type="button" title={t.retryCalculation} aria-label={`${t.retryCalculation}: ${job.name || job.formula}`} disabled={busy || acting || job.status === 'running' || job.status === 'cancelling' || job.status === 'queued'} onClick={() => void act('retry_job', { id: job.id })}><Glyph path={retryIcon} /></button>
        </div>
      </li>)}
    </ul>
    {page.nextCursor !== null ? <button type="button" className="history-more" disabled={loading} onClick={() => void refresh(page.nextCursor)}>{t.moreHistory}</button> : null}
  </div>;
}

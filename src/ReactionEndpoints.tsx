import { useEffect, useState } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { errorMessage, messages, type Locale } from './i18n';

type Endpoint = { name: string; formula: string; elements: number[]; charge: number; multiplicity: number };
type Reaction = { revision: number; reactant: Endpoint | null; product: Endpoint | null; mapping: number[] | null; mismatch: string | null };

function Mapping({ reaction, locale, pending, onSave }: { reaction: Reaction; locale: Locale; pending: boolean; onSave: (mapping: number[]) => void }) {
  const t = messages[locale];
  const reactant = reaction.reactant!;
  const product = reaction.product!;
  const [draft, setDraft] = useState(() => {
    if (reaction.mapping) return reaction.mapping;
    const used = new Set<number>();
    return reactant.elements.map(element => {
      const index = product.elements.findIndex((candidate, i) => candidate === element && !used.has(i));
      used.add(index);
      return index + 1;
    });
  });
  const unique = new Set(draft).size === draft.length;
  return <form onSubmit={event => { event.preventDefault(); onSave(draft); }}>
    <div className="reaction-map">
      {draft.map((target, i) => <label key={i}>#{i + 1} → <select aria-label={`${t.reactant} #${i + 1} → ${t.product}`} disabled={pending} value={target}
        onChange={event => setDraft(draft.map((value, index) => index === i ? Number(event.target.value) : value))}>
        {product.elements.map((element, index) => element === reactant.elements[i] ? <option key={index} value={index + 1}>#{index + 1}</option> : null)}
      </select></label>)}
    </div>
    <button type="submit" disabled={pending || !unique}>{t.saveMapping}</button>
    {' '}<small role="status">{!unique ? errorMessage('reaction_mapping', locale) : reaction.mapping?.every((value, i) => value === draft[i]) ? t.mappingSaved : t.mappingDraft}</small>
  </form>;
}

export function ReactionEndpoints({ locale, documentRevision, atomCount, locked }: { locale: Locale; documentRevision: number; atomCount: number; locked: boolean }) {
  const t = messages[locale];
  const [reaction, setReaction] = useState<Reaction | null>(null);
  const [pending, setPending] = useState(false);
  const [notice, setNotice] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const accept = (value: Reaction) => setReaction(current => !current || value.revision >= current.revision ? value : current);

  useEffect(() => {
    if (!isTauri()) return;
    const subscription = listen<Reaction>('reaction-changed', event => accept(event.payload));
    let disposed = false;
    void subscription.then(() => invoke<Reaction>('reaction_snapshot')).then(value => {
      if (!disposed) accept(value);
    }).catch(cause => { if (!disposed) setError(String(cause)); });
    return () => { disposed = true; void subscription.then(unlisten => unlisten()).catch(() => {}); };
  }, []);

  async function run(command: string, args: Record<string, unknown>) {
    if (!reaction) return;
    setPending(true);
    setNotice(false);
    setError(null);
    try { accept(await invoke<Reaction>(command, { ...args, expectedRevision: reaction.revision })); if (command === 'start_neb') setNotice(true); }
    catch (cause) { setError(String(cause)); }
    finally { setPending(false); }
  }

  return <details className="reaction-editor"><summary>{t.reactionEndpoints}</summary>
    <div className="reaction-endpoints">
      {(['reactant', 'product'] as const).map(side => {
        const endpoint = reaction?.[side];
        return <div key={side}>
          <strong>{t[side]}</strong>
          <span>{endpoint ? `${endpoint.formula}${endpoint.name ? ` · ${endpoint.name}` : ''} · q=${endpoint.charge} · 2S+1=${endpoint.multiplicity}` : '—'}</span>
          <button type="button" disabled={!reaction || pending || locked || !atomCount} onClick={() => void run('capture_endpoint', { endpoint: side, documentRevision })}>{t.captureEndpoint}</button>
        </div>;
      })}
    </div>
    {reaction && (reaction.mismatch ? <small role="status">{errorMessage(reaction.mismatch, locale)}</small> :
      <Mapping key={reaction.revision} reaction={reaction} locale={locale} pending={pending || locked}
        onSave={mapping => void run('save_atom_mapping', { mapping })} />)}
    {reaction?.mapping ? <div className="reaction-map">
      <button disabled={pending || locked} onClick={() => void run('start_neb', { climb: false, documentRevision })}>{t.enqueueNeb}</button>
      <button disabled={pending || locked} onClick={() => void run('start_neb', { climb: true, documentRevision })}>{t.enqueueCiNeb}</button>
    </div> : null}
    {notice ? <small role="status">{t.queued}</small> : null}
    {error ? <small role="alert">{errorMessage(error, locale)}</small> : null}
  </details>;
}

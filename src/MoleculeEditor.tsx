import { useState, type FormEvent } from 'react';
import { messages, type Locale } from './i18n';

export type Selection = { index: number; symbol: string; position: number[]; inferredNeighbors: number; bonds: [number, number][] };
export type Edit =
  | { kind: 'add'; symbol: string; position: number[] }
  | { kind: 'move'; atom: number; position: number[] }
  | { kind: 'delete'; atom: number }
  | { kind: 'bond'; first: number; second: number; order: number }
  | { kind: 'drag'; aspect: number; start: number[]; end: number[] };

const paths = {
  add: 'M12 4v16M4 12h16',
  move: 'M12 2v20M2 12h20M9 5l3-3 3 3m-6 14 3 3 3-3M5 9l-3 3 3 3m14-6 3 3-3 3',
  delete: 'M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7',
  bond: 'M5 10h14M5 14h14',
};
type Mode = keyof typeof paths;

export function MoleculeEditor({ locale, disabled, selected, onEdit }: {
  locale: Locale; disabled: boolean; selected: Selection | null; onEdit: (edit: Edit) => void;
}) {
  const t = messages[locale];
  const [mode, setMode] = useState<Mode | null>(null);
  const [first, setFirst] = useState<number | null>(null);
  const labels = { add: t.addAtom, move: t.moveAtom, delete: t.deleteAtom, bond: t.editBond };
  const second = first !== selected?.index ? selected?.index : undefined;
  const currentOrder = selected?.bonds.find(([atom]) => atom === first)?.[1] ?? 0;

  function choose(next: Mode) {
    if (next === 'delete' && selected) {
      onEdit({ kind: 'delete', atom: selected.index });
      return;
    }
    setMode(mode === next ? null : next);
    if (next === 'bond') setFirst(selected?.index ?? null);
  }

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const position = ['x', 'y', 'z'].map(axis => Number(data.get(axis)));
    if (mode === 'add') onEdit({ kind: 'add', symbol: String(data.get('symbol')), position });
    if (mode === 'move' && selected) onEdit({ kind: 'move', atom: selected.index, position });
    if (mode === 'bond' && first && second) onEdit({ kind: 'bond', first, second, order: Number(data.get('order')) });
  }

  const position = mode === 'move' ? selected?.position : (selected?.position ?? [-1, 0, 0]).map((value, i) => value + (i === 0 ? 1 : 0));

  return <div className="molecule-editor">
    <div className="viewport-tools" aria-label={t.moleculeEditor}>
      {(Object.keys(paths) as Mode[]).map(action => <button key={action} type="button" title={labels[action]}
        aria-label={labels[action]} aria-pressed={action === 'delete' ? undefined : mode === action}
        disabled={disabled || (action !== 'add' && !selected && mode !== action)}
        onClick={() => choose(action)}>
        <svg aria-hidden="true" viewBox="0 0 24 24"><path d={paths[action]} /></svg>
      </button>)}
      <button type="button" className="info" aria-label={t.editorHelp}>ⓘ<span role="tooltip">{t.editorHelp}</span></button>
    </div>
    {mode ? <form key={`${mode}-${selected?.index}`} className="editor-fields" onSubmit={submit}>
      {mode === 'add' ? <label>{t.elementSymbol}<input name="symbol" defaultValue="H" maxLength={2} size={2} required disabled={disabled} /></label> : null}
      {(mode === 'add' || mode === 'move') && position ? <>
        {['x', 'y', 'z'].map((axis, i) => <label key={axis}>{axis.toUpperCase()} (Å)
          <input name={axis} type="number" min="-10000" max="10000" step="any" defaultValue={position[i]} required disabled={disabled} />
        </label>)}
        <button type="submit" disabled={disabled}>{t.apply}</button>
      </> : null}
      {mode === 'bond' ? second && first ? <>
        <label>{t.bondOrder} · {first}–{second}
          <select name="order" defaultValue={currentOrder} disabled={disabled}>
            <option value="0">{t.noBond}</option><option value="1">{t.singleBond}</option>
            <option value="2">{t.doubleBond}</option><option value="3">{t.tripleBond}</option>
          </select>
        </label>
        <button type="submit" disabled={disabled}>{t.apply}</button>
      </> : <span role="status">{t.pickBondPartner} · {first}</span> : null}
    </form> : null}
  </div>;
}

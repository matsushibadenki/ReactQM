import { useEffect, useRef, useState, type ReactNode, type DragEvent, type KeyboardEvent } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { messages, type Locale } from './i18n';
import type { DockId, LayoutProjection, PanelId, Zone } from './useLayout';
const glyphs = { float: 'M14 3h7v7m0-7L10 14M10 5H3v16h16v-7', right: 'M3 4h18v16H3ZM14 4v16', bottom: 'M3 4h18v16H3ZM3 14h18', tabs: 'M3 7h18v13H3Zm0 0V3h8v4', split: 'M3 4h18v16H3ZM3 12h18', reset: 'M4 10a8 8 0 1 1 0 5M4 4v6h6' };
function Glyph({ name }: { name: keyof typeof glyphs }) { return <svg viewBox="0 0 24 24" aria-hidden="true"><path d={glyphs[name]} /></svg>; }
export const panelTitle = (id: PanelId, locale: Locale) => id === 'result' ? messages[locale].result : messages[locale].jobHistory;
// Shortcuts are scoped to panel titles; editing controls retain their normal keys.
function panelKey(event: KeyboardEvent, panel: PanelId, state: LayoutProjection) {
  if (!event.altKey || event.ctrlKey || event.metaKey || !isTauri()) return false;
  const zones: Record<string, Zone> = { ArrowRight: 'right', ArrowDown: 'bottom', ArrowUp: 'floating' };
  if (!event.shiftKey && zones[event.key]) {
    event.preventDefault();
    if (!event.repeat && !state.pending) void state.update({ kind: 'move', panel, zone: zones[event.key] });
    return true;
  }
  if (event.shiftKey && ['ArrowLeft', 'ArrowRight'].includes(event.key)) {
    event.preventDefault();
    const zone = state.layout.panels.find(p => p.id === panel)?.zone;
    if (!zone || zone === 'floating' || state.pending || event.repeat) return true;
    const items = state.layout.panels.filter(p => p.zone === zone);
    const index = items.findIndex(p => p.id === panel);
    const target = index + (event.key === 'ArrowLeft' ? -1 : 1);
    if (target >= 0 && target < items.length) {
      const before = event.key === 'ArrowLeft' ? items[target].id : items[target + 1]?.id;
      void state.update({ kind: 'move', panel, zone, before });
    }
    return true;
  }
  return false;
}
function focusPanelElement(target: HTMLElement) {
  target.focus();
  const chromeBottom = document.querySelector('.app-chrome')?.getBoundingClientRect().bottom ?? 0;
  const top = target.getBoundingClientRect().top;
  if (top < chromeBottom + 8) window.scrollBy({ top: top - chromeBottom - 8, behavior: 'instant' });
}
function usePanelFocus(state: LayoutProjection, floating?: PanelId) {
  const consumed = useRef<string | null>(null);
  useEffect(() => {
    const request = state.focusRequest;
    if (!request || state.pending || state.layout.revision < request.revision) return;
    const key = `${request.panel}:${request.revision}`;
    if (consumed.current === key) return;
    const id = floating ? `heading-${floating}` : state.layout.panels.find(p => p.id === request.panel)?.zone === 'floating' ? `show-${request.panel}` : `tab-${request.panel}`;
    if (floating && request.panel !== floating) return;
    const target = document.getElementById(id);
    if (target) { focusPanelElement(target); consumed.current = key; }
  }, [state.focusRequest, state.layout, state.pending, floating]);
}
function PanelButtons({ panel, zone, state, locale }: { panel: PanelId; zone: Zone; state: LayoutProjection; locale: Locale }) {
  const t = messages[locale];
  return <div className="panel-actions">
    {(['right', 'bottom', 'floating'] as const).filter(target => target !== zone).map(target => <button key={target} type="button" disabled={!isTauri() || state.pending} title={target === 'right' ? t.dockRight : target === 'bottom' ? t.dockBottom : t.floatPanel} aria-label={`${target === 'right' ? t.dockRight : target === 'bottom' ? t.dockBottom : t.floatPanel}: ${panelTitle(panel, locale)}`} onClick={() => void state.update({ kind: 'move', panel, zone: target })}><Glyph name={target === 'floating' ? 'float' : target} /></button>)}
  </div>;
}
export function FloatingPanel({ panel, state, locale, children }: { panel: PanelId; state: LayoutProjection; locale: Locale; children: ReactNode }) {
  usePanelFocus(state, panel);
  const heading = useRef<HTMLHeadingElement>(null);
  const floating = state.layout.panels.find(p => p.id === panel)?.zone === 'floating';
  useEffect(() => { if (floating && heading.current) focusPanelElement(heading.current); }, [floating]);
  // A dock action removes this projection; the native window is destroyed by Rust.
  if (!floating) return <p role="status">{messages[locale].docking}</p>;
  return <section className="dock-panel floating-panel"><div className="panel-heading"><h2 ref={heading} id={`heading-${panel}`} tabIndex={0} title={messages[locale].panelKeyboard} onKeyDown={event => panelKey(event, panel, state)}>{panelTitle(panel, locale)}</h2><PanelButtons panel={panel} zone="floating" state={state} locale={locale} /></div>{children}</section>;
}
export function LayoutTools({ state, locale }: { state: LayoutProjection; locale: Locale }) {
  const t = messages[locale];
  return <div className="layout-tools">
    {state.layout.panels.filter(p => p.zone === 'floating').map(p => <button key={p.id} id={`show-${p.id}`} type="button" disabled={state.pending} onClick={() => void state.update({ kind: 'activate', panel: p.id })} title={t.showPanel}><Glyph name="float" />{panelTitle(p.id, locale)}</button>)}
    <button type="button" disabled={!isTauri() || state.pending} title={t.resetLayout} aria-label={t.resetLayout} onClick={() => void state.update({ kind: 'reset' })}><Glyph name="reset" /></button>
    <button type="button" className="info" aria-label={`${t.layoutHelp} ${t.panelKeyboard}`}>ⓘ<span role="tooltip">{t.layoutHelp} {t.panelKeyboard}</span></button>
  </div>;
}
export function DockWorkspace({ state, locale, molecule, panels }: { state: LayoutProjection; locale: Locale; molecule: ReactNode; panels: Record<PanelId, ReactNode> }) {
  const t = messages[locale];
  usePanelFocus(state);
  const [dragging, setDragging] = useState<PanelId | null>(null);
  const [over, setOver] = useState<DockId | null>(null);
  const [width, setWidth] = useState<number | null>(null);
  const grid = useRef<HTMLDivElement>(null);
  const resizing = useRef(false);
  const { layout } = state;
  function drag(event: DragEvent, panel: PanelId) {
    if (!isTauri() || state.pending) { event.preventDefault(); return; }
    event.dataTransfer.setData('text/reactqm-panel', panel); event.dataTransfer.effectAllowed = 'move'; setDragging(panel);
  }
  function drop(event: DragEvent, zone: DockId, before?: PanelId) {
    event.preventDefault(); event.stopPropagation(); setOver(null); setDragging(null);
    const panel = event.dataTransfer.getData('text/reactqm-panel');
    if (panel === 'result' || panel === 'history') void state.update({ kind: 'move', panel, zone, before });
  }
  function area(dock: DockId) {
    const items = layout.panels.filter(p => p.zone === dock);
    const mode = dock === 'right' ? layout.rightMode : layout.bottomMode;
    const selected = dock === 'right' ? layout.rightActive : layout.bottomActive;
    const active = items.find(p => p.id === selected)?.id ?? items[0]?.id;
    return <div className={`dock-area ${dock}-dock ${over === dock ? 'drop-over' : ''} ${!items.length ? 'empty-dock' : ''}`} data-dock={dock}
      onDragOver={event => { if (dragging) { event.preventDefault(); setOver(dock); } }} onDragLeave={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setOver(null); }} onDrop={event => drop(event, dock)}>
      {!items.length ? <span className="drop-caption">{dock === 'right' ? t.dockRight : t.dockBottom}</span> : <>
        <div className="dock-strip"><div role={mode === 'tabs' ? 'tablist' : undefined} aria-label={dock === 'right' ? t.dockRight : t.dockBottom} className="panel-tabs">
          {items.map(p => <button key={p.id} type="button" draggable={!state.pending && isTauri()} onDragStart={event => drag(event, p.id)} onDragEnd={() => { setDragging(null); setOver(null); }} onDrop={event => drop(event, dock, p.id)}
            tabIndex={mode === 'tabs' && active !== p.id ? -1 : 0} onKeyDown={event => {
              if (panelKey(event, p.id, state)) return;
              if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || state.pending) return;
              if (mode !== 'tabs' || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
              event.preventDefault();
              const index = items.findIndex(item => item.id === p.id);
              const next = items[event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : (index + (event.key === 'ArrowRight' ? 1 : items.length - 1)) % items.length].id;
              document.getElementById(`tab-${next}`)?.focus(); void state.update({ kind: 'activate', panel: next });
            }} role={mode === 'tabs' ? 'tab' : undefined} aria-selected={mode === 'tabs' ? active === p.id : undefined} aria-controls={mode === 'tabs' ? `panel-${p.id}` : undefined} id={`tab-${p.id}`} disabled={state.pending} title={`${t.movePanel}. ${t.panelKeyboard}`} onClick={() => void state.update({ kind: 'activate', panel: p.id })}>{panelTitle(p.id, locale)}</button>)}
        </div><button type="button" disabled={!isTauri() || state.pending} aria-label={mode === 'tabs' ? t.splitPanels : t.tabPanels} title={mode === 'tabs' ? t.splitPanels : t.tabPanels} onClick={() => void state.update({ kind: 'mode', dock, mode: mode === 'tabs' ? 'split' : 'tabs' })}><Glyph name={mode === 'tabs' ? 'split' : 'tabs'} /></button></div>
        {items.filter(p => mode === 'split' || p.id === active).map(p => <section key={p.id} id={`panel-${p.id}`} className="dock-panel" tabIndex={-1} role={mode === 'tabs' ? 'tabpanel' : undefined} aria-labelledby={`tab-${p.id}`}>
          <div className="panel-heading"><h2 tabIndex={0} title={t.panelKeyboard} onKeyDown={event => panelKey(event, p.id, state)} draggable={isTauri() && !state.pending} onDragStart={event => drag(event, p.id)} onDragEnd={() => { setDragging(null); setOver(null); }}>{panelTitle(p.id, locale)}</h2><PanelButtons panel={p.id} zone={dock} state={state} locale={locale} /></div>{panels[p.id]}</section>)}
      </>}
    </div>;
  }
  const hasRight = layout.panels.some(p => p.zone === 'right');
  return <div ref={grid} className={`dock-workspace ${dragging ? 'panel-dragging' : ''} ${hasRight ? 'has-right' : ''}`} style={{ '--dock-width': `${width ?? layout.rightWidth}%` } as React.CSSProperties}>
    <div className="molecule-column">{molecule}</div>
    {hasRight ? <div className="dock-divider" role="separator" aria-label={t.resizePanels} aria-orientation="vertical" aria-valuemin={25} aria-valuemax={60} aria-valuenow={width ?? layout.rightWidth} tabIndex={isTauri() ? 0 : -1}
      onKeyDown={event => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); void state.update({ kind: 'resize', width: Math.max(25, Math.min(60, layout.rightWidth + (event.key === 'ArrowLeft' ? 2 : -2))) }); } }}
      onPointerDown={event => { if (!isTauri() || state.pending) return; resizing.current = true; event.currentTarget.setPointerCapture(event.pointerId); }}
      onPointerMove={event => { if (resizing.current && grid.current) { const rect = grid.current.getBoundingClientRect(); setWidth(Math.max(25, Math.min(60, Math.round((rect.right - event.clientX) / rect.width * 100)))); } }}
      onPointerUp={event => { if (!resizing.current) return; resizing.current = false; event.currentTarget.releasePointerCapture(event.pointerId); if (width !== null) void state.update({ kind: 'resize', width }).finally(() => setWidth(null)); }} onPointerCancel={() => { resizing.current = false; setWidth(null); }} /> : null}
    {area('right')}{area('bottom')}
  </div>;
}

import { StrictMode, useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { messages, type Locale } from './i18n';
import './style.css';

type Snapshot = { locale: Locale; atomCount: number; rendererReady: boolean };
const native = isTauri();

function App() {
  // A read-only projection; the authoritative document and preferences live in Rust.
  const [snapshot, setSnapshot] = useState<Snapshot>({ locale: 'ja', atomCount: 0, rendererReady: false });
  const [connected, setConnected] = useState(false);
  const [failed, setFailed] = useState(false);
  const t = messages[snapshot.locale];

  async function refresh() {
    try {
      setSnapshot(await invoke<Snapshot>('workspace_snapshot'));
      setConnected(true);
      setFailed(false);
    } catch { setFailed(true); }
  }

  useEffect(() => {
    if (!native) return;
    let disposed = false;
    let stop: (() => void) | undefined;
    void listen<Snapshot>('workspace-changed', event => {
      if (!disposed) setSnapshot(event.payload);
    }).then(unlisten => {
      if (disposed) { unlisten(); return; }
      stop = unlisten;
      void refresh();
    }).catch(() => { if (!disposed) setFailed(true); });
    return () => { disposed = true; stop?.(); };
  }, []);

  useEffect(() => { document.documentElement.lang = snapshot.locale; }, [snapshot.locale]);

  async function changeLocale(locale: Locale) {
    if (!native) { setSnapshot(current => ({ ...current, locale })); return; }
    try {
      setSnapshot(await invoke<Snapshot>('set_locale', { locale }));
      setFailed(false);
    } catch { setFailed(true); }
  }

  return <main>
    <header>
      <strong>ReactQM <small>0.1</small></strong>
      <select aria-label={t.language} value={snapshot.locale} onChange={event => void changeLocale(event.target.value as Locale)}>
        <option value="ja">日本語</option><option value="en">English</option><option value="zh-CN">简体中文</option>
      </select>
    </header>
    <section aria-labelledby="foundation-title">
      <div className="heading"><h1 id="foundation-title">{t.foundation}</h1><button className="info" aria-label={t.info}>ⓘ<span role="tooltip">{t.info}</span></button></div>
      <dl>
        <div><dt>{t.document}</dt><dd>{t.untitled}</dd></div>
        <div><dt>{t.atoms}</dt><dd>{snapshot.atomCount}</dd></div>
        <div><dt>{t.renderer}</dt><dd>wgpu · {snapshot.rendererReady ? t.ready : t.pending}</dd></div>
      </dl>
      <footer role="status">{failed ? t.error : native ? (connected ? t.native : t.connecting) : t.preview}
        {failed ? <button onClick={() => void refresh()}>{t.retry}</button> : null}
      </footer>
    </section>
  </main>;
}

createRoot(document.getElementById('root')!).render(<StrictMode><App /></StrictMode>);

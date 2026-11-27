import { useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { errorMessage, type Locale } from './i18n';

const labels = {
  en: { title: 'Research', model: 'Model ID', question: 'Research question', send: 'Send', busy: 'Waiting for response', local: 'Local server', include: 'Include the current result summary', help: 'Send the question to the selected provider. The optional summary contains formula, charge, spin, calculation settings, and energy.' },
  ja: { title: '研究', model: 'モデルID', question: '研究の質問', send: '送信', busy: '応答待ち', local: 'ローカルサーバー', include: '現在の計算結果の概要を添付', help: '選択した接続先に質問を送信します。概要を添付すると分子式、電荷、スピン、計算条件、エネルギーも送信します。' },
  'zh-CN': { title: '研究', model: '模型 ID', question: '研究问题', send: '发送', busy: '等待响应', local: '本地服务器', include: '附加当前结果摘要', help: '向所选服务发送问题。附加摘要时，也会发送分子式、电荷、自旋、计算条件和能量。' },
};

export function Research({ locale, enabled, hasResult }: { locale: Locale; enabled: boolean; hasResult: boolean }) {
  const t = labels[locale];
  const inFlight = useRef(false);
  const [pending, setPending] = useState(false);
  const [answer, setAnswer] = useState('');
  const [error, setError] = useState<string | null>(null);

  return <details>
    <summary>{t.title}</summary>
    <form onSubmit={async event => {
      event.preventDefault();
      if (!enabled || inFlight.current) return;
      const data = new FormData(event.currentTarget);
      inFlight.current = true;
      setPending(true);
      setError(null);
      setAnswer('');
      try {
        setAnswer(await invoke<string>('ask_research', { query: {
          provider: data.get('provider'), model: data.get('model'), prompt: data.get('prompt'),
          includeResult: hasResult && data.has('include'),
        } }));
      } catch (cause) {
        setError(String(cause));
      } finally {
        inFlight.current = false;
        setPending(false);
      }
    }}>
      <fieldset disabled={!enabled || pending} className="dynamics-fields">
        <select name="provider" aria-label={t.title}>
          <option value="gemini">Google Gemini API</option>
          <option value="openai">OpenAI API</option>
          <option value="local">{t.local}</option>
        </select>
        <label>{t.model}<input name="model" required maxLength={128} /></label>
        <label className="research-question">{t.question}<textarea name="prompt" required rows={4} maxLength={12000} /></label>
        <label><span><input type="checkbox" name="include" disabled={!hasResult} /> {t.include}</span></label>
        <button type="submit">{t.send}</button>
        <button type="button" className="info" aria-label={t.help}>ⓘ<span role="tooltip">{t.help}</span></button>
      </fieldset>
    </form>
    {pending ? <p role="status">{t.busy}</p> : null}
    {error ? <p role="alert">{errorMessage(error, locale)}</p> : null}
    {answer ? <div className="research-answer">{answer}</div> : null}
  </details>;
}

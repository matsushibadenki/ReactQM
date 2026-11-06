export type Locale = 'en' | 'ja' | 'zh-CN';
const en = {
  foundation: 'Development foundation',
  document: 'Document',
  untitled: 'Untitled',
  atoms: 'Atoms',
  renderer: 'Renderer',
  pending: 'Not initialized',
  ready: 'Ready',
  connecting: 'Connecting…',
  native: 'Rust connected',
  preview: 'Browser preview',
  language: 'Language',
  info: 'Molecular editing and calculations are planned. This screen checks the application foundation.',
  error: 'Connection failed',
  retry: 'Retry',
};
export const messages: Record<Locale, typeof en> = {
  en,
  ja: {
    foundation: '開発基盤', document: 'ドキュメント', untitled: '名称未設定', atoms: '原子数',
    renderer: 'レンダラー', pending: '未初期化', ready: '準備完了', connecting: '接続中…', native: 'Rust接続済み', preview: 'ブラウザプレビュー',
    language: '言語', info: '分子編集と計算は今後実装します。この画面ではアプリの基盤を確認できます。',
    error: '接続に失敗しました', retry: '再試行',
  },
  'zh-CN': {
    foundation: '开发基础', document: '文档', untitled: '未命名', atoms: '原子数',
    renderer: '渲染器', pending: '未初始化', ready: '已就绪', connecting: '连接中…', native: 'Rust已连接', preview: '浏览器预览',
    language: '语言', info: '分子编辑和计算功能将在后续实现。此界面用于检查应用基础。',
    error: '连接失败', retry: '重试',
  },
};

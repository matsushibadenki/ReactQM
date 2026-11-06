# ReactQM

量子化学による反応シミュレーションアプリの開発基盤です。Tauri 2 / Rustを中核に、React UI、Rust / wgpu描画、独立Python Workerを組み合わせます。

現在は基盤と診断機能のみです。分子編集、ドッキング / フローティング、ネイティブ3D描画、GUIからの計算実行はまだありません。

- [ロードマップ](ROADMAP.md)
- [アーキテクチャと境界](docs/ARCHITECTURE.md)

## 必要環境

macOS / Apple Siliconで準備。Node.js 22.12以上（またはVite 7対応版）、npm、Rust 1.90以上、Xcode、Python 3.12、uvが必要です。今回のホストには既存のツールチェーンがあり、グローバルツールの置換は行っていません。

## セットアップ

プロジェクトのルートで実行します。

```sh
npm ci
uv sync --project worker --locked --python python3.12
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
```

Python依存は `worker/.venv` にインストールします。別プロジェクトのvenvを有効化していても、`--active` は付けずReactQM用の環境を使用してください。

この作業環境ではキャッシュをプロジェクト内に分離しました。再利用する場合:

```sh
export CARGO_HOME="$PWD/.cache/cargo"
export UV_CACHE_DIR="$PWD/.cache/uv"
export npm_config_cache="$PWD/.cache/npm"
```

## 起動

```sh
npm run tauri dev
```

Web UIだけ確認する場合:

```sh
npm run dev
```

ブラウザは `http://127.0.0.1:1420/`。ブラウザでは「ブラウザプレビュー」、Tauriでは接続成功後に「Rust接続済み」と表示します。言語メニューは日本語・English・简体中文を切り替えます。

English: Run `npm run tauri dev` for the native shell, or `npm run dev` for the browser preview. Molecular editing, native rendering and integrated computation are planned; the current screen checks the development foundation.

简体中文：运行 `npm run tauri dev` 启动桌面基础界面，或运行 `npm run dev` 打开浏览器预览。分子编辑、原生渲染和集成计算将在后续阶段实现。

## 検証

```sh
npm run build
npm run check:rust
npm run test:rust
npm run check:gpu
npm run check:worker
npm run test:worker
npm run smoke:chemistry
```

`check:gpu` はwgpu adapter / deviceの初期化のみを確認し、描画の完成を意味しません。`smoke:chemistry` はH₂の実際のRHF/STO-3Gエネルギーと勾配を計算し、ASE / RDKitの読み込み、SQLite / HDF5の一時ファイル往復を確認します。サンプルデータを偽の反応結果として表示することはありません。

## 構成

```text
src/                  React UI・日英中の辞書
src-tauri/src/        Rust Document / State / Renderer・Tauri commands
src-tauri/examples/   GPU device probe
worker/               専用venv・科学計算依存・診断CLI
docs/                 アーキテクチャ
ROADMAP.md            フェーズ・完了条件・進捗
```

## 導入時の検証実績

2026-11-06、Apple M4 / arm64、Node 26.12.1、Rust 1.99.1、Python 3.12.16で確認。

| 項目 | 結果 |
|---|---|
| Web型検査・本番ビルド | 成功 |
| Rust check / unit test | 成功（1 test） |
| macOS開発用.appのビルド | 成功（署名・公証なし） |
| Tauri実ウインドウ | `tauri://localhost`で起動、Rust接続済み、Rust経由の英語切替を確認 |
| wgpu adapter / device | Apple M4 / Metalで成功 |
| Worker import / version診断 | 全依存の読み込み成功 |
| H₂ RHF/STO-3G | 収束、-1.1167593073964255 hartree |
| 勾配・ASE・RDKit・SQLite・HDF5 | スモークテスト成功 |
| ブラウザUI | Chrome、幅320 / 375 / 1100px、3言語切替とtooltipを確認、横はみ出しなし |
| ブラウザのエラー | ページ識別・空白画面・Vite overlay・consoleを確認、エラーなし |

導入版: Tauri Rust 2.12.5 / JS・CLI 2.12.4、React 19.3.1、Vite 7.3.7、TypeScript 5.9.3、wgpu 27.0.1、PySCF 2.15.0、ASE 3.29.0、RDKit 2026.9.1、h5py 3.16.0。正確な依存ツリーはlockfileを参照。

注意点:

- 導入されたPySCFではOpenMPが利用できず、スレッド数指定が効かない警告が出ます。CPU並列化性能は未検証です。
- Rustの推移依存 `block 0.1.6` に将来のRust互換性警告があります。現在のビルドとテストは成功しています。
- 実行サンドボックス内ではMetal adapterが見つかりませんでした。GPUプローブは通常のホスト権限で成功しています。
- Browser skill/runtimeが利用できなかったため、画面検証は一時フォルダに導入したPlaywrightと既存Chromeを使用しました。Playwrightは製品依存に含めていません。
- CUDA、Windows / Linux、署名・公証・別マシン配布は未検証です。ロードマップでは ⭕️ [Pending] としています。

今回生成した開発用アプリは `src-tauri/target/debug/bundle/macos/ReactQM.app` です。Workerはまだ同梱・接続していません。再生成する場合:

```sh
npm run tauri -- build --debug --bundles app --no-sign --config '{"bundle":{"active":true}}'
```

アイコンの元データは `public/reactqm.svg`。再生成は `npm run tauri -- icon public/reactqm.svg --output src-tauri/icons`。

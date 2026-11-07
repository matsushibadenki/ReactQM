# ReactQM

Tauri 2 / Rustを中核に、React UI、Rust / wgpu描画、独立Python Workerを組み合わせる量子化学アプリです。

現在は **XYZ入力 → wgpuの分子3D表示 → RHF/STO-3Gの実計算 → 結果表示 → 保存／再読込** がmacOSで動作します。計算履歴はSQLiteへ自動記録し、再起動後も検証して復元できます。永続待機キューで複数の計算を順に実行できます。結果・履歴パネルはドック／独立表示と配置保存に対応しました。構造編集と独立窓の位置・サイズ復元は次の段階です。

![ReactQMのmacOS実画面：H₂の3D表示とRHF/STO-3Gの収束結果](docs/images/reactqm-macos-h2.jpg)

macOS実画面（2026-11-06）。wgpu / MetalによるH₂の球棒表示と、PySCF RHF/STO-3Gの収束結果です。

English: Native macOS app showing H₂ in 3D and a converged RHF/STO-3G calculation.

简体中文：macOS原生应用，展示H₂的3D球棍模型及已收敛的RHF/STO-3G计算结果。

- [ロードマップ](ROADMAP.md)
- [アーキテクチャ・科学計算の境界](docs/ARCHITECTURE.md)

## 必要環境とセットアップ

macOS / Apple Siliconで検証。Node.js 22.12以上（またはVite 7対応版）、npm、Rust 1.90以上、Xcode、Python 3.12、uvを使用します。

プロジェクト直下で実行します。既存のプロジェクト内キャッシュを利用する場合は、絶対パスを指定してください。

```sh
export CARGO_HOME="$PWD/.cache/cargo"
export UV_CACHE_DIR="$PWD/.cache/uv"
export npm_config_cache="$PWD/.cache/npm"
npm ci
uv sync --project worker --locked --python python3.12
cargo fetch --manifest-path src-tauri/Cargo.toml --locked
npm run tauri dev
```

Python依存は `worker/.venv` に分離します。別の仮想環境が有効でも `uv --active` は付けないでください。

UIのみ確認する場合は `npm run dev` を実行し、`http://127.0.0.1:1420/` を開きます。ブラウザでは計算・取り込み・保存を無効にし、模擬結果は表示しません。

## 最初の計算

1. ツールバーの **H₂** または **H₂O** を押すか、「XYZを取り込む」を展開してXYZを入力し「適用」を押します。ファイルアイコンからXYZファイルを選ぶこともできます。
2. 必要に応じて、取り込み前に電荷・スピン（2S）を設定します。
3. 上部の「待機列に追加」を押します。「待機列を再開」で投入順に実行します。実行中の分子名・SCF反復数・取消操作も上部に表示します。
4. 収束した場合に全エネルギー（hartree / Eh）とエンジン版・計算時間を表示します。計算記録からjob IDと入力SHA-256を確認できます。
5. 保存アイコンで現在の分子と結果を保存します。再読込アイコンで保存時点に戻せます。再起動後も同じ保存先から再読込できます。

分子の取り込みと再読込はUndo／Redoに対応します。異なる分子へ切り替えると古い計算結果は消え、Undoでは元の構造と対応する結果を一緒に復元します。履歴は最大32段で、アプリ終了時に破棄します。

English: Use **H₂** or import a basic XYZ file, then run the RHF/STO-3G calculation. Save stores the molecule and result; Reload restores the saved project. Undo/Redo restores matching molecule/result pairs. The browser preview cannot compute.

简体中文：点击 **H₂** 或导入基本XYZ文件，然后运行RHF/STO-3G计算。保存会保留分子及结果，重新加载可恢复已保存的项目。撤销／重做会同时恢复匹配的结构与结果。浏览器预览不能执行计算。

## 計算履歴と再起動後の復元

計算カードの「計算履歴」に、実行した計算が自動で残ります。プロジェクトの保存操作をしていなくても、アプリ再起動後に履歴から分子と結果を復元できます。

- 履歴の左の矢印アイコン：入力・結果JSON・HDF5のSHA-256とサイズを照合して復元。破損・欠落した成果物は復元せず、「破損」と表示します。
- 右の再生アイコン：保存した分子と計算条件を、新しいUUIDで待機列に追加。元の試行との関係も記録します。表示中の分子は置き換えません。
- 起動時、完了記録のない実行中ジョブは「中断」にします。取り消し要求が記録されているジョブは「取消済み」にします。正常終了後の完了記録がある場合は、成果物を検証して結果を回収します。
- 履歴は20件ずつ表示します。詳細からUUID、元の試行、移行記録の検証基準を確認できます。

以前のJSON形式の履歴は、SQLite作成時に取り込みます。旧形式の成功結果は初回登録時点を整合性の基準とし、詳細に移行履歴であることを表示します。完了記録がすでにある成果物は、その既存のチェックサムを維持して再索引します。不正な入力やUUIDでは登録しません。

現在の再計算は最初からの実行です。SCFチェックポイントからの継続とHDF5配列の再読込は次の段階です。復元で行うHDF5の検査はバイト整合性の検査であり、量子化学の物理的精度の検証ではありません。

English: Calculations are recorded automatically. Restore verifies input/result/HDF5 checksums before restoring the molecule and scalar result. Recalculate starts a new attempt with the saved input and settings. Interrupted jobs restart from the beginning; checkpoint continuation is still planned.

简体中文：计算会自动记录。恢复时验证输入、结果和HDF5校验和，再恢复分子与标量结果。重新计算会按保存的输入和条件创建新任务。中断的计算从头执行；检查点续算尚未实现。

## 永続待機キューと固定ツールバー

文書名・言語切替・読み込み・Undo／Redo・保存・待機列への追加を上部に固定しました。実行状態・待機件数・一時停止／再開・実行中ジョブの取消も、スクロール位置にかかわらず操作できます。分子表示や履歴はその下をスクロールします。操作エラーも上部に表示し、閉じるボタンで消せます。

- 追加時点の入力・計算条件をSQLiteと入力ファイルに固定し、投入順（FIFO）に1件ずつ実行します。待機は最大100件、一覧は20件ずつ表示します。
- 実行中でも別の分子を読み込み、次の計算を追加できます。編集中の文書が変わった場合や再起動前に投入したジョブの結果は、履歴から復元します。同一セッションで入力を変更していない場合のみ現在の結果へ反映します。
- 一時停止は次のジョブの開始を止めます。実行中の計算は継続します。停止する場合は上部の取消ボタンを使います。
- 待機行の停止アイコンで、そのジョブだけを取り消せます。取り消した待機ジョブはWorkerを起動しません。すでに実行が始まっていた場合は状態を更新し、上部から実行中の計算を取り消します。
- アプリ再起動後も待機ジョブのUUID・入力・順序を保持し、一時停止で復元します。「待機列を再開」で続けます。通常終了では新規開始を止め、実行中ジョブを取り消して子プロセスの回収を待ち、後続の待機列を残します。
- Workerの計算失敗や取消後も、実行可であれば次へ進みます。成果物の記録・保存に失敗した場合は待機列を一時停止します。

English: Add immutable inputs to the FIFO queue and resume from the pinned toolbar. One Worker runs at a time. Pausing does not stop the active job; waiting jobs can be cancelled individually. Restart preserves pending UUIDs and order, with the queue paused until resumed. Input editing stays available while jobs run.

简体中文：将固定输入加入FIFO队列，并从顶部固定工具栏继续执行。每次只运行一个Worker。暂停不会停止当前计算，可单独取消等待任务。重启后保留等待任务的UUID和顺序，点击继续后再执行。计算过程中仍可编辑分子输入。

## パネルのドッキングと独立表示

計算結果と履歴・待機列を、右側／下部のドックまたは独立ウインドウに配置できます。各パネルのアイコンから移動先を選べます。パネル名をドックへドラッグすると移動し、タブ上へ重ねると並べ替えます。ドック右端のアイコンでタブ／分割表示を切り替えます。

- 右側の境界をドラッグ、または境界へTabでフォーカスして左右キーで幅を変更します。狭い窓では縦に並べます。
- 独立窓の閉じるボタンは元のドックへ戻します。メイン窓の上部から独立パネルを再表示できます。
- パネルの順序・ドック先・タブ選択・表示形式・幅を自動保存し、再起動時に復元します。独立窓の位置とサイズは未保存で、中央・標準サイズで開きます。
- 全窓が同じRustの文書・計算結果・キューを表示します。独立履歴からの復元もメイン文書とUndoへ反映します。独立表示だけで新しいWorkerは起動しません。
- 上部の配置リセットで標準配置に戻せます。不正な配置ファイルは初期化せず、エラーとリセット操作を表示します。
- 分子のMetal描画はメイン窓に保持します。独立窓への分子描画面の移動は後続項目です。

English: Move result/history panels to the side or lower dock, group them as tabs, resize the side dock, or float them in native windows. Close a floating window to redock it. Layout is restored after restart; all windows share the same Rust document. Floating-window coordinates and molecule detachment are still planned.

简体中文：结果和历史面板可停靠到右侧或底部、合并为标签页、调整侧栏宽度，或在原生独立窗口中显示。关闭独立窗口会返回原停靠区。重启后恢复布局，所有窗口共享同一Rust文档。独立窗口的位置保存和分子视图分离尚未实现。

## 分子の3D表示

macOSではRustが所有するwgpu / Metal描画面をWebViewの予約領域に重ねます。原子配列・メッシュ・描画フレームはJavaScriptへ転送しません。H₂Oサンプルで球棒モデルの奥行きと元素色を確認できます。

- ドラッグ／矢印キー：回転。スクロール／＋・−：拡大縮小。
- 全体表示ボタン／F／Home：カメラを初期位置へ戻す。
- 原子をクリック／Enter／原子選択ボタン：選択。元素・1始まりの番号・Å座標を表示。
- 結合アイコン：距離から推定した結合の表示切替。説明はⓘのhover／focusに表示。

結合は `距離 ≤ 1.2 × 共有結合半径の和` による表示用の推定で、希ガスを除外します。結合次数・価数の検証は行いません。原子の大きさも模式的です。カメラ・選択・結合の表示設定は保存されず、分子変更時はカメラと選択をリセットします。

English: Drag or use arrow keys to rotate, scroll or +/− to zoom, F to fit, click or Enter to select atoms. Bonds are geometry-based visual hints, not validated chemical bond orders. The 3D surface currently requires the macOS Tauri application.

简体中文：拖动或方向键旋转，滚动或+/−缩放，F显示全貌，点击或Enter选择原子。化学键仅为基于几何距离的显示推测，并非经过验证的键级。3D视图目前需要macOS Tauri应用。

## 現在の計算範囲

- 基本XYZの単一フレーム、Å単位、1〜64原子、最大16 KiB。
- 有限座標、原子間距離0.1 Å以上、|座標| ≤ 10,000 Å、|電荷| ≤ 64、電子数とスピンの整合性をRustで検証。
- 文書としては全元素を識別しますが、**実計算はH〜Ne・スピン2S = 0の閉殻系、RHF/STO-3G** に限定しています。
- SCFは最大50反復、収束閾値 `1e-9`、タイムアウト120秒。同時実行は1ジョブです。
- 全エネルギーは核間反発を含み、ZPE・熱補正は含みません。収束は物理的精度や反応予測の正しさを保証する表示ではありません。
- 未収束・異常終了・不正応答・入力との不一致は結果として採用しません。

H₂サンプルは0.74 Å、電荷0、2S = 0。検証値は約 **−1.1167593074 hartree** です。

## 保存先と開発用設定

通常はTauriのアプリデータ領域（このmacOSでは `~/Library/Application Support/dev.reactqm.desktop`）に保存します。

- `jobs.sqlite3`、`jobs.sqlite3-wal`、`jobs.sqlite3-shm`：SQLite計算索引とトランザクション用ファイル。
- `workspace.lock`：作業領域の排他ロック。同じ領域への二重起動時は計算・保存を止めます。
- `layout.json`：schema v1のパネル配置。文書・原子配列・計算成果物を含みません。
- `workspace.json`：保存操作をした時点の分子と結果。現在は単一の保存スロットです。
- `jobs/<UUID>/input.json`：条件・原子配置・単位・入力hash。
- `jobs/<UUID>/worker.log`：Workerの診断ログ。
- `jobs/<UUID>/result.h5`：座標・エネルギー・勾配と単位属性。
- `jobs/<UUID>/result.json`、`status.json`：成功結果・終了状態。
- `jobs/<UUID>/completion.json`：正常終了後にRustが作成する、入力・結果・HDF5のSHA-256とサイズ。SQLiteにも完了記録自体のSHA-256を保持します。

保存は一時ファイルからの原子的置換です。破損したプロジェクトは読み込まず、現在の状態を維持します。成果物は保存ボタンと独立してjobフォルダに残り、SQLite履歴から検証して復元できます。DBの破損や未対応バージョンを検出した際にデータを自動で初期化することはありません。

環境変数 `REACTQM_DATA_DIR` で保存先、`REACTQM_PYTHON` でPython実行ファイルを上書きできます。絶対パスを指定してください。UI検証では `/private/tmp/reactqm-ui-qa` を使用しました。

Workerは現在のチェックアウトのソースとvenvを使います。開発用.appは単体配布できる完成品ではありません。

## 検証コマンド

```sh
npm run build
npm run test:rust
npm run test:worker
npm run test:integration
npm run test:catalog
npm run test:queue
npm run check:worker
npm run check:gpu
npm run test:render
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings
```

`test:rust` のプロセステストには `worker/.venv` が必要です。`test:integration` は実際にRustからWorkerを起動し、H₂計算・HDF5生成・結果の保存／再読込まで確認します。`check:gpu` はadapter / deviceの初期化です。`test:render` は実GPUで通常・部分表示・選択・回転／暗色・縦長の5画像を描き、空白画像・元素色・選択色・部分表示の画素一致を検証します。画像は `/private/tmp/reactqm-render/` に出力し、`REACTQM_RENDER_OUTPUT` で出力先を変更できます。画素の読み戻しは検証用Rustプロセス内に限定しています。

2026-11-06に描画・基本計算を、2026-11-07に履歴・復旧機能を検証しました。環境はApple M4 / arm64、Node 26.12.1、Rust 1.99.1、Python 3.12.16です。

| 確認内容 | 結果 |
|---|---|
| Web型検査・本番ビルド、macOS開発用.app | 成功 |
| Rust unit 31件 + process 2件 | 成功。履歴・復元・破損拒否・取消競合・別プロセス終了・排他・ページ分割・既存検証基準の維持を含む |
| Python 4件 | 成功 |
| Clippy / all-targets / warnings-as-errors | 成功 |
| Rust→Worker→PySCF→HDF5→保存／再読込 | 成功、H₂ −1.1167593073964257 hartree |
| Worker異常系 | 異常終了・不正／過大応答・別job ID・未収束・無応答／大量出力のtimeout・取消と子プロセス回収を確認 |
| Tauri実画面 | H₂実計算、保存／再読込、不正座標拒否、分子変更、Undo／Redo、取消後の操作復帰を確認 |
| Chromeプレビュー | 日英中、320 / 375 / 1100px、明暗テーマ、keyboard tooltip、横はみ出しなし |
| ブラウザエラー | ページ識別・空白画面・Vite overlay・consoleを確認、エラーなし |
| wgpu描画プローブ | Apple M4 / Metal、4×MSAA、5ケース成功。部分表示が元画像の対応範囲と一致 |
| ネイティブ3D表示 | H₂ / H₂O、回転・ズーム・全体表示・原子選択・結合切替、1100×760 → 1025×1242 → 元サイズ、スクロール、H₂実計算・履歴同期を確認 |
| 永続化の実計算プローブ | H₂の実計算を2回実行し、SQLite・再生成後の復元・試行の関連・破損の隔離を確認 |
| 履歴のTauri実画面 | 保存ボタンなしの終了／再起動 → 復元 → 再計算 → 取消 → 破損拒否 → 正常な結果の復元を確認 |
| 永続キュープローブ | 実H₂計算2件、FIFO・再起動後の保持・待機取消でWorker未起動・単一claim・Worker失敗後の継続を確認 |
| キューのTauri実画面 | H₂／H₂O追加・待機取消・再起動・一時停止・実行中終了・後続保持・H₂O再開と結果復元を確認 |
| 固定ツールバー | 主要操作とキュー操作を上部固定。Metalの切り取りとポップアップを含むスクロールを確認 |
| 履歴の画面レイアウト | 画面検査用データで日英中、320 / 375 / 1100px、明暗、ページ分割／再読込、長い名前、操作不可状態を確認。consoleエラーなし |
| パネルの実画面 | 結果／履歴の独立窓、閉じた際の再ドック、下部タブ、起動時の配置復元、独立履歴からの共有文書復元、実H₂Oの結果表示を確認 |
| パネルのブラウザ検証 | 実ポインタドラッグ・キーボード幅調整、タブ／分割／移動、日英中 × 320 / 375 / 1100px × 明暗。横はみ出し・consoleエラーなし |
| HiDPI・画面間移動 | ⭕️ [Pending] 1×環境で実機確認。実Retina・異倍率画面間移動・ネイティブ暗色合成は未確認 |

Browser plugin not available（browser skill/runtimeがこのセッションにない）ため、ブラウザ検証は一時導入のPlaywrightと既存Chromeを使用しています。ネイティブの操作検証はComputer Useで実施しました。Playwrightは製品依存に含めていません。

## 既知の制限

- 描画はmacOSの単一mainウインドウのみ。結果／履歴の独立窓には対応していますが、ブラウザの代替3D描画、複数の分子描画面、device lossからの自動復旧は未実装です。GPU初期化に失敗した場合はUIにエラーと再試行を表示します。
- SCFチェックポイントからの継続、待機順の手動変更／優先度、HDF5配列の再読込、DB破損の自動修復は未実装です。中断ジョブは手動で再計算できます。
- 履歴は正常終了の記録とバイト整合性を検証します。OSによる孤立Workerの回収、電源断・ディスク故障、複数マシン間での同時利用は未検証です。
- PySCFのこのwheelではOpenMPが利用できません。CPU並列化性能は未検証です。
- Rustの推移依存 `block 0.1.6` に将来のRust互換性警告があります。現在のビルド・テスト・Clippyは成功しています。
- 新しくコピーした検証用.appの初回起動でWebViewが白画面となり、再読込が必要なケースがありました。その後の起動と独立窓の復元は確認済みですが、初回起動の安定性は継続確認します。
- Metal検証では通常のホスト権限が必要です。実行サンドボックス内ではadapterが見つかりませんでした。
- Windows / Linuxのネイティブ動作、CUDA、署名・公証・別マシン配布は ⭕️ [Pending] です。

## 開発用.appとソース構成

```sh
npm run tauri -- build --debug --bundles app --no-sign --config '{"bundle":{"active":true}}'
```

生成先は `src-tauri/target/debug/bundle/macos/ReactQM.app` です。アイコンの元データは `public/reactqm.svg`。

```text
src/                       React UI・日英中の辞書
src-tauri/src/document.rs   XYZと科学入力の検証
src-tauri/src/state.rs      共有状態・履歴・revision
src-tauri/src/layout.rs     パネル配置と検証・保存
src-tauri/src/panel_windows.rs 独立窓のライフサイクルと共有配置命令
src/DockWorkspace.tsx       ドック・タブ・分割・ドラッグ・幅調整
src/useLayout.ts            revision付き配置の読取投影
src-tauri/src/jobs.rs       Workerプロトコル・プロセス管理
src-tauri/src/catalog.rs    SQLite履歴・待機列・完了記録・復元・中断検出
src-tauri/src/scheduler.rs  単一Worker・FIFO実行・一時停止・終了時回収
src/QueueToolbar.tsx        固定キュー操作
src/useQueue.ts             共有する読取専用キュー投影
src/JobHistory.tsx          履歴一覧・復元／再計算の命令
src-tauri/src/project.rs    結果検証・原子的な保存／読込
src-tauri/src/renderer.rs   Rust所有のカメラ・選択・表示状態
src-tauri/src/renderer/     wgpuパイプライン・分子メッシュ・macOS表示面
src/MoleculeViewport.tsx    表示領域・入力命令・小さな状態の投影
worker/                    独立Python環境・実計算・診断CLI
ROADMAP.md                 進捗・完了条件・次の優先順位
```

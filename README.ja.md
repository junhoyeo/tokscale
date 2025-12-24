<!-- <CENTERED SECTION FOR GITHUB DISPLAY> -->

<div align="center">

[![Tokscale](./.github/assets/hero.png)](https://tokscale.ai)

</div>

> 複数のプラットフォームでAIコーディングアシスタントのトークン使用量とコストを追跡するための高性能CLIツールと可視化ダッシュボード。

<div align="center">

[![GitHub Release](https://img.shields.io/github/v/release/junhoyeo/tokscale?color=0073FF&labelColor=black&logo=github&style=flat-square&v=2)](https://github.com/junhoyeo/tokscale/releases)
[![GitHub Contributors](https://img.shields.io/github/contributors/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square&v=2)](https://github.com/junhoyeo/tokscale/graphs/contributors)
[![GitHub Forks](https://img.shields.io/github/forks/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square&v=2)](https://github.com/junhoyeo/tokscale/network/members)
[![GitHub Stars](https://img.shields.io/github/stars/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square&v=2)](https://github.com/junhoyeo/tokscale/stargazers)
[![GitHub Issues](https://img.shields.io/github/issues/junhoyeo/tokscale?color=0073FF&labelColor=black&style=flat-square&v=2)](https://github.com/junhoyeo/tokscale/issues)
[![License](https://img.shields.io/badge/license-MIT-white?labelColor=black&style=flat-square&v=2)](https://github.com/junhoyeo/tokscale/blob/master/LICENSE)

[English](README.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [简体中文](README.zh-cn.md)

</div>

<!-- </CENTERED SECTION FOR GITHUB DISPLAY> -->

| Overview | Models |
|:---:|:---:|
| ![TUI Overview](.github/assets/tui-overview.png) | ![TUI Models](.github/assets/tui-models.png) | 

| Daily Summary | Stats |
|:---:|:---:|
| ![TUI Daily Summary](.github/assets/tui-daily.png) | ![TUI Stats](.github/assets/tui-stats.png) | 

| Frontend (3D Contributions Graph) | Wrapped 2025 |
|:---:|:---:|
| <a href="https://tokscale.ai"><img alt="Frontend (3D Contributions Graph" src=".github/assets/frontend-contributions-graph.png" width="700px" /></a> | <a href="#wrapped-2025"><img alt="Wrapped 2025" src=".github/assets/wrapped-2025.png" width="700px" /></a> |

> **[`bunx tokscale submit`](#ソーシャルプラットフォームコマンド)を実行して、使用量データをリーダーボードに送信し、公開プロフィールを作成しましょう！**

## 概要

**Tokscale**は以下のプラットフォームからのトークン消費を監視・分析するのに役立ちます：

| ロゴ | クライアント | データ場所 | サポート |
|------|----------|---------------|-----------|
| <img width="48px" src=".github/assets/client-opencode.png" alt="OpenCode" /> | [OpenCode](https://github.com/sst/opencode) | `~/.local/share/opencode/storage/message/` | ✅ 対応 |
| <img width="48px" src=".github/assets/client-claude.jpg" alt="Claude" /> | [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | `~/.claude/projects/` | ✅ 対応 |
| <img width="48px" src=".github/assets/client-openai.jpg" alt="Codex" /> | [Codex CLI](https://github.com/openai/codex) | `~/.codex/sessions/` | ✅ 対応 |
| <img width="48px" src=".github/assets/client-gemini.png" alt="Gemini" /> | [Gemini CLI](https://github.com/google-gemini/gemini-cli) | `~/.gemini/tmp/*/chats/` | ✅ 対応 |
| <img width="48px" src=".github/assets/client-cursor.jpg" alt="Cursor" /> | [Cursor IDE](https://cursor.com/) | `~/.config/tokscale/cursor-cache/`経由でAPI同期 | ✅ 対応 |

[🚅 LiteLLMの価格データ](https://github.com/BerriAI/litellm)を使用してリアルタイム価格計算を提供し、階層型価格モデルとキャッシュトークン割引をサポートしています。

### なぜ「Tokscale」？

このプロジェクトは**[カルダシェフ・スケール(Kardashev Scale)](https://ja.wikipedia.org/wiki/%E3%82%AB%E3%83%AB%E3%83%80%E3%82%B7%E3%82%A7%E3%83%95%E3%83%BB%E3%82%B9%E3%82%B1%E3%83%BC%E3%83%AB)**に触発されています。これは天体物理学者ニコライ・カルダシェフがエネルギー消費量に基づいて文明の技術的発展レベルを測定するために提案した方法です。タイプI文明は惑星上で利用可能なすべてのエネルギーを活用し、タイプIIは恒星の全出力を捕捉し、タイプIIIは銀河全体のエネルギーを支配します。

AI支援開発の時代において、**トークンは新しいエネルギー**です。トークンは私たちの思考力を動かし、生産性を高め、創造的な成果を駆動します。カルダシェフ・スケールが宇宙規模でエネルギー消費を追跡するように、Tokscaleは AI増強開発のランクを上げながらトークン消費を測定します。カジュアルユーザーでも毎日数百万のトークンを消費する人でも、Tokscaleは惑星級開発者から銀河級コードアーキテクトへの旅を視覚化するのに役立ちます。

## 機能

- **インタラクティブTUIモード** - OpenTUIによる美しいターミナルUI（デフォルトモード）
  - 4つのインタラクティブビュー：概要、モデル、日別、統計
  - キーボード＆マウスナビゲーション
  - 9色テーマのGitHubスタイル貢献グラフ
  - リアルタイムフィルタリングとソート
  - ゼロフリッカーレンダリング（ネイティブZigエンジン）
- **マルチプラットフォームサポート** - OpenCode、Claude Code、Codex CLI、Cursor IDE、Gemini CLI全体の使用量追跡
- **リアルタイム価格** - 1時間ディスクキャッシュ付きでLiteLLMから現在の価格を取得
- **詳細な内訳** - 入力、出力、キャッシュ読み書き、推論トークン追跡
- **ネイティブRustコア** - 10倍高速な処理のため、すべての解析と集計をRustで実行
- **Web可視化** - 2Dと3Dビューのインタラクティブ貢献グラフ
- **柔軟なフィルタリング** - プラットフォーム、日付範囲、年別フィルタリング
- **JSONエクスポート** - 外部可視化ツール用のデータ生成
- **ソーシャルプラットフォーム** - 使用量の共有、リーダーボード競争、公開プロフィール閲覧

## インストール

### クイックスタート

```bash
# Bunをインストール（まだインストールしていない場合）
curl -fsSL https://bun.sh/install | bash

# bunxで直接実行
bunx tokscale
```

これだけです！セットアップ不要で完全なインタラクティブTUI体験が得られます。

> **[Bun](https://bun.sh/)が必要**: インタラクティブTUIはゼロフリッカーレンダリングのためにOpenTUIのネイティブZigモジュールを使用しており、Bunランタイムが必要です。

> **パッケージ構造**: `tokscale`は`@tokscale/cli`をインストールするエイリアスパッケージです（[`swc`](https://www.npmjs.com/package/swc)のように）。どちらもネイティブRustコア（`@tokscale/core`）を含む同じCLIをインストールします。


### 前提条件

- [Bun](https://bun.sh/)（必須）
- （オプション）ソースからネイティブモジュールをビルドするためのRustツールチェーン

### 開発環境セットアップ

ローカル開発またはソースからビルドする場合：

```bash
# リポジトリをクローン
git clone https://github.com/junhoyeo/tokscale.git
cd tokscale

# Bunをインストール（まだインストールしていない場合）
curl -fsSL https://bun.sh/install | bash

# 依存関係をインストール
bun install

# 開発モードでCLIを実行
bun run cli
```

> **注**: `bun run cli`はローカル開発用です。`bunx tokscale`でインストールすると、コマンドが直接実行されます。以下の使用法セクションはインストールされたバイナリコマンドを示しています。

### ネイティブモジュールのビルド（オプション）

ネイティブRustモジュールは並列ファイルスキャンとSIMD JSON解析により約10倍高速な処理を提供します：

```bash
# ネイティブコアをビルド（リポジトリルートから実行）
bun run build:core

# インストールを確認
bun run cli graph --benchmark
```

## 使用方法

### 基本コマンド

```bash
# インタラクティブTUIを起動（デフォルト）
tokscale

# 特定のタブでTUIを起動
tokscale models    # モデルタブ
tokscale monthly   # 日別ビュー（日別内訳を表示）

# レガシーCLIテーブル出力を使用
tokscale --light
tokscale models --light

# 明示的にTUIを起動
tokscale tui

# 貢献グラフデータをJSONとしてエクスポート
tokscale graph --output data.json

# JSONとしてデータを出力（スクリプト/自動化用）
tokscale --json                    # デフォルトのモデルビューをJSON形式で
tokscale models --json             # モデル内訳をJSON形式で
tokscale monthly --json            # 月別内訳をJSON形式で
tokscale models --json > report.json   # ファイルに保存
```

### TUI機能

インタラクティブTUIモードは以下を提供します：

- **4つのビュー**: 概要（チャート + トップモデル）、モデル、日別、統計（貢献グラフ）
- **キーボードナビゲーション**:
  - `1-4`または`←/→/Tab`: ビュー切り替え
  - `↑/↓`: リスト操作
  - `c/n/t`: コスト/名前/トークンでソート
  - `1-5`: ソーストグル（OpenCode/Claude/Codex/Cursor/Gemini）
  - `p`: 9色テーマを循環
  - `r`: データ更新
  - `e`: JSONにエクスポート
  - `q`: 終了
- **マウスサポート**: タブ、ボタン、フィルターをクリック
- **テーマ**: Green、Halloween、Teal、Blue、Pink、Purple、Orange、Monochrome、YlGnBu
- **設定の永続化**: テーマ設定は`~/.config/tokscale/tui-settings.json`に保存

### プラットフォーム別フィルタリング

```bash
# OpenCodeの使用量のみ表示
tokscale --opencode

# Claude Codeの使用量のみ表示
tokscale --claude

# Codex CLIの使用量のみ表示
tokscale --codex

# Gemini CLIの使用量のみ表示
tokscale --gemini

# Cursor IDEの使用量のみ表示（事前に`tokscale cursor login`が必要）
tokscale --cursor

# フィルターを組み合わせ
tokscale --opencode --claude
```

### 日付フィルタリング

日付フィルターはレポートを生成するすべてのコマンドで機能します（`tokscale`、`tokscale models`、`tokscale monthly`、`tokscale graph`）：

```bash
# クイック日付ショートカット
tokscale --today              # 今日のみ
tokscale --week               # 過去7日間
tokscale --month              # 今月

# カスタム日付範囲（包括的、ローカルタイムゾーン）
tokscale --since 2024-01-01 --until 2024-12-31

# 年別フィルター
tokscale --year 2024

# 他のオプションと組み合わせ
tokscale models --week --claude --json
tokscale monthly --month --benchmark
```

> **注**: 日付フィルターはローカルタイムゾーンを使用します。`--since`と`--until`は両方とも包括的です。

### ソーシャルプラットフォームコマンド

```bash
# Tokscaleにログイン（GitHub認証用にブラウザを開く）
tokscale login

# ログイン中のユーザーを確認
tokscale whoami

# 使用量データをリーダーボードに送信
tokscale submit

# フィルター付きで送信
tokscale submit --opencode --claude --since 2024-01-01

# 送信内容をプレビュー（ドライラン）
tokscale submit --dry-run

# ログアウト
tokscale logout
```

<img alt="CLI Submit" src="./.github/assets/cli-submit.png" />

### Cursor IDEコマンド

Cursor IDEはセッショントークンによる別途認証が必要です（ソーシャルプラットフォームのログインとは異なる）：

```bash
# Cursorにログイン（ブラウザからセッショントークンが必要）
tokscale cursor login

# Cursor認証ステータスとセッションの有効性を確認
tokscale cursor status

# Cursorからログアウト（保存された資格情報を削除）
tokscale cursor logout
```

**資格情報の保存**: Cursorセッショントークンは`~/.config/tokscale/cursor-credentials.json`に保存されます。使用量データは`~/.config/tokscale/cursor-cache/`にキャッシュされます。

**Cursorセッショントークンの取得方法:**
1. ブラウザで https://www.cursor.com/settings を開く
2. 開発者ツールを開く（F12）
3. **オプションA - Networkタブ**: ページで何らかのアクションを行い、`cursor.com/api/*`へのリクエストを見つけ、Request Headersの`Cookie`ヘッダーを確認し、`WorkosCursorSessionToken=`の後の値のみをコピー
4. **オプションB - Applicationタブ**: Application → Cookies → `https://www.cursor.com`に移動し、`WorkosCursorSessionToken`クッキーを見つけてその値をコピー（クッキー名ではなく値）

> ⚠️ **セキュリティ警告**: セッショントークンはパスワードのように扱ってください。公開したり、バージョン管理にコミットしたりしないでください。トークンはCursorアカウントへの完全なアクセス権を付与します。

### 出力例（`--light`バージョン）

<img alt="CLI Light" src="./.github/assets/cli-light.png" />

### 環境変数

大規模データセットや特別な要件を持つ上級ユーザー向け：

| 変数 | デフォルト | 説明 |
|----------|---------|-------------|
| `TOKSCALE_NATIVE_TIMEOUT_MS` | `300000`（5分） | ネイティブサブプロセス処理の最大時間 |
| `TOKSCALE_MAX_OUTPUT_BYTES` | `52428800`（50MB） | ネイティブサブプロセスからの最大出力サイズ |

```bash
# 例：非常に大きなデータセット用にタイムアウトを増加
TOKSCALE_NATIVE_TIMEOUT_MS=600000 tokscale graph --output data.json

# 例：長年のデータを持つパワーユーザー用に出力制限を増加
TOKSCALE_MAX_OUTPUT_BYTES=104857600 tokscale --json > report.json
```

> **注**: これらの制限はハングやメモリ問題を防ぐための安全対策です。ほとんどのユーザーは変更する必要がありません。

## フロントエンド可視化

フロントエンドはGitHubスタイルの貢献グラフ可視化を提供します：

### 機能

- **2Dビュー**: クラシックなGitHub貢献カレンダー
- **3Dビュー**: トークン使用量に基づく高さのアイソメトリック3D貢献グラフ
- **複数のカラーパレット**: GitHub、GitLab、Halloween、Winterなど
- **3ウェイテーマトグル**: Light / Dark / System（OS設定に従う）
- **GitHub Primerデザイン**: GitHubの公式カラーシステムを使用
- **インタラクティブツールチップ**: ホバーで詳細な日別内訳を表示
- **日別内訳パネル**: クリックでソース別、モデル別の詳細を確認
- **年別フィルタリング**: 年間を移動
- **ソースフィルタリング**: プラットフォーム別フィルター（OpenCode、Claude、Codex、Cursor、Gemini）
- **統計パネル**: 総コスト、トークン、活動日数、連続記録
- **FOUC防止**: Reactハイドレーション前にテーマを適用（フラッシュなし）

### フロントエンドの実行

```bash
cd packages/frontend
bun install
bun run dev
```

[http://localhost:3000](http://localhost:3000)を開いてソーシャルプラットフォームにアクセスしてください。

## ソーシャルプラットフォーム

Tokscaleには使用量データを共有し、他の開発者と競争できるソーシャルプラットフォームが含まれています。

### 機能

- **リーダーボード** - すべてのプラットフォームで最もトークンを使用している人を確認
- **ユーザープロフィール** - 貢献グラフと統計を含む公開プロフィール
- **期間フィルタリング** - 全期間、今月、今週の統計を表示
- **GitHub統合** - GitHubアカウントでログイン
- **ローカルビューアー** - 送信せずにプライベートにデータを表示

### はじめに

1. **ログイン** - `tokscale login`を実行してGitHubで認証
2. **送信** - `tokscale submit`を実行して使用量データをアップロード
3. **表示** - Webプラットフォームを訪問してプロフィールとリーダーボードを確認

### データ検証

送信されたデータはレベル1検証を受けます：
- 数学的整合性（合計が一致、負の値なし）
- 未来の日付なし
- 必須フィールドの存在
- 重複検出

## Wrapped 2025

Spotify Wrappedにインスパイアされた、AIコーディングアシスタントの年間使用量をまとめた美しいレビュー画像を生成します。

<div align="center">

![Wrapped 2025](.github/assets/wrapped-2025.png)

</div>

### コマンド

```bash
# 現在の年のWrapped画像を生成
tokscale wrapped

# 特定の年のWrapped画像を生成
tokscale wrapped --year 2025
```

### 含まれる内容

生成される画像には以下が含まれます：

- **総トークン数** - 年間のトークン消費量
- **トップモデル** - コスト順にランク付けされた最も使用したAIモデル3つ
- **トップクライアント** - 最も使用したプラットフォーム3つ（OpenCode、Claude Code、Cursorなど）
- **メッセージ数** - AIとのインタラクション総数
- **活動日数** - 少なくとも1回のAIインタラクションがあった日数
- **コスト** - LiteLLM価格に基づく推定総コスト
- **連続記録** - 最長の連続活動日数
- **貢献グラフ** - 年間活動のビジュアルヒートマップ

生成されたPNGはソーシャルメディア共有に最適化されています。コミュニティとあなたのコーディングの旅を共有しましょう！

## 開発

> **クイックセットアップ**: すぐに始めたい場合は、上記のインストールセクションの[開発環境セットアップ](#開発環境セットアップ)を参照してください。

### 前提条件

```bash
# Bun（必須）
bun --version

# Rust（ネイティブモジュール用）
rustc --version
cargo --version
```

### 実行方法

[開発環境セットアップ](#開発環境セットアップ)に従った後：

```bash
# ネイティブモジュールをビルド（オプションだが推奨）
bun run build:core

# 開発モードで実行（TUIを起動）
cd packages/cli && bun src/cli.ts

# またはレガシーCLIモードを使用
cd packages/cli && bun src/cli.ts --light
```

<details>
<summary>高度な開発</summary>

### プロジェクトスクリプト

| スクリプト | 説明 |
|--------|-------------|
| `bun run cli` | 開発モードでCLIを実行（BunでTUI） |
| `bun run build:core` | ネイティブRustモジュールをビルド（リリース） |
| `bun run build:cli` | CLIのTypeScriptをdist/にビルド |
| `bun run build` | coreとCLI両方をビルド |
| `bun run dev:frontend` | フロントエンド開発サーバーを実行 |

**パッケージ固有スクリプト**（パッケージディレクトリ内から）：
- `packages/cli`: `bun run dev`、`bun run tui`
- `packages/core`: `bun run build:debug`、`bun run test`、`bun run bench`

**注**: このプロジェクトは**Bun**をパッケージマネージャー兼ランタイムとして使用しています。TUIはOpenTUIのネイティブモジュールのためBunが必要です。

### テスト

```bash
# ネイティブモジュールをテスト（Rust）
cd packages/core
bun run test:rust      # Cargoテスト
bun run test           # Node.js統合テスト
bun run test:all       # 両方
```

### ネイティブモジュール開発

```bash
cd packages/core

# デバッグモードでビルド（コンパイルが速い）
bun run build:debug

# リリースモードでビルド（最適化済み）
bun run build

# Rustベンチマークを実行
bun run bench
```

### グラフコマンドオプション

```bash
# グラフデータをファイルにエクスポート
tokscale graph --output usage-data.json

# 日付フィルタリング（すべてのショートカットが使用可能）
tokscale graph --today
tokscale graph --week
tokscale graph --since 2024-01-01 --until 2024-12-31
tokscale graph --year 2024

# プラットフォーム別フィルター
tokscale graph --opencode --claude

# 処理時間ベンチマークを表示
tokscale graph --output data.json --benchmark
```

### ベンチマークフラグ

パフォーマンス分析用の処理時間を表示：

```bash
tokscale --benchmark           # デフォルトビューと共に処理時間を表示
tokscale models --benchmark    # モデルレポートをベンチマーク
tokscale monthly --benchmark   # 月別レポートをベンチマーク
tokscale graph --benchmark     # グラフ生成をベンチマーク
```

### フロントエンド用データの生成

```bash
# 可視化用データをエクスポート
tokscale graph --output packages/frontend/public/my-data.json
```

### アーキテクチャ

```
tokscale/
├── packages/
│   ├── cli/src/            # TypeScript CLI
│   │   ├── cli.ts          # Commander.jsエントリーポイント
│   │   ├── tui/            # OpenTUIインタラクティブインターフェース
│   │   │   ├── App.tsx     # メインTUIアプリ（Solid.js）
│   │   │   ├── components/ # TUIコンポーネント
│   │   │   ├── hooks/      # データフェッチ＆状態
│   │   │   ├── config/     # テーマ＆設定
│   │   │   └── utils/      # フォーマットユーティリティ
│   │   ├── sessions/       # プラットフォームセッションパーサー
│   │   │   ├── claudecode.ts  # Claude Codeパーサー
│   │   │   ├── codex.ts       # Codex CLIパーサー
│   │   │   ├── gemini.ts      # Gemini CLIパーサー
│   │   │   └── opencode.ts    # OpenCodeパーサー
│   │   ├── cursor.ts       # Cursor IDE統合
│   │   ├── graph.ts        # グラフデータ生成
│   │   ├── pricing.ts      # LiteLLM価格フェッチャー
│   │   └── native.ts       # ネイティブモジュールローダー
│   │
│   ├── core/               # Rustネイティブモジュール（napi-rs）
│   │   ├── src/
│   │   │   ├── lib.rs      # NAPIエクスポート
│   │   │   ├── scanner.rs  # 並列ファイル探索
│   │   │   ├── parser.rs   # SIMD JSON解析
│   │   │   ├── aggregator.rs # 並列集計
│   │   │   ├── pricing.rs  # コスト計算
│   │   │   └── sessions/   # プラットフォーム固有パーサー
│   │   ├── Cargo.toml
│   │   └── package.json
│   │
│   ├── frontend/           # Next.js可視化＆ソーシャルプラットフォーム
│   │   └── src/
│   │       ├── app/        # Next.jsアプリルーター
│   │       └── components/ # Reactコンポーネント
│   │
│   └── benchmarks/         # パフォーマンスベンチマーク
│       ├── runner.ts       # ベンチマークハーネス
│       └── generate.ts     # 合成データジェネレーター
```

#### ハイブリッドTypeScript + Rustアーキテクチャ

Tokscaleは最適なパフォーマンスのためにハイブリッドアーキテクチャを使用しています：

1. **TypeScriptレイヤー**: CLIインターフェース、価格フェッチ（ディスクキャッシュ付き）、出力フォーマット
2. **Rustネイティブコア**: すべての解析、コスト計算、集計

```
┌─────────────────────────────────────────────────────────────┐
│                     TypeScript (CLI)                        │
│  • LiteLLMから価格を取得（ディスクキャッシュ、1時間TTL）        │
│  • 価格データをRustに渡す                                     │
│  • フォーマットされた結果を表示                                │
└─────────────────────┬───────────────────────────────────────┘
                      │ pricing entries
                      ▼
┌─────────────────────────────────────────────────────────────┐
│                    Rustネイティブコア                         │
│  • 並列ファイルスキャン（rayon）                              │
│  • SIMD JSON解析（simd-json）                               │
│  • 価格データを使用したコスト計算                              │
│  • モデル/月/日別の並列集計                                   │
└─────────────────────────────────────────────────────────────┘
```

ネイティブモジュールが利用可能な場合、すべての重い計算はRustで行われます。ネイティブモジュールがインストールされていない場合、CLIは完全な互換性のためにTypeScript実装に自動的にフォールバックします（パフォーマンスは低下）。

#### 主要技術

| レイヤー | 技術 | 用途 |
|-------|------------|---------|
| CLI | [Commander.js](https://github.com/tj/commander.js) | コマンドライン解析 |
| TUI | [OpenTUI](https://github.com/sst/opentui) + [Solid.js](https://www.solidjs.com/) | インタラクティブターミナルUI（ゼロフリッカーレンダリング） |
| ランタイム | [Bun](https://bun.sh/) | 高速JavaScriptランタイム（必須） |
| テーブル | [cli-table3](https://github.com/cli-table/cli-table3) | ターミナルテーブルレンダリング（レガシーCLI） |
| 色 | [picocolors](https://github.com/alexeyraspopov/picocolors) | ターミナルカラー |
| ネイティブ | [napi-rs](https://napi.rs/) | Rust用Node.jsバインディング |
| 並列性 | [Rayon](https://github.com/rayon-rs/rayon) | Rustデータ並列処理 |
| JSON | [simd-json](https://github.com/simd-lite/simd-json) | SIMD高速化解析 |
| フロントエンド | [Next.js 16](https://nextjs.org/) | Reactフレームワーク |
| 3D可視化 | [obelisk.js](https://github.com/nicklockwood/obelisk.js) | アイソメトリック3Dレンダリング |

### パフォーマンス

ネイティブRustモジュールは大幅なパフォーマンス向上を提供します：

| 操作 | TypeScript | Rustネイティブ | 高速化 |
|-----------|------------|-------------|---------|
| ファイル探索 | ~500ms | ~50ms | **10倍** |
| JSON解析 | ~800ms | ~100ms | **8倍** |
| 集計 | ~200ms | ~25ms | **8倍** |
| **合計** | **~1.5秒** | **~175ms** | **~8.5倍** |

*約1000セッションファイル、100kメッセージのベンチマーク*

#### メモリ最適化

ネイティブモジュールは以下を通じて約45%のメモリ削減も提供します：

- ストリーミングJSON解析（ファイル全体のバッファリングなし）
- ゼロコピー文字列処理
- マップリデュースによる効率的な並列集計

#### ベンチマークの実行

```bash
# 合成データを生成
cd packages/benchmarks && bun run generate

# Rustベンチマークを実行
cd packages/core && bun run bench
```

</details>

## サポートプラットフォーム

### ネイティブモジュールターゲット

| プラットフォーム | アーキテクチャ | ステータス |
|----------|--------------|--------|
| macOS | x86_64 | サポート |
| macOS | aarch64（Apple Silicon） | サポート |
| Linux | x86_64（glibc） | サポート |
| Linux | aarch64（glibc） | サポート |
| Linux | x86_64（musl） | サポート |
| Linux | aarch64（musl） | サポート |
| Windows | x86_64 | サポート |
| Windows | aarch64 | サポート |

## セッションデータ保持

デフォルトでは、一部のAIコーディングアシスタントは古いセッションファイルを自動的に削除します。正確な追跡のために使用履歴を保持するには、クリーンアップ期間を無効化または延長してください。

| プラットフォーム | デフォルト | 設定ファイル | 無効化設定 | ソース |
|----------|---------|-------------|-------------------|--------|
| Claude Code | **⚠️ 30日** | `~/.claude/settings.json` | `"cleanupPeriodDays": 9999999999` | [ドキュメント](https://docs.anthropic.com/en/docs/claude-code/settings) |
| Gemini CLI | 無効 | `~/.gemini/settings.json` | `"sessionRetention.enabled": false` | [ドキュメント](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/session-management.md) |
| Codex CLI | 無効 | N/A | クリーンアップ機能なし | [#6015](https://github.com/openai/codex/issues/6015) |
| OpenCode | 無効 | N/A | クリーンアップ機能なし | [#4980](https://github.com/sst/opencode/issues/4980) |

### Claude Code

**デフォルト**: 30日のクリーンアップ期間

`~/.claude/settings.json`に追加：
```json
{
  "cleanupPeriodDays": 9999999999
}
```

> 非常に大きな値（例：`9999999999`日 ≈ 2700万年）を設定すると、事実上クリーンアップが無効になります。

### Gemini CLI

**デフォルト**: クリーンアップ無効（セッションは永久に保持）

クリーンアップを有効にしてから無効にしたい場合は、`~/.gemini/settings.json`で削除するか`enabled: false`に設定：
```json
{
  "general": {
    "sessionRetention": {
      "enabled": false
    }
  }
}
```

または非常に長い保持期間を設定：
```json
{
  "general": {
    "sessionRetention": {
      "enabled": true,
      "maxAge": "9999999d"
    }
  }
}
```

### Codex CLI

**デフォルト**: 自動クリーンアップなし（セッションは永久に保持）

Codex CLIには組み込みのセッションクリーンアップがありません。`~/.codex/sessions/`のセッションは無期限に保持されます。

> **注**: これに対する機能リクエストがあります：[#6015](https://github.com/openai/codex/issues/6015)

### OpenCode

**デフォルト**: 自動クリーンアップなし（セッションは永久に保持）

OpenCodeには組み込みのセッションクリーンアップがありません。`~/.local/share/opencode/storage/`のセッションは無期限に保持されます。

> **注**: [#4980](https://github.com/sst/opencode/issues/4980)を参照

---

## データソース

### OpenCode

場所: `~/.local/share/opencode/storage/message/{sessionId}/*.json`

各メッセージファイルの内容：
```json
{
  "id": "msg_xxx",
  "role": "assistant",
  "modelID": "claude-sonnet-4-20250514",
  "providerID": "anthropic",
  "tokens": {
    "input": 1234,
    "output": 567,
    "reasoning": 0,
    "cache": { "read": 890, "write": 123 }
  },
  "time": { "created": 1699999999999 }
}
```

### Claude Code

場所: `~/.claude/projects/{projectPath}/*.jsonl`

アシスタントメッセージの使用量データを含むJSONL形式：
```json
{"type": "assistant", "message": {"model": "claude-sonnet-4-20250514", "usage": {"input_tokens": 1234, "output_tokens": 567, "cache_read_input_tokens": 890}}, "timestamp": "2024-01-01T00:00:00Z"}
```

### Codex CLI

場所: `~/.codex/sessions/*.jsonl`

`token_count`イベントを含むイベントベース形式：
```json
{"type": "event_msg", "payload": {"type": "token_count", "info": {"last_token_usage": {"input_tokens": 1234, "output_tokens": 567}}}}
```

### Gemini CLI

場所: `~/.gemini/tmp/{projectHash}/chats/session-*.json`

メッセージ配列を含むセッションファイル：
```json
{
  "sessionId": "xxx",
  "messages": [
    {"type": "gemini", "model": "gemini-2.5-pro", "tokens": {"input": 1234, "output": 567, "cached": 890, "thoughts": 123}}
  ]
}
```

### Cursor IDE

場所: `~/.config/tokscale/cursor-cache/`（Cursor API経由で同期）

CursorデータはセッショントークンでCursor APIから取得され、ローカルにキャッシュされます。認証するには`tokscale cursor login`を実行してください。セットアップ手順は[Cursor IDEコマンド](#cursor-ideコマンド)を参照。

## 価格

Tokscaleは[LiteLLMの価格データベース](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json)からリアルタイム価格を取得します。

**キャッシュ**: 価格データは1時間TTLで`~/.cache/tokscale/pricing.json`にディスクキャッシュされます。これにより、価格データを最新に保ちながら高速な起動を確保します。

価格には以下が含まれます：
- 入力トークン
- 出力トークン
- キャッシュ読み取りトークン（割引）
- キャッシュ書き込みトークン
- 推論トークン（o1などのモデル用）
- 階層型価格（200kトークン以上）

## コントリビューション

コントリビューションを歓迎します！以下の手順に従ってください：

1. リポジトリをフォーク
2. 機能ブランチを作成（`git checkout -b feature/amazing-feature`）
3. 変更を加える
4. テストを実行（`cd packages/core && bun run test:all`）
5. 変更をコミット（`git commit -m 'Add amazing feature'`）
6. ブランチにプッシュ（`git push origin feature/amazing-feature`）
7. プルリクエストを開く

### 開発ガイドライン

- 既存のコードスタイルに従う
- 新機能にはテストを追加
- 必要に応じてドキュメントを更新
- コミットは集中的かつアトミックに

## 謝辞

- インスピレーションを与えてくれた[ccusage](https://github.com/ryoppippi/ccusage)、[viberank](https://github.com/sculptdotfun/viberank)、[Isometric Contributions](https://github.com/jasonlong/isometric-contributions)
- ゼロフリッカーターミナルUIフレームワーク[OpenTUI](https://github.com/sst/opentui)
- リアクティブレンダリングの[Solid.js](https://www.solidjs.com/)
- 価格データの[LiteLLM](https://github.com/BerriAI/litellm)
- Rust/Node.jsバインディングの[napi-rs](https://napi.rs/)
- 2Dグラフ参照の[github-contributions-canvas](https://github.com/sallar/github-contributions-canvas)

## ライセンス

<p align="center">
  <a href="https://github.com/junhoyeo">
    <img src=".github/assets/labtocat-on-spaceship.png" width="540">
  </a>
</p>

<p align="center">
  <strong>MIT © <a href="https://github.com/junhoyeo">Junho Yeo</a></strong>
</p>

このプロジェクトが興味深いと感じたら、**スターを付けてください ⭐** または[GitHubでフォロー](https://github.com/junhoyeo)して旅に参加してください（すでに1.1k以上が乗船中）。私は24時間コーディングし、定期的に驚くべきものを出荷しています—あなたのサポートは無駄になりません。

# cc one

> **あなたの AI CLI 使用量は、あなたのもの。** cc one は AI CLI がすでに書き出しているセッションログを読み取り、トークン・コスト・キャッシュ効率・トレンドに変換します。ローカルファーストのデスクトップダッシュボードで、自分が管理する GitHub リポジトリ経由の複数端末間同期もオプションで利用できます。

[![Version](https://img.shields.io/github/v/release/Buktal/cc-one?color=blue&label=version)](https://github.com/Buktal/cc-one/releases)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/Buktal/cc-one/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](./LICENSE)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

[English](./README.md) | [简体中文](./README.zh-CN.md) | **日本語** | [更新履歴](./CHANGELOG.ja-JP.md)

<img src="./docs/images/ad-ja.png" alt="cc one ダッシュボード" width="800">

---

## 背景

**Claude Code、Codex、Gemini CLI、Grok CLI、OpenCode** のセッションは、それぞれディスクにログを書き出します——トークン、キャッシュヒット、支出——それらすべてはプレーンテキストとして端末に残されたままです。cc one はそのログを読み取り、**いくら使ったか、何を得たか、トークンがどこへ向かったか**を鮮明な形にします。プロバイダーの切り替えも管理し、各 CLI が実際に読み込む設定ファイルを書き込みます。

製品全体は 2 つの原則に従います：

- **ローカルファースト。** フル機能のダッシュボードはゼロネットワークで動作します——ディスク上のログだけがすべての依存です。
- **ログに対して厳密に読み取り専用。** cc one はセッションログもツールの動作も決して変更しません。唯一の書き込みはプロバイダー切り替えで、常に明示的な操作であり、必ず事前にバックアップされます。

複数端末間同期はオプションのレイヤーであり、利用の前提条件ではありません。

## スクリーンショット

| | ライト | ダーク |
| --- | --- | --- |
| **ダッシュボード** | <img src="./docs/images/light-usage.png" alt="ダッシュボード（ライト）" width="320"> | <img src="./docs/images/dark-usage.png" alt="ダッシュボード（ダーク）" width="320"> |
| **消費量** | <img src="./docs/images/light-consumption.png" alt="消費量（ライト）" width="320"> | <img src="./docs/images/dark-consumption.png" alt="消費量（ダーク）" width="320"> |
| **グランスモード** | <img src="./docs/images/light-floating-card.png" alt="グランスモード（ライト）" width="320"> | <img src="./docs/images/dark-floating-card.png" alt="グランスモード（ダーク）" width="320"> |

## 機能

### プロバイダー

- **5 つの AI CLI、1 つの設定ハブ** —— 切り替えはその CLI が実際に読み込む設定ファイルを、制御フィールドのみマージし、書き込み前に必ずバックアップして書き込みます。
- **59 のビルトインプリセット** —— プリセットを選択し、キーを入力して切り替えます。CC-Switch、ローカル設定、CC One バックアップからのインポート、リスト全体の JSON エクスポートに対応。
- **モデルロールマッピング** —— 5 ロール（Sonnet / Opus / Fable / Haiku / Subagent）、それぞれにモデルと 1M コンテキストトグル。ワンクリックでベンダーのモデルリストを取得。
- **構造は同期、キーは非同期** —— プロバイダーリストは端末単位で同期。端末の外へ出るものからは常に API キーが除去されます。

### ダッシュボード

- **4 バケットのトークン経済** —— input / output / cache creation / cache read を、請求書に一致する 1 つのモデルに正規化。
- **キャッシュヒット率・モデル分布・リクエスト数とコスト** —— モデルはトークン順、それぞれにキャッシュヒット率を表示。合計は収集時点で固定。
- **トレンドとリクエストログ** —— 今日と昨日の差分付きの多指標トレンドチャート。呼び出し単位のログはクリックで完全な詳細を展開。

### セッション

- **ブラウズ可能な履歴** —— すべての会話をプロジェクト単位で整理し、全文検索と期間・ソース・モデル・デバイスによるフィルタに対応。
- **完全なトランスクリプトが即座に** —— 収集時にローカルへ保存。シンタックスハイライト付きのマークダウン描画、セッション内検索とジャンプに対応。
- **ローカルとお気に入り** —— 端末の外に出ないプライベートグループと、ソースデバイス表示付きで同期されるお気に入りグループ。端末の外に出るのはお気に入り登録したセッションだけです。

### 同期（オプション）

- **自分のリポジトリ、プレーンテキスト** —— 使用量は人間が読める端末別・日付別の JSONL として、あなたが管理する GitHub リポジトリへ投影。間にサードパーティのサーバーはありません。
- **構造的に無衝突** —— 端末分離書き込み、決定的な成果物、自己修復 rebase。
- **システムプロキシ対応**。ダッシュボード・グランスカード・ミニバーは単一端末に絞り込み可能。

### ライブラリ

- **ドラッグで中継アップロード** —— ファイル / ディレクトリをウィンドウにドロップすると、同期リポジトリ経由でその端末のサブツリーへ。
- **アプリ内プレビューと手動エクスポート** —— 画像、テキスト / JSON、それ以外はサンドボックス描画。任意のパスへのエクスポートに対応。

### コストと価格

- モデル別に編集可能な価格、ワンクリックでの LiteLLM コスト表取得、価格なしレコードの再請求、JSON インポート / エクスポート。

### エクスペリエンス

- **グランスモード** —— 画面端のミニバー、またはダッシュボードを映すフローティングカード。各形態の配置は個別に保持されます。
- **マルチスキンテーマ**、トレイ常駐のバックグラウンド収集（5〜60 秒）、署名付き自動更新。UI は English / 简体中文 / 日本語。

## ローカルに留まるもの、同期されるもの

| | スタンドアロン | 同期モード（リポジトリバインド済み） |
| --- | --- | --- |
| 使用量・コスト・トレンド・リクエストログ | ローカルのみ | 端末間で同期 |
| セッションのトランスクリプトとお気に入り | ローカルのみ | お気に入り登録したセッションのみ同期、他はローカル |
| プロバイダー構造 | ローカルのみ | 端末単位で同期——API キーは決して同期されない |
| ライブラリファイル | ローカルのみ | 端末間で同期 |
| 設定・スキン・価格上書き・ローカルグループ | ローカル | ローカル——リポジトリには一切書き込まれません |

リポジトリをバインドして同期を有効にしない限り、データが端末の外へ出ることはありません。アクセストークンも端末に留まり、リポジトリに書き込まれることはありません。

## 仕組み

```
  AI CLI セッションログ
  (Claude Code · Codex · Gemini CLI · Grok CLI · OpenCode)
          │  (読み取り専用)
          ▼
       Collect ──────▶ ローカルストア (SQLite) ──────▶ ダッシュボード & セッション
          │
          │  (オプション · 同期モード)
          ▼
     成果物 (プレーンテキスト、端末別 + 日付別)
          │
    push / pull は自分の GitHub リポジトリ経由
          │
          ▼
       他の端末
```

[Tauri 2](https://tauri.app/) アプリです：Rust バックエンドが収集、ローカルストア（唯一の真実の源）、プロバイダー設定の書き込み、オプション同期を担当し、React フロントエンドが生成された型安全な IPC バインディングを通じてダッシュボードを描画します。

## クイックスタート

お使いの OS のインストーラは **[Releases](https://github.com/Buktal/cc-one/releases)** ページからダウンロードできます。

| OS | インストーラ |
| --- | --- |
| **Windows** | `.msi` または `.exe`（NSIS）セットアップ |
| **macOS** | `.dmg`（Apple Silicon / arm64） |
| **Linux** | `.deb`、`.AppImage`（一部で `.rpm`） |

**初回起動：** cc one を起動すると、ローカルの AI CLI セッションログをスキャンしてダッシュボードが自動的に表示されます。アカウントもサインインもネットワークも不要。複数端末で利用する場合は、**設定**で自分が管理する GitHub リポジトリをバインドして同期を有効にしてください。

> **macOS の注意：** 現在のビルドは未署名です。初回起動時はアプリを右クリック → **開く**、または隔離属性を除去してください：
> ```bash
> xattr -dr com.apple.quarantine /Applications/cc one.app
> ```

## ソースからビルド

**前提条件：** [Node.js](https://nodejs.org/) 20+ LTS + [Yarn 4](https://yarnpkg.com/)（[Corepack](https://nodejs.org/api/corepack.html) 経由）、および安定版 [Rust](https://www.rust-lang.org/) とお使いの OS の [Tauri 前提条件](https://tauri.app/start/prerequisites/)。

```bash
corepack enable  # package.json で固定された Yarn バージョンを有効化
yarn install     # 依存関係をインストール
yarn dev         # 開発モードでデスクトップアプリを実行
yarn dist        # リリースバイナリをビルド
yarn check       # 静的チェック（Biome + tsc + Rust fmt/clippy）——CI と同じゲート
yarn test        # テストスイートを実行
```

**技術スタック：** [Tauri 2](https://tauri.app/) (Rust) · [React 19](https://react.dev/) · [TypeScript](https://www.typescriptlang.org/) · [Vite](https://vite.dev/) · [Tailwind CSS v4](https://tailwindcss.com/) · [shadcn/ui](https://ui.shadcn.com/) · [Redux Toolkit](https://redux-toolkit.js.org/) · [Recharts](https://recharts.org/)

## コントリビュート

issue と提案は歓迎します。PR の前には `yarn check` と `yarn test` を実行してください。大きな機能の場合は、先に issue でアプローチを議論してください。

## ライセンス

[MIT](./LICENSE) © cc one Contributors

[![LINUX DO](https://img.shields.io/badge/LINUX%20DO-%E3%82%B3%E3%83%9F%E3%83%A5%E3%83%8B%E3%83%86%E3%82%A3%E8%AA%8D%E5%AE%9A-blue?style=flat-square&logo=linux)](https://linux.do)

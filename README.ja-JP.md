<div align="center">

# CC One

**AI コーディング CLI向けの使用量ダッシュボードとプロバイダ管理ツール**

[![Release](https://img.shields.io/github/v/release/Buktal/cc-one)](https://github.com/Buktal/cc-one/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blueviolet)](https://github.com/Buktal/cc-one/releases/latest)

[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja-JP.md)

<img src="./docs/images/dashboard-overview.png" alt="CC One ダッシュボード" width="800">

</div>

CC One は、AI コーディング CLI のローカルセッションログを Token・コスト・トレンド・セッションの使用量分析に変換し、プロバイダ設定を管理するデスクトップアプリケーションです。データはすべてローカルの SQLite に保存されます。マルチデバイス同期を有効にした場合、データはユーザー所有のプライベート Git リポジトリを介して交換され、アクセストークンはローカルマシンから外に出ません。

## 機能

### 使用量ダッシュボード

- 5 つの視点:概要・デバイス・プロジェクト・セッション・リクエスト。
- Token の 4 分類集計(入力・出力・キャッシュ作成・キャッシュ読み取り)と、キャッシュヒット率・リクエスト数・コスト。
- 使用量トレンド、日別リクエスト数、ターン数と所要時間の分布、モデル別比率、デバイス別・プロジェクト別ランキング。
- リクエスト単位のログ。各リクエストのコスト明細(入力 / 出力 / キャッシュ読み取り / キャッシュ書き込み、課金モデル、停止理由)を展開表示。

### セッションワークベンチ

<table>
<tr>
<td><img src="./docs/images/sessions-workbench.png" alt="セッションワークベンチ" width="420"></td>
<td><img src="./docs/images/session-detail.png" alt="セッション詳細" width="420"></td>
</tr>
</table>

- プロジェクト・グループ・お気に入りの 3 種類のビューでセッションをナビゲート。一括でお気に入り登録・グループ設定・削除が可能。
- 会話全文の閲覧、ターン単位のナビゲーション、セッション内検索、メッセージのコピーに対応。
- ローカルグループはローカルマシンのみに保存。お気に入りとお気に入りグループはデバイス間で同期。

### プロバイダ管理(Beta)

- アプリごとに独立したプロバイダプールと、単一アクティブ方式(OpenCode は追加方式:複数プロバイダが同一設定ファイルに共存)。
- 内蔵の設定プリセット、モデルロールマッピング(Claude Code の 1M コンテキストフラグを含む)、アプリ別の共通設定スニペット。マージ時はプロバイダの設定を優先。
- 切り替え時に書き換えるのはプロバイダが管理するフィールドのみ。設定ファイルのその他のフィールドはそのまま保持。書き込みはアトミックかつバックアップ付きで、内容に変化がない場合は冪等。
- CC-Switch および既存の設定ファイルからのインポートに対応。

### マルチデバイス同期

- データはユーザー所有のプライベート Git リポジトリを介し、HTTPS とパーソナルアクセストークンで交換。トークンはローカルマシンにのみ保存。
- 使用量データ・お気に入りセッション・お気に入りグループ・デバイス別ファイルライブラリを、デバイス単位で分離して Git でバージョン管理。

<table>
<tr>
<td><img src="./docs/images/floating-card.png" alt="フローティングカード" width="260"></td>
<td>

### フローティング使用量カード

画面端に貼り付くコンパクトなカードで、本日の消費量(Token 4 分類・キャッシュヒット率・リクエスト数・コスト)を一覧表示。メインウィンドウをトレイに最小化すると自動で収納されます。

</td>
</tr>
</table>

### その他

- モデル課金単価の設定:内蔵価格テーブル、カスタムエントリ、LiteLLM からの取得、インポート・エクスポート。
- デバイスごとのファイルライブラリ。同期リポジトリでバージョン管理。
- GitHub Releases 経由のアプリ内自動アップデート。
- UI は English / 日本語 / 中文対応。ライト・ダークテーマとアクセントスキン。
- トレイ常駐とバックグラウンド収集に対応。

## 対応 CLI

| CLI | 使用量解析 | プロバイダ管理 |
| --- | :---: | :---: |
| Claude Code | ✓ | ✓ 単一アクティブ |
| Codex CLI | ✓ | ✓ 単一アクティブ |
| Gemini CLI | ✓ | ✓ 単一アクティブ |
| Grok CLI | ✓ | ✓ 単一アクティブ |
| OpenCode | ✓ | ✓ 追加方式 |

## インストール

[Releases](https://github.com/Buktal/cc-one/releases/latest) からパッケージをダウンロードしてください。

| プラットフォーム | パッケージ |
| --- | --- |
| Windows(x64) | `CC.One_*_x64-setup.exe` または `CC.One_*_x64_en-US.msi` |
| macOS(Apple Silicon) | `CC.One_*_aarch64.dmg` |
| Linux(x86_64) | `CC.One_*_amd64.deb` / `CC.One_*_1.x86_64.rpm` / `CC.One_*_amd64.AppImage` |

アプリケーションは GitHub Releases の更新を確認し、アプリ内で自動アップデートします。

## ソースからビルド

前提条件:Node.js ≥ 20、Yarn 4、Rust ツールチェーン、[Tauri 2 のプラットフォーム依存関係](https://v2.tauri.app/start/prerequisites/)。

```sh
yarn install
yarn dev      # 開発モードで起動
yarn check    # 静的検査と型検査(Biome、tsc、clippy、rustfmt)
yarn test     # テストスイート(Vitest、cargo test)
yarn dist     # 配布パッケージのビルド
```

## ライセンス

[MIT](LICENSE)

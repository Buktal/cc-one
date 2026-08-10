# cc one

> **你的 AI CLI 用量，由你掌控。** cc one 读取你的 AI CLI 已经写下的会话日志，把它们变成 token、成本、缓存效率与趋势——一个本地优先的桌面看板，并可选地通过你掌控的 GitHub 仓库在多设备间同步。

[![Version](https://img.shields.io/github/v/release/Buktal/cc-one?color=blue&label=version)](https://github.com/Buktal/cc-one/releases)
[![平台](https://img.shields.io/badge/%E5%B9%B3%E5%8F%B0-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/Buktal/cc-one/releases)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](./LICENSE)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

[English](./README.md) | **简体中文** | [日本語](./README.ja-JP.md) | [更新日志](./CHANGELOG.zh-CN.md)

<img src="./docs/images/ad-zh.png" alt="cc one 看板" width="800">

---

## 问题

每次你用 AI CLI——**Claude Code、Codex、Gemini CLI、Grok CLI、OpenCode**——它都会在磁盘上写下一份会话日志。输入 token、输出 token、缓存命中与未命中、花掉的钱：全都以纯文本躺在你的机器上，无人问津。cc one 读取这些日志，把它们变成清晰的图景：**你花了多少、换来了什么、token 用在了哪里。**

整个产品由两点立场所塑造：

- **本地优先。** 完整看板在零网络环境下即可工作。日志本来就在你的磁盘上——有它就够了。
- **只读。** cc one 只*读取*会话日志，绝不修改，也绝不干预这些工具的行为。它们照常运行，一如往常。

多设备同步纯粹是一层 **opt-in** 的叠加能力，绝非使用本应用的前提。

## 截图

| | 浅色 | 深色 |
| --- | --- | --- |
| **看板** | <img src="./docs/images/light-usage.png" alt="看板（浅色）" width="320"> | <img src="./docs/images/dark-usage.png" alt="看板（深色）" width="320"> |
| **消耗** | <img src="./docs/images/light-consumption.png" alt="消耗（浅色）" width="320"> | <img src="./docs/images/dark-consumption.png" alt="消耗（深色）" width="320"> |
| **速览模式** | <img src="./docs/images/light-floating-card.png" alt="速览模式（浅色）" width="320"> | <img src="./docs/images/dark-floating-card.png" alt="速览模式（深色）" width="320"> |

## 功能

### 看板

- **四桶 token 口径** —— input / output / cache creation / cache read，把各 CLI 的原生语义（如 Codex 含缓存的 input）归一成一套贴合真实计费的一致模型。
- **缓存命中率** —— `cache_read / (input + cache_creation + cache_read)`，与上游用量口径对齐。
- **请求数与成本** —— 请求总数与总成本（USD），在采集入库时冻结。
- **用量趋势** —— 多线 token-vs-cost 时间图，每个指标一条序列。
- **逐调用请求日志** —— 来源、模型、token 明细、成本、回合时长，以及 `stop_reason` / `service_tier` 徽标。
- **逐回合视图** —— 整个回合的成本与墙钟时长，与单次调用计时分开。

### 会话

- **每一次对话都可浏览的历史** —— AI CLI 跑过的每个会话，按项目目录归类，支持标题与路径的全文搜索；按时间范围、来源、模型、设备筛选。
- **完整原文，即时可看** —— 每个会话的对话在采集时写入本地数据库，因此任何会话——无论是否收藏——都能打开按角色着色的完整原文，不再重读可能正在写入的日志文件。
- **双 tab、两套组织方式** —— **本地** tab 罗列本机采集的全部会话，归入绝不离开本机的私有分组；**收藏** tab 罗列跨设备收藏的会话，归入全局一致的收藏分组，每个条目带来源设备标记。
- **收藏跨设备同步** —— 点一次星标，该会话的原文就经你的同步仓库到达所有设备；取消收藏，处处消失。只有收藏的会话才会离开你的机器。
- **逐会话经济账** —— 每个会话展示请求数、token 明细与成本，从用量记录现场求和——从不重复存储。

### 同步（可选）

- **独立或同步两种模式** —— 完全离线运行，或绑定一个你掌控的 GitHub 仓库跨设备对齐数据。
- **你自己的仓库、纯文本产物** —— 用量投影为人类可读、按设备按天切分的 JSONL（`data/<device>/usage-YYYY-MM-DD.jsonl`），写入你掌控的仓库。中间不经过任何第三方服务。
- **设备隔离、零冲突** —— 每台设备只写自己的 `data/<device>/` 子目录，并发 push 永不碰撞。若一台设备在 push 竞速中落败，下次同步会把它的本地提交 rebase 到远端 tip 上自愈。
- **确定性产物** —— 采集只写本地存储；push 时从存储逐字节重算每个脏天的产物文件，两台设备永远不会对同一文件的内容产生分歧。
- **跟随系统代理** —— push/fetch 遵循 OS 代理（Clash/Mihomo、企业网关），Synced 模式在其后开箱即用。
- **设备作用域视图** —— 把看板、速览卡与迷你条限定到单台设备；本地遗忘对端，失联设备自动清除。

### 库（Library）

- **拖入即中转上传** —— 把文件 / 目录拖进窗口，即经同步仓库 push 到该设备的子目录；任意深度的嵌套目录均可。
- **应用内预览** —— 图片按宽适配、ctrl+滚轮缩放；文本与 JSON 主题化渲染、pretty-print；其余内容在沙箱 iframe 中加载。
- **手动导出** —— 保存条目到自选路径；cc one 从不知晓目标路径，也绝不写入 AI 工具自身的配置目录。
- **安全的覆盖** —— 同名同类型直接覆盖（git 历史兜底）；同名异类型拒绝。
- **按设备、零冲突** —— 每台设备持有自己的子树；遗忘对端时可选将其文件迁移到本机 `from-<peer>/` 下，或删除。

### 成本与定价

- **逐模型可编辑定价** —— 覆盖种子价格；cc one 用你的数字。
- **从 LiteLLM 拉取** —— 一键获取最新模型成本表。
- **Rebill 补账** —— 为采集时没有价格的记录补齐成本，不动已有历史。
- **定价书可携带** —— 定价表支持 JSON 导入导出。

### 体验

- **轻量速览模式** —— 贴边迷你条常驻显示今日总数，或展开为复用看板的悬浮卡；full ⇄ expanded ⇄ tucked 三形态任意互切，每形态各自记忆位置。
- **多皮肤主题** —— 五套强调色与图表配色（Neutral / Sage / Azure / Crimson / Mauve），整体换肤不动内容。
- **托盘常驻、后台采集** —— 增量扫描器以 5s–60s 间隔保持看板新鲜，无需保留窗口。
- **自动更新 + 三语言** —— 直接从 GitHub Releases 安装签名更新；界面支持 English、简体中文、日本語。

## 什么留在本地，什么参与同步

| | 独立模式 | 同步模式（已绑定仓库） |
| --- | --- | --- |
| 用量、成本、趋势、请求日志 | 仅本地 | 跨设备同步 |
| 会话原文与收藏 | 仅本地 | 收藏的会话同步；其余留在本地 |
| 库文件 | 仅本地 | 跨设备同步 |
| 设置、皮肤、定价覆盖、本地分组 | 本地 | 本地——绝不写入仓库 |

未绑定仓库并开启同步前，没有任何数据离开你的机器。你使用的访问令牌留在本机，永不写入仓库。

## 工作原理

```
  AI CLI 会话日志
  (Claude Code · Codex · Gemini CLI · Grok CLI · OpenCode)
          │  (只读)
          ▼
       Collect ──────▶ 本地存储 (SQLite) ──────▶ 看板与会话
          │
          │  (可选 · 同步模式)
          ▼
    产物文件 (纯文本，按设备 + 日期)
          │
    push / pull 经你的 GitHub 仓库
          │
          ▼
       其他设备
```

一个 [Tauri 2](https://tauri.app/) 应用：Rust 后端负责采集、本地存储与可选的 Git 仓库同步；React 前端通过生成的类型安全 IPC 绑定渲染看板。采集器是插件化的 provider 模型（Claude Code、Codex、Gemini CLI、Grok CLI、OpenCode），本地存储是唯一事实来源，同步是把该存储投影为纯文本产物的可选层。

## 快速开始

从 **[Releases](https://github.com/Buktal/cc-one/releases)** 页面获取对应平台的安装包。

| 平台 | 安装包 |
| --- | --- |
| **Windows** | `.msi` 或 `.exe`（NSIS）安装程序 |
| **macOS** | `.dmg`（Apple Silicon / arm64） |
| **Linux** | `.deb`、`.AppImage`（部分版本提供 `.rpm`） |

**首次运行：** 启动 cc one——它会扫描本地的 AI CLI 会话日志，看板随即填充。无需账号、无需登录、无需联网。若要在多台机器间查看用量，在 **设置** 中开启同步，并指向一个你掌控的 GitHub 仓库。

> **macOS 提示：** 当前构建未签名。首次启动时请右键点击应用 → **打开**，或去除隔离属性：
> ```bash
> xattr -dr com.apple.quarantine /Applications/cc one.app
> ```

## 从源码构建

**前置条件：** [Node.js](https://nodejs.org/) 20+ LTS + [Yarn 4](https://yarnpkg.com/)（经 [Corepack](https://nodejs.org/api/corepack.html)），以及稳定版 [Rust](https://www.rust-lang.org/) 和你操作系统的 [Tauri 前置依赖](https://tauri.app/start/prerequisites/)。

```bash
corepack enable  # 激活 package.json 中钉定的 Yarn 版本
yarn install     # 安装依赖
yarn dev         # 以开发模式运行桌面应用
yarn dist        # 构建发布版二进制
yarn check       # 静态检查（Biome + tsc + Rust fmt/clippy）——与 CI 同一门槛
yarn test        # 运行测试套件
```

**技术栈：** [Tauri 2](https://tauri.app/) (Rust) · [React 19](https://react.dev/) · [TypeScript](https://www.typescriptlang.org/) · [Vite](https://vite.dev/) · [Tailwind CSS v4](https://tailwindcss.com/) · [shadcn/ui](https://ui.shadcn.com/) · [Redux Toolkit](https://redux-toolkit.js.org/) · [Recharts](https://recharts.org/)

## 常见问题

**为什么叫 cc one（归一）？** —— "cc" 让它站在 Claude Code 生态家族里（cc-switch、cc-connect 的同伴），"one" 是一个中枢：所有 AI CLI 的用量、配置、同步、未来的 agent 桥接，都收敛进这一个归你所有的工具。中文名「归一」——九九归一。旧名 VaultOne。

**cc one 会把我的数据发送到任何地方吗？** 不会。一切数据都从本地日志读取、存在本地。唯一让数据离开你机器的途径是你主动开启同步——而那时它去的也是*你掌控的* GitHub 仓库，且是纯文本。

**需要 API key 或代理吗？** 不需要。cc one 解析你的 AI CLI 已经写下的日志文件，从不调用模型供应商。

**它会修改我的日志吗？** 绝不会。cc one 对会话日志与 AI 工具配置严格只读。

**支持哪些 AI CLI？** Claude Code、Codex、Gemini CLI、Grok CLI 与 OpenCode——各自以原生日志格式（JSONL、JSON 或 SQLite）解析，token 语义归一为同一套模型。

**为什么用 GitHub 仓库做同步？** 因为你本来就有、免费，而且让数据始终握在自己手里——纯文本产物存在你掌控的仓库，不经过第三方服务。设备隔离 + 自愈 rebase 让多设备并发同步保持无冲突。

## 贡献

欢迎提交 issue 与建议。提交 PR 前请运行 `yarn check` 与 `yarn test`。较大功能请先开 issue 讨论方案。

## 许可证

[MIT](./LICENSE) © cc one Contributors

# cc one

> **Your AI CLI usage, owned by you.** cc one reads the session logs your AI CLIs already write and turns them into tokens, cost, cache efficiency, and trends — a local-first dashboard with optional multi-device sync through a GitHub repo you control.

[![Version](https://img.shields.io/github/v/release/Buktal/cc-one?color=blue&label=version)](https://github.com/Buktal/cc-one/releases)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/Buktal/cc-one/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](./LICENSE)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

**English** | [简体中文](./README.zh-CN.md) | [日本語](./README.ja-JP.md) | [Changelog](./CHANGELOG.md)

<img src="./docs/images/ad-en.png" alt="cc one dashboard" width="800">

---

## The problem

Every time you work with an AI CLI — **Claude Code, Codex, Gemini CLI, Grok CLI, OpenCode** — it writes a session log to disk. Tokens in, tokens out, cache hits and misses, money spent: it all sits in plain text on your machine, unread. cc one reads those logs and turns them into a clear picture: **what you spent, what you got, and where your tokens went.**

Two stances shape the whole product:

- **Local-first.** The full dashboard works with zero network. The logs are already on your disk — that's all it needs.
- **Read-only.** cc one only ever *reads* session logs. It never modifies them and never touches the tools' behavior. They keep running exactly as before.

Multi-device sync is a purely **opt-in** layer on top — never a precondition.

## Screenshots

| | Light | Dark |
| --- | --- | --- |
| **Dashboard** | <img src="./docs/images/light-usage.png" alt="Dashboard (light)" width="320"> | <img src="./docs/images/dark-usage.png" alt="Dashboard (dark)" width="320"> |
| **Consumption** | <img src="./docs/images/light-consumption.png" alt="Consumption (light)" width="320"> | <img src="./docs/images/dark-consumption.png" alt="Consumption (dark)" width="320"> |
| **Glance mode** | <img src="./docs/images/light-floating-card.png" alt="Glance mode (light)" width="320"> | <img src="./docs/images/dark-floating-card.png" alt="Glance mode (dark)" width="320"> |

## Features

### Dashboard

- **Four-bucket token economics** — input, output, cache creation, and cache read, normalized from each CLI's native semantics (e.g. Codex's cache-inclusive input) into one consistent model that matches your bill.
- **Cache-hit rate** — `cache_read / (input + cache_creation + cache_read)`, aligned with how upstream usage is counted.
- **Requests & cost** — total request count and total cost (USD), frozen at collection time.
- **Usage trends** — multi-line token-vs-cost chart over time, one series per metric.
- **Per-call request log** — source, model, token breakdown, cost, turn duration, and `stop_reason` / `service_tier` chips.
- **Per-turn view** — whole-turn cost and wall-clock duration, separate from single-call timing.

### Sessions

- **A browsable history of every conversation** — every session your AI CLIs ran, grouped under its project directory, with full-text search across titles and paths. Filter by time range, source, model, and device.
- **Full transcripts, instantly** — every session's conversation is stored in the local database at collect time, so any session — favorited or not — opens its complete transcript with color-coded roles, without re-reading a log file that may still be mid-write.
- **Two tabs, two ways to organize** — a **Local** tab for everything collected on this machine, sorted into private groups that never leave it; a **Favorites** tab for the sessions you starred across all devices, sorted into synced groups, each entry marked with its source device.
- **Favorites sync across devices** — star a session once and its transcript travels through your sync repo to every other device; unstar it and it disappears everywhere. Only favorited sessions ever leave your machine.
- **Per-session economics** — each session shows its request count, token breakdown, and cost, computed live from the usage records — never double-stored.

### Sync (optional)

- **Standalone or Synced** — run fully offline, or bind a GitHub repo you own to align data across devices.
- **Your own repo, plain-text artifacts** — usage is projected into human-readable, per-device, per-day JSONL (`data/<device>/usage-YYYY-MM-DD.jsonl`) in a repo you control. No third-party server in the middle.
- **Device-isolated, conflict-free** — each device writes its own `data/<device>/` subtree, so concurrent pushes never collide. If a device loses a push race, the next sync rebases its local commits onto the remote tip and self-heals.
- **Deterministic artifacts** — collection writes the local store only; a push regenerates each changed day's artifact byte-for-byte from the store, so two devices can never disagree on a file's content.
- **System-proxy aware** — push/fetch follows the OS proxy (Clash/Mihomo, corporate gateways), so Synced mode just works behind one.
- **Device-scoped views** — filter the dashboard, the glance card, and the tucked bar to a single device; forget a peer locally, and stale peers auto-clear.

### Library

- **Drag-to-relay upload** — drop a file or directory onto the window to push it through your sync repo into that device's subtree; nested directories work at every depth.
- **In-app preview** — images fit-to-width with ctrl+wheel zoom; text and JSON render themed and pretty-printed; everything else loads in a sandboxed iframe.
- **Manual export** — save an entry to a path you choose; cc one never learns the target path and never writes into an AI tool's config dir.
- **Safe overwrites** — same-name same-kind overwrites (git history is the safety net); same-name different-kind is rejected.
- **Per-device, zero conflict** — each device holds its own subtree; forgetting a peer offers to migrate its files into yours (`from-<peer>/`) or delete them.

### Cost & pricing

- **Editable per-model pricing** — override seed prices; cc one uses your numbers.
- **Pull from LiteLLM** — fetch the latest model cost map with one click.
- **Rebill** — backfill records that had no price when collected, without re-costing existing history.
- **Portable pricing book** — import and export your pricing table as JSON.

### Experience

- **Lightweight glance mode** — tuck a mini-bar to the screen edge that always shows today's total, or expand it into a floating card mirroring the dashboard. Full ⇄ expanded ⇄ tucked, each shape remembering its own placement.
- **Multi-skin theming** — five accent and chart palettes (Neutral, Sage, Azure, Crimson, Mauve), recolor the whole app without touching content.
- **Tray-resident background collection** — an incremental scanner keeps the dashboard fresh (5s–60s intervals), no window needed.
- **Auto-update & three languages** — signed updates straight from GitHub Releases; UI in English, 简体中文, or 日本語.

## What stays local, what syncs

| | Standalone | Synced (repo bound) |
| --- | --- | --- |
| Usage, cost, trends, request log | Local only | Syncs across devices |
| Session transcripts & favorites | Local only | Favorited sessions sync; the rest stay local |
| Library files | Local only | Syncs across devices |
| Settings, skins, pricing overrides, local groups | Local | Local — never written to the repo |

Nothing leaves your machine unless you bind a repo and enable sync. The access token you use stays on your machine and is never written to the repo.

## How it works

```
  AI CLI session logs
  (Claude Code · Codex · Gemini CLI · Grok CLI · OpenCode)
          │  (read-only)
          ▼
       Collect ──────▶ Local store (SQLite) ──────▶ Dashboard & Sessions
          │
          │  (optional · Synced mode)
          ▼
   Artifacts (plain text, per device + date)
          │
    push / pull via your GitHub repo
          │
          ▼
      Other devices
```

A [Tauri 2](https://tauri.app/) app: a Rust backend handles collection, the local store, and optional Git-repo sync; a React frontend renders the dashboard through generated, type-safe IPC bindings. The collector is a pluggable provider model (Claude Code, Codex, Gemini CLI, Grok CLI, OpenCode), the local store is the single source of truth, and sync is an opt-in projection of that store into plain-text artifacts.

## Quick start

Grab the installer for your OS from the **[Releases](https://github.com/Buktal/cc-one/releases)** page.

| OS | Installer |
| --- | --- |
| **Windows** | `.msi` or `.exe` (NSIS) setup |
| **macOS** | `.dmg` (Apple Silicon, arm64) |
| **Linux** | `.deb`, `.AppImage` (`.rpm` where available) |

**First run:** launch cc one — it scans your local AI CLI session logs and the dashboard fills in. No account, no sign-in, no network. To see usage across machines, enable sync in **Settings** and point cc one at a GitHub repo you control.

> **macOS note:** builds are currently unsigned. On first launch, right-click the app → **Open**, or strip the quarantine attribute:
> ```bash
> xattr -dr com.apple.quarantine /Applications/cc one.app
> ```

## Build from source

**Prerequisites:** [Node.js](https://nodejs.org/) 20+ LTS + [Yarn 4](https://yarnpkg.com/) (via [Corepack](https://nodejs.org/api/corepack.html)), and [Rust](https://www.rust-lang.org/) stable with the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your OS.

```bash
corepack enable  # activate the Yarn version pinned in package.json
yarn install     # install dependencies
yarn dev         # run the desktop app in development
yarn dist        # build a release binary
yarn check       # static checks (Biome + tsc + Rust fmt/clippy) — same gates as CI
yarn test        # run the test suite
```

**Tech stack:** [Tauri 2](https://tauri.app/) (Rust) · [React 19](https://react.dev/) · [TypeScript](https://www.typescriptlang.org/) · [Vite](https://vite.dev/) · [Tailwind CSS v4](https://tailwindcss.com/) · [shadcn/ui](https://ui.shadcn.com/) · [Redux Toolkit](https://redux-toolkit.js.org/) · [Recharts](https://recharts.org/)

## FAQ

**Does cc one send my data anywhere?** No. Everything is read from local logs and stored locally. The only way data leaves your machine is if you opt into sync — and then it goes to a GitHub repo *you* own, as plain text.

**Does it need an API key or a proxy?** No. cc one parses the log files your AI CLIs already write; it never calls the model providers.

**Does it modify my logs?** Never. cc one is strictly read-only with respect to session logs and AI tool configuration.

**Which AI CLIs are supported?** Claude Code, Codex, Gemini CLI, Grok CLI, and OpenCode — each parsed from its native log format (JSONL, JSON, or SQLite), with token semantics normalized into one model.

**Why a GitHub repo for sync?** Because you already have one, it's free, and it keeps your data in your hands — plain-text artifacts in a repo you control, no third-party service. Device isolation plus self-healing rebases keep concurrent multi-device sync conflict-free.

## Contributing

Issues and suggestions are welcome. Before a PR, run `yarn check` and `yarn test`. For larger features, open an issue to discuss the approach first.

## License

[MIT](./LICENSE) © cc one Contributors

[![LINUX DO](https://img.shields.io/badge/LINUX%20DO-%E7%A4%BE%E5%8C%BA%E8%AE%A4%E5%8F%AF-blue?style=flat-square&logo=linux)](https://linux.do)
  

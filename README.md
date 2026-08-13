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

And when you switch between AI CLIs — or switch providers behind them — cc one manages that too, writing the exact config file each CLI reads.

Two stances shape the whole product:

- **Local-first.** The full dashboard works with zero network. The logs are already on your disk — that's all it needs.
- **Read-only toward your logs.** cc one only ever *reads* session logs. It never modifies them and never touches the tools' behavior — they keep running exactly as before. The one exception: switching a provider writes the config your CLI reads, and it's always your explicit action, backed up before the write.

Multi-device sync is a purely **opt-in** layer on top — never a precondition.

## Screenshots

| | Light | Dark |
| --- | --- | --- |
| **Dashboard** | <img src="./docs/images/light-usage.png" alt="Dashboard (light)" width="320"> | <img src="./docs/images/dark-usage.png" alt="Dashboard (dark)" width="320"> |
| **Consumption** | <img src="./docs/images/light-consumption.png" alt="Consumption (light)" width="320"> | <img src="./docs/images/dark-consumption.png" alt="Consumption (dark)" width="320"> |
| **Glance mode** | <img src="./docs/images/light-floating-card.png" alt="Glance mode (light)" width="320"> | <img src="./docs/images/dark-floating-card.png" alt="Glance mode (dark)" width="320"> |

## Features

### Providers

- **All five AI CLIs, one config hub** — Claude Code, Codex, Gemini CLI, Grok CLI, and OpenCode each get their own provider list: name, category, endpoint, and auth. Switching a provider writes the *real* config file that CLI reads — Codex's `config.toml` + `auth.json`, Gemini CLI's `settings.json` + env, Grok's `config.toml`, OpenCode's `opencode.json` — merging only controlled fields and backing up the previous file first.
- **18 built-in presets** — Claude official + AWS Bedrock, eleven domestic vendors (Kimi, DeepSeek, GLM, Volcengine, DouBao, Baidu, Alibaba, StepFun, MiniMax, MiMo …), and four popular aggregators (SiliconFlow, OpenRouter, ModelScope, Novita). Pick one, drop in your key, switch.
- **Raw settings editor** — every provider carries its full settings snapshot. A built-in JSON editor shows the whole thing, formats on demand, and flags parse errors instead of silently discarding them.
- **Model role mapping** — five roles (Sonnet / Opus / Fable / Haiku / Subagent), each with its own model and a 1M-context toggle. One click fetches the vendor's model list; "apply to all" spreads one model across every role.
- **Import from anywhere** — bring providers in from a CC-Switch export, your local config files, or a CC One backup — three sources treated as equals, with an opencode.json import previewing its changes before landing. Export your whole list as JSON anytime.
- **Provider structure syncs, keys don't** — the provider list rides your sync repo per device, byte-stable; API keys are stripped from anything that leaves your machine.

### Dashboard

- **Four-bucket token economics** — input, output, cache creation, and cache read, normalized from each CLI's native semantics (e.g. Codex's cache-inclusive input) into one consistent model that matches your bill.
- **Cache-hit rate** — `cache_read / (input + cache_creation + cache_read)`, aligned with how upstream usage is counted.
- **Token-first model distribution** — the usage breakdown ranks models by tokens, each with its own cache-hit rate.
- **Requests & cost** — total request count and total cost (USD), frozen at collection time.
- **Usage trends** — multi-line token-vs-cost chart over time, one series per metric, with a today-vs-yesterday delta.
- **Per-call request log** — source, model, token breakdown, cost, turn duration, and `stop_reason` / `service_tier` chips; click any row to unfold its full details.
- **Per-turn view** — whole-turn cost and wall-clock duration, separate from single-call timing.

### Sessions

- **A browsable history of every conversation** — every session your AI CLIs ran, grouped under its project directory, with full-text search across titles and paths. Filter by time range, source, model, and device.
- **Full transcripts, instantly** — every session's conversation is stored in the local database at collect time, so any session — favorited or not — opens its complete transcript without re-reading a log file that may still be mid-write.
- **Transcripts render as markdown** — code blocks and JSON are syntax-highlighted and themed; Claude Code subagent runs appear as their own sessions with an agent-type badge.
- **Search and jump inside a session** — find text across the transcript with hit-highlighting, then jump straight to the message via the numbered turn panel beside it.
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
- **Multi-skin theming** — five accent and chart palettes (Neutral, Sage, Azure, Crimson, Mauve) recolor the whole app without touching content; dark mode gets a three-tier surface ladder so pages, modals, and inputs read at the right depth.
- **Tray-resident background collection** — an incremental scanner keeps the dashboard fresh (5s–60s intervals), no window needed.
- **Auto-update & three languages** — signed updates straight from GitHub Releases; UI in English, 简体中文, or 日本語.

## What stays local, what syncs

| | Standalone | Synced (repo bound) |
| --- | --- | --- |
| Usage, cost, trends, request log | Local only | Syncs across devices |
| Session transcripts & favorites | Local only | Favorited sessions sync; the rest stay local |
| Provider structure | Local only | Syncs per device — API keys never sync |
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

A [Tauri 2](https://tauri.app/) app: a Rust backend handles collection, the local store, provider config writes, and optional Git-repo sync; a React frontend renders the dashboard through generated, type-safe IPC bindings. The collector is a pluggable provider model (Claude Code, Codex, Gemini CLI, Grok CLI, OpenCode), the local store is the single source of truth, and sync is an opt-in projection of that store into plain-text artifacts.

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

**Why "cc one"?** — "cc" places it in the Claude Code ecosystem (next to cc-switch and cc-connect), and "one" is the hub where every AI CLI's usage, config, sync, and future agent bridges converge into a single tool you own. In Chinese it's 归一 — everything returns to one hub. It was previously called VaultOne.

**Does cc one send my data anywhere?** No. Everything is read from local logs and stored locally. The only way data leaves your machine is if you opt into sync — and then it goes to a GitHub repo *you* own, as plain text.

**Does it need an API key or a proxy?** No. cc one parses the log files your AI CLIs already write; it never calls the model providers.

**Does it modify my logs?** Never. cc one is strictly read-only with respect to session logs. Switching a provider is the only action that writes a config file — always on your explicit click, always backed up first.

**Which AI CLIs are supported?** Claude Code, Codex, Gemini CLI, Grok CLI, and OpenCode — each parsed from its native log format (JSONL, JSON, or SQLite), with token semantics normalized into one model.

**Why a GitHub repo for sync?** Because you already have one, it's free, and it keeps your data in your hands — plain-text artifacts in a repo you control, no third-party service. Device isolation plus self-healing rebases keep concurrent multi-device sync conflict-free.

## Contributing

Issues and suggestions are welcome. Before a PR, run `yarn check` and `yarn test`. For larger features, open an issue to discuss the approach first.

## License

[MIT](./LICENSE) © cc one Contributors

[![LINUX DO](https://img.shields.io/badge/LINUX%20DO-Recognized%20Community-blue?style=flat-square&logo=linux)](https://linux.do)

# cc one

> **Your AI CLI usage, owned by you.** cc one reads the session logs your AI CLIs already write and turns them into tokens, cost, cache efficiency, and trends — a local-first dashboard with optional multi-device sync through a GitHub repo you control.

[![Version](https://img.shields.io/github/v/release/Buktal/cc-one?color=blue&label=version)](https://github.com/Buktal/cc-one/releases)
[![Platforms](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/Buktal/cc-one/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](./LICENSE)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-orange.svg)](https://tauri.app/)

**English** | [简体中文](./README.zh-CN.md) | [日本語](./README.ja-JP.md) | [Changelog](./CHANGELOG.md)

<img src="./docs/images/ad-en.png" alt="cc one dashboard" width="800">

---

## Why

Every session with **Claude Code, Codex, Gemini CLI, Grok CLI, or OpenCode** writes a log to disk — tokens, cache hits, spend — all in plain text, unread. cc one reads those logs and turns them into a clear picture: **what you spent, what you received, and where your tokens went.** It also manages provider switching, writing the exact config file each CLI reads.

Two principles govern the design:

- **Local-first.** The full dashboard works with zero network — the logs already on your disk are all it needs.
- **Read-only toward your logs.** cc one never modifies session logs or tool behavior. The sole write is provider switching: always your explicit action, always backed up first.

Multi-device sync is an opt-in layer, never a precondition.

## Screenshots

| | Light | Dark |
| --- | --- | --- |
| **Dashboard** | <img src="./docs/images/light-usage.png" alt="Dashboard (light)" width="320"> | <img src="./docs/images/dark-usage.png" alt="Dashboard (dark)" width="320"> |
| **Consumption** | <img src="./docs/images/light-consumption.png" alt="Consumption (light)" width="320"> | <img src="./docs/images/dark-consumption.png" alt="Consumption (dark)" width="320"> |
| **Glance mode** | <img src="./docs/images/light-floating-card.png" alt="Glance mode (light)" width="320"> | <img src="./docs/images/dark-floating-card.png" alt="Glance mode (dark)" width="320"> |

## Features

### Providers

- **One config hub for all five CLIs** — switching a provider writes the real config file that CLI reads, merging controlled fields only, with the previous file backed up first.
- **59 built-in presets** — select one, enter your API key, and switch. Import from CC-Switch, local config files, or a CC One backup; export the whole list as JSON anytime.
- **Model role mapping** — five roles (Sonnet / Opus / Fable / Haiku / Subagent), each with its own model and a 1M-context toggle; one click fetches the vendor's model list.
- **Structure syncs, keys never** — the provider list syncs per device; API keys are stripped from anything that leaves your machine.

### Dashboard

- **Four-bucket token economics** — input / output / cache creation / cache read, normalized across CLIs into one model that matches your bill.
- **Cache-hit rate, model distribution, requests & cost** — models ranked by tokens, each with its own cache-hit rate; totals frozen at collection time.
- **Trends & request log** — multi-metric trend chart with today-vs-yesterday delta; per-call log with token breakdown, cost, and full details on click.

### Sessions

- **Browsable history** — every conversation grouped by project, full-text search, filters by time / source / model / device.
- **Complete transcripts, instantly** — stored locally at collect time, rendered as markdown with syntax highlighting; in-session search with hit-highlighting and jump.
- **Local & Favorites** — private local groups that never leave this machine; favorited sessions sync across devices with source-device badges. Only favorited sessions ever leave your machine.

### Sync (optional)

- **Your own repo, plain text** — usage projected into human-readable, per-device, per-day JSONL in a GitHub repo you control; no third-party server in the middle.
- **Conflict-free by construction** — device-isolated writes, deterministic artifacts, self-healing rebases.
- **System-proxy aware**; dashboard, glance card, and tucked bar can be scoped to a single device.

### Library

- **Drag-to-relay upload** — drop files or directories into the window; they travel through your sync repo into that device's subtree.
- **In-app preview & manual export** — images, text/JSON, sandboxed rendering for the rest; export to a path you choose.

### Cost & pricing

- Editable per-model pricing, one-click LiteLLM cost-map fetch, rebill for records collected without a price, JSON import/export.

### Experience

- **Glance mode** — a tucked mini-bar or a floating card mirroring the dashboard; each shape retains its placement.
- **Multi-skin theming**, tray-resident background collection (5s–60s), signed auto-updates, UI in English / 简体中文 / 日本語.

## What stays local, what syncs

| | Standalone | Synced (repo bound) |
| --- | --- | --- |
| Usage, cost, trends, request log | Local only | Syncs across devices |
| Session transcripts & favorites | Local only | Favorited sessions sync; the rest stay local |
| Provider structure | Local only | Syncs per device — API keys never sync |
| Library files | Local only | Syncs across devices |
| Settings, skins, pricing overrides, local groups | Local | Local — never written to the repo |

Nothing leaves your machine unless you bind a repo and enable sync. The access token stays on your machine and is never written to the repo.

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

A [Tauri 2](https://tauri.app/) app: a Rust backend handles collection, the local store (the single source of truth), provider config writes, and optional sync; a React frontend renders the dashboard through generated, type-safe IPC bindings.

## Quick start

Download the installer for your OS from the **[Releases](https://github.com/Buktal/cc-one/releases)** page.

| OS | Installer |
| --- | --- |
| **Windows** | `.msi` or `.exe` (NSIS) setup |
| **macOS** | `.dmg` (Apple Silicon, arm64) |
| **Linux** | `.deb`, `.AppImage` (`.rpm` where available) |

**First run:** launch cc one — it scans local AI CLI session logs and the dashboard populates automatically. No account, no sign-in, no network. For cross-machine usage, enable sync in **Settings** with a GitHub repo you control.

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

## Contributing

Issues and suggestions are welcome. Before a PR, run `yarn check` and `yarn test`. For larger features, open an issue to discuss the approach first.

## License

[MIT](./LICENSE) © cc one Contributors

[![LINUX DO](https://img.shields.io/badge/LINUX%20DO-Recognized%20Community-blue?style=flat-square&logo=linux)](https://linux.do)

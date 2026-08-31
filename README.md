<div align="center">

# CC One

**Usage dashboard and provider manager for AI coding CLIs**

[![Release](https://img.shields.io/github/v/release/Buktal/cc-one)](https://github.com/Buktal/cc-one/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blueviolet)](https://github.com/Buktal/cc-one/releases/latest)

[English](README.md) | [简体中文](README.zh-CN.md) | [日本語](README.ja-JP.md)

<img src="./docs/images/dashboard-overview.png" alt="CC One dashboard" width="800">

</div>

CC One is a desktop application that turns the local session logs of AI coding CLIs into usage analytics — tokens, cost, trends and sessions — and manages their provider configurations. Data is stored locally in SQLite. When multi-device sync is enabled, data is exchanged through a private Git repository that you own; the access token never leaves the machine.

## Features

### Usage dashboard

- Five perspectives: overview, devices, projects, sessions and requests.
- Token accounting in four buckets — input, output, cache creation, cache read — with cache hit rate, request counts and cost.
- Usage trends, daily request volume, turn and duration distributions, model share, device and project rankings.
- Request-level log with per-request cost breakdown (input / output / cache read / cache write, billed model, stop reason).

### Session workbench

<table>
<tr>
<td><img src="./docs/images/sessions-workbench.png" alt="Session workbench" width="420"></td>
<td><img src="./docs/images/session-detail.png" alt="Session detail" width="420"></td>
</tr>
</table>

- Sessions navigated by project, group or favorites; batch favorite, group and delete.
- Full transcript reading with turn navigation, in-session search and message copy.
- Local groups stay on the device; favorites and favorite groups sync across devices.

### Provider management (Beta)

- A separate provider pool per app, with one active provider per app (OpenCode is managed additively: multiple providers coexist in its config).
- Built-in configuration presets, model role mapping (including Claude Code's 1M-context flag), and per-app shared config snippets merged under provider precedence.
- Switching rewrites only the fields the provider owns; all other fields in the config file are preserved. Writes are atomic, backed up, and idempotent when nothing changed.
- Import from CC-Switch and from live config files.

### Multi-device sync

- Data is exchanged through a private Git repository you own, over HTTPS with a personal access token; the token is stored only on the machine.
- Usage data, favorited sessions, favorite groups and the per-device file library are versioned in the repository, namespaced by device.

<table>
<tr>
<td><img src="./docs/images/floating-card.png" alt="Floating usage card" width="260"></td>
<td>

### Floating usage card

A compact card pinned to the screen edge shows today's consumption at a glance — token buckets, cache hit rate, requests and cost — and tucks itself away when the main window is minimized to the tray.

</td>
</tr>
</table>

### Also included

- Configurable model pricing: built-in price table, custom entries, LiteLLM fetch, import and export.
- Per-device file library, versioned through the sync repository.
- In-app auto-update via GitHub Releases.
- English / 日本語 / 中文 interface; light and dark themes with accent skins.
- Runs in the system tray with background collection.

## Supported CLIs

| CLI | Usage parsing | Provider management |
| --- | :---: | :---: |
| Claude Code | ✓ | ✓ single-active |
| Codex CLI | ✓ | ✓ single-active |
| Gemini CLI | ✓ | ✓ single-active |
| Grok CLI | ✓ | ✓ single-active |
| OpenCode | ✓ | ✓ additive |

## Installation

Download a package from [Releases](https://github.com/Buktal/cc-one/releases/latest):

| Platform | Package |
| --- | --- |
| Windows (x64) | `CC.One_*_x64-setup.exe` or `CC.One_*_x64_en-US.msi` |
| macOS (Apple Silicon) | `CC.One_*_aarch64.dmg` |
| Linux (x86_64) | `CC.One_*_amd64.deb` / `CC.One_*_1.x86_64.rpm` / `CC.One_*_amd64.AppImage` |

The application checks GitHub Releases for updates and upgrades itself in place.

## Building from source

Prerequisites: Node.js ≥ 20, Yarn 4, a Rust toolchain, and the [Tauri 2 platform dependencies](https://v2.tauri.app/start/prerequisites/).

```sh
yarn install
yarn dev      # run the app in development mode
yarn check    # lint and type checks (Biome, tsc, clippy, rustfmt)
yarn test     # test suites (Vitest, cargo test)
yarn dist     # build distributable packages
```

## License

[MIT](LICENSE)

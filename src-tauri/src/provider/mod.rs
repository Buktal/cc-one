//! Provider（供应商）写盘逻辑模块。
//!
//! 后端 `provider` module：`store`（DB CRUD，`db/store_providers.rs`）、
//! `sync`（per-device `providers.json` 结构同步，本目录 `sync.rs`）、`live`
//! （写盘分派点 `write_live(app, provider)`：claude 分支把 settingsConfig
//! 受控合并进 `~/.claude/settings.json`，codex 分支在 `live_codex`——TOML
//! 受控合并进 `~/.codex/config.toml` + 受控写 auth.json，gemini 分支待后续
//! 批次；各分支同一套「只合并受控字段、非受控字段原地保留、备份 + 原子
//! 写」语义）、`snippet`（通用配置片段：手写片段 + 启用开关，写盘时合并进
//! 受控字段，存本机 config.json 不同步）、`export_import`（全部供应商导出 /
//! 导入一份 JSON 文档，手动迁移，不走 git 同步），以及 `model_fetch`（拉取
//! 供应商的模型列表：OpenAI 兼容 `GET /v1/models`，候选 URL 构造是纯函数、
//! 失败错误串带分桶标签——见本目录 `model_fetch.rs`）。

pub mod export_import;
pub mod import_ccswitch;
pub mod import_live;
pub mod live;
pub mod live_codex;
pub mod live_gemini;
pub mod live_grok;
pub mod live_opencode;
pub mod model_fetch;
pub mod snippet;
pub mod sync;

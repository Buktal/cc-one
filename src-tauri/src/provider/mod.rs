//! Provider（供应商）写盘逻辑模块。
//!
//! 后端 `provider` module：`store`（DB CRUD，`db/store_providers.rs`）、
//! `sync`（per-device `providers.json` 结构同步，本目录 `sync.rs`）、`live`
//! （把供应商的 settingsConfig 合并写进用户本机 `~/.claude/settings.json`——
//! 写盘语义：只合并受控字段、非受控字段原地保留、备份 + 原子写）、`snippet`
//! （通用配置片段：手写片段 + 启用开关，写盘时合并进受控字段，存本机
//! config.json 不同步）、`export_import`（全部供应商导出 / 导入一份 JSON
//! 文档，手动迁移，不走 git 同步），以及 `model_fetch`（拉取供应商的模型
//! 列表：OpenAI 兼容 `GET /v1/models`，候选 URL 构造是纯函数、失败错误串
//! 带分桶标签——见本目录 `model_fetch.rs`）。

pub mod export_import;
pub mod live;
pub mod live_gemini;
pub mod model_fetch;
pub mod snippet;
pub mod sync;

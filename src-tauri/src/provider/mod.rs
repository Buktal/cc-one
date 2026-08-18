//! Provider（供应商）写盘逻辑模块。
//!
//! 后端 `provider` module：`store`（DB CRUD，`db/store_providers.rs`）、
//! `sync`（per-device `providers.json` 结构同步，本目录 `sync.rs`）、
//! `live_adapter`（per-app live 行为单一 seam：每个 app 的 live 文件 / 写盘 /
//! 快照 / 片段层策略 / 模型协议都在 `impl App` 一处声明，见本目录
//! `live_adapter.rs`——「按 App 分派」不再散落各处）、`live`（claude 分支写盘 +
//! 共用原语：受控合并 / 备份 / 原子写 / 无变化无操作，各分支同一套「只合并
//! 受控字段、非受控字段原地保留、备份 + 原子写」语义）、`live_codex` /
//! `live_gemini` / `live_grok` / `live_opencode`（其余 app 的写盘实现）、
//! `keys`（密钥位置清单 + strip / restore 成对纯函数：push / 导出剥、pull
//! 回填共用，见本目录 `keys.rs`）、`snippet`（通用配置片段：手写片段 + 启用
//! 开关，写盘时合并进受控字段，存本机 config.json 不同步）、`export_import`
//! （全部供应商导出 / 导入一份 JSON 文档，手动迁移，不走 git 同步）、`import`
//! （导入冲突规划 store 层 seam：归一化为 Provider 后按冲突键策略
//! (app,id) / name / liveKey 去重落库——导出文档 / CC-Switch / live 三条
//! 路径共用，见本目录 `import.rs`），以及 `model_fetch`（拉取供应商的模型
//! 列表：OpenAI 兼容 `GET /v1/models`，候选 URL 构造是纯函数、失败错误串带
//! 分桶标签——见本目录 `model_fetch.rs`）。

pub mod export_import;
pub mod import;
pub mod import_ccswitch;
pub mod import_live;
pub mod keys;
pub mod live;
pub mod live_adapter;
pub mod live_codex;
pub mod live_gemini;
pub mod live_grok;
pub mod live_opencode;
pub mod model_fetch;
pub mod snippet;
pub mod sync;

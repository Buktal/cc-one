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
//! `live_toml`（TOML 载体共用机制：live / target 解析、片段补缺失、片段校验
//! 与合并骨架——codex / grok 共用，经 `live` re-export）、
//! `keys`（密钥位置清单 + strip / restore 成对纯函数：push / 导出剥、pull
//! 回填共用，见本目录 `keys.rs`）、`snippet`（通用配置片段：手写片段 + 启用
//! 开关，写盘时合并进受控字段，存本机 config.json 不同步）、`export_import`
//! （全部供应商导出 / 导入一份 JSON 文档，手动迁移，不走 git 同步）、`import`
//! `import`（导入冲突规划 store 层 seam：归一化为 Provider 后按冲突键策略
//! (app,id) / name / liveKey 去重落库——导出文档 / CC-Switch / live 三条
//! 路径共用，见本目录 `import.rs`）、`settings_codec`（各 app settings_config
//! 形状编解码单源：字段名 / 密钥键名常量 + typed 值 ⇄ 文本的 build / parse
//! 双向，见本目录 `settings_codec.rs`）、以及 `model_fetch`（拉取供应商的
//! 模型列表：OpenAI 兼容 `GET /v1/models`，候选 URL 构造是纯函数、失败错误串
//! 带分桶标签——见本目录 `model_fetch.rs`）。`activation` 是激活编排（单激活
//! 「切换」、附加模式「加入 / 移出 live」与「删除供应商」的组合次序权威：
//! 「写盘成功才落激活态」「live 撤除成功才删行」，命令层只留薄壳，见本目录
//! `activation.rs`）。

pub mod activation;
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
pub(crate) mod live_toml;
pub mod model_fetch;
pub mod settings_codec;
pub mod snippet;
pub mod sync;

#[cfg(test)]
pub(crate) mod testutil;

// ------------------------------------------------------------ 安全 parity --
//
// ADR-0010 的第二道防线（架构审查候选⑩）：受控字段与凭据键模式的前端 TS 镜像
// （src/features/providers/snippet.ts）过去只靠注释里的「必须逐字一致」人肉
// 守护；现在 Rust 权威组装出 fixture JSON（repo 内
// src/features/providers/security-parity.json），本测试裁决它是否与权威同步，
// vitest 侧的 security-parity.test.ts 裁决 TS 镜像是否与同一份 fixture 等价
// ——两侧各自独立红灯，改任一侧都必须重新生成 fixture 并更新另一侧。
#[cfg(test)]
mod security_parity {
    use serde_json::json;

    use super::live::CONTROLLED_FIELDS;
    use super::snippet::{SENSITIVE_CONTAINS, SENSITIVE_EXACT, SENSITIVE_SUFFIXES};
    use crate::model::App;

    const FIXTURE_REL: &str = "../src/features/providers/security-parity.json";

    fn file_names(paths: Vec<std::path::PathBuf>) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    /// 从 Rust 权威组装期望值。live 文件名取自 `App::live_paths`（单一事实
    /// 来源，五个 app 同一条读取面）——不是手抄第二份字面量。
    fn authoritative_tables() -> serde_json::Value {
        let names = |app: App| -> Vec<String> {
            file_names(app.live_paths().expect("home dir resolves in tests"))
        };
        json!({
            "controlled_fields": CONTROLLED_FIELDS,
            "sensitive": {
                "exact": SENSITIVE_EXACT,
                "suffixes": SENSITIVE_SUFFIXES,
                "contains": SENSITIVE_CONTAINS,
            },
            "live_files": {
                "claude": names(App::Claude),
                "codex": names(App::Codex),
                "gemini": names(App::Gemini),
                "grok": names(App::Grok),
                "opencode": names(App::OpenCode),
            },
        })
    }

    #[test]
    fn ts_mirror_fixture_matches_security_authority() {
        let expected = authoritative_tables();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_REL);
        if std::env::var("UPDATE_SECURITY_PARITY").as_deref() == Ok("1") {
            let pretty = serde_json::to_string_pretty(&expected).unwrap();
            std::fs::write(&path, format!("{pretty}\n")).expect("write parity fixture");
        }
        let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "读取 parity fixture 失败（{e}）：{} —— 在仓库根运行 \
                 `UPDATE_SECURITY_PARITY=1 cargo test security_parity` 生成",
                path.display()
            )
        });
        let committed: serde_json::Value =
            serde_json::from_str(&raw).expect("fixture is valid JSON");
        assert_eq!(
            committed, expected,
            "parity fixture 已落后于 Rust 权威 —— 改了受控字段 / 凭据键表 / live 路径？\
             重新生成：UPDATE_SECURITY_PARITY=1 cargo test security_parity，\
             并同批次更新 TS 镜像（vitest security-parity.test.ts 会同时给 TS 侧判据）"
        );
    }
}

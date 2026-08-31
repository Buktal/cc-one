//! 从 CC-Switch 本机数据源读入原始数据：SQLite（`providers` / `settings` 表）
//! 与旧版 `config.json` → [`CcSwitchProvider`] / [`UniversalProvider`]。
//!
//! CC-Switch 导入分两半，本文件是**读源半**：把 CC-Switch 侧的原始数据宽容
//! 反序列化成转换的输入类型。**转换半**（父模块 [`super`]）做纯函数翻译 +
//! 统一供应商展开，由命令层喂 [`crate::provider::import`] 的 store 层 seam
//! 落库。读源在本模块做完、转换在父模块做完，命令层只留薄壳（`commands::
//! ccswitch`：拿连接 / 读文件 → 本模块 → 父模块 `collect_ccswitch_imports`）。
//!
//! 宽容原则：多余字段忽略、可选字段缺失归零；JSON 列 / 旧 JSON 解析失败按
//! null / `{}` 处理、解析不出的条目跳过——单条坏数据不让整个导入崩。
//!
//! 本模块不碰文件系统路径：SQLite 连接与 JSON 文本由命令层拿到后喂进来。

use rusqlite::{Connection, OptionalExtension};

use serde_json::Value;

use crate::error::{AppError, AppResult};
use crate::provider::import_ccswitch::{CcSwitchProvider, UniversalProvider};

/// 从 CC-Switch SQLite 读 `providers` 表全部行为 [`CcSwitchProvider`]。
/// `settings_config` / `meta` 列是 JSON 文本，这里解析为 [`Value`]；解析失败按
/// null / `{}` 处理（容错——单条坏数据不让整个导入崩）。
pub fn read_providers_from_db(conn: &Connection) -> AppResult<Vec<CcSwitchProvider>> {
    let mut stmt = conn.prepare(
        "SELECT id, app_type, name, settings_config, website_url, category, \
         sort_index, notes, icon, icon_color, meta FROM providers",
    )?;
    let rows = stmt.query_map([], |row| {
        let settings_text: String = row.get(3)?;
        let meta_text: String = row.get::<_, Option<String>>(10)?.unwrap_or_default();
        let settings_config: Value = serde_json::from_str(&settings_text).unwrap_or(Value::Null);
        let meta: Value =
            serde_json::from_str(&meta_text).unwrap_or_else(|_| Value::Object(Default::default()));
        Ok(CcSwitchProvider {
            id: row.get(0)?,
            app_type: row.get(1)?,
            name: row.get(2)?,
            settings_config,
            website_url: row.get::<_, Option<String>>(4)?,
            category: row.get::<_, Option<String>>(5)?,
            sort_index: row.get::<_, Option<i64>>(6)?,
            notes: row.get::<_, Option<String>>(7)?,
            icon: row.get::<_, Option<String>>(8)?,
            icon_color: row.get::<_, Option<String>>(9)?,
            meta,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AppError::Config(format!("读取 CC-Switch providers 表失败: {e}")))
}

/// 从 CC-Switch SQLite 读 `settings` 表的 `universal_providers`（统一供应商 map）。
/// 键不存在 → 空列表（CC-Switch 可能没有统一供应商）。
pub fn read_universals_from_db(conn: &Connection) -> AppResult<Vec<UniversalProvider>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'universal_providers'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let Some(text) = text else {
        return Ok(vec![]);
    };
    let map: std::collections::HashMap<String, UniversalProvider> = serde_json::from_str(&text)
        .map_err(|e| AppError::Config(format!("解析 universal_providers 失败: {e}")))?;
    Ok(map.into_values().collect())
}

/// 解析 CC-Switch 旧版 JSON（`config.json`，`MultiAppConfig` 结构）：`apps.<app_type>
/// .providers` 是 id → provider 的 map。id 取 map key、app_type 取外层 app key
/// （旧 JSON 的 provider 对象里通常没这俩字段，这里补上）。旧 JSON 不含统一供应商。
pub fn parse_legacy_json(text: &str) -> AppResult<(Vec<CcSwitchProvider>, Vec<UniversalProvider>)> {
    let v: Value = serde_json::from_str(text)
        .map_err(|e| AppError::Config(format!("CC-Switch config.json 解析失败: {e}")))?;
    let mut providers = Vec::new();
    if let Some(apps) = v.get("apps").and_then(|a| a.as_object()) {
        for (app_type, app_cfg) in apps {
            if let Some(pm) = app_cfg.get("providers").and_then(|p| p.as_object()) {
                for (id, prov) in pm {
                    let mut prov = prov.clone();
                    if let Some(obj) = prov.as_object_mut() {
                        obj.insert("id".into(), Value::String(id.clone()));
                        obj.insert("appType".into(), Value::String(app_type.clone()));
                    }
                    if let Ok(p) = serde_json::from_value::<CcSwitchProvider>(prov) {
                        providers.push(p);
                    }
                }
            }
        }
    }
    Ok((providers, vec![]))
}

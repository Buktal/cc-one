// Codex 应用的 settingsConfig codec：auth + TOML 受控合并的文本级读写。
//
// Codex settingsConfig 形状：`{"auth": {"OPENAI_API_KEY"?: ...}, "config":
// "<TOML 文本>"}`。`auth.OPENAI_API_KEY` 为空 / 缺失 = 登录态版（不写
// auth.json）；`config` 是 TOML 文本，写盘时按受控键整块合并进
// ~/.codex/config.toml（用户手动的非受控字段原样保留）。纯函数：写盘语义
// 的镜像，但更宽容（坏输入不抛、归一为空），用于表单读写。

import { parseJsonObjectLenient } from "@/lib/json"

type CodexConfig = { auth: Record<string, string>; config: string }

/** 解析 Codex settingsConfig 文本为 `{auth, config}`。宽容：空 / 垃圾 / 非对象
 *  → `{auth:{}, config:""}`；非对象 auth 当 `{}`、非字符串 config 当 `""`
 *  处理——表单遇到手改坏的快照也不崩，写回时按归一结果继续。 */
export function parseCodexConfig(text: string): CodexConfig {
  const obj = parseJsonObjectLenient(text)
  if (!obj) return { auth: {}, config: "" }
  const auth =
    obj.auth !== null &&
    typeof obj.auth === "object" &&
    !Array.isArray(obj.auth)
      ? Object.fromEntries(
          Object.entries(obj.auth as Record<string, unknown>).filter(
            (entry): entry is [string, string] => typeof entry[1] === "string",
          ),
        )
      : {}
  const config = typeof obj.config === "string" ? obj.config : ""
  return { auth, config }
}

/** 读 Codex settingsConfig 的 `auth.OPENAI_API_KEY`（API Key 版的凭据）。 */
export function codexApiKey(text: string): string {
  return parseCodexConfig(text).auth.OPENAI_API_KEY ?? ""
}

/** 读 Codex settingsConfig 的 `config`（config.toml 的 TOML 文本）。 */
export function codexConfigToml(text: string): string {
  return parseCodexConfig(text).config
}

/** 把 API Key 写入 Codex settingsConfig 的 `auth.OPENAI_API_KEY`：非空 key 写入、
 *  空 key 删除该键（回归登录态版）。保留 auth 其余键与 config 字段不动。 */
export function withCodexApiKey(text: string, key: string): string {
  const { auth, config } = parseCodexConfig(text)
  const next = { ...auth }
  if (key) next.OPENAI_API_KEY = key
  else delete next.OPENAI_API_KEY
  return JSON.stringify({ auth: next, config }, null, 2)
}

/** 把 TOML 文本写入 Codex settingsConfig 的 `config` 字段。保留 auth 与其余
 *  字段不动（受控合并的目标值就是这里整段 TOML）。 */
export function withCodexConfigToml(text: string, toml: string): string {
  const { auth } = parseCodexConfig(text)
  return JSON.stringify({ auth, config: toml }, null, 2)
}

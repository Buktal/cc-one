// 各应用供应商「草稿种子 / 保存收敛」的归属（架构审查候选①）。表单骨架
// （provider-form-sheet）不再内联任何 app 特殊步骤：真·新建时的种子注入与
// 保存时的物化/校验/归一/meta 记录都收进这里按 app 分派——此前这两步各有
// 一处 claude 专属逻辑且散在骨架里，其中「新建默认 1M」对全部 app 无门禁执
// 行，让非 claude 的空草稿/预设快照被 withEnvInText 无条件补出一个空的
// `env` 键并随保存落库。分派后非 claude 走显式直通分支，「哪一步是 claude
// 特殊的」只有此处一个答案。纯函数、可测。

import {
  MODEL_ROLES,
  normalizeBasicFieldsInText,
  withRoleOneMInText,
} from "@/features/providers/codecs/claude"
// 单向依赖：draft → derive（meta 记录用）；derive 不重导出本模块，
// 表单骨架直接从 codecs/draft 取这两个端口。
import { withMetaTemplateValues } from "@/features/providers/derive"
import {
  extractTemplateVars,
  replaceTemplateVarsInText,
} from "@/features/providers/template-vars"

import type { App } from "@/types/generated/bindings"

/** 真·新建（非编辑/复制）草稿的 settingsConfig 种子。目前只有 claude 有新
 *  建策略——给支持 1M 的角色模型加 `[1M]` 标记（空模型无标记可加，跳过）；
 *  其余 app 恒等返回，「{} 留成 {}」是语义决策而非默认落到 claude 规则再靠
 *  巧合不变量救回。 */
export function seedDraftText(app: App, baseText: string): string {
  if (app !== "claude") return baseText
  return MODEL_ROLES.reduce(
    (text, def) =>
      def.supportsOneM ? withRoleOneMInText(text, def.id, true) : text,
    baseText,
  )
}

/** 保存收敛的结果：ok 时给出最终 settingsConfig 与 meta 文本；不 ok 时列出
 *  未填值的 `${VAR}` 名（调用方据此提示），占位符绝不进持久化快照。 */
export type DraftFinalizeResult =
  | { ok: true; settingsConfig: string; meta: string }
  | { ok: false; unfilled: string[] }

/** 保存前的按 app 收敛：物化模板变量 → 残留校验 → 字段归一 → meta 记录。
 *  目前仍只有 claude 需要处理（模板变量物化与基础字段归一都是它的规则——
 *  模板变量走的是 settings.json 快照文本），其它 app 的 configText 即真相源，
 *  meta 原样保留。 */
export function finalizeDraft(
  app: App,
  configText: string,
  templateValues: Record<string, string>,
  baseMeta: string,
): DraftFinalizeResult {
  if (app !== "claude") {
    return { ok: true, settingsConfig: configText, meta: baseMeta }
  }
  const materialized = replaceTemplateVarsInText(configText, templateValues)
  const unfilled = extractTemplateVars(materialized)
  if (unfilled.length > 0) return { ok: false, unfilled }
  return {
    ok: true,
    // 归一从物化后的文本重读：端点收尾 trim、空端点/key 清键——输入过程不
    // trim，保存才 trim；字段值已直写 configText，归一不做二次合并。
    settingsConfig: normalizeBasicFieldsInText(materialized),
    // 变量值随 meta 记录供重编预填（live 文件不落 meta）；未用变量的空 map
    // 自动移除该键，保留未知 meta 键。
    meta: withMetaTemplateValues(baseMeta, templateValues),
  }
}

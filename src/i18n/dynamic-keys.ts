// i18n 动态键的封闭枚举表（架构扫描候选⑨a）：界面里 `t(\`prefix.${v}\`)` 形式
// 的动态键——三语键集一致性测试（locales.test.ts）只比较静态键，新增枚举值
// 不会触发它，界面就裸显键名。本表把每个动态键的取值域集中在这里，测试断言
// 「枚举值 × 三语键都存在」。每个表带两道锁：`satisfies` 防拼写错（单向——
// 子集也满足）；`Equal` 类型级完备检查防「权威 union 新增成员而本表没跟上」
// （双向，见文件底部 declare 检查）。两条锁住后，新增枚举值必然在编译期被
// 发现，三语键由测试断言兜底。

import {
  MODEL_ROLES,
  type ModelRoleId,
} from "@/features/providers/codecs/claude"
import type { MissingRequiredField } from "@/features/providers/missing"
import type { ModelsFetchErrorKind } from "@/features/providers/model-fetch"
import type {
  App,
  AppError,
  ProviderCategory,
} from "@/types/generated/bindings"

/** `providers.app.${app}` — 五个应用池。与 bindings 的 `App` union 一致。 */
export const APP_KEYS = [
  "claude",
  "codex",
  "gemini",
  "grok",
  "opencode",
] as const satisfies readonly App[]

/** `providers.category.${category}` — 供应商分类。与 bindings 的
 *  `ProviderCategory` union 一致。 */
export const CATEGORY_KEYS = [
  "official",
  "cn_official",
  "aggregator",
  "cloud_provider",
  "custom",
] as const satisfies readonly ProviderCategory[]

/** `providers.form.role.${role}` — 五角色模型。直接从 `MODEL_ROLES` 派生
 *  （表单显示顺序即此顺序），不在别处手抄。 */
export const ROLE_KEYS = MODEL_ROLES.map(
  (r) => r.id,
) satisfies readonly ModelRoleId[]

/** `providers.switchConfirm.missing.${m}` — 必填检查的缺失项。与
 *  `providerMissingRequired` 的返回类型（MissingRequiredField）一致。 */
export const MISSING_REQUIRED_FIELDS = [
  "endpoint",
  "apiKey",
  "templateVars",
] as const satisfies readonly MissingRequiredField[]

/** `providers.toast.fetchModels.${kind}` — 模型获取失败的分桶。与
 *  `ModelsFetchErrorKind` union 一致。 */
export const FETCH_MODEL_ERROR_KINDS = [
  "auth",
  "endpoint",
  "timeout",
  "format",
  "network",
] as const satisfies readonly ModelsFetchErrorKind[]

/** `providers.liveImport.extractGroups.${kind}` — 片段候选三组。live-import
 *  弹层的 GROUPS 渲染表以 `(typeof EXTRACT_GROUP_KINDS)[number]` 类型化，
 *  新增组必须同步本表（本表是权威源，无需 Equal 检查）。 */
export const EXTRACT_GROUP_KINDS = ["endpoint", "model", "behavior"] as const

/** `errors.${type}` — 结构化后端错误的类型（localizeStructuredError 按
 *  `errors.<type>` 取翻译键）。与 bindings 的 `AppError["type"]` union 一致。 */
export const APP_ERROR_TYPES = [
  "Config",
  "Db",
  "SourceParser",
  "Pricing",
  "Sync",
  "FetchModels",
  "Internal",
] as const satisfies readonly AppError["type"][]

// ---- 类型级完备检查 ----
//
// `satisfies` 是单向约束（枚举表 ⊆ 权威 union 即通过），防不了「权威新增
// 成员而表没跟上」——那正是本护栏要拦的漏键场景。Equal 用函数恒等技巧做
// 双向比较：表与权威 union 不完全相等 → 编译错误。`declare const` 不产生
// 运行时值，也不触发 noUnusedLocals。

type Equal<A, B> =
  (<T>() => T extends A ? 1 : 2) extends <T>() => T extends B ? 1 : 2
    ? true
    : false
type Expect<T extends true> = T

declare const _appKeysExhaustive: Expect<Equal<App, (typeof APP_KEYS)[number]>>
declare const _categoryKeysExhaustive: Expect<
  Equal<ProviderCategory, (typeof CATEGORY_KEYS)[number]>
>
declare const _roleKeysExhaustive: Expect<
  Equal<ModelRoleId, (typeof ROLE_KEYS)[number]>
>
declare const _missingFieldsExhaustive: Expect<
  Equal<MissingRequiredField, (typeof MISSING_REQUIRED_FIELDS)[number]>
>
declare const _fetchKindsExhaustive: Expect<
  Equal<ModelsFetchErrorKind, (typeof FETCH_MODEL_ERROR_KINDS)[number]>
>
declare const _errorTypesExhaustive: Expect<
  Equal<AppError["type"], (typeof APP_ERROR_TYPES)[number]>
>

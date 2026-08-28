// 应用（App）供应商管理能力事实的唯一归属（架构审查候选②）。后端在
// provider/live_adapter.rs 以 impl App 单点穷尽按应用分派；此前前端 counterpart
// 缺位——同类事实散在七个文件里以裸身份比较重新声明（isAdditive = app ===
// "opencode"、isTomlApp、LIVE_FILE 手抄表……），同一谓词甚至两份并存，加
// 第六个应用时没有任何机制提示该补哪些行。本表收编全部：
//
// - additive     单激活 / 附加模式（ADR-0007，镜像后端 App::is_additive_mode）
// - liveFile     live 配置文件名（标题与提示文案用；权威路径在后端
//                App::live_paths——改名需两侧同步，一致性由评审守护）
// - snippet      通用配置片段支持形态：none（附加模式无「切换」概念，片段
//                无处合并）/ write-layer(TOML 编辑器、合并发生在写盘层——
//                前端没有 live 全文，无法键级判定) / settings-config(JSON
//                编辑器、settings_config 层合并可键级子集判定，ADR-0010)
// - modelFetch   拉模型列表的参数提取；null = 该应用无此入口（codex / grok）。
//                per-app 差异只在这一层——错误分桶与结果填充走共用路径。
// - newDraftText 新建空草稿的 settingsConfig 形状（emptyProvider 用）
// - formPartition 表单分区组件（form-partition.ts 的契约）：provider-form-sheet
//                骨架一次查表渲染，不再持有任何 app 名字。
//
// 纪律：组件不再写 app === "xxx" 来表达这些事实，一律查表——加应用时补齐
// 本表一行即完成全部能力声明。依赖保持单向（本表 ← codecs/snippet/presets/
// model-fetch；derive 消费本表，本表绝不反向 import derive）。formPartition
// 引用的分区组件只 import codecs/* 与 form-partition 类型、绝不 import 本表
// 或 derive 聚合桶——否则 derive → 本表 → 分区 → derive 成环。

import {
  configApiKey,
  configEndpoint,
} from "@/features/providers/codecs/claude"
import { geminiApiKey, geminiBaseUrl } from "@/features/providers/codecs/gemini"
import {
  openCodeApiKey,
  openCodeBaseUrl,
} from "@/features/providers/codecs/opencode"
import { ClaudeFormFields } from "@/features/providers/components/claude-form-fields"
import { CodexFormFields } from "@/features/providers/components/codex-form-fields"
import { GeminiFormFields } from "@/features/providers/components/gemini-form-fields"
import { GrokFormFields } from "@/features/providers/components/grok-form-fields"
import { OpenCodeFormFields } from "@/features/providers/components/opencode-form-fields"
import type { FormPartition } from "@/features/providers/form-partition"
import {
  type FetchArgsResult,
  presetModelsUrl,
} from "@/features/providers/model-fetch"
import { PROVIDER_PRESETS } from "@/features/providers/presets"
import {
  geminiSnippetIssue,
  geminiSnippetMissingKeys,
  snippetMissingKeys,
} from "@/features/providers/snippet"

import type { App } from "@/types/generated/bindings"

/** 通用配置片段的按应用支持形态。判别联合本身就是 ADR-0010 的合并层分层：
 *  settings-config 层（claude/gemini）可键级判定、写盘层（codex/grok）不可、
 *  附加模式（opencode）根本没有切换动作可挂。 */
export type SnippetSupport =
  | { readonly kind: "none" }
  | { readonly kind: "write-layer" }
  | {
      readonly kind: "settings-config"
      /** 子集判定（镜像各自受控字段语义）：片段里出现、激活供应商配置里缺
       *  失的键；解析不了 → []（不误导）。 */
      readonly subsetKeys: (configText: string, snippetText: string) => string[]
      /** 片段草稿保存前问题（advisory 提示用，实际拒绝仍由后端）；无此检查
       *  的应用省略。 */
      readonly draftIssue?: (snippetText: string) => string | null
    }

/** 一个应用的供应商管理能力事实。字段全部只读——事实不随表单状态变化。 */
export interface AppProfile {
  readonly additive: boolean
  readonly liveFile: string
  readonly snippet: SnippetSupport
  readonly modelFetch: ((configText: string) => FetchArgsResult) | null
  readonly newDraftText: string
  /** 表单分区：该应用的字段区组件（claude 的模板变量/模型映射、opencode 的
   *  headers/models 编辑器、其余应用的直写字段……），从表单态出 props 的
   *  契约见 form-partition.ts。 */
  readonly formPartition: FormPartition
}

/** 单激活 OpenAI 兼容形状共用的「端点 + key 均必填」参数提取（claude 额外带
 *  modelsUrl 覆写、gemini 端点可空且形状固定，各自单列）。 */
function requireEndpointAndKey(
  app: App,
  baseUrl: string,
  apiKey: string,
  modelsUrl: string | null = null,
): FetchArgsResult {
  if (!baseUrl.trim()) return { ok: false, missing: "endpoint" }
  if (!apiKey.trim()) return { ok: false, missing: "key" }
  return { ok: true, args: { app, baseUrl, apiKey, modelsUrl } }
}

/** 各应用的编辑器语言由 snippet 形态决定（write-layer 写 TOML、其余 JSON）。 */
export function snippetSupportLanguage(
  support: SnippetSupport,
): "json" | "toml" {
  return support.kind === "write-layer" ? "toml" : "json"
}

export const APP_PROFILES: Record<App, AppProfile> = {
  claude: {
    additive: false,
    liveFile: "settings.json",
    snippet: {
      kind: "settings-config",
      subsetKeys: snippetMissingKeys,
    },
    modelFetch(configText) {
      // 端点等于某预设默认值时带上该预设声明的 modelsUrl 覆写（如火山
      // /api/compatible 拼不出正确候选，必须精确指路）。
      const baseUrl = configEndpoint(configText).trim()
      const result = requireEndpointAndKey(
        "claude",
        baseUrl,
        configApiKey(configText),
      )
      return result.ok
        ? {
            ok: true,
            args: {
              ...result.args,
              modelsUrl: presetModelsUrl(baseUrl, PROVIDER_PRESETS),
            },
          }
        : result
    },
    newDraftText: '{\n  "env": {}\n}',
    formPartition: ClaudeFormFields,
  },
  codex: {
    additive: false,
    liveFile: "config.toml",
    snippet: { kind: "write-layer" },
    modelFetch: null,
    newDraftText: "{}",
    formPartition: CodexFormFields,
  },
  gemini: {
    additive: false,
    liveFile: ".env",
    snippet: {
      kind: "settings-config",
      subsetKeys: geminiSnippetMissingKeys,
      // gemini 特有：凭据/端点/扁平顶层键问题警告（TS 镜像后端
      // is_sensitive_config_key 与 validate_snippet 的 gemini 分支）。
      draftIssue: geminiSnippetIssue,
    },
    modelFetch(configText) {
      // Gemini 端点形状固定（GET /v1beta/models），不走 modelsUrl 覆写；
      // 端点可空（后端 gemini_models_url 处理空→默认 generativelanguage 端
      // 点）——key 是唯一必填项。
      const key = geminiApiKey(configText).trim()
      if (!key) return { ok: false, missing: "key" }
      return {
        ok: true,
        args: {
          app: "gemini",
          baseUrl: geminiBaseUrl(configText),
          apiKey: key,
          modelsUrl: null,
        },
      }
    },
    newDraftText: "{}",
    formPartition: GeminiFormFields,
  },
  grok: {
    additive: false,
    liveFile: "config.toml",
    snippet: { kind: "write-layer" },
    modelFetch: null,
    newDraftText: "{}",
    formPartition: GrokFormFields,
  },
  opencode: {
    additive: true,
    liveFile: "opencode.json",
    snippet: { kind: "none" },
    modelFetch(configText) {
      // opencode 附加模式无登录态版——key 一律必填（附加模式的「启用」同样
      // 走这份校验：加入 live 后不可用不如先要求填齐）。
      return requireEndpointAndKey(
        "opencode",
        openCodeBaseUrl(configText),
        openCodeApiKey(configText),
      )
    },
    newDraftText: "{}",
    formPartition: OpenCodeFormFields,
  },
}

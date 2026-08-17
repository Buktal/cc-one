// Source tag → 来源可读名映射 (来源筛选下拉用).
//
// `source` 是每条用量记录上的稳定 source tag (即 RawUsage.source, 如
// "claude_code" / "codex_cli"). 筛选下拉显示可读名而非 snake_case 原值,
// 未知 tag 原样回退, 保证未来新增 source 在补映射前也能正常显示.
//
// 这里只给"平台名". request-log-table.tsx 的「来源」列复用本表 (原始 tag 放
// 该列 title 悬停提示), 不在此重抄映射.

import type { SourceTag } from "@/lib/source-tags"

const SOURCE_LABELS: Record<SourceTag, string> = {
  claude_code: "Claude Code",
  codex_cli: "Codex",
  gemini_cli: "Gemini CLI",
  opencode: "OpenCode",
  grok_cli: "Grok",
}

/** 把 source tag 转成展示名, 未知 tag 原样返回. */
export function sourceLabel(tag: string): string {
  // 查表入口以 string 索引（未知 tag 原样回退）；Record<SourceTag,…> 的键集
  // 完整性由类型守住——新增 SOURCE_TAGS 而漏译时这里编译失败。
  const lookup: Partial<Record<string, string>> = SOURCE_LABELS
  return lookup[tag] ?? tag
}

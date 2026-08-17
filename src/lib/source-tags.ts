// 跨视图共享：筛选「全部」哨兵、来源 tag 清单与标签查表。

/** 筛选下拉「全部」选项的哨兵值：各视图（来源 / 设备 / 模型 / 分组 / 库
 *  范围）的「all」选项共用同一哨兵串——value 为它即表示「不约束」（后端
 *  参数 null = 不约束，哨兵只活在 UI 层）。单一事实来源（#72：此前 7 处
 *  字面量各自定义，改值要改七个地方）。 */
export const ALL_FILTER = "__all__"

/** 五个用量来源（= 被解析的 CLI 应用）的稳定 source tag 清单——usage /
 *  sessions 两视图的筛选选项与标签映射都以它为键集（单一事实来源，#72：
 *  此前三处各自枚举，新增来源要同步改三张表、漏一处就漏翻译）。显示名分
 *  两种口径、归各视图的 source-labels：usage 短名（"Codex"）、sessions
 *  全名（"Codex CLI"）。 */
export const SOURCE_TAGS = [
  "claude_code",
  "codex_cli",
  "gemini_cli",
  "grok_cli",
  "opencode",
] as const

export type SourceTag = (typeof SOURCE_TAGS)[number]

/** 按运行时的 string tag 查完整标签表：已知 tag 命中表值，未知 tag 原样回退
 *  （新来源在补映射前也能正常显示）。表的键集完整性由
 *  Record<SourceTag,…> 类型守住——新增 SOURCE_TAGS 而漏译时编译失败；
 *  以 string 索引的降级只发生在这一处。 */
export function sourceTagLabel(
  labels: Record<SourceTag, string>,
  tag: string,
): string {
  const lookup: Partial<Record<string, string>> = labels
  return lookup[tag] ?? tag
}

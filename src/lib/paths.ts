// 路径展示的共享纯函数。归属 lib/（跨 feature 的派生计算归属地）：项目身份的
// basename 展示同时被会话侧（左树 / 卡片 / 表格）与全站项目筛选下拉消费。

/** Display basename of a project identity — the tree / cards / tables / the
 *  project filter dropdown show the short name with the full path on hover.
 *  Handles both path separators; a path that is all separators (or empty) has
 *  no basename and renders as-is (the caller decides the「未知项目」
 *  placeholder). */
export function projectBasename(dir: string): string {
  const trimmed = dir.replace(/[\\/]+$/, "")
  const slash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"))
  return slash === -1 ? trimmed : trimmed.slice(slash + 1)
}

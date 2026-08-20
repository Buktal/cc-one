// App shell: topbar (navigation + status + actions + window controls, see
// title-bar.tsx) + scrollable content. View switching via viewSlice (no
// react-router); the active view is rendered by App. 侧栏 / 竖屏 TopNav /
// StatusBar 已撤（#105 定稿 variant-a-v2：导航并入标题栏一行，竖屏沿用同一
// 套断点退化）；更新日志与版本从 shell 移出，由设置页「关于」区承接。

import { TitleBar } from "./title-bar"

export function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="bg-app text-foreground flex h-screen w-screen flex-col overflow-hidden">
      <TitleBar />
      {/* pt-3：顶栏（Windows/Linux 态有底线）与内容的间隔；px-4/pb-4 沿用
          侧栏时代的四周留白，看板与日志在宽屏铺满贴边（窄内容如 settings
          各自内部 max-w 居中）。 */}
      <div className="flex min-h-0 flex-1 overflow-hidden px-4 pb-4 pt-3">
        {/* min-h-0: the main is a flex item on the column's main axis, where
            min-height:auto would let tall content stretch it past the viewport
            instead of scrolling internally. min-h-full: 内容至少占满滚动容器，
            但允许更高——超高内容（如 providers 的多卡片堆叠）由外层
            overflow-auto 滚动，而不是被 h-full 锁死在视口高度上裁掉。 */}
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          <div className="flex-1 overflow-auto">
            <div className="flex min-h-full w-full flex-col">{children}</div>
          </div>
        </main>
      </div>
    </div>
  )
}

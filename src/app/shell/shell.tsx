// App shell: topbar (navigation + status + actions + window controls, see
// title-bar.tsx) + content. View switching via viewSlice (no react-router);
// the active view is rendered by App. 侧栏 / 竖屏 TopNav / StatusBar 已撤
// （#105 定稿 variant-a-v2：导航并入标题栏一行，竖屏沿用同一套断点退化）；
// 更新日志与版本从 shell 移出，由设置页「关于」区承接。
//
// 内容区两种滚动模型，由视图声明（Shell 的 fill prop）：
// - 文档型（默认）：内容包在 overflow-auto 滚动 wrapper 里，高度随内容
//   增长，超高由外层滚动条承接（providers / settings 的长卡片堆叠）。
// - 满高型（fill）：内容直接挂 main 的 flex 列，高度严格 = main 分配的
//   视口剩余空间，不存在外层滚动——各面板必须自带滚动容器（sessions
//   工作台）。不能用 min-h-full/max-h-full 之类百分比在这两种模型间折
//   补：滚动 wrapper 的高度是 auto，Chromium 下子元素的百分比 max-height
//   解析为 none，锁不住。

import { TitleBar } from "./title-bar"

export function Shell({
  children,
  fill = false,
}: {
  children: React.ReactNode
  /** 满高型视图：跳过滚动 wrapper，视图高度被 main 的 flex 分配夹死。 */
  fill?: boolean
}) {
  return (
    <div className="bg-app text-foreground flex h-screen w-screen flex-col overflow-hidden">
      <TitleBar />
      {/* pt-3：顶栏（Windows/Linux 态有底线）与内容的间隔；px-4/pb-4 沿用
          侧栏时代的四周留白，看板与日志在宽屏铺满贴边（窄内容如 settings
          各自内部 max-w 居中）。 */}
      <div className="flex min-h-0 flex-1 overflow-hidden px-4 pb-4 pt-3">
        {/* min-h-0: the main is a flex item on the column's main axis, where
            min-height:auto would let tall content stretch it past the viewport
            instead of scrolling internally. */}
        <main className="flex min-h-0 min-w-0 flex-1 flex-col">
          {fill ? (
            children
          ) : (
            /* min-h-full: 内容至少占满滚动容器，但允许更高——超高内容（如
               providers 的多卡片堆叠）由这层 overflow-auto 滚动，而不是被
               h-full 锁死在视口高度上裁掉。
               -mr-4 + pr-2: 滚动条「坐进」右留白——负外边距抵消外层的
               pr-4，滚动条贴到窗口右缘；8px 空隙 + 8px 轨道 = 16px，与左
               留白严格相等（左右空白对称），滚动条也不再贴着卡片边缘。 */
            <div className="flex-1 -mr-4 overflow-auto pr-2">
              <div className="flex min-h-full w-full flex-col">{children}</div>
            </div>
          )}
        </main>
      </div>
    </div>
  )
}

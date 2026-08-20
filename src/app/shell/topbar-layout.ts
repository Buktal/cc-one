// 顶栏平台布局参数（#105 定稿 variant-a-v2-left-nav；平台窗口控制结论抄自
// O_DeepSeek_Desktop ADR 0003）。macOS 走系统红绿灯（tauri.conf 的
// titleBarStyle Overlay + trafficLightPosition），顶栏只负责让出空间且不画
// 底线——Overlay 下系统标题栏是透明覆盖，画线/背景块会形成双层标题栏视觉；
// Windows/Linux 由 Rust builder 的 cfg(target_os) 分支关系统装饰，顶栏右侧
// 自绘三键贴右缘（46px × 满高，同系统按钮布局）。
//
// 全部纯函数：模块顶层不触碰 window / navigator（architecture.md 的外部资源
// 句柄规则），vitest 纯 node 环境可安全 import。

export type Platform = "macos" | "windows" | "linux"

/** UA → 平台。未知/缺失一律按 linux 兜底（兜底侧与 Linux 真机同形：自绘
 *  三键 + drag region；真机 UA 恒存在，兜底只影响测试/异常环境）。 */
export function detectPlatform(ua: string): Platform {
  if (/windows/i.test(ua)) return "windows"
  if (/macintosh|mac os x/i.test(ua)) return "macos"
  return "linux"
}

/** 当前平台（渲染期调用；navigator 缺失时按 linux 兜底，测试环境安全）。 */
export function currentPlatform(): Platform {
  return detectPlatform(
    typeof navigator === "undefined" ? "" : navigator.userAgent,
  )
}

/** 顶栏按平台的形态参数。
 *  - macos：左侧 84px 避让系统红绿灯（trafficLightPosition x:13 + 三粒
 *    12px 与 2×8px 间距 + 19px 间隔），无底部分隔线，右侧无自绘窗口控制。
 *  - windows / linux：左常规内边距、右零内边距（自绘三键贴真右缘），底部分
 *    隔线与内容区划界。 */
export interface TopbarLayout {
  paddingClass: string
  borderClass: string
  /** 行内是否渲染自绘窗口控制（macOS 用系统红绿灯，不渲染）。 */
  windowControls: boolean
}

export function topbarLayout(platform: Platform): TopbarLayout {
  switch (platform) {
    case "macos":
      return {
        paddingClass: "pr-2 pl-[84px]",
        borderClass: "",
        windowControls: false,
      }
    case "windows":
    case "linux":
      return {
        paddingClass: "pr-0 pl-3.5",
        borderClass: "border-border border-b",
        windowControls: true,
      }
  }
}

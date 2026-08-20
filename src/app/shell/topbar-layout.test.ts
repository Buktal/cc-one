// 平台探测与顶栏形态参数的表驱动测试：UA 判定、未知兜底 linux、mac/win 两
// 形态的避让/底线/自绘三键开关（#105 验收点 mac 避让 + win 贴缘的纯函数层）。

import { describe, expect, it } from "vitest"

import { currentPlatform, detectPlatform, topbarLayout } from "./topbar-layout"

describe("detectPlatform", () => {
  const cases: Array<{ ua: string; want: ReturnType<typeof detectPlatform> }> =
    [
      // Windows：NT 内核 UA（Chromium/WebView2 同串）。
      {
        ua: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/126.0 Safari/537.36",
        want: "windows",
      },
      // macOS：Macintosh。
      {
        ua: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/17.4 Safari/605.1.15",
        want: "macos",
      },
      // Linux：X11 / Linux 字样（Linux 上无 "windows"/"mac" 命中词）。
      {
        ua: "Mozilla/5.0 (X11; Linux x86_64; rv:127.0) Gecko/20100101 Firefox/127.0",
        want: "linux",
      },
      // 未知 / 空 UA：兜底 linux（与 Linux 真机同形）。
      { ua: "", want: "linux" },
      { ua: "some-unknown-agent", want: "linux" },
    ]
  it.each(cases)("$ua → $want", ({ ua, want }) => {
    expect(detectPlatform(ua)).toBe(want)
  })

  it("currentPlatform falls back to linux without navigator", () => {
    // node 环境（vitest 默认）无 navigator —— 兜底 linux，不抛异常。
    expect(currentPlatform()).toBe("linux")
  })
})

describe("topbarLayout", () => {
  it("macos: 红绿灯避让 + 无底线 + 系统窗口控制（不自绘）", () => {
    const m = topbarLayout("macos")
    expect(m.paddingClass).toContain("pl-[84px]")
    expect(m.borderClass).toBe("")
    expect(m.windowControls).toBe(false)
  })

  it("windows / linux: 常规左内边 + 右零内边（三键贴缘）+ 底线 + 自绘三键", () => {
    for (const p of ["windows", "linux"] as const) {
      const l = topbarLayout(p)
      expect(l.paddingClass).toContain("pr-0")
      expect(l.paddingClass).toContain("pl-3.5")
      expect(l.borderClass).toContain("border-b")
      expect(l.windowControls).toBe(true)
    }
  })
})

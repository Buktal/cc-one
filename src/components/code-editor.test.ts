// CodeEditor 写回管道的纯决策层测试。编辑器本体挂 CodeMirror 6（需要 DOM），
// node-only vitest 测不了组件——与 collapse-trigger 同一策略：把可测的决策
// 提纯成模块级纯函数，测试跑的正是五处调用点运行的那份生产代码。TOML 整理
// 后端用 fake trigger 注入，验证 formatDoc 的语言分派与容错契约。

import { describe, expect, it, vi } from "vitest"

import { formatDoc, planDocApply } from "@/components/code-editor"

/** TOML 整理后端 fake：无视入参，按预设结果 resolve。少参函数可赋给
 *  FormatTomlBackend（参数逆变），toHaveBeenCalledWith 照常核对原文送入。 */
function fakeTomlBackend(result: { data?: string; error?: unknown }) {
  return vi.fn(() => Promise.resolve(result))
}

describe("planDocApply", () => {
  const snapshot = "current doc"

  it("doc 仍是快照且整理改变了内容 → write（整篇替换并回传）", () => {
    expect(planDocApply(snapshot, snapshot, "formatted")).toBe("write")
  })

  it("doc 已是目标形（幂等）→ settle：不发空事务但仍回传（粘贴原文必须补传）", () => {
    expect(planDocApply(snapshot, snapshot, snapshot)).toBe("settle")
  })

  it("用户在异步整理间隙已输入（doc ≠ 快照）→ stale：不写回不覆盖", () => {
    expect(planDocApply("user typed more", snapshot, "formatted")).toBe("stale")
  })

  it("stale 优先于 settle：内容虽幂等，用户的编辑也不被触碰", () => {
    expect(planDocApply("user typed more", snapshot, snapshot)).toBe("stale")
  })
})

describe("formatDoc", () => {
  it("json：本地 tidyJson（排版 + 键字母序），不触达 TOML 后端", async () => {
    const backend = fakeTomlBackend({ data: "SHOULD NOT BE USED" })
    const out = await formatDoc("json", '{"b":1,"a":2}', backend)
    expect(out).toBe('{\n  "a": 2,\n  "b": 1\n}')
    expect(backend).not.toHaveBeenCalled()
  })

  it("json：容错——无效 JSON（截断）也展开成可读结构，不抛错", async () => {
    const out = await formatDoc("json", '{"a":1,', fakeTomlBackend({}))
    expect(out).toBe('{\n  "a": 1,')
  })

  it("toml：走注入后端，原文送入，返回其整理结果", async () => {
    const backend = fakeTomlBackend({ data: '# tidy\nkey = "v"\n' })
    const out = await formatDoc("toml", 'key="v"', backend)
    expect(backend).toHaveBeenCalledWith('key="v"')
    expect(out).toBe('# tidy\nkey = "v"\n')
  })

  it("toml：后端报错保持原文（容错契约：调用管道无失败分支）", async () => {
    const backend = fakeTomlBackend({ error: { code: "backend" } })
    const out = await formatDoc("toml", 'key="v"', backend)
    expect(out).toBe('key="v"')
  })

  it("toml：后端空手响应（无 data）同样保持原文", async () => {
    const backend = fakeTomlBackend({})
    const out = await formatDoc("toml", 'key="v"', backend)
    expect(out).toBe('key="v"')
  })
})

// ADR-0010 的第二道防线（架构审查候选⑩，vitest 侧）：TS 镜像表必须与 Rust
// 权威生成的 fixture 完全一致。链路：Rust 权威（provider::live 受控字段、
// provider::snippet 敏感键三表、App::live_paths / opencode 存储路径）
// → UPDATE_SECURITY_PARITY=1 cargo test security_parity 生成本目录
// security-parity.json → 本文件裁决 TS 镜像与该 fixture 等价。漂移任意一侧
// 都会在对应测试套件红灯——「后端拦了、前端还能写回」的人肉守护由此退役。

import { readFileSync } from "node:fs"

import { describe, expect, it } from "vitest"

import type { App } from "@/types/generated/bindings"
import { APP_PROFILES } from "./app-profiles"
import {
  CONTROLLED_FIELDS,
  SENSITIVE_CONTAINS,
  SENSITIVE_EXACT,
  SENSITIVE_SUFFIXES,
} from "./snippet"

const fixture: {
  controlled_fields: string[]
  sensitive: { exact: string[]; suffixes: string[]; contains: string[] }
  live_files: Record<App, string[]>
} = JSON.parse(
  readFileSync(new URL("./security-parity.json", import.meta.url), "utf-8"),
)

/** 集合等价 + 数量相等 = 内容逐项一致且无重复（顺序无关——两张端对匹配
 *  语义上都是集合谓词；表顺序不参与任何行为）。 */
function expectSameItems(
  actual: readonly string[],
  expected: readonly string[],
) {
  expect([...actual].sort()).toEqual([...expected].sort())
  expect(new Set(actual).size).toBe(actual.length)
}

describe("security parity — TS mirror vs Rust authority", () => {
  it("controlled fields match the write-path authority", () => {
    expectSameItems(CONTROLLED_FIELDS, fixture.controlled_fields)
  })

  it("sensitive-key tables (exact / suffixes / contains) are verbatim", () => {
    // ADR-0010：前后端判定不一致的后果是「后端拦了、前端还能写回」。
    expectSameItems(SENSITIVE_EXACT, fixture.sensitive.exact)
    expectSameItems(SENSITIVE_SUFFIXES, fixture.sensitive.suffixes)
    expectSameItems(SENSITIVE_CONTAINS, fixture.sensitive.contains)
  })

  it("per-app live config file names agree with backend paths", () => {
    // app-profiles.liveFile 是后端 App::live_paths 的 UI 镜像（下拉标题 /
    // 提示文案）；每行取权威路径序列的首个文件名核对，opencode 是附加模式
    // 的存储文件 opencode.json。
    for (const app of Object.keys(fixture.live_files) as App[]) {
      expect(APP_PROFILES[app].liveFile).toBe(fixture.live_files[app][0])
    }
  })
})

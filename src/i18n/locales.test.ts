// 三语 locale 键集一致性护栏：zh / en / ja 的键集合必须完全一致（复数后缀
// 归一后比较——en 用 i18next 复数键 `*_one`/`*_other`，zh/ja 用单键表达同一
// 概念）。三语漂移已发生两次（#62：ja 缺 17 键致 fallbackLng=en 混入英文、
// zh/en 缺 live.added 裸显键名），缺键在界面上的表现是裸键名或英文串入，
// 测试在 CI 层拦住第三次。

import { describe, expect, it } from "vitest"

import en from "@/locales/en.json"
import ja from "@/locales/ja.json"
import zh from "@/locales/zh.json"

/** i18next 复数后缀（en 实际只用 _one/_other，全集一并归一）。 */
const PLURAL_SUFFIX = /_(one|other|zero|two|few|many)$/

function normalizedKeys(locale: Record<string, string>): Set<string> {
  return new Set(Object.keys(locale).map((k) => k.replace(PLURAL_SUFFIX, "")))
}

describe("locale key sets stay identical across zh/en/ja", () => {
  const sets = {
    zh: normalizedKeys(zh),
    en: normalizedKeys(en),
    ja: normalizedKeys(ja),
  }

  it("zh and en cover the same keys", () => {
    expect([...sets.en].filter((k) => !sets.zh.has(k))).toEqual([])
    expect([...sets.zh].filter((k) => !sets.en.has(k))).toEqual([])
  })

  it("zh and ja cover the same keys", () => {
    expect([...sets.ja].filter((k) => !sets.zh.has(k))).toEqual([])
    expect([...sets.zh].filter((k) => !sets.ja.has(k))).toEqual([])
  })

  it("en plural keys always come in _one/_other pairs", () => {
    const raw = Object.keys(en)
    for (const k of raw.filter((k) => PLURAL_SUFFIX.test(k))) {
      const base = k.replace(PLURAL_SUFFIX, "")
      const suffix = k.slice(base.length)
      if (suffix === "_one") {
        expect(
          raw.includes(`${base}_other`),
          `复数键 ${k} 缺 _other 搭档（i18next en 需要 _one/_other 成对）`,
        ).toBe(true)
      }
    }
  })
})

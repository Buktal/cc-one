// 三语 locale 键集一致性护栏：zh / en / ja 的键集合必须完全一致（复数后缀
// 归一后比较——en 用 i18next 复数键 `*_one`/`*_other`，zh/ja 用单键表达同一
// 概念）。三语漂移已发生两次（#62：ja 缺 17 键致 fallbackLng=en 混入英文、
// zh/en 缺 live.added 裸显键名），缺键在界面上的表现是裸键名或英文串入，
// 测试在 CI 层拦住第三次。

import { describe, expect, it } from "vitest"

import {
  APP_ERROR_TYPES,
  APP_KEYS,
  CATEGORY_KEYS,
  EXTRACT_GROUP_KINDS,
  FETCH_MODEL_ERROR_KINDS,
  MISSING_REQUIRED_FIELDS,
  ROLE_KEYS,
} from "@/i18n/dynamic-keys"
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

/** 动态键护栏（i18n/dynamic-keys.ts 的封闭枚举）：`t(\`prefix.${v}\`)` 的
 *  每个取值在 zh/en/ja 都必须有键——静态键集比较测不到它们，新增枚举值
 *  时这套断言是唯一的拦截（漏键的界面表现是裸键名）。 */
describe("dynamic i18n keys exist in all three locales", () => {
  const dynamicKeyCases: [string, readonly string[]][] = [
    ["providers.app", APP_KEYS],
    ["providers.category", CATEGORY_KEYS],
    ["providers.form.role", ROLE_KEYS],
    ["providers.switchConfirm.missing", MISSING_REQUIRED_FIELDS],
    ["providers.toast.fetchModels", FETCH_MODEL_ERROR_KINDS],
    ["providers.liveImport.extractGroups", EXTRACT_GROUP_KINDS],
    ["errors", APP_ERROR_TYPES],
  ]
  const locales = { zh, en, ja }

  for (const [prefix, values] of dynamicKeyCases) {
    for (const value of values) {
      const key = `${prefix}.${value}`
      it(`${key} exists in zh/en/ja`, () => {
        for (const [name, locale] of Object.entries(locales)) {
          expect(
            normalizedKeys(locale).has(key),
            `${key} 缺于 ${name}（动态键 ${prefix}.\${value} 的取值）`,
          ).toBe(true)
        }
      })
    }
  }
})

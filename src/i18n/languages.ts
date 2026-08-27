// Supported display languages. The single source of truth that the
// Rust `Language` enum, this registry, the `src/locales/*.json` files, and the
// dayjs locale map must all stay in agreement with. To add a language: extend
// Rust `Language`, drop a JSON in `src/locales/`, register it here, and add the
// dayjs locale import below.

import dayjs from "dayjs"
import relativeTime from "dayjs/plugin/relativeTime"
import "dayjs/locale/ja"
import "dayjs/locale/zh-cn"

import type { Language } from "@/types/generated/bindings"

// relativeTime（`fromNow()`）的唯一注册点（架构审查Ⅲ候选②）。dayjs 插件是
// 全局可变状态，注册曾散落 5 处组件，还有用 fromNow 却从未注册的面
//（usage 设备分区）——后者靠别的模块先 import 的顺序运气活着，隔离单测一
// import 就抛异常。收进本文件（dayjs 语言层的归属地）后，i18n 入口
//（@/i18n，providers 必经）import 本模块即注册就绪。extend 幂等，重复注册
// 无副作用。
dayjs.extend(relativeTime)

export interface LanguageOption {
  /** Rust `Language` code (serde lowercase). */
  code: Language
  /** Native-name label shown in the selector — so users find theirs regardless of the active UI language. */
  nativeName: string
  /** dayjs locale name; drives relative-time (`fromNow`). */
  dayjsLocale: string
}

export const LANGUAGES: readonly LanguageOption[] = [
  { code: "en", nativeName: "English", dayjsLocale: "en" },
  { code: "zh", nativeName: "中文", dayjsLocale: "zh-cn" },
  { code: "ja", nativeName: "日本語", dayjsLocale: "ja" },
]

const byCode = new Map<Language, LanguageOption>(
  LANGUAGES.map((o) => [o.code, o]),
)

/** Set dayjs's global locale so `fromNow()` follows the display language. */
export function setDayjsLocale(code: Language): void {
  dayjs.locale(byCode.get(code)?.dayjsLocale ?? "en")
}

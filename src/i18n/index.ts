// i18next init. Resources are inlined (single-bundle Tauri app — no
// lazy loading needed). The language is NOT auto-detected from navigator or
// localStorage; it is driven by the persisted Rust preference via
// `i18n.changeLanguage` in <LanguageSync> (providers.tsx). Defaults to English,
// matching the app default and the dayjs default — so there is no flash of a
// wrong language before the preference query resolves.

import i18n from "i18next"
import { initReactI18next } from "react-i18next"

// dayjs 的语言层（locale 注册 + relativeTime 插件，即 fromNow）与 i18next
// 同属「展示语言」的全局配置——import 本模块即一并就绪（架构审查Ⅲ候选②，
// 此前插件注册散落各组件）。
import "@/i18n/languages"
import en from "@/locales/en.json"
import ja from "@/locales/ja.json"
import zh from "@/locales/zh.json"

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    zh: { translation: zh },
    ja: { translation: ja },
  },
  lng: "en",
  fallbackLng: "en",
  // Flat dotted keys (e.g. "settings.general.collectInterval") — the hierarchy
  // lives in the key name, not in nested JSON. Keeps the locale files flat,
  // sortable, and trivially diff-able for key-consistency checks.
  keySeparator: false,
  interpolation: { escapeValue: false },
  returnNull: false,
})

export default i18n

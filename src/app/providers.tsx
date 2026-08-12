// App providers: Redux <Provider> + next-themes <ThemeProvider> +
// base-ui <TooltipProvider> + Toaster. next-themes toggles `.dark` on <html>;
// attribute="class" matches the `@custom-variant dark` in index.css. The Toaster
// relies on useTheme() so the ThemeProvider must wrap it, else toasts never
// follow the active theme. TooltipProvider is mounted once here so every
// <Tooltip> in the tree shares delay/hover config without re-wrapping.

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { ThemeProvider, useTheme } from "next-themes"
import type { ReactNode } from "react"
import { useEffect } from "react"
import { Provider } from "react-redux"
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import "@/i18n"
import { useSkinEffect } from "@/hooks/use-skin"
import { LanguageSync } from "@/i18n/LanguageSync"

import { CloseRequestedDialog } from "./close-requested-dialog"
import { vaultApi } from "./store/api"
import { setMode } from "./store/slices/viewSlice"
import { store } from "./store/store"
import { INVALIDATE_STORE } from "./store/tags"

/** Reflects the persisted color skin onto <html data-skin> (multi-skin
 *  theming). Must live inside the Redux <Provider> — it reads prefs. */
function SkinEffect() {
  useSkinEffect()
  return null
}

/** Pushes the resolved (actual) dark/light theme to the Rust tray so its icon
 *  badge matches the sidebar. Must live inside <ThemeProvider> — it reads
 *  useTheme(). Pushes resolvedTheme (not the user's "system" choice): `system`
 *  only resolves here via the OS theme, and the tray needs a concrete icon. */
function TrayThemeSync() {
  const { resolvedTheme } = useTheme()
  useEffect(() => {
    void invoke("set_tray_theme", { dark: resolvedTheme !== "light" })
  }, [resolvedTheme])
  return null
}

export function AppProviders({ children }: { children: ReactNode }) {
  // Event-driven refresh: Rust emits `usage_changed` after any whole-Store
  // write (collect / sync). Invalidate the aggregate `Store` tag so every
  // Store-derived read (usage / logs / models / devices / sessions / synced
  // providers) refetches — see src/app/store/tags.ts for the single source of
  // truth. One tag replaces a per-domain list that had drifted (the collect /
  // sync mutations once forgot Sessions).
  useEffect(() => {
    const off = listen("usage_changed", () => {
      store.dispatch(vaultApi.util.invalidateTags(INVALIDATE_STORE))
    })
    return () => {
      off.then((unlisten) => unlisten())
    }
  }, [])

  // Mirror of `usage_changed` for session writes (favorite / group / title /
  // group CRUD). Backend emits `sessions_changed` after each write; invalidate
  // the whole `Sessions` tag so every active session query (list + the open
  // transcript) refetches.
  useEffect(() => {
    const off = listen("sessions_changed", () => {
      store.dispatch(vaultApi.util.invalidateTags(["Sessions"]))
    })
    return () => {
      off.then((unlisten) => unlisten())
    }
  }, [])

  // Mirror of `usage_changed` for provider CRUD. Backend emits
  // `providers_changed` after each provider write; invalidate the whole
  // `Providers` tag so the active provider list refetches in place.
  useEffect(() => {
    const off = listen("providers_changed", () => {
      store.dispatch(vaultApi.util.invalidateTags(["Providers"]))
    })
    return () => {
      off.then((unlisten) => unlisten())
    }
  }, [])

  // Tray left-click means "show the full dashboard". If the
  // window is in lightweight mode, morph back — setMode("full") is a no-op when
  // already full, and useWindowMode restores the window geometry on the change.
  useEffect(() => {
    const off = listen("tray-show-main", () => {
      store.dispatch(setMode("full"))
    })
    return () => {
      off.then((unlisten) => unlisten())
    }
  }, [])

  return (
    <Provider store={store}>
      <LanguageSync />
      <SkinEffect />
      <ThemeProvider
        attribute="class"
        defaultTheme="dark"
        enableSystem
        disableTransitionOnChange
      >
        <TrayThemeSync />
        <TooltipProvider>
          {children}
          <CloseRequestedDialog />
          <Toaster richColors closeButton />
        </TooltipProvider>
      </ThemeProvider>
    </Provider>
  )
}

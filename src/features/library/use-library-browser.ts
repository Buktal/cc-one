// Library browser state + actions, extracted from LibraryView so the
// component shrinks to pure rendering. Owns: the scan/devices queries + the
// three mutations, the device-scope / subpath navigation state, webview
// drag-drop upload collection, per-row rename/export/delete busy state, the
// preview target, and the derived device picker + breadcrumb.
//
// The Tauri webview handle (getCurrentWebview) is fetched lazily inside the
// drag-drop effect — never at module top — so this module imports cleanly in
// vitest's node-only environment (architecture.md: "外部资源句柄延迟获取").

import { getCurrentWebview } from "@tauri-apps/api/webview"
import { open } from "@tauri-apps/plugin-dialog"
import { useEffect, useMemo, useState } from "react"
import { useTranslation } from "react-i18next"
import {
  useDeleteFromLibraryMutation,
  useDevicesQuery,
  useExportFromLibraryMutation,
  useRenameInLibraryMutation,
  useScanLibraryQuery,
} from "@/app/store/api"
import { usePagedBrowser } from "@/hooks/use-paged-browser"
import { useMutateWithToast } from "@/hooks/use-toast-mutation"
import { deviceOptionLabel } from "@/lib/device-labels"
import { DEFAULT_PAGE_SIZE } from "@/lib/pagination"
import { usePersistedState } from "@/lib/persistence"
import type { LibraryEntry } from "@/types/generated/bindings"
import {
  buildBreadcrumb,
  filterEntriesByName,
  splitEntryPath,
  upFromSubpath,
} from "./derive"

// 每页条数密度跨重启记忆（键名沿用 sessions-page-size 的约定）——与请求日
// 志 / sessions 表同一密度档。
const PAGE_SIZE_KEY = "cc-one:library-page-size"

export function useLibraryBrowser() {
  const { t } = useTranslation()
  const [pageSize, setPageSize] = usePersistedState<number>(
    PAGE_SIZE_KEY,
    DEFAULT_PAGE_SIZE,
  )
  // 空串 = 「全部设备」：哨兵值由共享 FilterSelect 在组件内处理，状态只存
  // 「空串 = 全部」域的纯设备 id。
  const [deviceScope, setDeviceScope] = useState("")
  const [subpath, setSubpath] = useState("")
  // Client-side name filter over the current directory's scan (the backend
  // returns a full directory, so filtering needs no Rust changes).
  const [search, setSearch] = useState("")
  const [dragging, setDragging] = useState(false)
  const [pendingPaths, setPendingPaths] = useState<string[] | null>(null)
  const [preview, setPreview] = useState<LibraryEntry | null>(null)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameVal, setRenameVal] = useState("")
  const [busyRelPath, setBusyRelPath] = useState<string | null>(null)

  const atRoot = subpath === ""
  const scope = deviceScope || "all"
  const showDevice = scope === "all"

  const {
    data: entries = [],
    isLoading,
    error: scanError,
    refetch: refetchScan,
  } = useScanLibraryQuery({
    deviceScope: scope,
    subpath,
  })
  // Search filter first, then the page the table renders. A directory's
  // entry list is a small fs scan (unlike the SQL-backed sessions/logs), so
  // filtering + slicing client-side is the right altitude — the DOM is the
  // thing that grows, and this caps it at one page.
  const filteredEntries = useMemo(
    () => filterEntriesByName(entries, search),
    [entries, search],
  )
  // 分页控制器（架构扫描候选⑧）：offset / 切片 / 翻页单一归属；导航、搜索
  // 或密度变化 → 回第 1 页（scope 身份变化，结构性规则——scope 里新增维度
  // 自动参与）。
  const browser = usePagedBrowser({
    scope: { deviceScope, subpath, search, pageSize },
    pageSize,
    total: filteredEntries.length,
  })
  const visibleEntries = browser.pageItems(filteredEntries)
  // Same source as the logs/dashboard device picker (listDevices), but NOT
  // filtered down to ≤1 — Library always lists every known device, even this
  // machine alone, so the picker is never empty.
  const { data: devices = [] } = useDevicesQuery()
  const [exportMut] = useExportFromLibraryMutation()
  const [deleteMut] = useDeleteFromLibraryMutation()
  const [renameMut] = useRenameInLibraryMutation()
  const runWithToast = useMutateWithToast()

  // Webview-level file drag-drop → collect dropped paths into the pending
  // upload dialog. (HTML5 drop events don't expose local file paths under
  // Tauri; onDragDropEvent is the supported path.) The webview handle is
  // fetched inside the effect so importing this hook never touches Tauri.
  useEffect(() => {
    let active = true
    let unlisten: (() => void) | undefined
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const p = event.payload
        if (p.type === "enter" || p.type === "over") setDragging(true)
        else if (p.type === "leave") setDragging(false)
        else if (p.type === "drop") {
          setDragging(false)
          if (p.paths.length > 0) setPendingPaths(p.paths)
        }
      })
      .then((un) => {
        if (active) unlisten = un
        else un()
      })
    return () => {
      active = false
      unlisten?.()
    }
  }, [])

  const deviceOptions = useMemo(
    () =>
      devices.map((d) => ({
        id: d.device_id,
        label: deviceOptionLabel(d, t),
      })),
    [devices, t],
  )

  const breadcrumb = useMemo(
    () =>
      buildBreadcrumb(deviceScope, subpath, deviceOptions).map((c) => ({
        key: c.key,
        label: c.label,
        onClick: () => {
          setDeviceScope(c.deviceScope)
          setSubpath(c.subpath)
        },
      })),
    [deviceScope, subpath, deviceOptions],
  )

  function drill(entry: LibraryEntry) {
    const { deviceId, rest } = splitEntryPath(entry.rel_path)
    setDeviceScope(deviceId)
    setSubpath(rest)
  }

  function goUp() {
    setSubpath(upFromSubpath(subpath))
  }

  async function onAddFiles() {
    const selected = await open({ multiple: true, directory: false })
    if (!selected) return
    const paths = Array.isArray(selected) ? selected : [selected]
    if (paths.length > 0) setPendingPaths(paths)
  }

  async function onExport(entry: LibraryEntry) {
    const dir = await open({ directory: true })
    if (!dir) return
    setBusyRelPath(entry.rel_path)
    try {
      await runWithToast(
        exportMut,
        { relPath: entry.rel_path, targetDir: dir },
        {
          success: { key: "library.toast.exported" },
          failed: { key: "library.toast.failed" },
        },
      )
    } finally {
      setBusyRelPath(null)
    }
  }

  async function onDelete(entry: LibraryEntry) {
    setBusyRelPath(entry.rel_path)
    try {
      await runWithToast(deleteMut, entry.rel_path, {
        success: { key: "library.toast.deleted" },
        failed: { key: "library.toast.failed" },
      })
    } finally {
      setBusyRelPath(null)
    }
  }

  function startRename(entry: LibraryEntry) {
    setRenaming(entry.rel_path)
    setRenameVal(entry.name)
  }

  async function commitRename(entry: LibraryEntry) {
    const name = renameVal.trim()
    if (!name || name === entry.name) {
      setRenaming(null)
      return
    }
    setBusyRelPath(entry.rel_path)
    try {
      const ok = await runWithToast(
        renameMut,
        { relPath: entry.rel_path, newName: name },
        {
          success: { key: "library.toast.renamed" },
          failed: { key: "library.toast.failed" },
        },
      )
      if (ok) setRenaming(null)
    } finally {
      setBusyRelPath(null)
    }
  }

  return {
    // scan data
    entries: visibleEntries,
    totalCount: filteredEntries.length,
    page: browser.page,
    totalPages: browser.totalPages,
    goToPage: browser.goToPage,
    pageSize,
    setPageSize,
    isLoading,
    scanError,
    refetchScan,
    // search
    search,
    setSearch,
    // device picker
    deviceOptions,
    // navigation state
    deviceScope,
    setDeviceScope,
    subpath,
    atRoot,
    showDevice,
    breadcrumb,
    // drag-drop upload
    dragging,
    pendingPaths,
    clearPendingPaths: () => setPendingPaths(null),
    onAddFiles,
    // row actions
    renaming,
    renameVal,
    setRenameVal,
    cancelRename: () => setRenaming(null),
    busyRelPath,
    drill,
    goUp,
    onExport,
    onDelete,
    startRename,
    commitRename,
    // preview
    preview,
    setPreview,
  }
}

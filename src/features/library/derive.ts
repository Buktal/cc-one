// Pure derivations for the library browser. Two families live here:
//
// - navigation math — splitting an entry's rel_path into device + subpath,
//   resolving a "go up" click to its target, and building the breadcrumb
//   trail. Extracted from the view so the rules are testable in isolation
//   (architecture.md: "关键不变量用代码表达") — the hook wires these to React
//   state, these own the math.
// - filename classification — the extension parse (extOf) plus the image /
//   theme-text predicates and the preview's JSON pretty-print. Every
//   "route by extension" decision (preview rendering, row icons, the kind
//   column, the upload dialog's folder guess) parses through the same extOf —
//   five call sites used to hand-roll it with drifting case handling.

/** One row in the device scope picker. Built in the hook (labels need i18n)
 *  and consumed by buildBreadcrumb to label the device crumb. */
export interface DeviceOption {
  id: string
  label: string
}

/** A breadcrumb entry as pure data: a render label plus the navigation target
 *  the hook wires to setDeviceScope / setSubpath on click. No callbacks live
 *  here so the structure stays testable without React. */
export interface BreadcrumbCrumb {
  key: string
  label: string
  deviceScope: string
  subpath: string
}

/** Text extensions rendered theme-side instead of in the browser's default
 *  (white) iframe rendering — the reason the dark-mode preview looked white.
 *  Case-insensitive; everything else (html / pdf / svg / unknown) keeps the
 *  native iframe. */
const THEME_TEXT_EXTS = new Set(["json", "md", "markdown", "txt", "log"])

/** Image extensions rendered as <img> (preview) / ImageIcon (icons) — one
 *  table behind both decisions so they can't disagree about a name. */
const IMAGE_EXTS = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"])

/**
 * The file name's extension, lowercased, without the leading dot; an
 * extensionless name ("LICENSE", "Makefile") yields "". This is the single
 * parse every extension-routed decision goes through. Semantics follow the
 * lastIndexOf form this module always used: the dot must exist somewhere, so
 * a leading dot counts as a separator (".gitkeep" → "gitkeep") — the
 * split(".").pop() copies elsewhere returned the bare name instead, one of
 * the drifts that motivated this function.
 */
export function extOf(name: string): string {
  const dot = name.lastIndexOf(".")
  if (dot < 0) return ""
  return name.slice(dot + 1).toLowerCase()
}

/**
 * Whether a library file is an image: the preview renders these natively
 * (<img> + Ctrl+wheel zoom) and the icon mapping picks ImageIcon.
 */
export function isImageName(name: string): boolean {
  return IMAGE_EXTS.has(extOf(name))
}

/**
 * Whether a library file should render as theme-styled text (pre) instead of
 * an iframe. JSON / Markdown / plain text / logs — the browser's native
 * rendering of these is white-on-black-inverted-agnostic and breaks dark mode.
 */
export function shouldThemeRender(name: string): boolean {
  return THEME_TEXT_EXTS.has(extOf(name))
}

/**
 * Pretty-print JSON with 2-space indent; any parse failure falls back to the
 * raw text (a .json file can be a JSONL stream or a non-JSON body).
 */
export function maybePrettyJson(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2)
  } catch {
    return text
  }
}

/**
 * Split a library entry's rel_path (`<deviceId>/<rest...>`) into the owning
 * device id and the subpath below it. Drilling into a directory uses this to
 * narrow deviceScope + subpath in one step.
 */
export function splitEntryPath(relPath: string): {
  deviceId: string
  rest: string
} {
  const [deviceId, ...rest] = relPath.split("/")
  return { deviceId, rest: rest.join("/") }
}

/**
 * Resolve a "go up" click: drop the last segment of `subpath`. `deviceScope` is
 * independent of `subpath` — the scan query takes them as separate args, and
 * `drill` sets deviceScope + a subpath that does NOT contain the device id — so
 * going up never touches deviceScope (only the scope picker returns to "all
 * devices"). Returns the new subpath.
 */
export function upFromSubpath(subpath: string): string {
  const parts = subpath.split("/").filter(Boolean)
  return parts.slice(0, -1).join("/")
}

/**
 * Case-insensitive name filter for the current directory's entries — the
 * library's search matches what the scan returned for this scope + subpath
 * (one directory level; the backend has no recursive search). A blank query
 * returns the list untouched.
 */
export function filterEntriesByName<T extends { name: string }>(
  entries: T[],
  query: string,
): T[] {
  const q = query.trim().toLowerCase()
  if (!q) return entries
  return entries.filter((e) => e.name.toLowerCase().includes(q))
}

/**
 * Build the breadcrumb trail as pure data (labels + navigation targets, no
 * callbacks). The device crumb is labelled from `deviceScope` (passed in —
 * `subpath` never carries the device id, matching how `drill` and the scan
 * query treat them as separate values); each following crumb adds one more
 * `subpath` segment. Every crumb keeps the same `deviceScope` — clicking one
 * navigates within the current device, only `subpath` changes. Returns an empty
 * list when subpath is empty (the scope picker, not the breadcrumb, handles
 * leaving the device).
 */
export function buildBreadcrumb(
  deviceScope: string,
  subpath: string,
  deviceOptions: DeviceOption[],
): BreadcrumbCrumb[] {
  if (!subpath) return []
  const deviceLabel =
    deviceOptions.find((o) => o.id === deviceScope)?.label ?? deviceScope
  const parts = subpath.split("/").filter(Boolean)
  const crumbs: BreadcrumbCrumb[] = [
    { key: deviceScope, label: deviceLabel, deviceScope, subpath: "" },
  ]
  for (let i = 0; i < parts.length; i++) {
    const sub = parts.slice(0, i + 1).join("/")
    crumbs.push({
      key: `${deviceScope}/${sub}`,
      label: parts[i],
      deviceScope,
      subpath: sub,
    })
  }
  return crumbs
}

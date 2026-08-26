// Shared pager footer for the paginated tables (request log / sessions /
// pricing / library). Left: "第 X / Y 页 · 共 N 条". Right: prev/next buttons
// plus a numbered page bar with ellipsis gaps (pageNumbers — the single
// sequence implementation), and — when the caller passes `pageSize` — a
// per-page size selector (sessions 工作台的 20/50/100). `loading` disables the
// controls while a page refetches so a page flip never goes feedback-less.
//
// total === 0 renders nothing — the「0 条 + 两个禁用按钮」strip under a
// centered empty state reads as broken（判断归这里，调用方不再各自复制
// `total > 0 &&` 条件）.
//
// Single-line guarantee: the footer is a @container — the page-info label
// never wraps (whitespace-nowrap) and never shrinks (shrink-0), the numbered
// page buttons hide below a 40rem container (a 7-page spread is ~250px of
// buttons; without this the next button would overflow and get clipped on
// narrow windows), the prev/next text labels need a 24rem container, and the
// per-page label needs a 28rem container.

import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Pagination,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
} from "@/components/ui/pagination"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { formatInt } from "@/lib/format"
import { pageNumbers } from "@/lib/pagination"

export function PaginationBar({
  page,
  totalPages,
  total,
  onPageChange,
  loading = false,
  pageSize,
}: {
  /** 1-based current page. */
  page: number
  totalPages: number
  /** Total row count for the page-info label. */
  total: number
  /** 1-based page to jump to. */
  onPageChange: (page: number) => void
  /** True while the next page refetches — disables every control. */
  loading?: boolean
  /** 每页条数选择器（缺省不渲染）：当前值 / 候选 / 回调。 */
  pageSize?: {
    value: number
    options: readonly number[]
    onChange: (n: number) => void
  }
}) {
  const { t } = useTranslation()
  if (total === 0) return null
  return (
    // @container: the narrow-window breakpoints below measure this footer's
    // own width (the card's content column), not the window — the sidebar's
    // collapse state must not shift the pager's behavior.
    <div className="@container text-muted-foreground mt-3 flex shrink-0 items-center justify-between gap-3 text-xs">
      {/* whitespace-nowrap + shrink-0: the label must hold one line even when
        the buttons need the whole remaining width — a wrapped label is what
        made the pager look broken on narrow windows. */}
      <span className="shrink-0 whitespace-nowrap">
        {t("pagination.pageInfo", {
          page,
          totalPages,
          total: formatInt(total),
        })}
      </span>
      <div className="flex shrink-0 items-center gap-2">
        <Pagination className="w-auto">
          <PaginationContent>
            <PaginationItem>
              <Button
                variant="outline"
                size="sm"
                disabled={page <= 1 || loading}
                onClick={() => onPageChange(page - 1)}
              >
                <ChevronLeftIcon />
                <span className="hidden @[24rem]:inline">
                  {t("pagination.prev")}
                </span>
              </Button>
            </PaginationItem>
            {/* Numbered pages need a 40rem-wide container; below that they
              hide (the page-info label already says "第 X / Y 页"). contents
              keeps the page buttons as direct flex items so the gap and
              wrapping behave as if the wrapper weren't there. */}
            <PaginationItem className="hidden @[40rem]:contents">
              {pageNumbers(page, totalPages).map((n, i, arr) =>
                n === "…" ? (
                  // Each gap sits before exactly one page number — key by that
                  // page so the key is stable and unique without an index.
                  <PaginationItem key={`gap-${arr[i + 1]}`}>
                    <PaginationEllipsis />
                  </PaginationItem>
                ) : (
                  <PaginationItem key={n}>
                    <Button
                      variant={n === page ? "default" : "ghost"}
                      size="icon-sm"
                      disabled={loading}
                      aria-current={n === page ? "page" : undefined}
                      onClick={() => onPageChange(n)}
                    >
                      {n}
                    </Button>
                  </PaginationItem>
                ),
              )}
            </PaginationItem>
            <PaginationItem>
              <Button
                variant="outline"
                size="sm"
                disabled={page >= totalPages || loading}
                onClick={() => onPageChange(page + 1)}
              >
                <span className="hidden @[24rem]:inline">
                  {t("pagination.next")}
                </span>
                <ChevronRightIcon />
              </Button>
            </PaginationItem>
          </PaginationContent>
        </Pagination>
        {pageSize ? (
          <div className="flex shrink-0 items-center gap-1.5">
            {/* 「每页」label needs a 28rem container — below that only the
              number + chevron remain (still self-explanatory next to the
              page-info). */}
            <span className="hidden whitespace-nowrap @[28rem]:inline">
              {t("pagination.perPage")}
            </span>
            <Select
              value={String(pageSize.value)}
              onValueChange={(v) => {
                if (v != null) pageSize.onChange(Number(v))
              }}
            >
              {/* 宽度自适应（trigger 本体 w-fit）：写死 w-15 会被 px-3 内距 +
                  箭头挤到裁掉「20」，删掉交给内容。 */}
              <SelectTrigger size="sm" aria-label={t("pagination.perPage")}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent align="end">
                {pageSize.options.map((n) => (
                  <SelectItem key={n} value={String(n)}>
                    {n}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : null}
      </div>
    </div>
  )
}

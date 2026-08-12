// Shared pager footer for the paginated tables (request log / sessions /
// pricing / library). Left: "第 X / Y 页 · 共 N 条". Right: prev/next buttons
// plus a numbered page bar with ellipsis gaps (pageNumbers — the single
// sequence implementation). `loading` disables the controls while a page
// refetches so a page flip never goes feedback-less.

import { ChevronLeftIcon, ChevronRightIcon } from "lucide-react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Pagination,
  PaginationContent,
  PaginationEllipsis,
  PaginationItem,
} from "@/components/ui/pagination"
import { formatInt } from "@/lib/format"
import { pageNumbers } from "@/lib/pagination"

export function PaginationBar({
  page,
  totalPages,
  total,
  onPageChange,
  loading = false,
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
}) {
  const { t } = useTranslation()
  return (
    <div className="text-muted-foreground mt-3 flex shrink-0 items-center justify-between text-xs">
      <span>
        {t("pagination.pageInfo", {
          page,
          totalPages,
          total: formatInt(total),
        })}
      </span>
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
              <span className="hidden sm:inline">{t("pagination.prev")}</span>
            </Button>
          </PaginationItem>
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
          <PaginationItem>
            <Button
              variant="outline"
              size="sm"
              disabled={page >= totalPages || loading}
              onClick={() => onPageChange(page + 1)}
            >
              <span className="hidden sm:inline">{t("pagination.next")}</span>
              <ChevronRightIcon />
            </Button>
          </PaginationItem>
        </PaginationContent>
      </Pagination>
    </div>
  )
}

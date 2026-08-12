import { describe, expect, it } from "vitest"

import { pageNumbers, paginate } from "@/lib/pagination"

describe("paginate", () => {
  it("totalPages is at least 1 even when empty", () => {
    expect(paginate(0, 0, 50)).toEqual({ totalPages: 1, page: 1 })
  })

  it("computes page from offset and clamps into range", () => {
    expect(paginate(100, 0, 50)).toEqual({ totalPages: 2, page: 1 })
    expect(paginate(100, 50, 50)).toEqual({ totalPages: 2, page: 2 })
    // offset past the end clamps to the last page
    expect(paginate(100, 999, 50)).toEqual({ totalPages: 2, page: 2 })
  })

  it("clamps when rows shrink beneath the offset (forget-a-device case)", () => {
    // Was on page 2 of 2 (offset 50, 100 rows); rows drop to 30 without the
    // offset resetting yet — page must clamp to 1, not report "2 / 1".
    expect(paginate(30, 50, 50)).toEqual({ totalPages: 1, page: 1 })
  })
})

describe("pageNumbers", () => {
  it("renders every page when 7 or fewer", () => {
    expect(pageNumbers(1, 1)).toEqual([1])
    expect(pageNumbers(4, 7)).toEqual([1, 2, 3, 4, 5, 6, 7])
  })

  it("keeps 1, last, and current ±1 with ellipsis gaps (Radix-style siblings)", () => {
    expect(pageNumbers(1, 10)).toEqual([1, 2, "…", 10])
    expect(pageNumbers(5, 10)).toEqual([1, "…", 4, 5, 6, "…", 10])
    expect(pageNumbers(10, 10)).toEqual([1, "…", 9, 10])
  })

  it("clamps the current page into range", () => {
    expect(pageNumbers(99, 10)).toEqual([1, "…", 9, 10])
    expect(pageNumbers(0, 10)).toEqual([1, 2, "…", 10])
  })
})

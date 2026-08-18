// deleteFlowReducer — the confirm-dialog timing state machine. The hook
// (useConfirmDelete) routes every state change through this pure function, so
// these tests are the production path: the "busy reset on both paths" /
// "close only on success" invariants used to live in three nearly-identical
// comments (providers / pricing / library) and are now a contract here.

import { describe, expect, it } from "vitest"

import {
  type DeleteFlowState,
  deleteFlowReducer,
} from "@/hooks/use-confirm-delete"

const idle = <T>(): DeleteFlowState<T> => ({ target: null, busy: false })

describe("deleteFlowReducer — holdOpen (providers / pricing)", () => {
  it("request opens the dialog with busy off", () => {
    expect(
      deleteFlowReducer(idle(), { kind: "request", target: "A" }, true),
    ).toEqual({ target: "A", busy: false })
  })

  it("confirm-start keeps the dialog open and sets busy", () => {
    expect(
      deleteFlowReducer(
        { target: "A", busy: false },
        { kind: "confirm-start" },
        true,
      ),
    ).toEqual({ target: "A", busy: true })
  })

  it("success closes the dialog and resets busy", () => {
    expect(
      deleteFlowReducer(
        { target: "A", busy: true },
        { kind: "confirm-done", ok: true },
        true,
      ),
    ).toEqual({ target: null, busy: false })
  })

  it("failure keeps the dialog open (retry inside) and resets busy", () => {
    expect(
      deleteFlowReducer(
        { target: "A", busy: true },
        { kind: "confirm-done", ok: false },
        true,
      ),
    ).toEqual({ target: "A", busy: false })
  })

  it("cancel closes and clears busy", () => {
    expect(
      deleteFlowReducer({ target: "A", busy: false }, { kind: "cancel" }, true),
    ).toEqual({ target: null, busy: false })
  })

  it("a late confirm-done after cancel is a no-op (busy already cleared)", () => {
    expect(
      deleteFlowReducer(idle(), { kind: "confirm-done", ok: true }, true),
    ).toEqual({ target: null, busy: false })
  })

  it("confirm-start with no target is a no-op", () => {
    expect(deleteFlowReducer(idle(), { kind: "confirm-start" }, true)).toEqual({
      target: null,
      busy: false,
    })
  })
})

describe("deleteFlowReducer — closeFirst (library: row spinner takes over)", () => {
  it("confirm-start closes immediately, busy never set", () => {
    expect(
      deleteFlowReducer(
        { target: "A", busy: false },
        { kind: "confirm-start" },
        false,
      ),
    ).toEqual({ target: null, busy: false })
  })

  it("the late confirm-done of the background delete is a no-op", () => {
    expect(
      deleteFlowReducer(idle(), { kind: "confirm-done", ok: true }, false),
    ).toEqual({ target: null, busy: false })
    expect(
      deleteFlowReducer(idle(), { kind: "confirm-done", ok: false }, false),
    ).toEqual({ target: null, busy: false })
  })
})

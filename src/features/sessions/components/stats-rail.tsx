// 右统计卡栏的壳 —— 三栏工作台第三栏（#108 定稿 variant-a）。卡片本体在
// stats-cards（口径分派：会话态 4 卡 / 项目·全量态 3+1 卡 / 分组态轻量汇总
// 一张，同一份渲染进右栏与浮卡两处）与 stats-identity-cards（两态身份卡）；
// 本件只管载体：右栏 aside、口径行、窄容器的统计入口。数据全部来自
// useSessionsBrowser 的 selection-free session_stats 读（aggregateStats 纯聚
// 合），无第二条统计路径。
//
// 窄档（外层命名容器 /sessions < 76rem）右栏整栏让位：统计入口缩成一个图标
// （NarrowStatsTrigger——列表态在卡片头右上、会话态在详情标题行），hover 原位
// 浮出统计小卡。fixed 浮动按钮会盖分页条、按钮 + 遮罩弹出层打断浏览，皆已撤。
// 76rem 的几何依据：树 13 + 右栏 16 + 完整轮次导航 14 + 详情最小可读 26 + 间隙
// ≈ 71.25rem，留约 5rem 阅读余量——低于它四列挤不下，右栏就地转图标（曾定
// 58rem：四列装不下，还把左树挤得让位、覆盖了几乎所有真实窗口，皆翻车；后按
// 手感上调至 76）。右栏与图标共用 /sessions 这把尺，76rem 一刻度互斥。

import { BarChart3 } from "lucide-react"
import { useRef } from "react"
import { useTranslation } from "react-i18next"
import { Button } from "@/components/ui/button"
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover"
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip"
import { useHoverIntent } from "@/hooks/use-hover-intent"
import type {
  SessionMessage,
  SessionRow,
  SessionStatsRow,
} from "@/types/generated/bindings"
import type { StatsAggregate } from "../derive"
import { StatsCards } from "./stats-cards"

/** 口径 tag 的文案键——按项目/按会话/按分组（原口径 tab 的文字留作 tag）。 */
const TAG_KEYS = {
  project: "sessions.stats.byProject",
  session: "sessions.stats.bySession",
  group: "sessions.stats.byGroup",
} as const

export type StatsScopeTag = keyof typeof TAG_KEYS

/** 统计数据切片 —— 右栏本体与窄容器浮卡消费同一份（口径派生只有一处）。 */
export interface StatsData {
  /** 口径 tag：会话态 / 项目态 / 分组态——由「是否打开会话 + 容器选中」派生
   *  （useSessionsBrowser），用户不可手切（口径 tab 已删）。 */
  scopeTag: StatsScopeTag
  /** 口径对象名（会话标题 / 项目 basename / 组名 / 「全部」）。 */
  scopeLabel: string
  /** 选中容器的聚合（未选中 = 全量）。 */
  aggregate: StatsAggregate
  /** 选中会话（会话态的数据源；null = 容器/全量态）。 */
  session: SessionRow | null
  sessionStats: SessionStatsRow | null
  transcript: SessionMessage[]
  transcriptLoading: boolean
  deviceLabel: (id: string) => string
  /** 项目态的身份卡数据（项目目录 + subagent 数）；非项目态为 null。 */
  projectIdentity: { dir: string; subagents: number } | null
}

export function StatsRail(props: StatsData) {
  return (
    // 右栏本体：58rem 容器以下整栏让位（NarrowStatsTrigger 浮卡接管）——见
    // 文件头注释的门槛推导。
    <aside className="border-border bg-card hidden min-h-0 w-64 shrink-0 flex-col gap-2 rounded-lg border p-2 @min-[76rem]/sessions:flex">
      <StatsScopeHeader
        scopeTag={props.scopeTag}
        scopeLabel={props.scopeLabel}
      />
      <div className="min-h-0 flex-1 overflow-y-auto pr-0.5">
        {/* 恒单列：窄档的两列迷你卡（单卡内容仅约 90px）连「累计时长」这类
            标签都装不下，故撤；整宽卡片里公式小字等长文案不再换行。 */}
        <div className="flex flex-col gap-2">
          <StatsCards {...props} />
        </div>
      </div>
    </aside>
  )
}

/** 口径行（scope 说明 + 口径 tag）——右栏与浮卡共用。 */
function StatsScopeHeader({
  scopeTag,
  scopeLabel,
}: Pick<StatsData, "scopeTag" | "scopeLabel">) {
  const { t } = useTranslation()
  return (
    <div className="flex min-w-0 items-center gap-2">
      <Tooltip>
        <TooltipTrigger
          render={
            <span className="min-w-0 flex-1 truncate text-right text-xs" />
          }
        >
          <span className="text-muted-foreground">
            {t("sessions.stats.scope")}{" "}
            <span className="text-accent-brand-strong font-medium">
              {scopeLabel}
            </span>
          </span>
        </TooltipTrigger>
        <TooltipContent side="left">{scopeLabel}</TooltipContent>
      </Tooltip>
      <span className="text-muted-foreground/70 border-border shrink-0 rounded-full border px-2 py-px text-[10px] leading-4">
        {t(TAG_KEYS[scopeTag])}
      </span>
    </div>
  )
}

// 开合规则与计时全在 use-hover-intent（进触发器/弹层立即亮出、离开留缓冲穿
// 缝隙再收；反射焦点恒等——此前 onFocus 直接收下 base-ui 关闭后的还焦，零输
// 入自持振荡反复弹窗，见该模块头注）。

/** 窄容器统计入口：图标 hover 原位浮出统计小卡（无遮罩、不打断浏览——弹出
 *  层与浮动按钮均已撤，见文件头注释）。宽档（/sessions ≥76rem）右栏本体接管，
 *  此件整体隐身。 */
export function NarrowStatsTrigger(props: StatsData) {
  const { t } = useTranslation()
  const { open, advance } = useHoverIntent()
  // 弹层 DOM 引用：blur 时分辨焦点是搬进弹层内部（内部编排）还是真去往外处。
  const popupRef = useRef<HTMLDivElement>(null)
  return (
    // 开合唯一入口是状态机：hover 进出、键盘聚焦、点击/Esc 的切换意图统统
    // 送进同一张表驱动转换表。显隐引用外层命名容器 /sessions——与右栏本体
    // 同一把尺、同一刻度（76rem）精确互斥：图标量的是「最近祖先容器」，本
    // 组件身处中列，若不点名 /sessions 就会量到窄得多的中列，永不合格（曾
    // 与右栏同屏翻车）；另外 "@?" 不是 Tailwind 语法，类不会生成。
    <span className="@min-[76rem]/sessions:hidden">
      <Popover
        open={open}
        onOpenChange={(o) => advance(o ? "enter" : "dismiss")}
      >
        <PopoverTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("sessions.stats.open")}
              onMouseEnter={() => advance("enter")}
              onMouseLeave={() => advance("leave")}
              onFocus={(e) =>
                advance(
                  e.currentTarget.matches(":focus-visible")
                    ? "enter-keyboard"
                    : "focus-reflected",
                )
              }
              onBlur={(e) =>
                advance(
                  e.relatedTarget instanceof Node &&
                    popupRef.current?.contains(e.relatedTarget)
                    ? "blur-inside"
                    : "blur-outside",
                )
              }
            />
          }
        >
          <BarChart3 className="text-muted-foreground size-4" />
        </PopoverTrigger>
        <PopoverContent
          ref={popupRef}
          align="end"
          sideOffset={8}
          className="max-h-[min(70vh,32rem)] w-80 gap-2 overflow-y-auto rounded-lg p-3"
          onMouseEnter={() => advance("enter")}
          onMouseLeave={() => advance("leave")}
        >
          <div className="grid grid-cols-1 gap-2">
            <StatsScopeHeader
              scopeTag={props.scopeTag}
              scopeLabel={props.scopeLabel}
            />
            <StatsCards {...props} />
          </div>
        </PopoverContent>
      </Popover>
    </span>
  )
}

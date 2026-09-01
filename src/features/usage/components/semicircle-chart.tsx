// SemicircleChart — half-ring composition (#119 档位类形态，替代四档进度
// 条). 弧段按占比相接成 180° 半环（左端起扫到右端），环心放合计；右侧行保
// 留「档位 · 数量 · 占比」的精确读数——弧长难比长短，形态给构成直觉、行给
// 数字。有序档位用主色的单色渐进阶（color-mix 主色 × 卡面，浅 = 少、深 =
// 多）：档位有天然次序，不该引入类目色相。段间 2px 卡面缝；每段 <title>
// 提供 hover 读数（原生 SVG 提示，弧段最多四段，逐段挂浮层无负担）。自绘
// SVG 而非 recharts RadialBar：180° 堆叠弧的段缝 / 环心文案 / 图例行在
// RadialBar 上都是逆着库走的组合 hack，二十行路径数学反而直读。数据由调用
// 方随全局筛选喂入，本件不自带统计窗口、不读查询。

import { formatPct } from "@/lib/format"

export interface SemicircleTier {
  label: string
  count: number
}

export function SemicircleChart({
  tiers,
  centerValue,
  centerLabel,
  formatValue,
  size = 190,
}: {
  /** Ordered tiers (少 → 多)；序即色深序。 */
  tiers: readonly SemicircleTier[]
  /** 合计读数（环心大字，调用方按 DSL 格式化）。 */
  centerValue: string
  centerLabel: string
  /** 档位数量与图例行值的格式化（计数类走 formatCount）。 */
  formatValue: (n: number) => string
  size?: number
}) {
  const total = tiers.reduce((s, t) => s + t.count, 0)
  const cx = size / 2
  const cy = size / 2 + 8
  const r = Math.round(size * 0.4)
  const rin = Math.round(size * 0.246)
  // 段间 2px 卡面缝：描边用卡面色（与原型一致），不用留缝角算术。
  const arc = (a0: number, a1: number, radius: number, sweep: 0 | 1) =>
    `${cx + radius * Math.cos(a0)} ${cy - radius * Math.sin(a0)} A ${radius} ${radius} 0 ${Math.abs(a1 - a0) > Math.PI ? 1 : 0} ${sweep} ${cx + radius * Math.cos(a1)} ${cy - radius * Math.sin(a1)}`

  // 单色渐进阶：主色向卡面收（无 alpha 叉），4 档 = 30% / 55% / 78% / 100%。
  const shade = (i: number) => {
    if (tiers.length <= 1) return "var(--primary)"
    const pct = 30 + (70 * i) / (tiers.length - 1)
    return `color-mix(in srgb, var(--primary) ${Math.round(pct)}%, var(--card))`
  }

  let a = Math.PI
  const arcs = tiers.map((tier, i) => {
    const span = (total > 0 ? tier.count / total : 0) * Math.PI
    const a1 = a - span
    const path =
      span > 0 ? `M ${arc(a, a1, r, 1)} L ${arc(a1, a, rin, 0)} Z` : undefined
    const node = (
      <path
        key={tier.label}
        d={path}
        fill={shade(i)}
        stroke="var(--card)"
        strokeWidth={2}
      >
        <title>
          {`${tier.label} · ${formatValue(tier.count)} · ${formatPct(total > 0 ? tier.count / total : 0)}`}
        </title>
      </path>
    )
    a = a1
    return node
  })

  return (
    <div className="flex items-center justify-center gap-5">
      <svg
        width={size}
        viewBox={`0 0 ${size} ${size - 40}`}
        role="img"
        aria-label={`${centerLabel} ${centerValue}`}
        className="block shrink-0"
      >
        {arcs}
        <text
          x={cx}
          y={cy - 14}
          textAnchor="middle"
          fontSize={24}
          fontWeight={600}
          fill="var(--foreground)"
          className="tabular-nums"
        >
          {centerValue}
        </text>
        <text
          x={cx}
          y={cy + 6}
          textAnchor="middle"
          fontSize={11}
          fill="var(--muted-foreground)"
        >
          {centerLabel}
        </text>
      </svg>
      <div className="flex min-w-0 flex-col gap-1">
        {tiers.map((tier, i) => (
          <div
            key={tier.label}
            className="flex items-center gap-1.5 text-xs whitespace-nowrap"
          >
            <span
              className="size-2 shrink-0 rounded-[2px]"
              style={{ backgroundColor: shade(i) }}
            />
            <span className="text-muted-foreground min-w-0 truncate">
              {tier.label}
            </span>
            <span className="ml-auto tabular-nums">
              {formatValue(tier.count)}
            </span>
            <span className="text-muted-foreground w-12 text-right tabular-nums">
              {formatPct(total > 0 ? tier.count / total : 0)}
            </span>
          </div>
        ))}
      </div>
    </div>
  )
}

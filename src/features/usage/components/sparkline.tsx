// Sparkline — the KPI band's cell-tail mini trend (#119: 纯数字格 → 格尾
// 迷你趋势线). 自绘 SVG 折线 + 当前点强调端点：尺寸是图标级定尺（例外条款
// ——一格宽的线不配给它 recharts，ResponsiveContainer 逐格挂的开销买不回
// 任何读数）。线弱化（muted 低透明），端点用格子 accent 色、卡面描边环
// （「线是形状，点是现在」）。少于 2 个点不渲染（无趋势可言）；全平序列
// 画中线（诚实于数据）。数据由调用方随全局筛选喂入（多日 = 逐日、单日 =
// 逐小时），本件不自带统计窗口、不读查询。

export function Sparkline({
  values,
  color = "var(--primary)",
  width = 72,
  height = 20,
}: {
  /** The series tail — caller owns the window resolution and the caliber. */
  values: readonly number[]
  /** Endpoint dot color — the cell's own accent keeps the glyph honest. */
  color?: string
  width?: number
  height?: number
}) {
  if (values.length < 2) return null
  const min = Math.min(...values)
  const max = Math.max(...values)
  const x = (i: number) => 2 + (i * (width - 8)) / (values.length - 1)
  const y = (v: number) =>
    3 + (height - 10) * (1 - (max === min ? 0.5 : (v - min) / (max - min)))
  const d = values
    .map((v, i) => `${i === 0 ? "M" : "L"} ${x(i)} ${y(v)}`)
    .join(" ")
  const last = values.length - 1
  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      aria-hidden="true"
      className="block"
    >
      <path
        d={d}
        fill="none"
        stroke="var(--muted-foreground)"
        strokeWidth={1.5}
        strokeOpacity={0.55}
      />
      <circle
        cx={x(last)}
        cy={y(values[last])}
        r={3.5}
        fill={color}
        stroke="var(--card)"
        strokeWidth={2}
      />
    </svg>
  )
}

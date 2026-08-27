# Images

本目录统一存放 README 所用截图。每个画面各一张浅色 + 一张深色（不按 UI 语言区分，三个 README 共用同一套图）。

```
docs/images/
├─ ad-en.png / ad-zh.png / ad-ja.png            各语言 Hero 横幅（同一画面，可共用一张裁剪）
├─ light-usage.png    / dark-usage.png          看板（Dashboard）
├─ light-consumption.png / dark-consumption.png 消耗（Consumption）
└─ light-floating-card.png / dark-floating-card.png   轻量速览模式（Glance mode）
```

| 文件名 | 内容 |
| --- | --- |
| `*-usage.png` | 完整看板：统计卡（token 四桶 / 缓存命中率 / 请求数 / 成本）+ 趋势图 + 请求日志 + 侧边栏 |
| `*-consumption.png` | 用量消耗视图 |
| `*-floating-card.png` | 轻量速览模式：贴边迷你条 + 可展开的悬浮卡 |

三个 README 都引用本目录的图：

- `README.md` / `README.zh-CN.md` / `README.ja-JP.md` → `./docs/images/`

> 替换截图时，**保持同名覆盖**即可，README 引用路径无需改动。

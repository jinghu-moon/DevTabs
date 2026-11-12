# Calendar Solid Cells and Minimum Width

2026-11-12。根据真机反馈统一日历格子样式，并修复统计栏缩窄后热力图消失的问题。

## 行为

- 未来日期与零提交日期显示完全相同的灰色背景色块，不再显示点号或空心方框。未来日期仍保留独立元数据，悬停显示“尚未到来”，不计为实际零提交日。
- 三栏布局中的统计栏最小宽度为 32 个终端字符列。12 周网格占 24 列，最宽星期标签占 3 列，面板左右内边距共 4 列，再保留 1 列余量。原来的 30 列减去内边距后不足以容纳中文热力图，触发窄宽度降级。
- 布局计算和右侧分隔线拖动均约束最小宽度；已有配置中保存的过窄比例也受约束。详情栏继续至少 60 列，不修改项目列表或双栏模式的既有最小宽度。
- 整个终端不足以容纳三栏时沿用响应式布局，可通过 F8 打开统计弹窗。全年图仍需更宽区域，本次保证最窄统计栏能完整显示近 12 周。

## 验证

实际 RED/GREEN 日志保存在 `artifacts/calendar-solid-minimum/`：旧代码分别复现未来日期不是实心格，以及统计栏最小宽度不足。修改后通过以下相关回归：

```powershell
cargo test --locked --lib --test overview --test project_pulse --test i18n --target-dir target/perf -- --test-threads=1
```

共 **156 通过、0 失败、8 忽略**，覆盖日历七种当前星期、未来日期语义，以及中英文在 160/180/200/260 列下读取极端保存比例、向右拖到极限后仍有 84 个日期命中区域。测试使用隔离数据和 TestBackend，不操作实际用户配置，不启动真实 Agent；真实 UI 由用户验收。

交付程序：

```powershell
.\artifacts\DevTabs-0.4.0-calendar-solid-minimum\devtabs.exe
```

构建指纹、静态检查及 help/version 冒烟结果保存在交付目录 `build.json` 和 `source-files.sha256`。

最终 fmt、Clippy（workspace/all-targets）、Release 及 help/version 冒烟均通过。`devtabs 0.4.0 / build aeb26e9f22dd63a9 / IPC DevTabs-v6`；程序 23490560 字节，SHA256 `77c5477c11f08ee4d1918b9f5da9cbf91a8d2d2ea1ed72137d3e3801fc6bdd6c`。

## 背景填充补正

随后根据真机反馈发现：语言比例色带使用空格加背景色，热力图使用前景色字符 `██`。后者依赖终端字形栅格化，可能因字体、行距和缩放产生细缝。

热力图改为两个空格加背景色，每个日期仍占两列。五级图例同步使用背景色填充，保持原宽度；色阶、12 周/全年布局、日期命中、未来日期提示和统计栏最小 32 列约束保持原有行为。同色日期连续填充，不额外插入空行或空列。不新增依赖。

调整既有测试后在旧实现下实际失败（3 项）；修改后相关回归 156 通过、0 失败、8 忽略，检查 TestBackend 每个日期两列均为空格且填充背景色，覆盖四主题、中英文、未来日期与全年布局。fmt、Clippy、Release 及安全命令冒烟结果见新版交付目录 `build.json`。日志在 `artifacts/calendar-background/`。终端实际像素效果由用户验收。

新版程序（保留上一版本）：

```powershell
.\artifacts\DevTabs-0.4.0-calendar-background\devtabs.exe
```

## 图例色块尺寸统一

底部“少 → 多”的五级图例也改为每级两个空格加背景色，与日期色块同宽；图例依然对齐日历右边缘。旧版每级仅一列，颜色看起来更窄长。既有图例测试先实际复现一列与两列的差异，修改后验证五个色阶、对应颜色和总宽度；总览相关回归 71 通过、0 失败、7 忽略。日志在 `artifacts/calendar-legend/`，新版程序为 `artifacts/DevTabs-0.4.0-calendar-legend/devtabs.exe`。

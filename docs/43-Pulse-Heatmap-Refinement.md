# Pulse Heatmap Refinement

2026-11-12。修复统计热力图末列看似缺格与说明文字过多的问题；只调整展示，不改变 Git 采集、12 周 UTC 分桶、完整目录扫描或缓存口径。

## 原因与交互

热力图每列一周、每行一个星期几。本周尚未到来的日期原先输出两个空格；11 月 12 日是周四，因此最后一列周五至周日看起来缺失。现在每行保留 12 个可见单元：已过去的日期使用填充块，零提交为灰色，有提交使用四级绿色；未来日期使用淡色 `··`，不把它当成零提交。

横轴直接标出“11 周前 / 本周”。移除重复的“项目脉搏”标题、活跃周数与最活跃周长说明，保留一个“近 12 周提交”标题及周期提交数，下面简要显示最近提交日期与连续活跃周数。历史总提交与周期数相同时省略重复数字，不同时继续显示历史总计。

底部图例缩短为 `UTC 少 ■■■■■ 多 ·· 未到`，方块颜色与实际热力图共用色阶。英文提供对应文案，窄栏继续使用周趋势图；加载与 Git 错误不显示虚假的零值。常改文件与目录热点的右对齐数值保持原有布局。

## 验证

- 修改前统计视图回归 6 通过、0 失败、1 忽略，日志 `artifacts/pulse-heatmap/before-tests.log`。
- 新增断言实际 RED：完整 12×7 可见网格及精简标题测试在旧版均失败，日志 `red-tests.log`。
- 修改后 lib、overview、project_pulse、i18n 合计 **145 通过、0 失败、8 忽略**，日志 `green-tests.log`。本次为相关自动回归，没有重跑上一轮完整 workspace，也没有运行真实终端 UI。
- 日期测试注入固定 UTC 日期，覆盖今天为星期一至星期日的全部情况，校验未来格子位置、零提交与未来样式差别、七行与十二列。中英文保留不同的历史总计，原有 1/2/8/26/28/40/60/76/120 格宽度回归继续通过。
- fmt、Clippy `-D warnings`、Release 构建、help/version 冒烟均通过。构建 `04f8ef60941c19de`，23,671,296 字节，SHA256 `5568ebe4cb1948631d516a1b43b6f4bfb5084f5de46f191609002d86e75bf690`；同交付目录 `build.json` 和 `source-files.sha256` 保存检查范围及源码指纹。

复现：

```powershell
cargo test --locked --lib --test overview --test project_pulse --test i18n --target-dir target/perf -- --test-threads=1
cargo clippy --locked --all-targets --target-dir target/i18n-check -- -D warnings
cargo fmt --check
```

测试程序使用独立目录，旧版本不覆盖：

```powershell
.\artifacts\DevTabs-0.4.0-pulse-heatmap\devtabs.exe
```

直接进入总览，选择项目后查看概况；窄屏按 F8。真实终端渲染由用户验收。

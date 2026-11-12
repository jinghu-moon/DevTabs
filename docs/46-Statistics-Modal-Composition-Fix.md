# Statistics Modal Composition Fix

2026-11-12。修复点击日历 `[年] / [+]` 后宽终端出现重复统计内容的问题。

## 原因与修复

原有代码先绘制总览各栏，再打开最大 180 列的居中统计弹窗。弹窗只清除自己的矩形，终端更宽时，两侧仍露出原统计栏的数字、横条和边框。截图中的右侧重复数值来自底层统计栏。

当前详情/统计弹窗打开时，先清除整个总览内容区，再绘制居中弹窗，保留顶部筛选和底部消息。弹窗左右留白，统计只显示一次；Esc 或 `[+]` 关闭后按现有栏宽恢复总览。统计弹窗标题注明 Esc，日历切换、缩放和鼠标命中保持原有行为。

截图底部 `launches.json: missing field task_ids` 是独立问题：多任务功能增加必填字段后，旧启动记录无法读取。现在缺失 `task_ids` 时按“没有已记录任务身份”读取，保持空列表，不从路径或分支猜测任务所有权。读取不重写文件，也不删除记录；原有宿主恢复检查仍会拒绝缺少权威任务身份的自动恢复。损坏 JSON 或字段类型错误继续报错。本轮未操作用户实际启动记录。

## 验证

实际 RED/GREEN 证据位于 `artifacts/statistics-modal/`：

- `red-tests.log` 在旧版检测到弹窗外背景符号泄漏；`green-tests.log` 同一断言通过。
- `journal-red.log` 实际复现旧记录缺少 `task_ids` 的读取失败；最终回归验证缺失字段可读、字段类型错误仍拒绝、读取不改文件。
- TestBackend 在 120/200/260 列通过鼠标分别点击 `[年]` 和 `[+]`，断言弹窗外内容区为空、热点数字仅出现一次、全年/近 12 周命中数正确，并验证点击关闭后整个内容区恢复原布局。
- 最终相关回归、静态检查与交付指纹见下方记录。真实 Windows Terminal UI 由用户验收。

```powershell
cargo test --locked --lib --test overview --test project_pulse --test reference_parity --test i18n --target-dir target/perf -- --test-threads=1
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo fmt --check
```

新测试程序（旧目录保留）：

```powershell
.\artifacts\DevTabs-0.4.0-statistics-modal\devtabs.exe
```

选择项目后点击 `[年] / [+]`，检查弹窗两侧没有原统计残留，Esc / `[+]` 关闭后恢复；全年需足够宽的终端。

最终相关回归 **173 通过、0 失败、9 忽略，5 个 suite**，实际退出码 0；fmt、Clippy、Release 与 help/version 冒烟通过。构建 `22987aff408f4578`，23512576 字节，SHA256 `142510c3a2615173b20b997f17b331a606484d9c1dcb0a9660a0a2f6bb21281e`。源码指纹及检查范围见交付目录 `build.json` / `source-files.sha256`。未重跑无关真实 Agent/UI 或昂贵性能负载。

## 未来日期样式补正

随后根据真机反馈，将紧凑布局中误恢复的点号改回每日期一个空心方框 `□`，仍占两个字符，保留日历宽度与命中。七种当前星期的既有断言先复现点号错误，再验证方框；日志在 `artifacts/calendar-future-cells/`。当时的测试程序为 `artifacts/DevTabs-0.4.0-calendar-future-cells/devtabs.exe`。此样式随后被文档 47 的统一灰色实心格取代。

# 29-Tree-Selection-Render-Fix

## 问题与原因

2026-11-11 真机截图显示：文件树最后一项被勾选后，下方空白区域连续出现 `*`。

`*` 是 DevTabs 的文件多选标记。tui-tree-widget 0.24.1 的 `TreeState::rendered_at` 返回给定位置上方最近的项目，因此末尾空白行也会返回最后一个项目。应用把它当作精确行查询，在整个侧栏高度上绘制标记，导致空白行被误判为同一已勾选文件。

相同查询用于文件点击、松开后预览、右键定位和悬停，不能只隐藏多余字符。

## 修复

在已有树视图缓存中记录实际绘制的 `hit_area`。其高度由已计算的展开行数、组件最终滚动偏移和可见区域决定；文件标签为单行。无需增加全树扫描或修改第三方依赖。

多选标记只遍历实际绘制行。文件交互统一经 `Sidebar::tree_hit`，先排除空白和边框，再调用组件查询。点击导致视口缓存失效时仍保留上次实际绘制区域，与组件的上次绘制命中信息保持一致。

源码：`src/sidebar.rs`、`src/sidebar/explorer.rs`。

## 回归

新增 `marked_last_tree_entry_does_not_fill_empty_rows`：

- 最后一项勾选后仅该行出现 `*`，空白行没有文件目标。
- 缓存重复绘制保持结果，取消勾选清除标记。
- 点击末尾空白不选中最后文件，不开始文件拖拽。
- 勾选后折叠目录仅保留目录实际所在行的标记。

修改前测试真实失败：期望 1 行标记，实际 25 行。证据为 `artifacts/tree-empty-before.log`；首次修复通过见 `artifacts/tree-empty-after.log`。

完整检查命令：

```powershell
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo test --locked --workspace --all-targets --target-dir target/perf -- --test-threads=1
cargo build --release --locked --target-dir target/perf
```

新构建保存至 `artifacts/DevTabs-0.4.0-tree-selection-fix/`，独立保留上一版程序。真实 UI 仍由用户验收，本轮不打开实际 WT／Agent 窗口。

最终完整回归为 25 个测试套件，288 项通过、0 项失败、22 项默认忽略；fmt、Clippy（`-D warnings`）、Release 构建和产物 `--help` 全部通过。构建 SHA256、源码摘要及日志索引见同目录 `build.json` 和 `source-files.sha256`。本轮未提交、推送。

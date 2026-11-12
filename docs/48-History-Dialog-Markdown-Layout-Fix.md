# History Dialog Markdown Layout Fix

2026-11-12。修复 F9 会话选择弹窗中列表下方的大面积空白和消息直接显示 Markdown 源码的问题。

## 原因与行为

旧实现把列表固定为弹窗内容高度的 35%，剩余条数不足时仍占据这些行；正文逐行显示原始字符串，未复用项目详情的 Markdown 渲染。

列表现在按筛选后的条数分配高度，最多 8 行，同时不超过可用高度的三分之一，超出部分沿用列表滚动；列表之后直接显示对话分隔线，其余高度全部留给消息。底部状态占一行。

正文复用 `preview::markdown_summary`，支持项目现有的文本 Markdown 排版，包括标题、强调、列表、表格与代码块。它采用文本预览，不调度图片解码或图形渲染。用户和 AI 与项目详情共用角色色标和消息侧线。

预览缓存只保留当前会话的排版结果。更换会话、宽度、主题或语言时重新排版；重新打开弹窗、后台历史刷新和消息正文加载后失效。滚动复用排版结果。该缓存不增加历史读取范围，读取和安全边界沿用现有适配器。

PageUp/PageDown 和鼠标滚轮继续滚动正文，右侧显示位置滚动条。渲染前将滚动位置限制到实际内容末尾，避免连续向下滚动后出现空白；新内容变短或窗口尺寸变化时也会校正位置。

## 验证

实际旧版 RED 日志：`artifacts/history-dialog-markdown/red.log`。新测试在旧实现下检测出列表无内容区域仍固定占高。

回归覆盖中英文、不同终端尺寸、1/7/20 条会话，验证列表高度、分隔间距、标题/粗体/代码围栏的 Markdown 转换，以及多条消息滚动至末尾仍显示最后内容、缓存复用、后台同身份会话更新后重新渲染、缩放、鼠标滚动和无结果筛选。

```powershell
cargo test --locked --lib --test overview --test project_pulse --test reference_parity --test i18n --target-dir target/perf -- --test-threads=1
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo fmt --check
```

相关回归 **176 通过、0 失败、9 忽略**，静态检查与 Release 构建结果、源码指纹记录在交付目录 `build.json` / `source-files.sha256`。TestBackend 验证不启动真实 Agent 或改写实际用户配置，真机 UI 由用户验收。

```powershell
.\artifacts\DevTabs-0.4.0-history-markdown\devtabs.exe
```

选择项目后按 F9，检查列表与对话之间没有不必要的大片空白，标题、粗体、表格和代码块已排版；PageDown 或滚轮滚到末尾后不应变为空白，筛选和切换会话仍可使用。

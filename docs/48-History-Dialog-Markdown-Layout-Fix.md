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

## 代码块背景与滚动条端点补正

真机反馈中，无语言围栏的空标题行显示一个小背景格，代码背景只与文字等宽，消息下方还出现零散背景格。文件预览和会话预览已经共用同一个 Markdown Writer；差异在输出适配：文件视口根据行样式填充背景，而会话文本行交给 Paragraph 后没有这一步。

本次在共用的 `markdown_summary` 输出适配层补齐每一条代码行（含空标题行、空代码行及换行尾部）的背景空格，保持文本区域全宽；把背景保留在内容 Span 上，避免调用方追加的角色侧线继承代码背景。总览详情和会话弹窗同时生效，语法解析、主题及排版继续共用原 Writer，不增加新的 Markdown 方案。

滚动条原来把总行数传作可滚动位置数量。在当前依赖的计算中，正确范围是 `总行数 - 视口行数 + 1`，其中 `+1` 包含位置 0。会话弹窗和共用 pane 均按同一口径修正；pane 同时按实际换行后的行数计算范围。顶部/底部滑块现在与正文位置一致。

使用用户原文作为 `tests/fixtures/history-code-background.md`。实际 RED 日志复现角色侧线被代码背景污染、末尾滑块仍在轨道中部；GREEN 验证浅/深色、不同宽度、代码全宽背景、角色边界和轨道两端。

额外捕获 CrosstermBackend 的实际 ANSI 字节，经项目现有 VT100 解析器回放，对照每一格背景，覆盖代码块、滚动往返和切换短中文消息，检查增量刷新不会在文字下方留下背景块；测试临时启用 ANSI 颜色输出后恢复进程设置，不启动真实 Agent 或终端窗口。真机像素效果仍由用户验收。

```powershell
cargo test --locked --lib --test overview --test reference_parity --test i18n --test markdown_viewport --test preview_rendering --target-dir target/perf -- --test-threads=1
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe
```

相关回归 **188 通过、0 失败、10 忽略**；fmt、Clippy、Release 及命令冒烟记录、构建指纹在新版交付目录 `build.json` / `source-files.sha256`，过程日志在 `artifacts/history-background-scrollbar/`。

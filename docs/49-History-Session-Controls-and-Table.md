# History Session Controls and Table

2026-11-12。根据真机反馈补齐会话 Markdown 的代码块操作，整理历史会话表与 F9 弹窗。

## 行为

- 文件预览、总览详情与会话弹窗继续共用 Markdown Writer，同时共用右上角语言/复制标签的布局函数。文本输出保留代码标题行和原始复制内容的关联，标题行不再只有空背景；无语言围栏不猜测语言，只显示复制按钮。
- 复制按钮只在对应标题行可见时注册点击区域。滚动、宽度变化、内容摘要截断、会话切换与缓存复用均保留正确的行位置；被其他弹窗遮挡的按钮无法点击。点击使用项目现有剪贴板接口，复制代码正文，不含围栏、语言标签或角色侧线。F9 底部显示短暂复制反馈。
- 中文角色由“你”改为“用户”，英文为 User。用户与 AI 继续使用不同色标。
- F9 打开时只绘制会话弹窗和空背景，跳过底层总览绘制；Esc 返回总览。提示与搜索区域按实际换行高度布局；列表与“最近对话”标题之间明确保留一行空白，区分选中会话与正文。
- “可续接会话”改名为“历史会话”。详情区增加名称、会话 ID、更新表头及分隔线。宽栏并排显示，窄栏将完整 ID 换行放在会话名称下，避免缩略后无法辨识。同一会话各行均能暂存该会话；默认展示数量和已有展开操作保持原样。
- ID 直接取现有历史适配器验证后的会话身份，不生成替代 ID。Codex 的 resume ID 已随历史读取，并参与会话名称/ID 搜索，本次增加可见展示。中英文目录同步更新。

## 验证

旧版 RED 在 `artifacts/history-session-controls/red.log` 复现弹窗外仍显示总览。新增及既有回归验证语言标签、复制内容、未知语言留空、离屏按钮移除、其他弹窗阻断、总览与 F9 两处命中位置、完整 ID 的宽/窄栏展示、弹窗背景隐藏、列表与正文标题间的一行留白、缓存与滚动边界，以及文件预览原有语言/复制行为。

```powershell
cargo test --locked --lib --test overview --test reference_parity --test i18n --test markdown_viewport --test preview_rendering --test project_pulse --target-dir target/perf -- --test-threads=1
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo fmt --check
```

相关回归 **197 通过、0 失败、10 忽略**。测试只检查剪贴板操作的内容与命中解析，不改实际剪贴板，不启动 Agent 或真实终端窗口；真机 UI 与点击复制由用户验收。静态检查、Release 与命令冒烟结果、源码指纹见交付目录 `build.json` / `source-files.sha256`。

```powershell
.\artifacts\DevTabs-0.4.0-history-session-controls\devtabs.exe
```

选择项目检查“历史会话”表头与 ID；按 F9 后检查底层总览已隐藏、列表与“最近对话”之间有一行空白、角色为“用户”、代码块右上角出现语言与复制入口。点击复制后粘贴到自行选择的位置，内容应为代码正文；Esc 后总览恢复。

## F9 弹窗表格补齐

F9 弹窗原有单列列表同步改为 Ratatui Table，显示“名称 / 会话 ID / 更新”三列表头和分隔线；ID 列固定 36 列，Codex UUID 完整展示，名称使用剩余宽度。顶部提示和搜索、底部正文预览、选中背景、输入筛选及原生选择器保持原有行为。

表头与数据命中区域分开，鼠标按实际表格滚动偏移选择会话，点击表头不改变选择；列表和“最近对话”之间保留一行空白，条数少时继续收缩高度。

新增测试在旧版实际复现缺少表头，修改后覆盖中英文、90/120/200 列、20 条会话、滚动选择、完整 ID、表头不可选和按 ID 筛选。相关 lib/overview/i18n 回归 **159 通过、0 失败、8 忽略**；静态检查、Release 和命令冒烟结果见新交付目录。过程日志在 `artifacts/history-dialog-table/`。

```powershell
.\artifacts\DevTabs-0.4.0-history-dialog-table\devtabs.exe
```

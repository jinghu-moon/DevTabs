# DevTabs

Windows Rust 项目工作区：原生 Codex / Claude Code 与 PowerShell Debug 会话由独立宿主管理。关闭标签页或退出前端后，会话继续运行；重新打开同一项目会复用工作区。

项目自身采用 [MIT License](LICENSE)。复用的 Codex Mermaid / 数学模块保留其 Apache-2.0 许可与来源说明，见 [第三方声明](THIRD-PARTY-NOTICES.md)。

当前版本 **0.4.0**，实现项目总览、共享 Debug 小窗、三种布局、文件树、代码/Markdown 预览、向 Agent 插入路径及单文件源码编辑。Markdown 阅读模式使用共享 Catppuccin 主题、按词换行、窄窗表格，以及有限 Mermaid / 数学公式排版。需要 Windows、PowerShell 7、Windows Terminal；所选 Agent 必须已在 PATH。编译需要 Rust 1.98+。

渲染改造与验证见 [12-Markdown-Renderer-Implementation.md](docs/12-Markdown-Renderer-Implementation.md)。测试样本见 [markdown-reading.md](tests/fixtures/markdown-reading.md)。

## 开始使用

本轮可执行文件为 `artifacts/DevTabs-0.4.0/devtabs.exe`。运行中的旧版 `target/release/devtabs.exe` 不会被覆盖。

四项性能优化后的测试程序为 `artifacts/DevTabs-0.4.0-optimized/devtabs.exe`。若同时运行旧版，重新编译不会更新已经打开的界面；总览也可能激活仍在运行的旧前端。测试当前渲染时，明确指定程序路径并加 `--new-instance`，在新的界面打开 `tests/fixtures/markdown-reading.md`：

```powershell
& "D:\100_Projects\110_Daily\new-project\artifacts\DevTabs-0.4.0-optimized\devtabs.exe" --project "D:\100_Projects\110_Daily\new-project" --agent codex --layout split --new-instance
```

可用 `Get-CimInstance Win32_Process -Filter "Name = 'devtabs.exe'" | Select-Object ProcessId, ExecutablePath, CommandLine` 核对实际运行路径。0.4.0 的本地源码渲染器已支持 `rust,no_run` 元数据、有限 Mermaid/公式、任务框和引用续行；0.3.2 的旧预览不能用于判断这些能力。针对这份完整样本，`cargo test --test markdown_viewport reading_fixture -- --test-threads=1` 覆盖实际 Sidebar 的 LF/CRLF、明暗主题、代码颜色、图形、公式及与完整渲染逐行一致性。

```powershell
# 首次设置默认 Agent 和项目根目录，可添加多个根
.\artifacts\DevTabs-0.4.0\devtabs.exe config agent codex
.\artifacts\DevTabs-0.4.0\devtabs.exe config root 'D:\100_Projects\110_Daily'

# 项目总览
.\artifacts\DevTabs-0.4.0\devtabs.exe

# 默认 Float：Agent 主界面 + 独立共享 Debug 小窗
.\artifacts\DevTabs-0.4.0\devtabs.exe --project 'D:\Projects\Demo' --agent claude --debug-command 'cargo run'

# 左右内嵌分屏 / Windows Terminal 两个标签页
.\artifacts\DevTabs-0.4.0\devtabs.exe --project 'D:\Projects\Demo' --layout split
.\artifacts\DevTabs-0.4.0\devtabs.exe --project 'D:\Projects\Demo' --layout tabs

# 原生历史选择器 / 显式另开实例 / 双 shell 诊断
.\artifacts\DevTabs-0.4.0\devtabs.exe --agent codex --resume
.\artifacts\DevTabs-0.4.0\devtabs.exe --new-instance
.\artifacts\DevTabs-0.4.0\devtabs.exe --agent shell --layout split
```

同一项目已经运行时，启动参数不会替换原会话；需要另一组进程时使用 `--new-instance`。标签页布局使用 DevTabs 命名的 WT 窗口，避免按标签序号或“最近窗口”定位。总览 Enter 优先激活可定位的现有工作区；否则打开视图并接管原会话，旧视图退出，不重启 shell。

## 操作

管理命令先按 **Ctrl+G**，松开，再按字母。其他输入交给当前原生程序。

| 操作 | 按键 |
| --- | --- |
| Agent / Debug / 下一会话 | A / D / N |
| 切换 Float → Split → Tabs | L |
| Debug 小窗切换置顶 | T |
| 正常停止 / 3 秒后强制停止 | K / Shift+F |
| 收起/展开文件树 / 聚焦文件树 | B / f（小写） |
| 插入选中文件绝对路径 / 恢复预览 | I / O |
| 重跑配置命令或上次托管命令 | R |
| 冻结选择 / 复制文本 | S / Y |
| 退出前端，后台继续 | X |
| 回到项目总览 | P |
| 关闭整个工作区的确认框 | Q |

Float 主界面右下角 **Open Debug** 恢复小窗；小窗的 **Collapse Debug** 或 `Ctrl+G X` 收起。关闭小窗也只分离前端。默认不置顶，恢复已打开的小窗保留置顶状态。切换 Float 项目时，共享小窗跟随当前项目。

总览支持搜索全部项目、方向键/鼠标选择、双击打开、Enter 新标签打开、Alt+Enter 启动选项；F2 收藏、F3 Debug 命令、F4 配置、F5 刷新，Ctrl+R 添加根目录、Ctrl+A 手动添加、Ctrl+E 排除。排序为收藏 → 最近 → 其他，运行中的项目标记 `[active]`。

滚轮查看历史；选择模式支持拖动、PgUp/PgDn 和复制。历史视图冻结时后台继续读取输出，恢复输入后回到实时画面。粘贴上限 1 MiB。默认保留 PowerShell Profile、Starship 与环境；`--no-profile` 用于诊断。

## 文件树、预览与编辑

Files 默认展开，目录按需加载，单击文件打开同一个只读预览。树与预览可同时显示；宽度不足时预览覆盖 Agent，缩窄窗口保持会话及最后有效尺寸。目录过大时使用 Load more；缓存上限 10,000 节点。F2 可释放折叠缓存、折叠全部、复制绝对/相对路径、在资源管理器定位。

默认使用终端现有 Nerd Font 和 devicons 字符，不安装或更改字体。字形显示不正常时，树内 F2 → Toggle file icons 切换 d/f 字符。`.` 切换隐藏项，I 切换忽略项；默认显示普通点文件，隐藏 .git 并遵循 ignore 规则。F5 手动刷新并重试监听。树内 `/` 仅搜索已加载名称，Enter 确认选择，再按 Enter 打开预览。

Markdown 默认阅读模式，m/M 切源码；阅读隐藏标题标记与代码围栏，保留标题和行内样式，代码块与文件共用高亮。TOML、PowerShell 等通过按需加载的扩展语法库高亮。其他文本支持行号、代码高亮和横纵滚动。预览内 `/` 查找，n / Shift+N 跳转，拖动选择后 Ctrl+C 复制；F2 可复制当前内容/源码、定位回树、打开磁盘版本和关闭面板。读取上限 1 MiB、10,000 行、单行 16,384 字符，超限只预览截断部分。受支持的 Mermaid 和数学公式转为终端字符排版；details 可折叠，其他 HTML 保留文本，图片保留说明/地址。链接需点击才打开。

只读预览正文与边框四周各留一格空白。Markdown 的代码块右上角显示语言和 `[Copy]`；点击可复制完整代码。阅读模式点击预览右上角 `[T]` 或按 `T` 打开可过滤的下拉大纲；支持鼠标选择、滚轮浏览，点击外部关闭。`Alt+Left` 返回上一次阅读位置。单击链接打开网页、相对文件或标题锚点；图片用系统应用打开。脚注可跳转并返回，details 可折叠，嵌套引用保留竖线。宽表格支持横向查看、滚动固定表头。文件树、预览和编辑区边框上的滚动条支持点击和拖动；源码/阅读切换、缩放和切文件保留阅读位置。实现和集中测试样例见 [阅读交互与滚动条](docs/18-Markdown-Reading-Interactions.md)。

预览支持 `Z` 最大化、`W` 代码换行、`D` 文件差异、`V` Markdown 源码对照；`G` 跳源码行或百分比。底部显示阅读位置和匹配数，搜索支持 Alt+C/W/R 切大小写、全词、正则。最多 8 个文件标签，单击或 Alt+PageUp/PageDown 切换；F2 提供全部入口。详见 [预览工作区](docs/19-Preview-Workspace.md)。

预览工具栏用可读名称显示常用操作，`[More]` / F2 收纳其他操作；灰色表示不可用，菜单显示开关状态。标签支持 `×` / 中键关闭及右键复制路径、关闭其他/右侧；悬停标签/标题显示绝对路径，Alt+C 复制当前路径。同名文件以目录区分，超过 8 个标签时提示淘汰文件。Alt+Left/Right 后退/前进，覆盖文件、标题、行号与搜索跳转。差异内 n / Shift+N 跳差异块，C 复制完整补丁。

预览内 B 添加带备注书签，Ctrl+B 打开项目书签列表；Enter/单击跳转，E 修改备注，Delete 删除，R 刷新。书签后台原子保存至数据目录的 bookmarks.json。I 打开文件信息，查看大小、编码、换行、UTC 修改时间和截断状态，C 复制；Esc 关闭。实现、边界与验收命令见 [预览操作与持久书签](docs/20-Preview-Controls.md)。图片仍使用系统应用打开。

从树拖一个文件到 Agent **内容区**，或选中文件后 Ctrl+G I，插入 `"绝对路径" `，不发送 Enter。Debug、边框、编辑器及 Shell 诊断会话不能作为目标。实际验证 Codex/Claude 的提示词编辑页；历史选择器、信任页面和模型工作中保持 Agent 原生行为，不保证输入框识别。成功提示仅表示输入链路接受，不表示模型已读取文件。

预览内 E 或标题 [E] 显式进入源码编辑（上限 512 KiB，编码后包含 BOM），点击文件仍默认只读。编辑器内普通字符均作为文本；Ctrl+S 保存、Ctrl+Z/Y 撤销/重做、Ctrl+C/X/V 复制/剪切/粘贴、Ctrl+F 查找、F3/F4 下一处/上一处、Tab/Shift+Tab 缩进。全部操作也可在 F2 菜单找到；WT 拦截快捷键时使用菜单。Markdown 的 Preview current content 使用未保存缓冲区，E 返回编辑。编辑标题 [S] 保存、[P] 预览当前缓冲区、[X] 关闭；预览内 M 或 More 菜单切换 Markdown 模式；底部 [Files] 收起/恢复文件树。Esc 优先清除查询/选区；F2 → End editing 或 Close preview 处理离开文档。

未保存时切文件、关闭面板、回总览、分离或正常退出会提示 S 保存、D 放弃、Esc 取消。切焦点、收起侧栏和切布局保留缓冲区；已有文档切至 Tabs 时保留当前主前端，单独启动 Debug 标签。外部打开明确区分保存后打开与打开磁盘版本。无自动保存、格式化或强制覆盖。

保存保留 UTF-8/UTF-8 BOM/UTF-16 LE/BE BOM、LF/CRLF 和末尾换行。混合换行、控制字符、截断文本、只读、UNC、链接/junction 和多硬链接保持只读。本地普通文件以同目录临时文件 + ReplaceFileW 保存，并核对完整磁盘快照及文件身份。外部修改或失败保留缓冲区；同路径多个 DevTabs 串行保存。ReplaceFileW 对新打开者可能出现短暂 NotFound，DevTabs 后台有界重试；不承诺外部读者能无间断打开。任意外部原子重命名仍有 Windows 检查至替换竞争窗口：提交备份核对发现竞争时保留 `.devtabs-*.bak` 被替换版本并明确报错，需外部处理；不承诺通用条件替换或断电持久性。强制杀进程和关闭 WT 不恢复未保存草稿。

## 生命周期与命令状态

关闭前端、关闭 WT 标签页或前端崩溃均不关闭后台会话。后台宿主退出、系统重启会结束会话；输出历史仅保存在内存。每会话默认 5000 行，`--history 0..20000` 可调整。

状态区分别显示 shell 生命周期与**托管命令**状态：Idle、Starting、Running、Exited(code)。托管命令是配置的 Agent/Debug/额外会话启动命令和 `run` 提交的命令；shell 中手输的任意命令不在此状态跟踪范围，也不推断 Agent 是否正在思考。

K 请求 Ctrl+C；等待 3 秒后 Shift+F 仅结束该会话的 shell 后代进程，保留 shell 和日志。R 通过私有 PSReadLine 绑定调度，取消未提交编辑行后执行托管命令。若个人 Profile 禁用了 PSReadLine，该请求等到下一次 prompt 执行。

`Ctrl+G Q`：Enter 请求可见会话 Ctrl+C，随后自行退出程序和 shell；等待 3 秒按 F 强制清理整个工作区（含额外会话）；Esc 取消。全部 shell 自行退出后宿主回收工作区。CLI 也支持明确关闭：

```powershell
devtabs list
devtabs --workspace '<WorkspaceId>'
devtabs stop '<WorkspaceId>' '<SessionId>'
devtabs stop '<WorkspaceId>' '<SessionId>' --force
devtabs run '<WorkspaceId>' '<SessionId>' --source 'cargo test'
devtabs layout '<WorkspaceId>' split
devtabs close '<WorkspaceId>'
devtabs shutdown  # 仅允许没有工作区时关闭宿主
```

## 配置与安装

配置保存在 `%APPDATA%\DevTabs\config.toml`，不会写入项目目录。`--data-dir` 或 `DEVTABS_HOME` 可隔离配置、宿主和共享小窗。格式与额外命名会话示例见 [实现设计](docs/03-Workspace-Implementation.md)。

```powershell
devtabs config add 'D:\Projects\Manual'
devtabs config exclude 'D:\Projects\Archive'
devtabs config favorite 'D:\Projects\Demo'
devtabs config agent claude --project 'D:\Projects\Demo'
devtabs config debug 'D:\Projects\Demo' 'cargo watch -x run'
devtabs config edit

# 当前用户安装到 %LOCALAPPDATA%\Programs\DevTabs，并添加用户 PATH
.\artifacts\DevTabs-0.4.0\devtabs.exe install

# 关闭新版工作区后，从另一份 exe 卸载；保留用户配置
.\artifacts\DevTabs-0.4.0\devtabs.exe uninstall
```

安装/卸载支持 `--directory` 指定目录、`--no-path` 跳过 PATH 修改。PATH 更新保留其他条目及 `%USERPROFILE%` 等变量；新终端获得新 PATH，已打开的 shell 保持原环境。不会修改 Profile 或 WT 配置。更新前先关闭安装目录内运行的 DevTabs。

## 性能统计

默认关闭。启动前设置 `$env:DEVTABS_PERF_DIR = "$PWD\artifacts\perf-session"` 后，每个新进程每 2 秒覆盖自己的 JSON 统计报告，正常退出补写。已运行宿主不会自动开启；需要完整统计时使用独立 `--data-dir` 启动工作区。结束后 `Remove-Item Env:DEVTABS_PERF_DIR`。启用方法、热点优化前后数据和回归证据见 [性能统计与优化](docs/09-Performance-Profiling-Optimization.md)。

## 开发与验证

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
cargo build --release --locked --target-dir target/perf

# 本机 Agent/Profile 与实际 WT 验证
cargo test --locked --test pty_windows -- --ignored --test-threads=1
cargo test --locked --test workspace_windows windows_terminal_layouts -- --ignored --nocapture

# 大目录与 100 项目 / 8 会话（包括预览、编辑和保存）性能验收
cargo test --release --locked --target-dir target/perf --test sidebar_windows release_large -- --ignored --nocapture
cargo test --release --locked --target-dir target/perf --test workspace_windows release_scale -- --ignored --nocapture
```

依赖版本由 Cargo.lock 固定，本轮采用核对后的最新稳定版本。侧栏与编辑升级结果、构建哈希和实测边界见 [侧栏验证](docs/07-File-Sidebar-Verification.md)、[编辑验证](docs/08-File-Editor-Verification.md)。原工作区升级记录见 [04](docs/04-Upgrade-Verification.md)。[01](docs/01-Embedded-Terminal-MVP.md)、[02](docs/02-MVP-Verification.md) 为前台 MVP 历史记录。

预览渲染修复、可直接打开对照的 Markdown 示例与测试证据见 [10](docs/10-Preview-Rendering-Fixes.md)。

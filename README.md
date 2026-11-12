# DevTabs

Windows Rust 项目工作区：原生 Codex / Claude Code 与 PowerShell Debug 会话由独立宿主管理。关闭标签页或退出前端后，会话继续运行；重新打开同一项目会复用工作区。

项目自身采用 [MIT License](LICENSE)。复用的 Codex Mermaid / 数学模块保留其 Apache-2.0 许可与来源说明，见 [第三方声明](THIRD-PARTY-NOTICES.md)。

当前版本 **0.4.0**，实现项目总览、界面内终端浮层、三种布局、文件树、代码/Markdown 预览、向 Agent 插入路径及单文件源码编辑。Markdown 阅读模式使用共享 Catppuccin 主题、按词换行、窄窗表格，以及有限 Mermaid / 数学公式排版。需要 Windows、PowerShell 7、Windows Terminal；所选 Agent 必须已在 PATH。编译需要 Rust 1.98+。

同项目多 Agent 并行开发已支持独立 worktree、同屏横排／竖排与轮次审阅，操作见文末；设计见 [并行工作区](docs/37-Parallel-Agent-Workspace-Design.md)，实现与真机验收边界见 [验证记录](docs/40-Parallel-Agent-Workspace-Verification.md)。

渲染改造与验证见 [12-Markdown-Renderer-Implementation.md](docs/12-Markdown-Renderer-Implementation.md)。测试样本见 [markdown-reading.md](tests/fixtures/markdown-reading.md)。

## 开始使用

最新测试程序为 `artifacts/DevTabs-0.4.0-history-background-scrollbar/devtabs.exe`，包含共享 Markdown 代码块全宽背景、会话滚动条端点修复、自适应高度的会话弹窗、Markdown 消息与角色标记、对话滚动边界保护、背景色填充的连续热力图、未来日期统一灰色块、统计栏最小 32 列保护、统计弹窗背景隔离和旧启动记录读取修复、紧凑连续色块日历、局部对齐的按钮与图例、月份刻度、12 周／全年切换、悬停详情与精简图例，以及无数量/时间硬截断的完整统计扫描、后台进度、明确的根目录文件标签，以及单行项目与可展开会话、最近用户消息修复、常驻返回总览按钮、提交热力图与改动热点，以及详情分区、可点击搜索和宿主断线恢复、按实际栏宽排版的统计卡片与目录扫描优化、响应式项目/详情/概况三栏、Markdown 工作上下文、可审阅启动队列、真实单 Agent 终端组合，并保留侧栏、图片、编辑、终端浮层与中英文界面；构建哈希与检查记录在同目录 `build.json`。当前 IPC 使用 v6 并按构建自动隔离宿主，避免新版连接到仍在运行的旧宿主；`--version` 可查看构建标识。旧版运行中的窗口和会话由旧版正常退出，新版不接管。详见 [宿主构建隔离](docs/31-Host-Build-Isolation.md)。

若同时运行旧版，重新编译不会更新已经打开的界面；同一构建的总览也可能激活已运行的前端。测试当前渲染时，明确指定程序路径并加 `--new-instance`，在新的界面打开 `tests/fixtures/markdown-reading.md`：

```powershell
& "D:\100_Projects\110_Daily\new-project\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe" --project "D:\100_Projects\110_Daily\new-project" --agent codex --layout split --new-instance
```

可用 `Get-CimInstance Win32_Process -Filter "Name = 'devtabs.exe'" | Select-Object ProcessId, ExecutablePath, CommandLine` 核对实际运行路径。0.4.0 的本地源码渲染器已支持 `rust,no_run` 元数据、有限 Mermaid/公式、任务框和引用续行；0.3.2 的旧预览不能用于判断这些能力。针对这份完整样本，`cargo test --test markdown_viewport reading_fixture -- --test-threads=1` 覆盖实际 Sidebar 的 LF/CRLF、明暗主题、代码颜色、图形、公式及与完整渲染逐行一致性。

```powershell
# 首次设置默认 Agent 和项目根目录，可添加多个根
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe config agent codex
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe config root 'D:\100_Projects\110_Daily'

# 项目总览
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe

# 只创建 Agent，一个 PTY；保留文件树与预览
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --project 'D:\Projects\Demo' --composition agent-only

# 默认 Float：Agent 主界面 + 界面内终端浮层
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --project 'D:\Projects\Demo' --agent claude --debug-command 'cargo run'

# 左右内嵌分屏 / Windows Terminal 两个标签页
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --project 'D:\Projects\Demo' --layout split
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --project 'D:\Projects\Demo' --layout tabs

# 原生历史选择器 / 显式另开实例 / 双 shell 诊断
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --agent codex --resume
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --new-instance
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe --agent shell --layout split
```

同一项目已经运行时，启动参数不会替换原会话；需要另一组进程时使用 `--new-instance`。标签页布局使用 DevTabs 命名的 WT 窗口，避免按标签序号或“最近窗口”定位。总览 Enter 优先激活可定位的现有工作区；否则打开视图并接管原会话，旧视图退出，不重启 shell。

## 总览操作

直接运行 exe 进入总览。宽度达到 160 列默认项目/详情/概况三栏，104–159 列优先项目/详情，60–103 列通过 F10/F8 显式打开详情/概况弹窗。拖动分隔线调整栏宽，各栏独立滚动。

- 点击搜索框或按 `/` 聚焦并即时筛选，Enter / Esc 返回项目列表；F6 切换筛选，选中标签以填充背景标识。
- Enter 打开当前项目；Space 或编号勾选，Ctrl+Enter 只执行勾选队列。点击底部摘要审阅、调序和删除，Alt+↑↓ 调序，Delete 移除，Ctrl+Z 撤销。
- ←→ 临时切换 Agent；Alt+Enter 调整本次启动，Ctrl+S 暂存入队；F4 保存默认设置，两者相互独立。会话历史 F9 只暂存选择，直接输入搜索，Alt+P 原生选择、Alt+N 新会话；项目箭头或 Alt+←→ 收起／展开会话子行。
- F3 编辑下一步便签，E 展开详情；改动文件单击选中后 Enter 只读查看，Esc 返回。Alt+拖动选择详情/概况文字，Ctrl+C 显式复制；无选区时复制项目路径。
- 命令面板的“加入全部待恢复”只追加队列，审阅后 Ctrl+Enter 执行；CLI `--restore` 仍直接恢复。F1 完整帮助，Ctrl+P 命令面板，Ctrl+L 语言，Ctrl+T 主题。

`--composition agent-only` 创建一个终端；`agent-debug` 创建两个；`custom` 创建 Agent、Debug 和配置的全部额外命名终端。切换组合不会删除额外终端配置，视觉布局由 `--layout` 独立决定。

设计与实测见 [总览重构](docs/33-Overview-Refactor-Design.md)、[验证记录](docs/34-Overview-Refactor-Verification.md)。统计面板新样式与老项目的速度对照见 [统计优化](docs/35-Statistics-Panel-Refinement.md)。详情、搜索与宿主连接修复见 [验证记录](docs/36-Overview-Detail-Interaction-Fixes.md)。本轮统计、会话、扫描口径与返回导航见 [项目脉搏](docs/38-Project-Pulse-and-Session-Navigation.md)。完整扫描的实测、资源取舍和复现命令见 [统计扫描验证](docs/42-Complete-Statistics-Scan-Verification.md)。全年视图和日期详情见 [日历重构](docs/44-Contribution-Calendar-Refactor.md)，当前格子样式及宽度修复见 [紧凑日历](docs/45-Compact-Calendar-Layout.md)，弹窗与旧启动记录修复见 [弹窗合成](docs/46-Statistics-Modal-Composition-Fix.md)。

统计日历默认近 12 周；点击顶部 `[年]` / `[1y]` 或聚焦统计后按 Y 切全年，`[+]` / Z 放大。悬停或点击格子查看日期与提交次数；全年需要宽窗，空间不足时保留近 12 周。

## 操作

管理命令先按 **Ctrl+G** 打开命令菜单，松开，再按字母或点击执行；Esc 取消。其他输入交给当前原生程序。

工作区底栏常驻可点击“← 总览”，返回后后台继续运行，并选回原项目。总览项目列表底部可点击“＋ 添加项目”。工作区采用单行底栏，阅读进度位于预览下边框。标题操作在聚焦/悬停时出现，更多入口始终保留；浮窗阴影参与图片裁剪。详见 [工作区界面与交互统一](docs/32-Workspace-Chrome.md)。

总览后续改版设计见 [总览界面重构设计](docs/33-Overview-Refactor-Design.md)，涵盖默认三栏、最近对话、启动队列、图形统计及分阶段验收；实现与验证见上述记录。

| 操作 | 按键 |
| --- | --- |
| Agent / Debug / 下一会话 | A / D / N |
| 切换 Float → Split → Tabs | L |
| 移动当前终端浮层 | T |
| 放大/还原当前面板（保留手动栏宽） | Z |
| 终端会话菜单：停靠／悬浮／收起 | W |
| 正常停止 / 3 秒后强制停止 | K / Shift+F |
| 收起/展开文件树 / 聚焦文件树 | B / f（小写） |
| 插入选中文件绝对路径 / 恢复预览 | I / O |
| 重跑配置命令或上次托管命令 | R |
| 冻结选择 / 复制文本 | S / Y |
| 退出前端，后台继续 | X |
| 回到项目总览 | P |
| 关闭整个工作区的确认框 | Q |

Agent、Debug 和额外命名终端均支持停靠、折叠和界面内悬浮。标题点击“收起／悬浮”，窄栏使用 `[...]`；Ctrl+G → W 打开菜单，D 停靠、F 悬浮、C 收起、M 移动、R 调整尺寸。拖动浮层标题移动，拖动右下角调整大小；键盘模式方向键调整，Enter 确认、Esc 取消。浮层始终在当前 TUI 内，不新开 WT 窗口或标签；位置和尺寸按字符格保存。收起只释放显示，后台继续运行；切换目标不重启进程。详见 [通用终端折叠与悬浮](docs/28-Collapsible-Terminal-Windows.md)。

总览支持项目/便签搜索、方向键/鼠标选择、双击打开、Enter 打开或复用工作区、Alt+Enter 启动选项。F2 收藏、F3 下一步便签、F4 项目设置表单、F5 刷新、F6 筛选、F7 排序、F8 统计、F9 Agent 历史、F10 详情；Space 按顺序勾选，Ctrl+Enter 批量启动，Ctrl+S 保存完整启动预设，R 重试失败项。Ctrl+P 打开中英文搜索命令面板，Ctrl+T 切主题；Ctrl+R 添加根目录、Ctrl+A 手动添加、Ctrl+E 排除。收藏/最近项目可分组折叠，宽屏并排显示详情和统计，窄屏使用弹窗。详见 [总览工作区](docs/25-Project-Overview-Workspace.md)。

总览新增 F1 帮助、Ctrl+Shift+A 可见项全选、Ctrl+N 新建 Git 项目、Ctrl+B 预设管理。预设 Enter 只套用，Alt+Enter 调整后 Ctrl+S 暂存，Ctrl+Enter 确认启动。F4 支持标题、颜色、Agent 参数与最近会话策略；命令面板可导出 `.devtabs.toml`、恢复中断工作区或重开上次组合。`--restore` / `--last` 也可直接调用；`--fresh` 强制新会话，`--new-window` 指定新 WT 窗口。F8 新增 12 周提交趋势和文件增删行，统计持久缓存、F5 强制刷新。见 [参考项目对标与验证](docs/27-Reference-Feature-Parity.md)。

界面国际化使用 `rust-i18n 4.2.4`，支持自动识别系统语言、简体中文和英文，资源编译进程序。总览 Ctrl+L，工作区 Ctrl+G → U 或 F2 → Language 切换并保存；`devtabs config language zh-cn` 持久设置，`devtabs --language en-us` 覆盖当前前端。用户路径、代码、命令和原生终端输出保持原文；Clap 帮助、底层错误和部分旧诊断文案仍为英文。实测与维护规则见 [国际化验证](docs/26-Internationalization-Verification.md)。

滚轮查看历史；选择模式支持拖动、PgUp/PgDn 和复制。历史视图冻结时后台继续读取输出，恢复输入后回到实时画面。粘贴上限 1 MiB。默认保留 PowerShell Profile、Starship 与环境；`--no-profile` 用于诊断。

## 文件树、预览与编辑

文件树／Agent、Agent／预览之间的竖分隔线支持左键左右拖动，释放保存，Esc 取消；文件树滚动条优先处理上下滚动，调整栏宽可拖其右侧紧邻的 Agent 左边框。图片与 Debug 浮层可以同时显示，仅重叠部分被遮住；浮层静止时终端输出不重新处理或传输图片。见 [图片共存与三栏拖动](docs/30-Image-Panel-Composition.md)。

Files 默认展开，目录按需加载，单击文件打开同一个只读预览。树与预览可同时显示；宽度不足时预览覆盖 Agent，缩窄窗口保持会话及最后有效尺寸。目录过大时使用 Load more；缓存上限 10,000 节点。F2 可释放折叠缓存、折叠全部、复制绝对/相对路径、在资源管理器定位。

侧栏支持 Ctrl+P 全项目模糊查找、自动定位、Git 状态与 G 仅已修改筛选、紧凑目录、Ctrl/Shift 多选、批量预览/复制/拖入 Agent，以及新建文件/目录、重命名和回收站删除。拖动分隔线调宽，按项目记忆。新建 Ctrl+N / Ctrl+Shift+N，重命名 F4，删除 Delete，全部操作均有 F2 / 右键菜单入口。详见 [侧边栏升级](docs/24-Sidebar-Explorer.md)。

默认使用终端现有 Nerd Font 和 devicons 字符，不安装或更改字体。字形显示不正常时，树内 F2 → Toggle file icons 切换 d/f 字符。`.` 切换隐藏项，I 切换忽略项；默认显示普通点文件，隐藏 .git 并遵循 ignore 规则。F5 手动刷新并重试监听。树内 `/` 仅搜索已加载名称，Enter 确认选择，再按 Enter 打开预览。

Markdown 默认阅读模式，m/M 切源码；阅读隐藏标题标记与代码围栏，保留标题和行内样式，代码块与文件共用高亮。TOML、PowerShell 等通过按需加载的扩展语法库高亮。其他文本支持行号、代码高亮和横纵滚动。预览内 `/` 查找，n / Shift+N 跳转，拖动选择后 Ctrl+C 复制；F2 可复制当前内容/源码、定位回树、打开磁盘版本和关闭面板。读取上限 1 MiB、10,000 行、单行 16,384 字符，超限只预览截断部分。受支持的 Mermaid 和数学公式转为终端字符排版；details 可折叠，其他 HTML 保留文本，本地静态图片显示为预览块，远程图片保留说明/地址。链接需点击才打开。

只读预览正文与边框四周各留一格空白。Markdown 的代码块右上角显示语言和 `[Copy]`；点击可复制完整代码。阅读模式点击预览右上角 `[T]` 或按 `T` 打开可过滤的下拉大纲；支持鼠标选择、滚轮浏览，点击外部关闭。`Alt+Left` 返回上一次阅读位置。单击链接打开网页、相对文件或标题锚点；本地图片点击进入独立图片标签，远程图片需明确点击后外部打开。脚注可跳转并返回，details 可折叠，嵌套引用保留竖线。宽表格支持横向查看、滚动固定表头。文件树、预览和编辑区边框上的滚动条支持点击和拖动；源码/阅读切换、缩放和切文件保留阅读位置。实现和集中测试样例见 [阅读交互与滚动条](docs/18-Markdown-Reading-Interactions.md)。

预览支持 `Z` 最大化、`W` 代码换行、`D` 文件差异、`V` Markdown 源码对照；`G` 跳源码行或百分比。底部显示阅读位置和匹配数，搜索支持 Alt+C/W/R 切大小写、全词、正则。最多 8 个文件标签，单击或 Alt+PageUp/PageDown 切换；F2 提供全部入口。详见 [预览工作区](docs/19-Preview-Workspace.md)。

预览工具栏用可读名称显示常用操作，`[More]` / F2 收纳其他操作；灰色表示不可用，菜单显示开关状态。标签支持 `×` / 中键关闭及右键复制路径、关闭其他/右侧；悬停标签/标题显示绝对路径，Alt+C 复制当前路径。同名文件以目录区分，超过 8 个标签时提示淘汰文件。Alt+Left/Right 后退/前进，覆盖文件、标题、行号与搜索跳转。差异内 n / Shift+N 跳差异块，C 复制完整补丁。

Windows Terminal 图片预览已接入 Sixel/Half-block、独立图片标签、后台解码与有界缓存、Markdown 可视区裁剪及性能统计；操作及真机验收见 [终端图片预览设计](docs/21-Terminal-Image-Preview-Design.md) 与 [实现验证](docs/22-Terminal-Image-Preview-Verification.md)。

预览内 B 添加带备注书签，Ctrl+B 打开项目书签列表；Enter/单击跳转，E 修改备注，Delete 删除，R 刷新。书签后台原子保存至数据目录的 bookmarks.json。I 打开文件信息，查看大小、编码、换行、UTC 修改时间和截断状态，C 复制；Esc 关闭。实现、边界与验收命令见 [预览操作与持久书签](docs/20-Preview-Controls.md)。图片为只读预览，O 使用系统应用打开原文件。

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
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe install

# 关闭新版工作区后，从另一份 exe 卸载；保留用户配置
.\artifacts\DevTabs-0.4.0-history-background-scrollbar\devtabs.exe uninstall
```

安装/卸载支持 `--directory` 指定目录、`--no-path` 跳过 PATH 修改。PATH 更新保留其他条目及 `%USERPROFILE%` 等变量；新终端获得新 PATH，已打开的 shell 保持原环境。不会修改 Profile 或 WT 配置。更新前先关闭安装目录内运行的 DevTabs。

## 终端图片预览

文件树可预览 PNG、JPEG、静态 WebP、静态 SVG；支持适应窗口、100%、缩放、平移、滚动条、信息、外部打开，以及 Markdown 中的本地图片。图片只读，拖到 Agent 仍仅插入绝对路径。GIF/APNG/动态 WebP、PDF 和自动远程图片下载不在支持范围内。

默认 `--image-protocol auto`：当前前端的协议与字符单元像素尺寸均确认后启用 Sixel，否则使用 Half-block。`--image-protocol halfblock` 可强制字符预览；已知单元尺寸时，可用 `--image-protocol sixel --image-cell-size 9x18` 进行诊断。单元尺寸必须使用本机实际值。

图片获得焦点时：`0` 适应，`1` 100%，`+/-` 缩放，方向键/滚轮平移，`Ctrl+滚轮` 缩放，`Shift+滚轮` 水平平移，`P` 切换画质，`I` 信息，`O` 外部打开，`F5` 重试，`Z` 最大化，`F2`/More 更多操作。Markdown 图片点击后进入图片标签，`Alt+左方向键` 返回。

适应模式同时考虑窗口与目标像素预算；100% 保持真实像素比例。Markdown 滚动时保留图片高度与缓存，暂时显示占位，停下 150 ms 后恢复可见图片；图片更新仅重绘相关区域，不清空整个终端。

SVG 使用 `resvg 0.49.0` 按可见区域与缩放比例直接绘制矢量，支持路径、渐变、裁剪及系统字体文字；脚本和动画不执行。SVG 内的图片引用（文件、URL、data image）暂时省略，信息面板会说明；首期仅支持 UTF-8、无 DTD 的静态 SVG。操作、样例与验证见 [SVG 预览](docs/23-Svg-Preview.md)。

资源限制、测试、性能对照与真机验收入口见 [设计](docs/21-Terminal-Image-Preview-Design.md)、[实现验证](docs/22-Terminal-Image-Preview-Verification.md)。图片由前端绘制，不透传 Agent/Debug 的图片协议。

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

## 并行任务、轮次审阅和提醒

同一终端内使用底栏「＋ 新任务」或 `Ctrl+G → +` 从冻结提交创建独立 worktree；不复制未提交文件、不切换原分支。`Ctrl+G → V` 切换横排/竖排，`[` / `]` 切任务，`C` 固定当前两任务，`Z` 放大/还原。`← 总览` 保留后台进程。

`Ctrl+G → g` 打开只读轮次审阅：`b` 建立基线、`e` 结束、`s` 设置非 Git 快照范围（相对目录，分号分隔），`l` 历史、`[` / `]` 选轮、方向键选文件、Enter 差异、`n` 下一差异块、`/` 搜索、`o` 显式打开当前文件。Git 记录包含 dirty 起点与中途提交；默认非 Git 只观察路径事件，不提供不存在的旧内容。自动捕获标明晚起点；Stop 仅代表 Agent 报告，不能当作最终成功。

`Ctrl+G → !` 或右下角待处理入口查看事项；`a` 确认、`m/u` 静音/取消、`1` 下一次结束、`5/3` 延后 5/30 分钟、`0` 取消、`g` 全局开关、`d/y` 诊断/复制。后台提醒不抢输入；系统通知点击只路由已有视图，不自动启动 Agent。真实横幅与 WT 精确激活仍待真机验证。

以下命令均需用户显式执行；安装 hooks 不修改 Provider 信任数据库，仍须在对应 Agent 内审阅/启用：

```powershell
devtabs hooks codex  # 或 claude；--config 指定配置，--remove 移除自己拥有的 hook
devtabs notify register  # 显式注册当前用户通知身份；--remove 移除
devtabs notify diagnostics
devtabs notify test  # 明确发送系统测试通知，不创建未读事项
devtabs notify after-check '<TaskId>'  # 仅受管检查结束时提醒
devtabs check '<WorkspaceId>' '<TaskId>' -- cargo test
devtabs check '<WorkspaceId>' '<TaskId>' --status
devtabs overlap '<TaskA>' '<TaskB>'  # 完整成果路径交集；查询不完整会报错
```

`.devtabs.toml` 可配置 `prepare` 命令数组及 `check`，每条命令使用 `argv`、`cwd`、`env`、`timeout_ms`；均为显式本地命令，不从 Agent 文字提取。初始化成功步骤按 task/命令摘要记录，失败重试跳过已成功步骤。检查使用独立受管进程，显示退出码、耗时及代码变化后的过期状态，不向 Agent/Debug 输入命令。详见 [实施证据与真机清单](docs/40-Parallel-Agent-Workspace-Verification.md)。

`Ctrl+G → M` 打开「任务合并与归档」：选择目标，`m` 准备 fast-forward 摘要、`p` 切换是否要求来源检查、`d` 准备清理、`a` 准备归档，Enter 明确确认，`q` 查持久进度。合并双方必须停止受管终端（保留视图草稿）；清理须先保存/放弃草稿、关闭来源终端，再分离相关前端。`y` 可复制已准备操作的执行命令到独立 shell，不自动执行。来源与目标 HEAD/路径/资源身份变化会拒绝旧操作。

```powershell
devtabs task list
devtabs task merge '<SourceTask>' '<TargetTask>' --require-check  # 只准备、打印摘要及 OperationId
devtabs task cleanup '<SourceTask>' '<MergedIntoTask>'  # 只准备，包含 ignored 文件保护
devtabs task close '<TaskId>' --confirm '<TaskId>'  # 明确终止本任务所有终端，其他任务保留
devtabs task apply '<OperationId>' --confirm '<OperationId>'  # 确认后重新验证并执行
devtabs task show '<OperationId>'  # 失败/部分成功查询
devtabs task archive '<TaskId>' --confirm '<TaskId>'  # 仅归档，不删除目录或停止终端
```

清理仅处理有持久创建所有权的隔离 worktree，要求已合入指定目标且无 tracked/untracked/ignored 内容；只调用普通 `git worktree remove` 与 `git branch -d`。原项目、外部采用目录和重建资源受到保护。目录移除与分支删除分步记录，失败保留真实阶段；原操作重试不删除后来重建的同名目录/分支。检查通过仅对来源代码版本有效，合入目标后仍需独立测试。外部编辑器、Git 和其他工具不受 DevTabs 锁控制，执行前应停止其并发写入。

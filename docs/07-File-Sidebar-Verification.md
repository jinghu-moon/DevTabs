# DevTabs File Sidebar Verification

日期：2026-11-10。交付版本 0.3.0。范围对应 [06 第 1–12 节](06-File-Sidebar-Implementation.md)，编辑阶段见 [08](08-File-Editor-Verification.md)。

## 实现与升级前证据

已实现独立 Files / Preview 面板、按需树加载、ignore/隐藏过滤、Nerd Font 图标及 d/f 回退、文件操作菜单、名称定位、代码/Markdown 预览、当前模式查找/选择复制、非递归 notify、内部拖放和键盘路径插入。覆盖预览保持 Agent 连接；绘制、命中和 resize 使用同一个布局结果。文件操作没有新增宿主服务或 IPC。

原始源码、锁文件、测试输出与基准保存在 [sidebar-baseline](../artifacts/sidebar-baseline/)，对应 `checks.json` 和 `00.log`–`03.log`。升级前 fmt、Clippy 和 release 基准退出码为 0；原完整测试退出码 101，已有 `detached_output_reconnect_and_layouts_keep_same_processes` 使用固定 2 秒等待，偶发未等到 TICK-29。修复为等待真实托管命令 Exited(0)，仍断言输出及 shell 身份；没有删除/跳过断言。

只读功能切点的完整检查见 [sidebar-verification/checks.json](../artifacts/sidebar-verification/checks.json)。当时通过 12 个单元、4 个 PTY、7 个侧栏、7 个宿主测试，并单独通过真实 Profile/Agent 启动及 WT 布局。该切点源码、锁文件、可执行文件及测量输出另存 [editor-baseline](../artifacts/editor-baseline/)，供第二阶段升级对比。原生文件插入的完整端到端验收在最终集成版本补齐，不把当时的启动 smoke 当作拖放已通过。

## 自动验收与原生 Agent

最终两阶段共用的检查命令、耗时及退出码见 [final-checks.json](../artifacts/editor-verification/final-checks.json)，完整集成日志为同目录 `00-final.log`–`07-final.log`；保存读者修复与最终交付复查见 `09-reader-review.log`、`10-native-ready.log`、`11-delivery.log`–`14-delivery.log`。后补普通读者并发保存测试发现的 Windows 命名窗口及修复见 08；修复后最终全量复查见 `09-reader-review.log`。

| 范围 | 实际证据 |
| --- | --- |
| 布局/焦点 | 文档宽度示例、全部临界宽高与隐藏/预览组合；窗口缩小恢复、菜单、内部拖动覆盖切换、Debug 非落点 |
| 树与加载 | ignore 否定规则、普通点文件、目录延迟加载、折叠选择修复、1,100 项分页、过期批次丢弃、30,000 项 release 夹具 |
| 预览 | UTF-8/BOM、两种 UTF-16、二进制/控制字符、限长、Markdown/源码切换、搜索、快速切文件和过期结果 |
| 更新 | 文件替换与删除保留旧内容/提示、滚动位置保留、F5；监听关闭后恢复重读 |
| 原生 Codex | 鼠标从树拖文件，绝对路径在 LEFTRIGHT 中间出现一次，保留两侧文字，不发 Enter |
| 原生 Claude | 在测试自行生成的目录进入原生提示词页，树定位后 Ctrl+G I 插入，保留两侧文字，不发 Enter |
| 既有回归 | 真实双 shell、输入隔离、输出洪流、重连、后台进程不重启、命令停止/强制/重跑、额外会话、配置安装和实际 WT 三布局 |

原生屏幕留档：[Codex 拖放](../artifacts/codex-file-drop.txt)、[Claude 插入](../artifacts/claude-file-insert.txt)。版本：Codex CLI 0.167.0、Claude Code 2.1.281。没有向模型提交提示词。Claude 测试仅对自己生成的临时夹具选择信任；不替用户信任实际项目。

CLI 的恢复会话选择器、信任页与正在工作的页面只保持原生行为。新目录没有可恢复的测试会话，未创建模型对话来伪造 resume 验收；已验证 resume 启动/选择器，进入已恢复会话后的提示词页仍需用户在自己的会话复核。插入成功仅表示宿主输入通道接受，不能据此识别 Agent 内部页面。

## 性能结果与问题修复

统一口径：100 项目、8 个活动 shell、4 个工作区前端、1 个总览、1 个宿主，共 6 个 DevTabs 进程；20 逻辑处理器。统计所有这些 DevTabs 工作集，不含 PowerShell/Agent、Windows Terminal/OpenConsole、编译器与测试进程。没有裁剪 working set。

升级前工作集 72.41 MiB；只读切点为 79.95 MiB，已连接前端首帧 77–93 ms，总览搜索 15.47 ms，空闲整机 CPU 0.117%。原始数值分别在两份基线目录。

最初 30,000 项测试发现合并节点反复线性去重，后期批次约 200–294 ms，失败保留在 `sidebar-verification/06-large.log`。修复为 HashSet 去重和缓存排序键，未改变验收阈值；复测日志 `07-large-fixed.log` 及最终 `editor-verification/05-final.log` 均通过。最终批次、绘制、冷/热渲染实测以 [sidebar-large-release.json](../artifacts/sidebar-large-release.json) 为准；达到 10,000 节点时明确不完整，未把全部 30,000 项塞入缓存。

带编辑/保存的最终扩展基准见 08。加载大目录的完整总耗时可超过 200 ms；目标针对单批，期间 UI 与 PTY 不等待整目录完成。

## 环境与尚需人工复核的边界

环境留档：[environment.json](../artifacts/editor-verification/environment.json)。Windows 11 build 26100、Rust 1.98.1、WT 1.24.12741.0；用户默认 FiraCode Nerd Font，Ctrl+C / Ctrl+V 由 WT 配置接管。未修改字体、Profile、WT 配置或参考项目。

自动化验证了字符宽度计算、树命中与普通字符回退，真实 WT 测试验证会话和布局；Nerd Font 实际字形外观、不同字号/缩放的私用区字形宽度仍需人工观察。Windows IME 候选框和右键粘贴边界见 08，不以控制台注入代替这些检查。图片/PDF/Office 内嵌预览、递归目录链接、文件管理与多文件编辑不属于本轮范围。

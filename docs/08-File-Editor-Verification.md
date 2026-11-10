# DevTabs File Editor Verification

日期：2026-11-10。交付版本 **0.3.0**，实现 [06 第 13 节](06-File-Sidebar-Implementation.md) 的单文件源码编辑；只读阶段记录见 [07](07-File-Sidebar-Verification.md)。

## 基线、组件与实现

第二阶段升级前源码、Cargo.lock、完整检查输出副本、release 可执行文件和性能结果保存在 [editor-baseline](../artifacts/editor-baseline/)。基线可执行文件 SHA-256 为 `376db92418fb6832d891e628c878e7821ade7274e9bd1d0fb9407f295ca0d45e`，详情见该目录 build.json；没有用 0.2.0 的旧结果冒充编辑阶段基线。

编辑使用 **ratatui-textarea 0.9.4** 的光标、选择、编辑操作、查找及撤销/重做，不自制编辑算法。与 Ratatui 0.30.2 / Crossterm 0.29.0 类型统一；regex 1.13.1。Syntect 5.3.0、tui-syntax-highlight 0.2.0 与 tui-markdown 0.3.13 统一 Oniguruma 后端，避免同时加载 fancy-regex；依赖树输出在 editor-verification。

一个活动文档保留一份可修改缓冲区，Document / Editor / Save 共享不可修改的磁盘快照。高亮结果共享 Arc，编辑模式不构建额外的预览查找文本副本；纯 txt/log 不加载语法集合。最多 2 个文件工作线程，任务队列 16、结果队列 8，所有磁盘写入、解码、高亮都在工作线程。

默认点击仍只读，E / [E] 显式编辑。已实现光标/选区、中文 Unicode 提交、复制剪切粘贴、撤销重做、查找、Tab 缩进、行号/高亮、Ctrl+S / [S] 保存、未保存确认、冲突保留、外部打开确认、Markdown 未保存缓冲区投影和后台监听。标题 [X] 关闭、[P] 投影、[M] 阅读/源码切换；底部 [Files] 可点击恢复侧栏。F2 菜单提供被 WT 拦截快捷键的替代入口。

## 内容与保存边界

编辑最终上限 **512 KiB（编码后含 BOM）/ 10,000 行 / 单行 16,384 字符**，只读读取仍允许 1 MiB。撤销历史上限 50 次并按约 4 MiB 保守预算缩减次数。超限粘贴或缩进以候选缓冲区验证，整体拒绝，不部分接受。

保留 UTF-8、UTF-8 BOM、UTF-16 LE/BE BOM、LF/CRLF 与末尾换行。混合换行/孤立 CR、控制字符、二进制/未知编码、截断内容、UNC、只读、重解析点路径及多硬链接保持只读。

保存采用以下实际机制：

1. 本地普通文件资格检查；取得按路径命名的 Windows mutex，串行化同路径 DevTabs 保存。
2. 以 READ | DELETE 共享持有只读句柄，排除直接写入；核对完整字节与 FileIdInfo 身份。已删除、变为只读/链接、同长度改动或文件替换均不能静默覆盖。
3. 同目录 create_new 临时文件，完整编码写入、sync_all，提交前再次核对文件身份。
4. ReplaceFileW 使用备份参数捕获实际替换版本。核对该备份的完整字节与身份；正常删除备份，遇到提交窗口内的外部替换则保留 `.devtabs-*.bak` 和缓冲区，并说明磁盘可能已经写入本次编辑，要求外部处理。
5. 重新读取验证写入内容；只标记本次快照为已保存。保存期间新输入仍为脏状态；重复保存合并，待切换动作在仍有新修改时再次确认。

**支持边界由实际 Windows 探针决定。** [windows-save-probe.json](../artifacts/sidebar-baseline/windows-save-probe.json) 显示：share READ 可阻止写入和重命名，但自身 ReplaceFileW 失败 32；share READ | DELETE 允许自身替换且阻止直接写入，同时也允许外部重命名。命名 mutex 只约束 DevTabs，不约束任意外部程序。因此不能声称通用条件替换、绝对零竞争或断电持久性。要求严格不发生路径覆盖的外部并发写入场景不在已验证保存范围；保留只读、复制及外部协调入口。

并发读者测试还实际发现：本机 ReplaceFileW 的命名流程可使新打开者短暂得到 NotFound。原失败保存在 [08-before-reader-fix.log](../artifacts/editor-verification/08-before-reader-fix.log)，没有将其改成“原子路径始终可用”。DevTabs 后台读取仅对此错误最多重试 4 × 5 ms；永久删除依旧报错并保留旧预览。复测使用该明确的读者协议，验证连续读取完整旧版或新版；任意外部程序无间断打开不作保证。这个边界和提交备份核对是两项不同性质，不能互相替代。

替换所需 Windows 权限/属性交由 ReplaceFileW 的合并语义；文件占用不满足共享规则时停止保存，保留缓冲区。备份异常时不强制回滚覆盖当前路径，以免再覆盖其间产生的新版本。进程被杀、关闭 WT 或断电不提供未保存草稿恢复。

## 检查结果与测试覆盖

最终命令、退出码和耗时见 [final-checks.json](../artifacts/editor-verification/final-checks.json)。`00-final.log`–`07-final.log` 保存集成回归；`08-before-reader-fix.log` 保留并发读者发现的问题，`09-reader-review.log` 的全量普通检查通过，但随后 Claude 就绪判断过早而失败；`10-native-ready.log` 等待实际版本标题及原生提示符后两种 Agent 均通过。`11-delivery.log`–`14-delivery.log` 保存最终质量检查、release 规模复测与构建输出。Claude 曾报告 fullscreen renderer 自动退回普通渲染；本轮验证的是它最终呈现的提示词页，不宣称其所有渲染模式均受支持。日志中较早的失败仍保留，不能只读取第一个退出码或将早期失败称为最终状态。

最终普通测试包括 15 单元、12 编辑、5 PTY、7 侧栏、7 宿主，共 **46**；另外执行了 4 个真实 Agent/Profile、1 个实际 WT、1 个大目录 release、1 个规模 release 测试。fmt / Clippy（warnings denied）/ 全量测试均以实际最终日志为准。

| 测试 | 实际检查 |
| --- | --- |
| 编码与字节 | 四编码 × LF/CRLF × 末尾有/无换行，无修改往返字节相同；保存后撤销/重做及脏标记 |
| 预算与选择 | 中文/Emoji、跨行缩进、普通字符不触发面板命令、整块粘贴拒绝、512 KiB 边界 |
| 冲突与权限 | 同长度变化、同字节不同身份替换、删除、只读、硬链接；无强制覆盖 |
| 并发与失败 | 两个 DevTabs 仅一个写入成功；直接写入排除；占用失败；中途写入/磁盘空间错误注入不截断原文件，清理临时文件 |
| 提交竞争 | 在真实替换原语前注入外部版本，错误明确且外部版本留在系统备份中；不将预先身份核对当作消除所有竞态 |
| 正常读者 | 10 次保存过程中按 DevTabs 有界读取协议读取完整旧/新文件；外部原始打开存在的命名窗口单独记录 |
| 状态与异步 | 保存时继续输入保留新脏标记，重复保存合并，保存后仍脏不自动导航，侧栏不活跃时仍接收保存结果 |
| Markdown | F2 预览未保存源码，返回编辑保留光标，旧渲染按请求/修订丢弃 |
| UI 入口 | 真正绘制出的 [Files]/[E]/[X] 控件点击，脏关闭取消保留内容，菜单与键盘共用状态判断 |
| 端到端 | 外层 ConPTY → 真正 DevTabs → 树定位/预览/编辑，连续两次相同 Unicode bracketed paste 均保留，保存一次，Agent 命令验证无输入泄漏；缩小恢复、脏分离取消/放弃 |

Windows 合并重复 KEY_EVENT_RECORD 后，粘贴起始 `200~` 中连续 0 可成为 repeat=2，原实现漏展开导致分隔符进入正文。本轮增加计数展开、独立回归及 Emoji/组合字符端到端留存验证。直接向 OpenConsole 注入裸 UTF-8 不是 IME/Windows Unicode 事件等价物；最终测试使用 Win32 UTF-16 控制台事件，未通过删去测试字符规避问题。

外部应用打开、剪贴板操作均保留系统错误反馈；剪切先复制成功再删除。没有自动打开测试模型请求或写入用户真实项目来验证保存。

## 性能与预算调整

测量包含 100 个项目、8 个 shell、4 个项目前端、1 个总览和宿主；同一时刻包含代码预览、Markdown 阅读、接近预算的编辑文件，以及第 4 个项目的文件树。全部 6 个 DevTabs 进程计入；Windows 20 逻辑处理器。CPU 采样 10 秒，峰值为保存期间约毫秒级轮询采样值，不是所有可能文件系统/负载的理论上界。

初始结果与修复证据均保留：

- `editor-scale-initial.log`：同时预览/编辑约 105.27 MiB，失败。
- `editor-scale-fixed.log`：共享磁盘快照及纯文本高亮优化后仍约 101.99 MiB，失败。
- `editor-scale-shared.log`：共享高亮文本并去除编辑器无用预览副本后约 98.88 MiB，通过当时的稳态测试。
- `editor-scale-save.log`：增加 1,016,000 字节文档保存任务后峰值约 107.08 MiB，失败。
- `editor-scale-budget.log`：编辑上限收缩至 512 KiB，508,000 字节夹具保存峰值约 99.70 MiB，通过。

随后 Save 也直接共享磁盘快照，避免再复制原始字节。最终数值位于 [scale-release.json](../artifacts/editor-verification/scale-release.json)，构建包生成后该文件同步最终复测。只读 1 MiB 和现有回归未删除；超过编辑预算明确保持只读，预算变化已回写 06 与 README。

最终测量（同一基准夹具，包含代码/Markdown/508,000 字节编辑与保存）：

| 指标 | 实测 |
| --- | --- |
| 已连接前端首帧 | 77.4–94.4 ms |
| 项目总览首帧 / 搜索反馈 | 94.1 / 15.8 ms |
| 代码 / Markdown / 文本打开 | 96.1 / 61.1 / 76.5 ms |
| 编辑 / 撤销 / 查找反馈 | 9.6 / 16.8 / 15.3 ms |
| 保存完成可见反馈 | 33.4 ms |
| 树展开稳态工作集 | 81.42 MiB |
| 同时预览/编辑工作集 | 93.82 MiB |
| 保存期间采样峰值工作集 | 97.35 MiB |
| 空闲整机 CPU | 0.500% |

## 交付与人工边界

独立构建产物：[DevTabs 0.3.0](../artifacts/DevTabs-0.3/devtabs.exe)。准确大小、构建时间及 SHA-256 见 [build.json](../artifacts/DevTabs-0.3/build.json) / [SHA256SUMS.txt](../artifacts/DevTabs-0.3/SHA256SUMS.txt)。本次交付 SHA-256：`5f9edf8b047ccaaabd2e3faf9ad4f7a5f4015a278a82e9b1ea9944c3478c6473`。不覆盖正在运行的旧 exe，不安装、不修改用户 Profile/终端配置。

已实测 WT 1.24.12741.0 三布局与会话身份；用户配置 FiraCode Nerd Font，Ctrl+C/V 由 WT 接管。控制台事件测试覆盖 Unicode 提交及重复粘贴，**尚未人工验证真实中文输入法候选确认/Esc、右键粘贴、Nerd Font 在不同字号下的外观，以及恢复既有会话后 Agent 的原生选区替换行为**。这些是环境交互验收边界，不能声称已由 TUI 测试替代。编辑/路径插入实现已完成；严格无间断路径访问、任意外部原子写入的零覆盖 CAS 不在交付保证中。

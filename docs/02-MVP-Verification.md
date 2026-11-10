> 历史文档：本文件记录 v0.1 前台 MVP；v0.2 的当前行为和验收见 [03](03-Workspace-Implementation.md)、[04](04-Upgrade-Verification.md)。

# DevTabs MVP Verification

日期：2026-11-09。对象：根目录 Rust 实现，不是 `refer/DevTabs` 参考工程。

当前结论：**前台双终端 MVP 已可运行；Agent 完整兼容性仍有关键项未测，继续验证。** Codex 新会话、原生 resume 和中文编辑已检查；Claude 的新会话／resume 入口均到达原生信任菜单，并验证上下移动。没有提交模型请求、恢复既有对话或确认信任设置。

## 环境

| 项目 | 实际版本／说明 |
| --- | --- |
| Windows | 10.0.26100（Windows 11） |
| Windows Terminal | 已安装版本 1.24.12741.0；本轮主要使用自动化 ConPTY 宿主 |
| PowerShell | 7.6.8，`pwsh.exe` |
| Rust / Cargo | 1.98.1 |
| Codex CLI | 0.167.0 |
| Claude Code | 2.1.281 |
| Rust 依赖 | Cargo.lock；直接依赖按本轮 crates.io 最新稳定版核对 |

常规集成测试在临时中文／空格目录中创建测试进程。只有显式运行的 Profile 检查使用个人 Profile；测试不修改 Profile、PATH、Terminal 设置或原访谈记录。参考工程未改动。

## 已完成检查

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
```

常规测试：9 个单元测试、4 个实际 ConPTY 集成测试通过；另有 2 个依赖本机环境的集成测试默认忽略，并已显式执行。

| 检查 | 观察结果 |
| --- | --- |
| 输入编码 | 控制键、应用光标、修饰方向键、粘贴边界和 SGR 鼠标坐标的字节断言通过 |
| Windows 输入 | 保留重复次数和扫描码；UTF-16 代理对合并；bracketed paste 合并为一次 Unicode 粘贴事件 |
| 输出解析 | 分片 UTF-8、游标查询、设备能力、窗口字符尺寸、模式查询和有界历史通过 |
| 双真实 PowerShell | PID 不同，目录相同；中文结果文件分别落在预期路径；输入隔离 |
| 实际 DevTabs 嵌套运行 | 在外层 ConPTY 中启动完整 TUI，切换焦点并输入命令；检查内层真实 shell PID |
| 缩放 | 两侧会话持续；外层缩小至 40 × 5 后显示提示，恢复后同一 Debug PID 仍可执行命令 |
| 取消退出 | 确认界面按 Esc 返回；两个 shell 继续运行 |
| 独立退出 | 一侧 `exit 7` 后准确显示退出码，另一侧仍能执行命令；两侧退出后 TUI 自动结束 |
| 进程树 | 测试 shell 退出后，仍存活的测试后代进程在 Session 释放时被 Job Object 清理；测试进程未受影响 |
| 输出压力 | 12,000 行中文输出排空，历史保持 5,000 行；另一侧写入结果文件的测得完成时间约 102 ms（一次本机观测） |
| 默认 Profile | 不传 `--no-profile`，Debug 启动命令执行成功；命令结束后继续输入成功 |
| 强制结束 | 关闭确认等待 3 秒后按 F；DevTabs 正常收尾，测试 Debug PID 结束 |

输出压力是短时突发测试，不代表 60 秒持续压力、Agent 菜单延迟或内存目标已达标。完成时间包含 PowerShell 执行和文件写入，不是纯键盘延迟。

本机额外检查可复现命令：

```powershell
cargo test --locked --test pty_windows installed_agents_startup_smoke -- --ignored --nocapture
cargo test --locked --test pty_windows personal_profile_debug_startup_and_force_close -- --ignored --nocapture
```

Agent 检查仅在新建空目录中打开界面。Codex 新会话输入 `DevTabs验证`，resume 中输入同一内容搜索，均不按 Enter；Claude 信任菜单向下再向上移动，保持“否”选项，不确认。屏幕文本记录写入忽略目录：`artifacts/codex-new.txt`、`codex-resume.txt`、`claude-new.txt`、`claude-resume.txt`。这些是解析后的字符屏幕记录，不能代替 Windows Terminal 的人工视觉、IME 或剪贴板验收。

## 按原验收编号记录

“部分”表示只验证了所述子项，不能据此将整项标为通过。“基础”表示通过公共终端层的检查，仍需在该 Agent 内复测。

| 编号 | Codex | Claude Code | 依据／剩余工作 |
| --- | --- | --- | --- |
| V01 双 shell 与隔离 | 基础通过 | 基础通过 | 真实双 PowerShell 与完整 TUI 两条路径通过 |
| V02 新会话与 resume | 部分 | 部分 | Codex 两入口、编辑／搜索通过；未恢复已有对话。Claude 两入口到达信任菜单，未继续进入对话／历史列表 |
| V03 IME、中文、Emoji | 部分 | 未测 | Codex 中文编辑、中文路径文件通过；UTF-16 Emoji 有单元测试。真实 IME、光标编辑和宽字符视觉需人工验收 |
| V04 多行粘贴 | 基础部分 | 基础部分 | Windows 粘贴事件组装、bracketed paste 字节测试通过；Agent 实际多行提交语义未验收 |
| V05 原生快捷键与长按 | 部分 | 部分 | Codex 编辑／搜索、Claude 菜单上下移动通过；Ctrl+C、补全、原生快捷键矩阵及真实长按未完整验收 |
| V06 图片／拖入路径 | 未测 | 未测 | 文本路径传递不等于图片附件；需分别按 Agent 原生方式测试 |
| V07 鼠标、复制、历史 | 基础部分 | 基础部分 | 鼠标编码、选择文本提取及有界历史通过；Windows 剪贴板实写及 Agent 鼠标实际交互未人工验收 |
| V08 持续缩放 | 基础通过 | 基础通过 | 真实 shell 与完整 TUI 的缩小、恢复和 PID 连续性通过；需在实际对话期间复测 |
| V09 终端协议 | 基础部分 | 基础部分 | 解析器查询／模式用例和启动界面通过；完整全屏 fixture、alternate screen 恢复未独立验收 |
| V10 持续输出压力 | 部分 | 部分 | 12,000 行突发输出＋另一侧 shell 输入通过；60 秒 Agent 并发交互、内存稳定性未测 |
| V11 真实构建／GUI | 未测 | 未测 | 启动命令结束后 shell 可用已测；`cargo run` / `cargo tauri dev` / 关闭被调试 GUI 的行为未测 |
| V12 单侧失败与退出 | 基础通过 | 基础通过 | shell 退出码与另一侧可用已测；不自动推断 Agent 或任意命令状态 |
| V13 退出与清理 | 部分 | 部分 | 正常退出、取消、3 秒后强制结束、遗留后代回收通过；宿主标签页硬关闭、仅杀 UI 和 panic 场景未独立验收 |
| V14 路径与 Profile | 基础通过 | 基础通过 | 中文／空格目录、保留本机 Profile 和 `--no-profile` 对照路径通过；Agent 本机启动检查采用 `--no-profile` |

没有进行原生 Windows Terminal 下同一 Agent 的完整交互对照，因此本表不构成“全部 Agent 兼容性通过”。后续人工检查优先 V03/V04/V05/V06，随后完成持续日志、真实构建和硬关闭场景。

## 实现中的关键决定

- 每个会话单独创建 ConPTY、解析器、写入队列和 Job Object。内部 `__host` 先加入 Job，再启动 PowerShell，保证 Profile 启动的后代也归属本会话。UI 持有 Job 所有权；不按进程名批量清理。
- `host_pid` 明确表示内部等待宿主；其退出码来自 PowerShell。`shell running` 不代表 Agent 就绪，也不代表上一条 Debug 命令仍运行。
- 关闭前让用户完成程序原生退出，再执行 shell 的 `exit`；中断请求不自动跟随 `exit`，避免把命令灌入未知前台程序。
- tui-term 仅绘制本地 `Screen` / `Cell` 适配器；禁用默认旧 vt100，使用 vt100 0.18.1 回调集中处理查询与模式。
- Windows 输入用原始控制台记录保留重复次数与修饰信息。自动化键盘模拟包含按下／释放事件；真实长按仍需人工检查，不能用合成连续 key-down 代替。
- 实测将交互 ConPTY 调为 1 × 1 后，PowerShell 可出现输入失效。UI 在小于 60 × 10 时保留原尺寸；Session 接口拒绝小于 2 的行列，并有回归断言。
- 调整尺寸时不跨 ConPTY 调用持有屏幕锁。每侧输出持续读取，历史有界；选择模式使用冻结快照。输入队列超载会明确提示未发送；协议响应排队失败也会显示 I/O 错误。
- 通过 dunce 规范化常见项目路径，避免普通路径显示为 PowerShell 的 `Microsoft.PowerShell.Core\FileSystem::\\?\...` 提示符；需要扩展路径时仍保留其语义。

可执行文件：`target/release/devtabs.exe`。首轮建议先运行 `--agent shell`，再分别体验两个 Agent。完整产品的后台存活、重连、共享 Debug 小窗及三种布局迁移仍是后续工作，不属于本次前台 MVP 的完成范围。

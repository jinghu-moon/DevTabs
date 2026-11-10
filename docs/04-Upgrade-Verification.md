# DevTabs Upgrade Verification

日期：2026-11-10。版本：0.2.0。环境：Windows 10.0.26100（Windows 11）、PowerShell 7.6.8、Windows Terminal 1.24.12741.0、Rust/Cargo 1.98.1；Codex 0.167.0、Claude 2.1.281。

## 升级前基线

修改前保存了 `artifacts/baseline/source`（Cargo.toml、Cargo.lock、README、src、tests）和实际输出 `artifacts/baseline/tests.txt`。基线 9 项单元测试通过，普通 ConPTY 集成测试 2 通过、2 失败、2 忽略。失败为真实 shell 提前退出和嵌套 TUI 等待 prompt 超时。因此本次不把旧版描述为全绿，也不将旧失败隐藏为新版本通过。

保留真实 ConPTY 测试，继续覆盖中文/空格路径、输入隔离、独立退出码、焦点、缩放、复制/终端协议、12,000 行输出与后代清理。加强小窗口测试：维持小尺寸超过 700 ms 后仍存活，放大后 shell PID 不变。测试使用唯一临时数据目录和 exe 副本，避免宿主锁住待编译文件；通过明确 Close/Shutdown 清理测试自己的会话。

## 六项实现和验收

| 顺序 | 交付 | 实际验证 |
| --- | --- | --- |
| 1 | 独立宿主、IPC、detach/reconnect | 分离期间继续输出；杀死前端后重连；shell PID 不变；前端退出不结束 Job |
| 2 | UUID、原子复用 | 4 个并发启动只产生同一工作区；默认复用；显式新实例生成不同 UUID |
| 3 | 共享 Debug、Float/Split/Tabs | 实际 WT 检查；布局循环保留 shell；默认非置顶、恢复不重置置顶；共享窗切换项目；收起恢复 |
| 4 | 托管状态、停止、强制、重跑 | native exit 23；Run/Rerun 保留 shell；先中断再延迟强制；未先正常停止拒绝强制；未提交编辑行不会被误执行 |
| 5 | 总览、发现、配置、额外会话 | 发现/排除；并发 6 个配置写入不丢失；命名会话相对目录、身份复用及前端切换；总览搜索实测 |
| 6 | 性能、用户安装/卸载 | 100 项目/8 shell；隔离目录安装、重复安装、更新备份及卸载；隔离 HKCU 键验证真实 PATH API 保留变量与其他路径 |

`cargo fmt --check`、`cargo clippy --locked --all-targets -- -D warnings` 通过。常规测试 **12 项单元 + 11 项集成通过**，没有失败。4 项本机环境/窗口/性能测试默认忽略，已另行实际执行：

- `installed_agents_startup_smoke`：Codex/Claude 各自新会话和 resume 共四种组合，检查输入/菜单移动，不提交模型任务或接受信任提示。
- `personal_profile_debug_startup_and_force_close`：真实个人 Profile、启动命令、继续交互及强制关闭，不修改 Profile。
- `windows_terminal_layouts_preserve_shells_and_debug_window_state`：实际 WT 共享窗项目切换、Float → Tabs → Split → Float、置顶、收起恢复、关闭与 PID 检查。
- `release_scale_100_projects_8_sessions`：下述性能测量，带阈值断言。

输出分别保存在 `artifacts/upgrade-tests.txt`、`artifacts/release-tests.txt`、`artifacts/agent-profile-tests.txt`、`artifacts/release-special-tests.txt`。Agent 启动屏幕的原始终端文本在 artifacts 内。

## 性能结果

100 项目，4 个工作区共 8 个真实 PowerShell，4 个双栏前端和 1 个总览。共 6 个持续 DevTabs 进程：后台宿主 + 5 个前端。原生辅助进程在握手后退出；统计没有排除任何 DevTabs 常驻辅助进程。

| 指标 | 实测 | 目标 | 结果 |
| --- | --- | --- | --- |
| 已存在会话的可交互首帧 | 75.7–94.3 ms | ≤200 ms | 通过 |
| 总览首次显示项目 | 62.1 ms | ≤200 ms | 通过 |
| 搜索输入至结果画面 | 13.8 ms | ≤50 ms | 通过 |
| 空闲 CPU，整机占比 | 0.49% | ≤1% | 通过 |
| 全部自有进程工作集合计 | 72.64 MiB | ≤100 MiB | 通过 |
| 全部自有进程 private bytes | 20.77 MiB | 供参考 | — |
| 100 项目扫描 | 3.06 ms | 供参考 | — |

CPU 采样 10 秒，本机 20 个逻辑处理器；同次结果换算到单逻辑核心为 9.84%。整机口径用于 1% 目标。工作集合计包含共享页的重复计数，private bytes 另列。排除 shell、Agent、WT/OpenConsole、编译器和测试驱动。首帧不是 Agent 模型或会话加载完成时间；已有会话附着成功即可转发输入。

首轮工作集合计约 159.5 MiB，未达目标。移除每会话常驻辅助进程、改成握手后退出，进程数由 14 降至 6，工作集降至约 72 MiB。没有用手工裁剪 working set 或排除自身进程来通过目标。

原始结果在 `artifacts/scale-release.json`。这是本机、指定规模和空闲窗口下的测量，不外推为任意 Agent、日志量的保证。复现命令：

```powershell
cargo test --release --locked --target-dir target/v2 --test workspace_windows release_scale -- --ignored --nocapture
```

## 安装和交付

构建输出为 `target/v2/release/devtabs.exe`，交付副本为 `artifacts/DevTabs/devtabs.exe`，避免覆盖用户仍运行的旧版。版本和 SHA-256 在同目录 `manifest.json`。本轮没有安装到真实用户 PATH；PATH 写入使用真实注册表 API，在临时 HKCU 键测试。安装/更新/卸载使用临时目录；执行 README 的 install 命令才修改真实用户 PATH。

不提供旧配置/IPC 兼容层。v0.2 配置位于 `%APPDATA%\DevTabs`，运行中的旧版会话不迁移。退出旧版仍遵循旧版前台生命周期；新版采用独立宿主。

## 已知边界

- 宿主异常退出、系统注销/重启会终止会话。内存历史有限，不是永久日志，不提供重启后恢复。
- 状态只跟踪托管命令，手输命令不显示执行状态，不推断 Agent 思考。PSReadLine 私有调度需要个人环境保留该模块。
- WT 没有按应用 UUID 聚焦隐藏标签的稳定 CLI。可见标题能定位时激活，否则打开新视图原子接管同一会话，保留进程，不保证回到原隐藏标签。
- 外部 Job 禁止 breakaway 时，后台仍受该外部宿主 Job 约束。普通 WT 关闭和杀死前端的路径已验证。
- IPC 读有期限，帧和并发有界；阻塞写尚不具备整个 RPC 的硬总超时。
- 中文文本经真实 ConPTY 验证；系统输入法候选窗、图片粘贴、全部第三方 TUI/键盘映射及长时间持续高频输出尚未完成专项人工验收，不声称支持图片终端协议。
- 额外命名会话通过 TOML 配置，尚无完整可视化会话编辑器。

参考工程和访谈原始文件保持原样。01/02 已标注为历史文档；当前运行说明见根目录 README。

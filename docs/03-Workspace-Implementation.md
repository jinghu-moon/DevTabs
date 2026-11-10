# DevTabs Workspace Implementation

日期：2026-11-10。版本：0.2.0。本文落实访谈与参考工程调研后的 1–6 项开发顺序，替代前台 MVP 架构。

## 从事实推导边界

终端程序需要持续存在的 ConPTY、输入流、输出读取者和生命周期所有者。窗口只显示数据。关闭窗口后进程仍需运行，所以生命周期所有者必须在独立进程。切换布局只改变视图，不能重新创建 shell。一个会话同一时刻只允许一个输入/尺寸所有者，避免窗口互相覆盖尺寸或混入输入。

路径用于寻找已有工作区，不是会话身份。工作区和会话使用 UUID；单一宿主状态锁内完成检查与创建。命令状态必须来自执行边界，不从输出文字、CPU 或 Agent 界面猜测。只引入实现这些约束必需的宿主、会话、视图和用户配置，不增加 Agent 协议层、对话数据库或 Web 服务。

## 1. 独立宿主与重连

`__serve` 是按用户数据目录隔离的独立后台进程。CreateProcess 不继承前端句柄；允许时从调用方 Job 脱离。若外部宿主禁止 Job breakaway，则仍受该外部 Job 生命周期约束。

本地命名管道仅向当前用户和 SYSTEM 授权，不开 TCP。协议为长度前缀 JSON，单帧最大 32 MiB，最多 64 个并发请求。读操作先检查可读数据，设 15 秒期限；写操作受阻塞命名管道背压约束，读期限不等于整个 RPC 的总超时。

宿主为每会话持有 ConPTY、Job、解析器、有限历史、输入队列和 shell 句柄。瞬时 `__host` 在创建 shell 前加入 Job，发布 shell PID，收到宿主打开 shell 句柄的确认后退出。宿主直接等待实际 shell；没有每会话常驻辅助进程。

Attach/Poll/Input/Resize/Detach 用 client UUID 标识视图，输入和尺寸共用同一租约；100 ms 轮询，3 秒无心跳后允许接管。主视图可以在附着操作内原子接管旧视图，旧视图失去输入权后退出。关闭前端或前端进程死亡不影响 Job 所有者。显式 Close 销毁整个工作区的 Job。

输入与查询响应队列有界，前端合并连续按键，保持粘贴和尺寸消息顺序。后台持续读取输出，分离时不会因没人绘制而堵塞。普通刷新发送可见画面及输入模式，历史按需读取。滚动/选择冻结本地画面，后台继续执行。

## 2. 身份和复用

项目路径规范化后按 Windows 大小写无关规则比较。Open 在同一状态锁内查找与创建，全部会话创建成功后才登记，失败自动清理部分创建的 Job。默认复用主实例，只有 `--new-instance` 才另开进程集合。重连和布局变化保持 UUID、PID、环境和当前目录。

身份在宿主生命周期内稳定，不提供重启系统后的进程恢复。全部 shell 退出后短暂保留最终画面，再回收工作区；没有工作区时宿主空闲 30 秒退出。

## 3. 布局和共享 Debug

Float 为默认：主视图 Agent，Debug 为共享普通 WT 小窗，默认不置顶，可切换置顶、收起和恢复。小窗使用按宿主隔离的命名互斥量。切换当前 Float 项目时重新附着其 Debug，标题明确项目与宿主身份，HWND 操作限定 WT 窗口类。

Split 在单个 Rust TUI 中显示 Agent/Debug，默认 70:30；额外命名会话可轮换至右侧。Tabs 使用 DevTabs 命名 WT 窗口中的两个独立标签，不依赖标签索引或 `-w 0` 最近窗口。进入 Tabs 可迁移前端，shell 保留在原宿主。

Layout 使旧租约失效，主视图重新附着，旧辅助视图退出。主视图接管和 client 登记为原子操作。辅助视图等待旧租约/窗口锁释放，支持旧前端关闭与新前端启动交叠。窗口小于 60×10 时保留连接和 ConPTY 尺寸，仅提示扩大窗口。

命名窗口与 `--useApplicationTitle` 行为依据 [Microsoft 文档](https://learn.microsoft.com/en-us/windows/terminal/command-line-arguments)。总览优先激活可定位工作区；窗口标题只能定位当前可见标签，不能可靠定位隐藏标签时打开新视图接管，不猜标签序号。

## 4. 命令控制

PowerShell integration 写原子状态 JSON，区分 shell 生命周期和 Idle/Starting/Running/Exited(code)。finally 路径记录命令中断，native 命令保留真实退出码。原生 Codex/Claude resume 由各 CLI 实现，不另建对话数据库。

正常停止发送 Ctrl+C；3 秒后用户可明确强制停止。强制仅枚举本会话 Job，排除 shell，保留环境和输出；关闭工作区才结束所有会话树。

Run 写请求文件并发送私有 Ctrl+Alt+F12 PSReadLine 绑定，取消未提交编辑行，在下一次 prompt 执行请求。命令运行中或请求待处理时拒绝叠加，每次 Run 重置停止计时。Profile 未提供该绑定时请求等到下一次 prompt。任意手输命令不属于托管状态范围，不推断 Agent 思考状态。

## 5. 项目发现和配置

多根目录仅扫描直属子目录，合并手动添加，应用排除并去重。按收藏、最近、路径排序，搜索覆盖全部项目。首次选全局 Agent，项目可覆盖 Agent、Debug 命令、布局和额外命名会话。总览支持键盘/鼠标、收藏、启动选项、常用设置输入及外部 TOML 编辑。

配置路径是 `%APPDATA%\DevTabs\config.toml`，与安装目录分离，避免覆盖参考工程旧配置。应用写操作在进程间锁内读取最新值、修改、原子替换；外部编辑保存后 F5 重载。项目目录不产生配置文件。

```toml
default_agent = "codex"
roots = ['D:\Projects', 'D:\Work']
added = ['E:\ManualProject']
excluded = ['D:\Projects\Archive']

[projects.'d:\projects\demo']
agent = "claude"
debug_command = "cargo watch -x run"
layout = "float"
favorite = true

[[projects.'d:\projects\demo'.sessions]]
name = "Docs"
command = "npm run docs"
directory = "docs"
```

相对会话目录以项目根解析。Agent/Debug 为固定核心会话，其余按配置创建。复用工作区不热替换启动配置。`--data-dir`/`DEVTABS_HOME` 隔离配置、宿主、运行文件和共享窗口身份。

## 6. 性能与安装

release 基准创建 100 项目、4 个双会话工作区，运行 4 个双栏视图和总览，测量进程启动至首帧、输入至搜索结果、10 秒空闲 CPU 和所有自有进程内存。使用 Windows 原生指标，排除 PowerShell/Agent、WT/OpenConsole 及测试驱动。CPU 同时报告单逻辑核心和整机占比，内存记录工作集合计和 private bytes。

当前用户安装到 `%LOCALAPPDATA%\Programs\DevTabs`：复制 staging，备份再替换目标，激活失败恢复备份。直接读写 HKCU Environment 的 PATH，保留原注册表类型、未展开变量及其他条目，广播环境变更。重复操作只调整自身条目。安装副本运行中需先关闭，卸载从另一份 exe 执行，保留配置。

依赖由 Cargo.lock 锁定稳定版。升级前源码和测试输出在 `artifacts/baseline`；升级后结果见 [04-Upgrade-Verification.md](04-Upgrade-Verification.md)。

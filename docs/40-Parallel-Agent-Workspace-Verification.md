# 并行 Agent 工作区实施记录

本记录关联 [设计](37-Parallel-Agent-Workspace-Design.md) 和 [TDD 任务](39-Parallel-Agent-Workspace-TDD-Implementation.md)。自动化不启动真实 Agent/WT 或发送系统通知；真机项单独记录。

## 阶段 0：基线和测试地基

2026-11-12 完成 T00–T02。升级前完整 workspace 360 通过、0 失败、27 忽略；升级前 Clippy 和 fmt 通过。保留旧项目脉搏可执行文件及本次源码 SHA-256 清单，输出在本机 `artifacts/parallel-workspace/stage-0/`，不提交构建产物。

RED：`parallel_foundations` 中超预算接收与旧 generation 覆盖两个行为断言实际失败；GREEN：3 通过、0 失败、1 子进程夹具忽略。截止时间测试确认清理自己的子进程不影响其他 fixture；配额在更新记账前拒绝，revision 只接受相同 generation 的递增结果。重构后 Clippy/fmt 通过，未改现有用户运行配置。

当前 Git 基线包含此前图片/SVG、侧栏、总览/项目脉搏等已在工作树中的实现及必要 vendor patch；不包含散落根目录的个人测试图片、参考 PowerShell 脚本和原始参考/访谈。它们没有被删除。新能力阶段 A–E 和用户真机验收尚待实施。

## 阶段 A：隔离任务与宿主路由

2026-11-12 完成隔离后端 A01–A05：持久 task/run/role、冻结提交的创建意图、独立分支/worktree、异步添加和操作查询；Git 工作在宿主输入锁之外。IPC 升级 v6。前端按任务显示、删除旧面板下标约定继续在 B 阶段落地，不宣称当前已有创建任务 UI。

实际 RED 包括仓库识别未实现、`..` 路径绕过嵌套检查、同提交同分支的外部 clone 被错误接管；GREEN 日志位于 `artifacts/parallel-workspace/stage-A/`。最终完整 workspace 370 通过、0 失败、28 忽略；最后新增的外部 clone 保护通过目标 suite 7/7，再通过 Clippy/fmt。忽略项含原性能测试及独立子进程夹具，不代表已测性能收益。

`parallel_tasks` 验证 dirty 起点不改变、基准提交冻结、重复执行保留用户修改、独立 index/common refs 缓存失效、持久意图对账、失败原因落盘、外部目录保护。`workspace_windows::running_dirty_task_can_add_an_isolated_task_without_restarting_or_crossing_input` 使用两个实际隔离 shell：A 的 PID/session ID 不变、B 工作树独立，两边收到的输入只写自己的 route.txt。测试不启动真实 Agent/WT。

创建失败保留资源，不自动强制删除；源工作树和主分支不切换、不 stash、不提交。目录祖先先规范化再检查，catalog 读取/写入限 8 MiB，重复操作不占新队列位置。真实 Agent conversation 关联和恢复仍由后续 C/D 的 Provider 验证路径完成。

## 阶段 B：当前终端内的并行面板

2026-11-12 自动验证 B01–B05。完整 workspace 379 通过、0 失败、28 忽略；Clippy/fmt 通过。随后将预览覆盖时保留的会话由固定 0 改为当前任务 Agent，目标 App 回归 22/22、Clippy 通过。日志位于 `artifacts/parallel-workspace/stage-B/`。

实际 RED 覆盖同屏第二个 Agent 缺失、快照重排丢身份、选择 C 覆盖固定 A/B、路径校验沿用失效租约、切任务后 A 的 Debug 仍覆盖 B。GREEN 验证稳定 ID 合并、实际输入租约复核、任务侧栏实体保留、固定组合/临时放大，以及 Debug 随任务上下文隐藏/恢复。补充回归验证同名文件的未保存草稿切回后不变，退出前端会揭示并保护隐藏草稿，两个工作树磁盘内容均未被保存覆盖；尺寸矩阵检查最小 40×10、舒适阈值和不重叠。

真实 ConPTY `parallel_task_form_adds_two_live_agents_to_one_frontend_without_stealing_input` 验证从表单准备和确认工作树、原前端同时显示 Main/Feature-B、创建后输入仍到 A、显式切换后到 B、原 PID 不变。fixture 的 wt.exe 是隔离陷阱，不使用用户 Windows Terminal 或真实 Agent。

使用入口：底栏「＋ 新任务」或 Ctrl+G → +；Tab 选择名称/提交引用/路径，Enter 准备并展示冻结提交，再 Enter 创建。创建进度在后台查询，Esc 退出表单可继续操作 A。Ctrl+G → V 切换 Agent 横排/竖排，`[` / `]` 或 1–9 切任务，C 固定当前两个可见任务/解除固定，E 显式替换第二个固定项。固定后进入隐藏任务使用临时放大，Z 可还原；后台事件不会替换组合或改焦点。Debug Float/Split/Tabs 与 Agent 排列分开；不因新增 Agent 自动启动额外 WT。

三项真机项保持未验证：原生 Agent 的输入模式、真实 WT Debug 标签的激活、Windows Terminal 的图片切换表现。没有宣称本阶段已有 Provider 完成检测或系统通知，它们属于 D。

## 阶段 C：总览、返回与恢复

2026-11-12 自动化验证 C01–C03。总览使用稳定任务行，历史对话保持独立；启动队列区分已有任务连接和历史会话启动。返回入口携带 project/workspace/task/session，清除隐藏原对象的筛选后揭示原任务。再次进入复用原进程和输入目标，不因历史记录数量增加项目或任务计数。

实际 RED：任务行数 0 而预期 2；恢复后 task ID 改变；局部重启入口被宿主输入锁拒绝。对应 GREEN 与完整回归日志在 `artifacts/parallel-workspace/stage-C/`。补充真实隔离双 shell 重启测试：B 单独退出/恢复时 A PID/session 不变；宿主崩溃恢复后 A/B task ID 和 cwd 保留，新 session/run 与旧身份不同，不自动启动 Debug。布局复制限 64 KiB，未知 schema 报错保留原记录；精确恢复缺少绑定时要求显式新建对话。

用户真实 Agent/WT 组合仍等待真机验证；自动化只启动 fixture shell，没有修改真实 hooks 或发送系统通知。

完整 workspace：381 通过、0 失败、28 忽略；Clippy `-D warnings`、fmt、diff-check 通过。

## 阶段 D：轮次文件记录、审阅、hooks 与通知

2026-11-12 实现 D01–D15 的开发路径，系统横幅、Provider 信任/真实事件和 WT 激活保持等待真机。证据目录 `artifacts/parallel-workspace/stage-D/`；不提交夹具内容、载荷、构建产物或性能日志。

- 文件记录：Git 冻结 HEAD＋起点实际 dirty 例外，中途提交仍核对净内容；非 Git 默认 notify 事件观察，限定范围才保存快照。SHA-256 独立对象、原子清单、8 MiB 单文件/128 MiB 单轮/512 MiB 任务/2 GiB 全局内容预算；观察路径和采集队列分别 4096/64。不可用、超额、规则变化和晚起点都有原因；不把未知当删除，不读取当前文件填补旧内容。
- 审阅：Ctrl+G → g 浮层，最近记录/100 路径分页、按需只读 diff、搜索和差异跳转、当前文件显式导航及全路径树标记。历史对象 hash 校验；已删除文件不创建占位。TestBackend 验证小窗口、可视行、翻页和焦点不变。Stop 继续复用逻辑轮基线与观察窗口，各报告版本独立封存。
- hooks：显式安装/移除、自有所有权 sidecar、备份/锁/原子写入，保留他人配置。PowerShell EncodedCommand 支持特殊路径。1 MiB/1.8 秒 helper、独立短 IPC 和接收确认；仅受管 Agent 注入 task/session/run/token，Debug 不注入。实际 shell fixture 验证重复 Stop、继续执行、旧确认和错误 run；无真实 Agent/WT。
- 状态/通知：只显示事实；Stop 为“已报告”，无最终回合能力时不自动检查。持久结果 ledger 与活动/采集 revision 分离。宿主单一发送源；官方 windows 0.62.2、专用 COM worker、32 队列/64 存活 Toast、10 分钟回收。普通结束按任务在 1 秒窗口内合并；紧急事件优先，挤出普通请求但保留未读，满紧急队列明确诊断。绑定原前端 lease 的点击路由，不夺其他前端输入、不新建 Agent。
- 待处理/策略：浮层及总览入口、结果版本确认、静音/类别/全局优先级、下一次结束、5/30 分钟延后、诊断复制；一次性提醒不跨宿主重启重放。配置存在不代表 Provider 信任或事件通过，Show 成功不代表用户看到。显式注册只改自有 AUMID；自动化不注册/发送真实通知。
- 运行与审阅：明确配置的 prepare/check、独立 Job、超时终止自己的子树、有界输出、task/run/checkout/代码版本绑定、手动检查、过期标记和仅检查提醒；重试跳过已成功步骤。完整 base→HEAD＋未提交＋untracked 路径求交，`devtabs overlap` 不使用前五热点。

真实 RED 日志：`round-red`、`bounds-red`、`events-red`、`hooks-red`、`policy-red`、`check-red`、`continuation-red`、`queue-red`；后两项分别暴露 Stop 继续丢失观察路径与通知队列行为缺失。GREEN 覆盖内容真值、CRLF/literal 路径、历史 GC、损坏/配额、真实 watcher、配置外部修改保护、独立结果已读、检查超时/初始化重试，以及多前端通知不误消费。

性能：相同 debug 构建、每任务 500 个约 1 KiB 文件、1/2/4 个任务依次建立基线、每组 5 样本。该数据是存储路径成本，不是并行 Agent 吞吐或 UI 帧率。优化前每个对象写入重复扫描存储目录，优化后每侧只扫描一次再增量记账。原始日志 `perf500-before`、`perf500-after`、`perf500-after-repeat`：

| 任务 | 优化前 p50/p95 ms | 优化后首组 | 优化后复测 |
| --- | --- | --- | --- |
| 1 | 681.4 / 842.7 | 588.1 / 634.9 | 663.6 / 752.5 |
| 2 | 875.5 / 1155.2 | 705.0 / 728.8 | 710.1 / 773.0 |
| 4 | 1270.6 / 1470.5 | 904.6 / 943.1 | 913.2 / 1005.1 |

两组方向一致，4 任务复测 p50 改善约 28%、p95 约 32%，保留优化；不把单任务噪声解释成稳定 14% 提速。最初 200 文件夹具噪声较大，未用于收益结论。文件树改动标记仅在响应更新时重建，不在每帧重复克隆全路径集。

支持边界：Claude 无可靠 turn ID 的版本只能给受限报告语义；不猜文本/静默/上下文压缩。特殊 filter/LFS/工作树编码不执行外部转换并标限制。Windows 通知需要显式身份注册，未打包 exe 的真实展示、进程内回调存活及普通/提升权限、多个 WT 标签的精确激活分别待用户验证；宿主关闭后的旧通知不冷启动。损坏/失联旧 run 不伪装为新 run 的结果；历史文件可用记录入口读取。

阶段 D 门禁：完整 workspace 404 通过、0 失败、29 忽略（新增显式性能夹具为第 29 项）；其后小幅修订通过 lib＋round_state/round_git/round_continuation 128 项，以及真实 hook/多前端路由目标测试。最终 Clippy `-D warnings`、fmt、diff-check 通过。真机状态仍为等待用户验证。

## 阶段 E：fast-forward、归档与资源清理

实现 E01–E03：显式来源/目标，确认前冻结两端 HEAD、分支、目录及资源身份，可选择要求已通过的来源代码版本检查。只执行 FF，不 commit/stash/rebase/fetch；忽略文件碰撞使用 `--no-overwrite-ignore` 保护。执行前后重验，Git 的具体拒绝原因进入操作日志。来源检查不等于目标集成结果通过。

工作区 `Ctrl+G → M` 和 `devtabs task` 均提供入口。归档只标记状态，目录/分支/终端/历史保留，恢复时跳过已归档任务而不阻断其他任务。准备摘要与执行分开，必须 Enter 或精确 `--confirm <OperationId>`。FF 要求双方受管终端先停止，清理要求来源终端退出、无受管检查/记录/资源事务，相关前端分离以保护可见和隐藏草稿；复制命令后在独立 shell 执行。操作系统共享/独占文件锁覆盖同一数据目录的其他 DevTabs 宿主，进程退出自动释放；不承诺阻止外部 Git/编辑器写入。

清理只认可创建事务＋目录创建身份＋Git 管理目录/common dir；原 checkout、外部资源默认只归档。Git `status --ignored` 验证 tracked/untracked/ignored 成果，确认 source HEAD 已进入指定目标。普通 worktree remove 后持久保存 DirectoryRemoved，再普通 branch -d；锁冲突可显示 DeletingBranch＋错误，查询/重试重新验证目录不存在及分支创建/修改身份。同提交重建分支、同路径重建目录均拒绝；已完成操作重试直接返回原结果，不碰新资源。

证据 `artifacts/parallel-workspace/stage-E/`：`red.log` 为尚未实现行为失败；`live-guard-red` 暴露持有会话状态锁时重复加锁导致 RPC 超时，`live-guard-green` 验证修复后及时拒绝且 A PID 不变；`independent-cleanup-red` 暴露错误锁住仍运行的目标 A，调整清理只锁待删除来源后，关闭 B 即可完成清理而 A 保持运行；`diagnostic-red` 暴露 Git 锁失败被通用“非仓库”掩盖，改为有界 stderr 并保留实际原因。

七项真实临时 Git 测试覆盖 dirty/HEAD 竞争/非 FF、ignored 成果与碰撞、原项目保护、来源检查过期、分支锁部分成功、重建路径/分支及其他宿主资源锁；TestBackend 验证归档必须二次确认、Esc 不改变焦点。shell fixture 验证运行中拒绝、显式关闭 B 后清理成功且 A PID 不变，自动化未使用真实 Provider/WT。

此前完整门禁 413 通过、0 失败、29 忽略；最后涉及 Git 错误与清理锁范围的改动完成后重新运行最终门禁，结果以下方最终构建记录为准。Windows/Provider 真机项由用户执行，尚未记录通过。

阶段 E 最后回归同时修复了两处跨功能边界：受管检查和活跃记录也持有资源共享锁；错误 run 的 End 先校验再取记录，采集失败保留可重试活动与锁。对应 `record-guard-red/green`。新增 opt-in 性能指标 `round.begin/end/diff/store`、`task.lifecycle`，沿用有界聚合，不保存文件名或输入正文。普通检查完成提醒也按同一全局/类别/静音策略发送，并复核原 run。

### 用户真机门禁

| 组 | 本次自动化 | 用户真机 |
| --- | --- | --- |
| U01 同屏隔离/草稿/返回 | TestBackend＋双 ConPTY shell | 待用户 |
| U02 Provider/信任/子 Agent/Stop 继续 | 官方载荷＋受管 helper shell fixture；无真实 Provider | 待用户核对安装版本与信任 |
| U03 Git 轮次/历史 | dirty、commit、CRLF、GC、内容真值 | 待用户 UI 审阅 |
| U04 非 Git 观察/限定快照 | 真实 watcher、范围/配额/丢失前态 | 待用户 UI 审阅 |
| U05 Windows 通知/激活 | 纯队列/策略＋原前端路由；不注册/真发 | 普通/Administrator、免打扰、WT 多标签均待用户 |
| U06 策略/检查 | 可控时钟、结果确认、独立 Job、超时/过期 | 待用户 |
| U07 合并/清理 | 临时 Git、资源锁、部分成功及重建资源拒绝 | 待用户 UI 确认 |

因此开发任务交付与真实组合验收分开：V01 的自动证据完成后仍保留 U01–U07 未验收状态，不以文档打勾替用户测试，也不发送真实通知或改写真实 hooks 来补齐。

10 万干净跟踪文件＋5 dirty＋2 untracked 夹具已实际运行（`100k-performance.log`）：1/2/4 个任务依次准备 Git 基线耗时 773/1535/3010 ms，每个基线仅 7 个例外，CAS 总对象也仅 7 个，不复制十万干净文件。另测 5 文件非 Git 范围快照 15 ms、事件观察准备 95 ms，仅记录各自能力成本，不作为同精度性能对比；夹具创建和清理共约 206 秒，不计入基线采集耗时。本组只验证资源与正确性，不声称有 p95/内存峰值结论。

最终回归发现并修复 Git 包装进程退出后后代持有管道的问题：每次 Git 命令进入专有 Windows Job，退出/超时清理该子树后才收集输出。`git-job-descendants` 用 fixture 启动实际子/孙 shell，验证终止后输出管道关闭；`git-job-regression` 文件树 21 通过。原来的挂起完整回归在确认 fixture 进程身份后解除；其运行结果不作为最终门禁。一次同 target 的新编译与旧测试进程重叠导致 LNK1104，已保留 `complete-tests.log`，待旧进程结束后串行重新执行，后续补测关闭自动记录和前端锁释放后，最终以 `completion-tests.log` 为准。

结果/通知策略落盘先串行化写入，再读取宿主最新状态，避免旧 ACK 的磁盘快照覆盖新事件；复制操作命令使用当前 exe 的绝对路径及 PowerShell 单引号转义，保证仍连接同一构建宿主。日志/测试载荷和测试图均未混入提交。

关闭记录的真实 RED/GREEN：`close-record-red` 验证关闭工作区后仍残留 active manifest，`close-record-green` 验证独立 helper 产生的同一 run 在终端退出后被 worker 封存。终端退出与 Apply 的检查共用 retire 路径，避免提前释放终端锁而遗漏记录锁；队列满保留可重试的退役标记。工作区整体关闭在销毁对应会话 Job 后提交封存，不阻塞宿主输入锁执行 Git/快照 I/O。

草稿保护的锁由前端进程自身持有，覆盖该前端已加载的全部任务（含隐藏任务），不依赖宿主的过期 client 清单；强杀前端后由 OS 自动释放，避免“已死前端”永久阻止清理。`frontend-lease-lifetime` 在真实 ConPTY 前端存活时验证独占锁被拒绝，杀前端后验证草稿锁释放，同时原 shell PID 保持不变，再重连。正常退出仍先经过已有保存/放弃/取消检查。

文件树 fixture 的外层 `.output()` 在子进程已退出时仍可能等待继承管道 EOF；改为 fixture 文件日志捕获并设置 40 秒截止，原有状态和内容断言保留。`bounded-explorer.log` 专项 21 通过、0 失败、1 忽略；Git 子孙进程清理测试独立保留。此前挂起回归不计通过，最终完整门禁单独记录。

### 最终自动门禁与测试构建

阶段 0–E 开发实现完成，最终源码完整回归 `completion-tests.log` 实际退出码 0：43 个 suite 汇总 415 通过、0 失败、30 忽略；包含文档测试。忽略项保留原来的环境/性能前提，不计作通过；十万文件与存储优化负载已按前述命令独立运行。Clippy `--workspace --all-targets -- -D warnings`、`cargo fmt --all --check`、`git diff --check` 均通过，Release 构建及 `--version`、`task --help` 安全 CLI 检查通过。最新日志为 `completion-clippy.log`、`completion-release.log`，不再引用较早构建的回归作为最终证据。

本机构建：`artifacts/DevTabs-0.4.0-parallel-workspace/devtabs.exe`，版本 0.4.0，build `c2265ce355c5258d`，IPC v6，23,428,608 bytes，SHA-256 `2604e43a0bf8e1429763998ebc6234efabfb741ba6379ae0deff1b917cc688b0`。产物旁 `build-manifest.json` 与证据目录 `source-manifest.json` 保存构建/源码指纹，后者覆盖 src/tests/crates/vendor 和 Cargo/build 输入；产物与本机日志不纳入 Git。

已分阶段提交推送：0 `93a683c`、A `7689ef4`、B `777bb59`、C `c3a02c3`、D `9a69944`；E 随本次最终交付提交。没有将用户个人图片、根目录参考脚本或个人测试文档纳入提交。

用户真机入口（从项目根目录运行；独立数据目录，不自动安装 hooks/注册通知）：

```powershell
.\artifacts\DevTabs-0.4.0-parallel-workspace\devtabs.exe --project . --agent codex --layout float --composition agent-only --new-instance --data-dir .\artifacts\parallel-workspace\manual
```

`Ctrl+G → +` 新建隔离任务；`Ctrl+G → g` 审阅轮次；`Ctrl+G → !` 待处理与通知策略；`Ctrl+G → M` 合并/归档/清理。Provider hooks/信任、Windows 横幅与回调、WT 标签激活和实际渲染仍按 U01–U07 验收，未自动测试真实组合，不声明整体验收已完成。

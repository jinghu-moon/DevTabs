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

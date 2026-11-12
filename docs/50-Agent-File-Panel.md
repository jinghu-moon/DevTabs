# Agent File Panel

2026-11-12。将“引用文件”和“修改文件”放在同一个 Agent 工作区底部，以左右表格呈现。Agent 保留原生终端和输入方式，DevTabs 单独管理文件信息。

## 布局与操作

- 默认从每个停靠 Agent 的区域分配 7 行。上方是真正缩小后的终端，PTY 行数跟随实际内容区变化，文件面板不会盖住 Agent 的最后几行。
- 左表为“引用文件 / 文件 / 行号”，右表为“修改文件 / 文件 / 状态”，中间有分隔线。相同文件可以同时出现，两个表的含义不合并。
- 引用文件单击进入已有文件预览，并尽可能跳到链接的行号。修改文件单击打开已有本轮差异窗口；不把当前磁盘内容替代历史内容。
- 鼠标滚轮分别滚动左右表格，滚动条支持点击或拖动轨道定位。点击表头、空白及滚动条不会向 Agent 发送鼠标输入。
- 拖动面板顶边调整高度；右上角 `[−]` 收起为一行数量摘要，`[+]` 恢复；`[↻]` 重试当前已完成轮次的数据。键盘使用 **Ctrl+G，松开，再按 `.`** 收起/展开。
- 拖动时 Esc 取消；窗口失焦、改变尺寸时取消未完成的拖动。高度和收起状态按任务保存到已有工作区布局文件，不增加独立配置体系。
- 两个并排或上下排列的 Agent 各有自己的面板、滚动位置和数据。调整一个面板不改变另一个。Debug、额外终端、独立 Debug 视图不分配文件面板；终端放大及悬浮状态优先保留终端空间。恢复停靠或普通布局后面板恢复。
- 高度不足时自动退为一行；极小窗口不显示面板。可用时至少为终端外框保留 8 行，避免面板挤掉整个 Agent。预览覆盖 Agent 的窄屏模式下不绘制底部面板。

## 数据从哪里来

### 引用文件

沿用现有 Hook 的可信会话身份与结束事件，增加读取 Hook 负载中的 `transcript_path`，随活动状态传给前端。原有配置的 Hook 事件不变，不增加实时终端抓屏、不解析终端颜色、不要求模型协助。

读取前验证 JSONL 的会话 ID 和 cwd，与当前任务的实际工作目录一致；相对链接相对该任务目录解析，不能使用原项目目录替代 worktree。只提取 Assistant 文本中的明确 Markdown 本地文件链接；支持绝对/相对路径以及 `:行:列`、`#L行`，同一路径去重。普通代码、纯文本文件名、工具输出、用户输入、子 Agent 和远程网址不作为引用。复用总览现有链接解析逻辑和 Codex/Claude 消息适配器，消息提取本身不再经过总览摘要的 4000 字符裁剪。

Codex 按日志中的 turn 标识筛选请求的已完成轮，后续轮次已经追加到日志时也不会混入。无 turn ID 的日志使用最近有效用户消息边界，这是降级口径；它不提供与有 turn ID 同等强度的延迟事件归属保证。未知格式或缺少路径显示不可用/部分结果，不从整个历史中猜一个“最近”会话。

### 修改文件

复用已有 `Review::Result { run, result }`，精确取对应结果，不回退到任意最新记录。面板显示本轮记录的完整路径列表；已有 IPC 只返回前 100 项详细元数据，未加载的后续项先显示“已记录”，点击仍按全局索引打开正确的分页差异。新增为绿色、删除为红色、修改为青色；无法比较为黄色。

Git 与指定范围快照继续使用已有前后内容；非 Git 观察模式明确显示“观察到”，不会编造旧内容或擅自初始化 Git。开始记录过晚、捕获预算或读取错误造成的不完整状态继续提示，详细原因进入现有差异审阅。查看面板本身不自动确认通知，也不确认任何变更。

## 更新与性能

- 普通 UI 帧只读取内存中的表格数据并绘制可视行。文件读取、JSON/Markdown 解析和 Review RPC 在后台完成，不扫描项目文件树。
- 触发身份包含任务、run、conversation、generation、result、turn、transcript 和实际 cwd。重复结束状态不反复创建工作；开始下一轮时保留上一次已完成轮的数据，新结果到达后更换。异步旧结果须通过身份检查，不能覆盖别的任务或轮次。
- 首次读取从日志末尾倒找当前轮边界，后续维护字节偏移。追加读取固定在本次打开时的长度；末尾半行（包括被拆开的中文 UTF-8）不推进偏移，下次继续读取。截断、头部变化、文件创建身份变化或绑定变化重新验证来源。
- 首次边界回读最多 32 MiB；单条 JSONL 记录最多解析 4 MiB，引用集合最多 2048 项。达到限制会显示“引用提取不完整”，找不到轮边界时不把无锚点片段当作本轮引用。这是会话适配器的内存/解析保护，不是项目目录扫描限制，不影响已有修改记录的完整路径列表。
- 同一前端限制同时处理的数据请求数。首个结果之后做一次延后复核，以接住日志和改动记录落盘的短暂延迟；失败最多自动尝试四次，之后用 `[↻]` 显式重试，不永久轮询历史文件。
- 性能观测继续使用 `DEVTABS_PERF_DIR`：新增 `files_panel.references`、`files_panel.draw`、`files_panel.reference_bytes`，不记录具体路径或正文。

## TDD 与验收

先新增“Agent 必须为底部面板保留真实空间、Debug 几何不变”的回归，在旧实现得到 RED，见 `artifacts/agent-files-panel/red.log`。实现后覆盖布局分区、最小高度、任务独立折叠和恢复、表头、左右独立滚动、轨道定位、拖动取消、过期返回、重复完成事件不重复派发、子 Agent 与用户/工具文本过滤、会话/cwd 验证、超出摘要长度的链接、JSONL 半行及 UTF-8、截断/替换，以及已读过后续轮日志时的重新定位。

```powershell
cargo test --locked --lib --test agent_file_changes --test file_references --test agent_events --test agent_hook_install --test i18n --test overview --test reference_parity --test markdown_viewport --test preview_workspace --test round_state --test round_continuation --test round_git --test round_observation --test notification_policy --test parallel_foundations --test activity_history --test notification_queue --test parallel_tasks --test perf_export --target-dir target/perf -- --test-threads=1
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo fmt --all --check
```

独立 Release 性能用例以约 8 MiB 的临时会话日志追加 30 轮，比较“每轮新建读取器并定位”与“复用增量偏移”，逐轮断言结果一致：

```powershell
cargo test --locked --release --test file_references incremental_reference_workload --target-dir target/parallel-release -- --ignored --nocapture --test-threads=1
```

相关回归 **261 通过、0 失败、13 忽略**；独立 Release 性能用例另行通过。120 文件的集成测试验证完整路径目录、分页后的第 120 项差异、精确 run/result、历史内容不被后续磁盘改动替代；两个任务使用同名文件时也保持独立。

一次 Release 夹具记录如下（首次身份校验读取也计入字节；耗时受机器负载影响，不作为真机延迟保证）：

| 30 轮处理方式 | 总读取字节 | P50 | P95 |
| --- | ---: | ---: | ---: |
| 每轮新建读取器并定位 | 9,868,300 | 0.696 ms | 0.777 ms |
| 复用增量偏移 | 37,900 | 0.136 ms | 0.298 ms |

读取量减少约 **99.6%**，两种方式逐轮断言结果相同。该对照衡量新增引用提取的两种读取策略，不代表旧版完整 UI 的整体加速倍数。

测试只使用临时文件、TestBackend 与已有隔离夹具，不启动真实 Agent，不修改实际 Hook 配置或剪贴板；真实终端显示与点击由用户验收。自动测试、静态检查、性能输出和 Release 交付证据分别在 `artifacts/agent-files-panel/` 与交付目录的 `build.json`。

## 真机测试

```powershell
.\artifacts\DevTabs-0.4.0-agent-files-panel\devtabs.exe --project . --agent codex --layout float --new-instance
```

引用路径依赖本次新增的 Hook 负载转发。如果原 Hook 配置仍调用旧交付目录里的 exe，仅启动新前端不会自动升级它。由用户显式执行新版本命令更新受管 Hook（保留其他 Hook，并按已有所有权/备份规则操作）：

```powershell
.\artifacts\DevTabs-0.4.0-agent-files-panel\devtabs.exe hooks codex
# 使用 Claude 时改为 hooks claude
```

之后在已有 Hook 可用的测试工作区完成一轮，让 Agent 的答复含有本地文件 Markdown 链接并实际修改一个文件，检查两表来源、数量与点击目标。再测试两个隔离任务的独立列表、面板调整、收起/恢复以及浮动 Debug 的覆盖顺序。缺少完成事件或日志路径时应显示等待/不可用；不要将无数据解释为没有修改。Hook 的安装与诊断沿用已有命令和用户确认流程，本次自动测试未替用户更改配置。

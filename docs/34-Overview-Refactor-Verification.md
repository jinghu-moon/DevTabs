# Overview Refactor Verification

## 1. 交付结果

按 [33 设计](33-Overview-Refactor-Design.md) 实现阶段 0–6。总览从全宽路径清单改为项目、工作上下文、图形概况三个区域，先找到项目，再确认上下文和启动方式。

独立产物：`artifacts/DevTabs-0.4.0-overview-refactor/devtabs.exe`。构建标识、SHA256、源码指纹、日志及测试统计见同目录 `build.json`。保留旧 `DevTabs-0.4.0-workspace-chrome` 包；本轮不自动安装、提交或推送。真机 UI 由用户验收。

- 构建标识：`578d6a124d412f3e`；IPC：`DevTabs-v5-578d6a124d412f3e`。
- 文件大小：17,633,792 字节。
- SHA256：`76adbff52d0953f37bacbb75690f2b0912659f7689b59ea07a6d9d0073cbd1c8`。
- 包内附 `logs/`、`fixtures/`、`performance.json` 和 140 个构建/测试输入的 `source-files.sha256`。

| 阶段 | 已实现 |
| --- | --- |
| 0 基线 | 升级前业务回归、保留旧包、新页面固定导航基线与双语 TestBackend 样本 |
| 1 骨架 | ≥160 列三栏，104–159 列详情优先，60–103 列显式弹窗；偏好、独立滚动和可拖动栏宽 |
| 2 列表 | 对齐列、窄栏双行、颜色标记、Git 语义、收藏/使用时间分组、筛选数量和搜索强调 |
| 3 上下文 | 下一步便签、同一启动草稿、改动文件、共享 Markdown 摘要、展开、最近提交和只读文件查看器 |
| 4 启动 | 真实单 Agent、有序队列、恢复审阅、调序/删除/撤销、实例身份、临时字段编辑和失败重试 |
| 5 概况 | 人类单位、语言条、目录横条、12 周提交柱、增删行、分来源加载和缓存 |
| 6 收尾 | 焦点、双击/拖动边界、滚动条、显式文字复制、弹层、I18N、完整回归和 Release 基准 |

## 2. 关键行为

- 当前项目、勾选队列、悬停和键盘焦点互相独立。悬停不换详情；滚轮只滚所在栏。编号和 Agent 点击不计入双击，只有项目行主体双击才启动。
- Enter 打开当前；Ctrl+Enter 只启动队列，空队列报错。焦点在详情/概况/队列或显式详情弹窗时，Ctrl+Enter 和 Alt+Enter 仍分别执行队列、打开启动选项。
- F4 持久保存项目默认值；←→ 或 Alt+Enter 只调整本次草稿，Ctrl+S 暂存入队。默认 Agent 决定筛选，临时切换不会使项目消失。F9 选择会话/原生选择器只暂存，不立即启动。
- Alt+Enter 内 Ctrl+D 编辑 Debug、Ctrl+A 编辑 Agent 参数 JSON 数组、Ctrl+E 编辑额外终端；Alt+N 新增额外项，Tab 依次编辑名称/命令/目录，Ctrl+Delete 删除当前额外项。字段 Enter 接受、Esc 取消当前输入；无效参数保留完整表单。
- 组合 `agent-only` 仅创建一个 Agent PTY；`agent-debug` 创建 Agent、Debug；`custom` 再创建全部配置额外项。切换组合保留额外配置。CLI、总览、F4 统一采用项目模型的默认组合规则：有额外配置且未显式指定时为 custom。
- 多实例必须明确选择，异步刷新按工作区 ID 保留选择；目标消失时取消选择并提示。草稿不同于运行中配置时显示激活提示，不修改已有进程。
- 总览“加入全部待恢复”按登记 ID 追加审阅队列，允许同路径多实例并保留已有勾选；Ctrl+Enter 才创建进程。CLI `--restore` 仍直接恢复。
- 改动文件单击选中、Enter 只读打开；路径链接 Ctrl+单击或选中后 Enter，支持 Windows 盘符和 `:行:列` / `#L行`。查看器复用 Sidebar 的异步读取和预览，禁用写操作，不创建 PTY，Esc 返回保留上下文。
- 详情/概况可 Alt+拖动选择当前可见文字、Ctrl+C 显式复制；没有选区时复制项目路径。选区期间冻结该文字视口，Esc、导航或新鼠标操作结束选区，不自动覆盖剪贴板。原生 Shift 选择取决于终端。
- 右键复用现有可搜索命令面板，包含项目、会话、组合、另开实例、便签、默认设置、复制路径和诊断操作。F1 提供完整帮助。

## 3. 后台与性能改造

没有新增 UI、Markdown 或 I18N 框架。继续使用 Ratatui 0.30.2，开启同版本的 rendered-line-info 特性计算换行后的滚动范围；Markdown 和双语资源沿用现有实现。

历史、源码/Git、磁盘使用三个独立有界通道和取消 generation。当前项目稳定 200 ms 后请求历史/源码，500 ms 后请求完整磁盘；F8 显式概况请求可以立即执行。切换项目、Agent、会话或刷新后校验请求身份；关闭概况不取消历史，F9 不接收概况回复。

历史上下文内存缓存最多 32 个项目/Agent 组合，当前摘要排版只缓存一份；源码和完整统计各最多 16 个项目。默认两条消息每条最多五行正文，展开最多六条，每条最多 120 行，渲染输入分别限制 2,048/8,192 字符；摘要不加载图片或执行复杂图形计算。沿用历史文件头/尾、条目、候选文件和超时上限。使用时间索引只采信包含有效会话 ID 与 cwd 的已知记录，不使用目录或 Git 修改时间。

定位并移除三个导航热点：每个可见项目克隆整份 Config、每次创建启动 Options 重建 Clap 解析器、定位列表锚点反复累计行高。改为按项目构造 LaunchSpec、OnceLock 默认 Options 模板、仅向前回溯一个视口的行高。详情点击区先按真实换行长度收敛滚动偏移，避免内容变短后点击旧位置。

复用 perf 输出，增加/接入：

- `overview.frame_and_flush`、`overview.input_to_frame`。
- `overview.context_cache_hits`、`overview.stale_results`、`overview.canceled_requests`、`overview.history_read_bytes`。
- 已有 `overview.filter`、`overview.stats`、`overview.history`、`overview.git` 和启动指标。

启用：

```powershell
$env:DEVTABS_PERF_DIR = "$PWD\artifacts\overview-live-perf"
.\artifacts\DevTabs-0.4.0-overview-refactor\devtabs.exe
# 测试结束后
Remove-Item Env:DEVTABS_PERF_DIR
```

## 4. Release 实测

同机、同一 Release 配置、200×50 TestBackend，100 / 1,000 个固定 CJK 项目，110 帧、前 10 帧预热、100 个样本。每次改变当前项目并绘制；不执行宿主 RPC、文件扫描或创建终端。

**前值是本次新页面的热点优化前版本，不能当作旧总览 UI 的性能。** 不缓存历史的固定导航场景保持相同 fixture、选项和导航序列，原始日志为 `artifacts/overview-navigation-before.log` / `overview-navigation-after.log`。

| 项目数 | 优化前 P50 / P95 | 优化后 P50 / P95 | P95 降低 |
| --- | ---: | ---: | ---: |
| 100 | 1.524 / 2.042 ms | 0.400 / 0.538 ms | 73.7% |
| 1,000 | 5.455 / 6.910 ms | 0.392 / 0.481 ms | 93.0% |

另测历史内容已经在内存中的场景：保留 32 个有界上下文，在这些项目之间导航，每条包含标题、列表、代码和表格；仍按总项目数分别测试。结果是完整渲染路径的单独测量，没有对应的优化前值：

| 项目总数 | 缓存上下文导航 P50 | P95 | 最大值 |
| --- | ---: | ---: | ---: |
| 100 | 0.501 ms | 0.619 ms | 0.978 ms |
| 1,000 | 0.562 ms | 0.892 ms | 0.930 ms |

P95 满足本次 TestBackend 16 ms 预算；这些数值不包含真实终端刷新/传输、磁盘冷读、真实 Agent 启动或用户感知延迟。缓存场景只循环 32 个就绪上下文，不能解释为同时缓存了全部 1,000 项目的完整对话。没有通过关闭默认详情获得收益。

```powershell
cargo test --release --locked --target-dir target/perf --lib overview_ -- --ignored --nocapture --test-threads=1
```

同一命令还生成 12 份双语、Catppuccin/Light、80×24 / 140×36 / 200×50 文本样本，位于 `artifacts/overview-fixtures/`；不代表已完成真机颜色验收。不能恢复的旧页面缓冲区未伪造补录，旧包与业务基线均保留。

## 5. 自动验证

| 范围 | 结果 | 日志 |
| --- | --- | --- |
| 升级前 lib / overview / reference_parity / i18n | 95 通过、0 失败、2 忽略，4 suites | `overview-refactor-before.log` |
| 最终相同范围（来自完整回归） | 116 通过、0 失败、5 忽略，4 suites | `overview-refactor-workspace.log` |
| 最终完整 workspace | **332 通过、0 失败、25 忽略**，28 suites | `overview-refactor-workspace.log` |
| 显式 Release 基准及样本输出 | 3 通过、0 失败 | `overview-navigation-after.log` |
| fmt / Clippy `-D warnings` | 通过 | `overview-refactor-fmt.log` / `overview-refactor-clippy.log` |
| Release 构建、产物 help/version、SHA256 与源码 manifest | 通过，具体标识见 build.json | `overview-refactor-release.log` / 包内 build.json |

新增重点覆盖：布局临界尺寸/偏好、独立滚动与栏宽取消/失败、队列身份与恢复审阅、草稿与默认值分离、跨 Agent/项目的异步回复、共享 Markdown、只读文件返回、路径行号、实例重排/消失、队列鼠标调序/删除/撤销、全局快捷键作用域、选择文本和组合默认值一致性。

隔离 ConPTY 测试确认 `agent-only` 宿主实际只有一个 Session/PTY，并验证恢复、预设和复用仍保持一个终端；纯绘制测试验证侧栏与三种布局，Debug 入口只提示而不创建会话。完整既有测试继续覆盖文件树、编辑、Markdown/图片/SVG、PTY、恢复、配置和构建隔离。

```powershell
cargo fmt --all --check
cargo test --locked --workspace --target-dir target/perf -- --test-threads=1
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo build --release --locked --target-dir target/perf
```

日志前缀：`artifacts/overview-refactor-`。忽略项包括需要已安装 Agent 的测试、其他专项基准与本轮显式执行的三项基准/样本输出；完整测试不自动启动真实 Windows Terminal 或用户安装的 Codex/Claude。

## 6. 真机入口与范围

```powershell
# 进入总览（需要已配置根目录或手动添加项目）
.\artifacts\DevTabs-0.4.0-overview-refactor\devtabs.exe

# 只创建 Agent，独立新实例；需要手动退出和清理
.\artifacts\DevTabs-0.4.0-overview-refactor\devtabs.exe --project . --agent codex --composition agent-only --new-instance
```

用户验收重点：三栏信息主次、长名称与双语字体、Markdown 摘要、分隔线/滚动条、双击和拖动、窄宽切换、F9 暂存、恢复审阅、多实例与单 Agent。自动化通过不代替这些结论。

首期自定义组合包含 Debug 与全部额外项，没有额外提供任意禁用其中 Debug 的开关。对话适配只支持已知 JSONL 格式，有界摘要不会承诺完整恢复所有历史；原生选择器可继续使用。Clap 帮助和底层第三方错误仍可能为英文。启动复用遵循现有工作区规则，只有显式另开实例才应用不同进程配置。

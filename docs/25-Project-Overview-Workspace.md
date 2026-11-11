# 25-Project-Overview-Workspace

总览已按 [33-Overview-Refactor-Design](33-Overview-Refactor-Design.md) 完成重构：默认响应式三栏、项目上下文预览和可审阅启动队列。最终测试、性能口径和独立产物见 [验证记录](34-Overview-Refactor-Verification.md)。

## 目标

总览界面把项目、宿主工作区和启动配置放在同一模型中。文件扫描、Git、统计、历史和启动均在有界后台 worker 执行；UI 线程只处理事件和绘制。自动化界面测试使用 Ratatui TestBackend，不打开真实终端窗口。

## I18N

`src/i18n.rs` 提供 `Auto`、`zh-CN`、`en-US` 三种语言，资源内嵌在 `src/i18n/*.json`，配置保存在 `Config.language`。总览使用 Ctrl+L／命令面板切换，工作区使用 Ctrl+G → U 或 F2 菜单切换。`devtabs config language zh-cn` 可以持久设置，`--language en-us` 只覆盖当前前端。语言只作用于 DevTabs 自己的界面文案，路径、文件名、命令、终端内容、Agent 输出和日志永远保留原文。

国际化引擎使用 `rust-i18n 4.2.4`：编译期读取 JSON，运行时由库执行查询、回退和 `%{name}` 命名参数替换。所有调用使用稳定资源 key；项目只保留语言偏好、系统语言识别及薄接口适配。界面不把翻译拼接进命令或日志，避免改变 shell 语义。总览弹窗、状态、筛选、排序、批量启动、设置、历史和统计使用中英文资源，既有侧栏菜单、操作提示与文件信息也接入同一目录。Clap 帮助、底层系统/第三方错误与部分旧诊断文案仍保留英文；当前不提供日期、数字和语言复数规则的完整地区格式化。库接入、资源契约和性能实测见 [国际化验证](26-Internationalization-Verification.md)。

## 总览模型

- 待恢复、收藏、本周、本月、更早、无使用记录分组，可折叠；每个项目只出现一次。最近使用时间取已知 DevTabs 启动和匹配默认 Agent 的有效会话记录较新值，提示说明来源。
- 运行中、有改动、待恢复、Codex、Claude、Shell 筛选；最近、名称、源码大小、提交活跃度排序。
- Space 保留勾选顺序；Ctrl+Enter 批量启动。启动条目使用 `Queued → Opening → Opened/Failed`，R 只重试失败项。
- 首次进入 ≥160 列默认三栏，104–159 列优先详情，60–103 列仅列表；F10/F8 显式控制详情/概况。自动降级不覆盖隐藏偏好；栏宽可拖动并持久化。
- 概况显示源码体积、实际文件逻辑体积、语言比例条、最大目录横条与 12 周提交柱。历史、源码/Git、磁盘使用独立有界通道和 generation；稳定 200 ms 请求上下文/源码，500 ms 请求磁盘。关闭概况不取消历史。
- F9 读取匹配的 Codex/Claude JSONL，选择会话只暂存同一启动草稿，不立即启动；默认详情以共享 Markdown 引擎显示最近两段、合计最多十行正文，E 展开。原生选择器仍可暂存。
- F4 使用逐字段表单编辑 Agent、布局、终端组合（Alt+M）、Debug 命令，以及每个额外会话的名称、命令、目录。Tab/上下切换字段，左右选择枚举，Alt+N 添加会话，Ctrl+D 删除会话，Ctrl+S 保存。验证和写入失败均保留输入；更新只合并表单字段，保留收藏、便签和侧栏偏好。配置写入采用现有原子更新与锁。
- Ctrl+P 打开可搜索命令面板，项目和预设也作为命令项；普通 Agent 输入不会被接管。

## 资源与取消

Git 使用 `porcelain=v2 --branch`，每个项目最多读取限定输出。统计分别计算文件逻辑字节和源码字节；逻辑体积包含构建产物，源码统计遵循忽略规则并排除常见构建/依赖目录，按 syntect/two-face 扩展名识别语言。最大目录以第一层目录汇总，根目录文件归到 `.`。统计每轮最多扫描 200,000 个条目，并按 3/5 秒时间预算提前返回；提前停止会标记部分结果。大小/活跃度排序在后台加载排名，概况按可见性、稳定时间及缓存按需采集；默认详情保留前五个改动文件、最近对话和提交。

历史只读 Codex/Claude 的已知 JSONL 格式，每个文件最多读取头部 64 KiB 和尾部 256 KiB，最多遍历 30,000 个条目、处理 512 个 JSONL、保留 15 个会话与每会话最近 6 段对话；工具和 Codex commentary 不作为对话正文。不支持的格式可使用 Agent 原生历史选择器。指定会话恢复只接受经过校验的会话 ID。

generation ticket 丢弃过期结果，发现请求只保留最新任务，窗口关闭时取消 worker。配置持久化与启动分开执行，批量启动不会阻塞表单保存。批量失败重试保留已创建的宿主工作区，避免前端打开失败后重复启动 Agent。宿主查询失败显示错误，不能把查询失败当作空闲。

## 验收

`cargo test --locked --all-targets --target-dir target/perf`；必须包含 `overview::view::responsive_layout_renders_all_languages_without_panics`、语言资源切换、Git 解析、会话格式和批量失败重试测试。人工真机测试重点检查 Ctrl+L、Ctrl+P、F4、F8、F9、Ctrl+Enter、R，以及窄/宽窗口下弹窗层级。

## 对标补齐

总览恢复操作先按登记 ID 追加审阅队列，Ctrl+Enter 执行；CLI `--restore` 保留直接恢复。真实 `agent-only` 不创建隐藏 Debug，预设/恢复均保留组合。Alt+Enter 内 Ctrl+D 编辑临时 Debug、Ctrl+A 编辑参数数组、Ctrl+E 编辑额外终端、Alt+N 新增，Tab 依次编辑名称/命令/目录；Ctrl+S 暂存不改默认。

恢复登记、`--last`、自动续接、Ctrl+N 新建项目、Ctrl+B 预设管理、F4 标题/颜色/参数、`.devtabs.toml`、12 周趋势与增删行、跨启动 TTL 缓存、F1 帮助、可点击筛选和 WT 窗口目标已补齐。F4 基础字段现为七项，额外会话从第八项开始。Ctrl+Shift+A 全选当前可见项；预设 Enter 只套用，Alt+Enter 调整后 Ctrl+S 暂存，Ctrl+Enter 再启动。详细边界、测试与性能证据见 [参考项目对标](27-Reference-Feature-Parity.md)。

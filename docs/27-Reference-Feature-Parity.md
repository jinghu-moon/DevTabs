# 27-Reference-Feature-Parity

## 本轮范围

本轮补齐 `refer/DevTabs` 对照审计中尚缺的启动、恢复、项目管理和统计功能。继续使用现有 Ratatui、ConPTY 宿主和后台 worker，不新增第二套工作区或配置系统；国际化复用 rust-i18n，搜索复用 nucleo-matcher，语言识别复用 syntect/two-face。本轮没有增加外部依赖。

| 参考能力 | 本项目落点 | 行为与入口 |
| --- | --- | --- |
| 中断后恢复 | `recovery.rs`、宿主登记 | `--restore` 直接恢复；总览命令面板先追加审阅队列，再 Ctrl+Enter 执行；按登记 ID 区分实例 |
| 重开上次组合 | 有序启动快照 | `--last` / 命令面板；优先复用已运行项目 |
| 自动续接会话 | 已有 JSONL 历史读取器 | 默认尝试最近可读会话；`--fresh` 强制新会话 |
| 新建项目 | `project_profile::create_project` | 总览 Ctrl+N；新建单层目录并执行 git init |
| 标题、颜色、参数 | Project / Options / WT 参数 | F4 设置；CLI 显式参数优先 |
| 项目内共享配置 | `.devtabs.toml` | 命令面板导出启动设置；不导出收藏、便签、侧栏偏好 |
| 完整预设管理 | LaunchSpec 启动快照 | Ctrl+B；Enter 套用、F2 重命名、Delete 删除 |
| 套用后调整 | 有序勾选与 staged Options | Alt+Enter 调整，Ctrl+S 暂存，Ctrl+Enter 启动 |
| Git 提交趋势 | Stats.weeks | F8：最近 12 个滚动周，从旧至新 |
| 文件增删行 | Git numstat -z | F8：已跟踪文件相对 HEAD 的暂存与未暂存合计，前六项 |
| 跨启动统计缓存 | 数据目录 stats/*.json | 源码及 Git 结果 10 分钟、逻辑体积 12 小时，F5 强制失效 |
| 快捷操作与帮助 | 总览命令与鼠标区域 | Ctrl+Shift+A 可见项全选/取消，F1 独立帮助，可点击筛选标签 |
| 窗口目标 | WT 命令构造 | Alt+Enter → W；当前窗口或新窗口，同批新窗口共用分组 |

既有总览、收藏/最近分组、便签、Git 状态、设置表单、主题、原生历史、命令面板、失败重试及文件树、预览、编辑继续保留。旧版的按键和配置格式不构成兼容承诺。旧版推测 Agent 思考/空闲状态的启发式不照搬，只展示实际 shell、托管命令和工作区状态。

后续总览重构已完成，最终行为和验证见 [33 设计](33-Overview-Refactor-Design.md) 与 [34 验证](34-Overview-Refactor-Verification.md)。本页下方性能数据属于原统计缓存专项，不代表本次总览整帧数据。

## 恢复与生命周期

工作区创建成功后原子写入数据目录 `launches.json`；关闭工作区或全部 shell 结束后清理登记。只关闭/分离前端保留宿主和登记。宿主崩溃、系统重启后登记保留，可通过 `--restore` 重建进程。活着的工作区按真实宿主 ID 排除，总览的“加入全部待恢复”只追加缺少的恢复条目，不创建进程、不覆盖已有队列；执行后恢复成功只替换对应旧记录；同一项目的多个实例分别登记。重复恢复不会重复创建已经恢复的工作区。

登记损坏会报错，不静默覆写。读取上限 4 MiB。启动组合按顺序保存完整 Options，排除前端 view、数据目录、恢复标识及临时窗口分组。`--last` 允许复用已有项目；需要多个独立实例时使用显式启动与 `--new-instance`。

恢复重新创建进程，不恢复终端输出、进程内存或未保存编辑。登记中已有明确 session ID 时按该 ID 续接；只知道“最近会话”时读取已知格式的 JSONL，以文件修改时间判断最近记录。新建会话尚未取得 session ID、历史格式未知或文件不可读时，不能承诺精确回到原对话。指定 ID、原生 `--resume` 选择器优先于自动续接；`--fresh` 禁止自动续接。

```powershell
.\artifacts\DevTabs-0.4.0-overview-refactor\devtabs.exe --restore
.\artifacts\DevTabs-0.4.0-overview-refactor\devtabs.exe --last --new-window
.\artifacts\DevTabs-0.4.0-overview-refactor\devtabs.exe --project 'D:\Projects\Demo' --agent codex --fresh
```

## 配置与参数

启动字段优先级：全局每项目配置 → 项目 `.devtabs.toml` → 本次显式 CLI 参数。总览后台读取同一份有效配置，显示、搜索、Agent 筛选和启动保持一致；预设先套用完整快照，后续调整保存到本次勾选组合。无效项目配置阻止启动并显示错误，修正后按 F5。

`.devtabs.toml` 示例：

```toml
agent = "codex"
layout = "split"
title = "DevTabs 开发"
color = "#63A4FF"
agent_args = ["--model", "your-model"]
resume_latest = true
debug_command = "cargo test"

[[sessions]]
name = "Build"
command = "cargo check"
directory = "."
```

F4 中 Agent 参数使用 JSON 字符串数组，避免按空格拆分、转义或注入 PowerShell 语句；CLI 用重复 `--agent-arg=...`。每个参数以 PowerShell 单引号 literal 传入，内部单引号成对转义。最多 64 个参数、每个 4096 字节，不允许 NUL/CR/LF；标题最多 256 字节且无控制字符，颜色限定 #RRGGBB。项目文件最大 128 KiB，拒绝未知字段。语法、验证或写入失败保留表单。

已有 `.devtabs.toml` 时，F4 同时更新该文件和全局启动字段，避免全局设置被项目文件遮蔽。各文件独立原子更新，未提供跨文件事务；若后续全局写入失败，项目文件可能已更新，应修正写入问题后重试。收藏、便签、历史与侧栏偏好始终只保存在本地 Config。

新建项目仅接受单段目录名，拒绝路径穿越、Windows 保留设备名和非法字符；已有目录不修改。Git 初始化失败时保留刚新建的目录并报告路径，不递归删除文件。新建成功后登记并选中项目，不自动启动 Agent。

## 统计、性能与刷新

逻辑体积是文件长度合计，不是磁盘分配空间。源码统计遵循忽略规则并排除常见依赖/构建目录，使用 syntect/two-face 扩展名或已知文件名识别语言。12 周趋势基于提交时间；增删行基于 `git diff HEAD --numstat -z --no-renames`，不统计未跟踪文件，二进制文件不伪造行数。

缓存使用规范化项目键与根目录、Git HEAD/index/refs 时间戳；内容变化在 TTL 内可能仍返回缓存，F5 可立即刷新。部分扫描结果仅缓存 10 秒；取消的扫描不覆盖有效缓存。单个缓存读取限制 128 KiB，损坏或过期时重算。逻辑体积可独立复用至 12 小时，避免每次重新遍历构建产物。缓存目前不设置跨项目文件数量上限，历史项目缓存可删除后重建。

F5 先取消旧详情与发现请求，等待后台写入失效标记，再重新请求数据；generation ticket 丢弃旧结果，防止旧缓存回填。历史 I/O 在宿主会话锁外执行，避免历史扫描阻塞其他终端输入。

Release 基准使用隔离的 2,000 个 Rust 文件：记录首次冷读，再在语法库已经初始化后执行 5 次无缓存扫描和 7 次持久缓存命中，比较各自中位数；逐次断言统计结果一致。原始数据：`artifacts/parity-cache-benchmark.json`。只测统计路径，不代表整帧 UI 或冷磁盘性能。

本机 Release 实测（2026-11-11）：

| 指标 | 耗时 |
| --- | ---: |
| 首次统计与缓存写入 | 238.528 ms |
| 预热后无缓存扫描中位数，5 次 | 232.693 ms |
| 文件缓存命中中位数，7 次 | 0.116 ms |

结果逐项一致，缓存命中相对此样本的无缓存扫描约快 2006 倍。此比例包含省去文件遍历和 Git 子进程调用的收益；缓存文件可能已在操作系统文件缓存内，不代表任意项目或首次启动的固定收益。

## 自动验证与真机入口

改造前测试记录：`artifacts/parity-before.log`。本轮专项覆盖宿主强杀与恢复、重复恢复、上次组合复用、关闭清理、配置覆盖与边界、预设完整快照、创建目录保护、Git 趋势/增删行、缓存过期/取消、刷新竞态、模糊搜索及中英文鼠标区域。

另有回归验证 `--data-dir=value` 生效、宿主校验失败返回非零状态，以及形似 DevTabs 参数的 Agent 字面参数不能修改数据目录。新发现项目先交付有效配置，再进入可操作列表，避免 Git 扫描期间漏用项目设置。

Agent 参数集成测试使用隔离的本地 native fixture 和显式 PowerShell 函数绑定，验证模拟历史续接与特殊字符 literal 传递，不依赖真实 Agent。UI 自动化仅使用 Ratatui TestBackend；ConPTY 测试在隔离数据目录后台运行，不打开 Windows Terminal 窗口。WT 窗口目标和参数通过纯命令构造测试；实际窗口行为仍由用户真机验收。

```powershell
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --target-dir target/i18n-check -- -D warnings
cargo test --locked --workspace --all-targets --target-dir target/perf -- --test-threads=1
cargo test --release --locked --test reference_parity --target-dir target/perf statistics_cache_benchmark -- --ignored --nocapture
cargo build --release --locked --target-dir target/perf
```

日志前缀为 `artifacts/parity-`。本轮独立测试构建：`artifacts/DevTabs-0.4.0-reference-parity/devtabs.exe`；同目录 `build.json` 记录最终 SHA256、测试数量与构建命令。上一轮产物保留。

真机重点：F1 帮助、Ctrl+N 创建、Ctrl+B 预设套用/重命名/删除、Alt+Enter 暂存、Ctrl+Enter 有序启动、F4 标题/颜色/参数、F8 趋势与增删行、F5 强制刷新，以及当前/新 WT 窗口目标。恢复测试仅在隔离样本进行，避免中断真实开发会话。

## 最终验证结果

2026-11-11 最终工作区回归：**275 通过、0 失败、22 项默认忽略**；fmt、Clippy（-D warnings）、Release 构建与独立程序 --help 检查通过。完整日志为 `artifacts/parity-final.log` 与 `parity-final-release.log`，忽略项包含需要真实 Agent、特定终端或显式性能参数的测试，以及本轮需手动运行的 Release 缓存基准；缓存基准已单独通过。

本轮程序：`artifacts/DevTabs-0.4.0-reference-parity/devtabs.exe`，17,156,608 bytes。SHA256：`428FFB3D9A1FFB2CAD3BB49DC2AB000807A83B63C2BFFEFC27ED5C5D69C8797C`。`build.json` 包含 388 条中英文资源规模、源码指纹、检查命令和原始性能结果。此产物未经过真机 UI 验收，由用户按上述入口验证。

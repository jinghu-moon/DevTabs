# Complete Statistics Scan Verification

2026-11-12。落实 [扫描调研](41-Statistics-Scan-Research.md)：移除磁盘/源码的 200,000 项和 3/5 秒硬截断，以完整结果对照验证并行优化。保留原有统计口径、忽略规则、后台取消与缓存，不通过排除构建目录换取磁盘提速。

## 1. 实现与统计口径

| 部分 | 实现 | 约束 |
| --- | --- | --- |
| 磁盘 | dua-core 6.1.0，ParentFirst，最多 4 个遍历线程，每线程排队批次设为 2 | 全项目普通文件逻辑长度；包含隐藏、忽略、构建、依赖和 .git 文件 |
| 源码 | 既有 ignore 改为最多 2 个线程的 parallel visitor | 保留忽略与语言识别规则，构建/依赖目录不算源码；不是磁盘总量口径 |
| 聚合 | 磁盘只保留目录 ID 到一级分组/类别的映射；源码 visitor 私有计数，结束时合并 | 不读取文件内容，不建立完整文件索引，不逐文件锁住共享映射 |
| 进度 | 磁盘开始时发布一次，之后约 250 ms 合并发布累计项数/大小/暂定目录排名 | 不伪造完成百分比；遍历前不知道总项数；进度发送使用 try_send，完成结果仍可靠发送 |
| 取消 | 排队等待、目录下降、条目消费时检查请求世代 | 取消结果不写入完成缓存；重新进入会重新请求；旧票据不能覆盖新选择 |
| 并发 | 源码、磁盘各有进程级可取消扫描许可 | 同类扫描不为每个项目重复创建一组同时活跃的遍历线程 |
| 缓存 | schema 4 → 5；拒绝扫描中缓存，淘汰旧版截断结果 | 仍为独立 TTL 的采样缓存，F5 强制刷新 |

磁盘不跟随符号链接或目录 junction；硬链接按每个路径计入逻辑长度，与旧口径一致。统计不是实际分配空间或去重后的物理占用。读取失败仍显式显示部分结果，不因取消/错误而宣称完整。

目录聚合覆盖全部一级目录，保留原先普通文件与构建/依赖分组，界面展示大小靠前的目录。小目录不在榜单不代表未计入。内部 `.` 分组是根目录直属文件，现显示为“根目录文件 / Root files”，不再像额外目录。构建/依赖分类只影响展示，不影响磁盘总计。

扫描进行中，磁盘大小标记为 `≥`，同时显示已扫描项数；排名标明扫描中。源码/Git 可先出现，不能被解释为磁盘扫描完成。中英文、窄宽布局、原有数值对齐均有回归覆盖。

## 2. 测试与证据

实施前完整 workspace 回归：**416 通过、0 失败、30 忽略**，日志 `artifacts/statistics-complete/baseline-tests.log`。

实际 RED/GREEN：`elapsed_time_never_truncates_scans_but_read_errors_remain_explicit` 在旧版因 TimeLimit 截断失败，日志 `time-red.log`；恢复该断言并替换扫描器后通过，日志 `time-green.log`。该测试通过传入已过去旧截止时间的起点覆盖旧分支，无需真实等待数秒；超过项数的完整性另有大夹具验证。没有声称所有新增测试均逐一执行过 RED。

新增或扩展的自动验证包括：

- 完整标准库磁盘基准与 dua-core 的 1/2/4 线程逐项比较文件数、字节数、类别及目录汇总；串行/并行 ignore 比较源码、语言及分类，包含嵌套忽略/否定规则与中文路径。
- 初次进度回调触发取消，不产生完成缓存；新票据重试成功，后续缓存命中不再遍历；schema 4 截断缓存与扫描中缓存不被复用。
- 真正创建临时 Windows directory junction，断言外部文件不被计入；只使用测试临时目录。
- 进度不能提前结束 UI pending 或更新时间；离开后再进入会重启被取消扫描，旧结果被拒绝；中英文根目录标签、进度与窄栏边界。
- 显式大夹具：**201,208 个目录项、201,003 个磁盘文件、4,221,163 字节**，源码 **201,001 个文件**；crates、构建、依赖、文档与总量符合预设真值。日志 `large-fixture.log`。夹具以硬链接节省物理空间，按每路径逻辑长度计数。

大夹具 Debug 扫描耗时为磁盘约 110 ms、源码约 1,114 ms；约 115 秒的创建、测试与清理总耗时不作为扫描耗时。真实 Agent、真实通知及 Windows Terminal UI 没有在本轮启动；真机 UI 由用户验收。

最终完整 workspace 回归：**422 通过、0 失败、32 忽略，43 个 suite（含文档测试）**，实际退出码 0，日志 `final-tests.log`。两项新增忽略测试分别为显式大夹具与真实项目测量，均已按前提独立执行通过。`cargo fmt --check`、Clippy `-D warnings`、Release 构建与 help/version 冒烟通过，日志 `final-clippy.log`、`final-release.log`；未把需要真实 Agent/终端的忽略项算作通过。

## 3. 真实项目性能对照

只读扫描 `D:/100_Projects/110_Daily/Yue-GPUI-Component`，Release，绕过 DevTabs 持久缓存。测试基准保留升级前标准库聚合与串行源码逻辑，移除其截断，得到相同工作量的完整基线。每组先预热一轮，再采五轮，交替正序/反序运行；每个样本均断言结果与完整基准相等。

| 完整扫描方式 | 首组 p50 / p95（ms） | 复测 p50 / p95（ms） |
| --- | ---: | ---: |
| 磁盘标准库串行 | 885.788 / 1,012.440 | 833.654 / 1,016.163 |
| dua-core 1 线程 | 1,325.907 / 1,488.966 | 1,170.726 / 1,484.259 |
| dua-core 2 线程 | 681.304 / 947.223 | 666.944 / 722.289 |
| **dua-core 4 线程（保留）** | **400.497 / 449.539** | **375.527 / 389.830** |
| 源码 ignore 串行 | 51.927 / 56.626 | 43.422 / 54.538 |
| **源码 ignore 并行 2 线程（保留）** | **26.997 / 28.974** | **24.527 / 25.593** |

复测 p50：磁盘耗时减少约 **55.0%**，源码约 **43.5%**；两组方向一致，因此保留这两项。dua-core 单线程比基线更慢，不能把更换底层枚举 API 本身解释成收益。收益来自实际完整路径中的并行遍历与聚合。

同口径结果：磁盘 **199,648 个文件 / 130,343,203,047 字节**，读取错误为 0；源码识别 **7,014 个文件 / 21,938,277 字节**。完整磁盘包含构建/依赖，没有删除或跳过 target。普通目录排名为 refer、.git、docs、**crates（44,052,809 字节，约 42.01 MiB）**、.playwright-mcp、examples；crates 在当前面板 Top 5 中可见。

资源取舍：复测磁盘标准库进程累计 CPU 中位数约 **843.75 ms**，四线程约 **1,593.75 ms**，用更多累计 CPU 换取较低墙钟时间；并发固定上限，不能宣称 CPU 消耗下降。源码 CPU 中位数约 46.875 → 31.25 ms，但 Windows 时间量化较粗，不据此声称固定比例。复测进程累计峰值工作集 **25,894,912 字节（约 24.7 MiB）**，样本结束工作集约 18.6 MB；这是同一测试进程的累计峰值，**不是各算法独立峰值，也不是完整应用内存**。

原始日志和汇总：`artifacts/statistics-complete/production-benchmark.log`、`production-benchmark-summary.json`、`production-benchmark-repeat.log`、`production-benchmark-repeat-summary.json`。首组与大夹具准备有环境负载重叠，第二组独立复测方向一致。五个样本的经验 p95 取最大值；这些是本机暖文件缓存结果，不代表冷磁盘、网络盘、任意项目或固定提速保证。文档 35 的老项目小夹具结论保留为历史，本轮直接比较的是升级前后完整扫描路径。

## 4. 调试指标与复现

保留 `overview.stats_source_scan`、`overview.stats_disk_scan`、`overview.stats_git` 时长，新增累计计数 `overview.stats_disk_entries`、`overview.stats_disk_files`、`overview.stats_source_files`、`overview.stats_progress_dropped`。前两项扫描时长从取得对应扫描许可后计量；`overview.stats` 计量 collect_inner，缓存/进度路径中可能只覆盖源码与 Git，不能将其当成完整磁盘请求耗时。端到端扫描对照使用上面的显式测试计时；计数跨扫描累计，不能当当前项目实时数。

```powershell
# 不启动真实 Agent/终端 UI
cargo fmt --check
cargo clippy --locked --all-targets --target-dir target/i18n-check -- -D warnings
cargo test --locked --workspace --target-dir target/perf -- --test-threads=1

# 显式 >200k 项夹具（会创建/清理临时硬链接文件）
cargo test --locked --lib --target-dir target/perf over_two_hundred_thousand -- --ignored --nocapture --test-threads=1

# 真实项目只读完整扫描对照（请暂停该目录中的并发构建/写入）
$env:DEVTABS_STATS_BENCH_ROOT = 'D:\100_Projects\110_Daily\Yue-GPUI-Component'
cargo test --release --locked --lib --target-dir target/parallel-release complete_project_scan_comparison -- --ignored --nocapture --test-threads=1
Remove-Item Env:DEVTABS_STATS_BENCH_ROOT
```

测试与原始测量代码在 `src/overview/stats/verification.rs`，常规回归不执行昂贵大夹具和指定真实目录的测量。开启 UI 性能报告仍使用 README 的 `DEVTABS_PERF_DIR` 入口。

## 5. 边界与交付

当前缓存仍是来源/版本/TTL 校验后的采样结果：正常源码约 10 分钟、磁盘最长约 12 小时，读错误较短重试；F5 可强制刷新。根目录 mtime 不能证明全部后代未改，**本轮没有实现监听驱动的脏目录增量维护**，不把缓存命中解释为实时精确统计。实时扫描也不是文件系统原子快照；扫描中持续构建、删除或改名时，以实际成功枚举的结果与读取错误说明为准。

本轮不引入 MFT 整盘索引、自制原生线程池、文件内容快照或新的持久文件数据库。ffsearch1 保持只读参考；采用其批量遍历、目录任务、局部聚合思路，通过现成库落地。

交付程序：`artifacts/DevTabs-0.4.0-complete-statistics/devtabs.exe`。旧交付目录保持原样。直接运行进入总览，选择项目后宽屏看概况、窄屏 F8，F5 强制重扫；检查扫描进度、根目录文件命名、crates 目录与扫描完成状态。

```powershell
.\artifacts\DevTabs-0.4.0-complete-statistics\devtabs.exe
```

- 构建标识：`c3674218f7997b2c`；IPC `DevTabs-v6`。
- SHA256：`e03ed1934e3c654191f99d0cc14f2d546fa4fe30d8780c6fea8cccab3b557d1d`。
- 文件大小：23,637,504 字节。help/version 冒烟通过；完整源码校验清单与验证日志索引见同目录 `build.json`。

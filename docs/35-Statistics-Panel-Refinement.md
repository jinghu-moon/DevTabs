# Statistics Panel Refinement

## 1. 实现范围

针对总览统计面板纵向堆叠、图条固定 20 格、语言图例缺少颜色对应、目录信息拆成两行的问题，参考 `refer/DevTabs/src/ui.rs` 的 `stats_lines`、`section`、`lang_bar`、`lang_legend`、`meter` 和 `sparkline` 重新排版。绘制集中在 `src/overview/view/statistics.rs`，同一入口服务常驻栏、F8 弹窗和文字选区。

- 标题使用正文粗体与细分隔线，说明和次要数值降低强调，主要字节数加粗。
- “源码/磁盘”各占一行，文件数使用千位分隔；宽度足够时显示口径说明，窄栏明确换行。
- 语言色条使用背景填充，准确覆盖整个正文宽度；图例横向排列、按格宽换行，圆点颜色与色条一致。Rust、HTML、Python、JavaScript、PowerShell 使用稳定语言色，其他语言继续沿用已有颜色算法。
- 目录名称、右对齐大小和横条处于同一行；条形有灰色轨道，非零小项至少一格。构建目录采用琥珀色，普通 vendor 不据名称标为可清理目录；提示不承诺删除安全。
- 近 12 周趋势与提交总数放在一组，零周是灰色基线，有提交的周是绿色；最近提交日期与趋势并排，长提交标题仍可在项目详情查看。
- 保留部分结果、未知、无数据和 Git 失败状态；不能以零值冒充正在加载。

正文排版依据实际宽度，不以固定图条或长字符串交给自动换行。中文、英文与深浅主题均有行为/样式回归，覆盖 26/40/60/76/120 格以及更小边界。76 格固定样例正文约 19 行。TestBackend 样本位于 `artifacts/statistics-layouts/`；纯文本看不到色条的背景色，颜色由自动化样式断言验证，真机 UI 由用户验收。

## 2. 扫描与加载优化

保留现有源码语言识别、忽略规则、逻辑字节口径和资源上限，没有用少算文件或提前中止换取速度。

1. 磁盘遍历改用标准库 ReadDir，利用 Windows 目录枚举附带的元数据；顶层目录用整数索引传播，减少逐文件路径解析、目录名复制和映射查找。仍不跟随链接，读取失败标记部分结果，最多 200,000 条目/3 秒，定期检查取消。
2. 源码与磁盘独立计时，避免磁盘扫描先耗掉源码预算。源码继续使用 ignore 与 syntect/two-face，最多 200,000 条目/5 秒。
3. 常见 Git 仓库一次 log 同时取得最近提交和 12 周计数；不足六条近期提交时补读旧提交，不丢失长期未活跃项目的上下文。非 Git 目录先以仓库标记排除，避免重复启动失败的 Git 命令；支持父目录仓库、worktree 的 .git 文件和显式 GIT_DIR。普通新建仓库若 HEAD 指向尚不存在的分支引用、且没有 packed-refs，可直接确认零提交；首次提交或存在打包引用时继续走真实 Git 查询。
4. 磁盘任务复用刚完成且版本/TTL 有效的源码/Git 缓存，不再次执行相同采集。缓存仍验证项目键和根目录/Git/刷新标记；F5、force 和取消规则保留。源码与磁盘采样时刻可以不同，符合原有独立 TTL。
5. 新增 perf 指标 `overview.stats_source_scan`、`overview.stats_disk_scan`、`overview.stats_git`，与既有 `overview.stats`、UI 帧与输入延迟统计一起定位热点。

曾试用 Windows 原生批量枚举接口进一步加速，但相对标准库版本没有稳定额外收益，已回退该 FFI 封装，避免无必要复杂度。保留有效的目录聚合、Git 查询和缓存复用优化。

## 3. 与老项目实测对照

使用老项目原样的 `refer/DevTabs/src/stats.rs` 编译独立 Release 测量程序，只适配 config 路径、系统时间与隐藏 Git 窗口函数；不改写旧算法。新旧均使用 opt-level 3、thin LTO 的 Release 配置，参考源码 SHA256 记录在 `artifacts/stats-reference-bench/reference.json`。

固定目录包含 3,000 个 Rust 文件、12,000 个构建文件，分别测非 Git 目录和 新初始化、尚无提交的 Git 仓库（Git info/exclude 排除 target）。每条路径预热一次，再记录七次；每轮逐项断言源码文件数/字节数、磁盘文件数/字节数相等，双方都必须扫描完成且不是部分结果。计时不包含创建 fixture，属于操作系统文件缓存已预热的无持久缓存采集。

| 场景 | 指标 | 老项目中位数 | 本项目中位数 | 提升 |
| --- | --- | ---: | ---: | ---: |
| 非 Git 目录 | 源码 | 127.597 ms | 5.572 ms | 22.90 倍 |
| 非 Git 目录 | 磁盘 | 9.975 ms | 9.477 ms | 1.05 倍 |
| Git 仓库 | 源码 | 130.225 ms | 8.807 ms | 14.79 倍 |
| Git 仓库 | 磁盘 | 10.239 ms | 9.599 ms | 1.07 倍 |

本项目磁盘遍历优化前同样例中位数为 20.083 ms，优化后见表。源码采集原先已快于老项目，本次主要优化磁盘与重复采集，没有宣称重新发明源码识别。

| 场景 | 旧版源码 + 磁盘中位数 | 新版完整采集（含 Git 概况） |
| --- | ---: | ---: |
| 非 Git 目录 | 137.099 ms | 15.214 ms |
| Git 仓库 | 140.464 ms | 18.641 ms |

完整采集对比中，旧值取同轮源码+磁盘耗时之和；新版还包括额外的 Git 概况，信息量更大；该项用于暴露端到端成本，不作为同等工作量的倍数对比。Git fixture 尚无提交，其 Git 概况经过零提交快速路径；有历史的大仓库仍需实际 Git 查询，不承诺同等耗时。此项不包含 UI debounce、绘制和真实终端传输，也不能解释为任意目录、网络盘或冷磁盘都有固定倍数收益。

原始数据：`artifacts/statistics-scan-before.json`、`statistics-scan-after.json`；每次样本、计数和场景均保留。两套项目的真实仓库语言分类/忽略规则不完全相同，因此对照使用两者口径重合的 Rust/构建文件 fixture；不将不同覆盖范围的数据强行比较。

独立 2,000 文件缓存专项：首次采集+写入 10.878 ms，预热后无缓存采集中位数 5.093 ms，持久缓存命中中位数 0.101 ms；逐项验证结果一致。旧专项日志保留，不能把旧时刻的绝对耗时和本次同时测量等同。

复现：

```powershell
# 需要本地 refer/DevTabs 源码和已缓存的 serde_json 依赖，不启动 WT/Agent
.\scripts\Benchmark-Statistics.ps1 -Label after

# 持久缓存专项，输出原有 parity-cache-benchmark.json；对照历史记录前先保留旧文件
cargo test --release --locked --target-dir target/perf --test reference_parity statistics_cache_benchmark -- --ignored --nocapture --test-threads=1
```

脚本在 artifacts 下生成独立参考测量工程，只读取参考源码；正常 cargo test 不依赖该参考目录。最终新旧对照采用相同 Release 配置；首次优化前记录仅有非 Git fixture，参考 harness 当时为单 codegen unit，因此优化前日志只用于定位磁盘热点，不混入最终新旧倍数。新增依赖为零。

## 4. 验证与独立产物

- 升级前：lib / overview / i18n 共 103 通过、0 失败、4 忽略（`stats-ui-before.log`）。
- 最终完整 workspace：**339 通过、0 失败、27 忽略，28 suites**（`stats-ui-workspace.log`）。
- 专项复测：lib / overview / reference_parity / i18n 122 通过、0 失败、7 忽略（`stats-ui-final-regressions.log`）；随后新增空仓库/打包引用测试，已在最终完整回归通过。
- 显式 Release 旧项目对照/布局输出 2 项通过；持久缓存专项 1 项通过。
- fmt、Clippy `-D warnings`、Release 构建通过，产物 help/version 与 SHA256 校验见 build.json。

新增验证包括：色条精确宽度/颜色对应、横排图例、目录列对齐、小项可见、窄宽不溢出、深浅主题、部分/未知/零值、控制字符、源码 TTL 复用及强制刷新、根目录/深层/零字节文件计数、子目录 Git 仓库、近期与旧提交的联合查询。紧凑排版使原滚动 fixture 能完整显示，已增加真实增删行数据，继续验证长面板独立滚动和拖动。

新产物：`artifacts/DevTabs-0.4.0-overview-stats-ui/devtabs.exe`；旧 `overview-refactor` 包保留。构建标识、SHA256、源码清单、日志和测量 JSON 见新目录 `build.json`。本轮没有启动真实 Windows Terminal 或已安装 Agent，也没有提交或推送。

- 构建标识：`4b9e7404e43eb80c`。
- SHA256：`f098aa0543de3a05211498b319caf2f46ad92eab24d4802ee93507dc719ae1bc`。
- 文件大小：17,704,960 字节；包内附 142 个构建/测试/复现脚本输入的源码校验清单。

```powershell
.\artifacts\DevTabs-0.4.0-overview-stats-ui\devtabs.exe
```

进入总览后，宽屏直接查看概况；窄屏按 F8。请在宽窄窗口分别检查语言色条、目录横条、滚动条、部分结果和刷新状态。

# DevTabs Performance Profiling and Optimization

日期：2026-11-10。交付版本 **0.3.1**。本轮先保存 0.3.0 源码与验收结果，再加入统计、运行热点负载，最后针对测量结果优化。没有新增依赖。

## 开启统计

```powershell
# 使用可写目录；从新进程启动时读取环境变量
$env:DEVTABS_PERF_DIR = Join-Path $PWD 'artifacts\perf-session'
.\artifacts\DevTabs-0.3.1\devtabs.exe --data-dir "$PWD\artifacts\perf-workspace" --project 'D:\Projects\Demo' --layout split

# 后续启动恢复默认，不影响已经启动的统计进程
Remove-Item Env:DEVTABS_PERF_DIR
```

独立 `--data-dir` 用于让本次前端及其新宿主都继承统计环境。已运行的宿主不会因设置变量而自动开启统计；不要为此终止其他工作区。使用现有数据目录也可以统计新前端。

默认关闭：热路径不取时间、不更新计数，也不启动统计线程或写入报告；配置仅初始化一次。开启后，每个进程增加一个导出线程，每 2 秒覆盖自己的 `devtabs-<pid>-<instance>.json`。正常返回时补写最后一段；强制退出可能丢失最后约 2 秒。报告数量随启动过的进程数增长，单个进程的样本不会无限累积。会话结束后可删除对应目录。

报告写入采用临时文件后替换，周期导出与退出导出串行，记录路径不等待导出锁。输出失败不影响终端；若没有报告，检查目录权限。公共 `perf::flush()` 返回实际 I/O 错误，测试覆盖失败路径。固定标签不记录文件路径、源码或输入内容。固定计数和 64 桶直方图约 18 KiB / 进程；开启后的线程及 JSON 构建另外占用内存。

```powershell
# 按进程查看累计时间最高的 12 项
Get-ChildItem $env:DEVTABS_PERF_DIR -Filter 'devtabs-*.json' | ForEach-Object {
    $report = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
    "PID $($report.pid), version $($report.version), uptime $($report.uptime_seconds)s"
    $report.timings | Sort-Object total_ms -Descending | Select-Object -First 12 |
        Format-Table name,calls,total_ms,mean_ms,p95_upper_ms,max_ms,over_50_ms
}
```

请在移除环境变量前执行上面的查看命令，或直接将目录路径赋给变量后读取。

## 指标及解释

统计覆盖帧绘制及终端输出提交、UI 组合与输入分发、树构建/组件绘制/视口复制、目录批处理、监听合并、读取解码、工作队列等待和结果背压、预览搜索/渲染/高亮、编辑候选复制/校验/绘制/保存，以及 IPC、宿主请求与快照、终端解析。

每项记录次数、累计/平均/最大耗时、p50/p95/p99 桶上界以及至少 50 ms 的次数。`files.queue_rejected` 使用次数表达拒绝，不解读其耗时。队列等待从提交记录至取出任务，不包含之后的其他锁等待。

这些是**包含子调用的墙钟耗时**。`sidebar.draw` 包含树/预览，`preview.render` 包含高亮，`host.request` 可以包含进程启动和锁等待。父子跨度及并发线程时间不能相加当作 CPU 时间；`ipc.poll` 高只说明调用耗时，不能直接判为 CPU 热点。直方图百分位为 2 的幂次桶上界，实时原子快照允许近似；下表负载测试另外保留每次耗时并排序，表中的 p95 是实际样本分位。

最终规模负载中，4 个会话前端的 `ipc.poll` 平均约 7.14–7.72 ms，而宿主快照生成平均约 0.068 ms，宿主终端解析平均约 0.012 ms；前端帧绘制并提交平均约 0.36–0.76 ms。IPC 调用时间不等于这些处理工作的耗时，因此本轮没有仅按累计等待时间修改轮询协议。

## 优化依据与实现

1. 文件树：Agent 输出触发重绘时，原实现每帧重新创建全部 TreeItem，并让组件重新展开全部已加载节点。现在以文件模型修订、宽度、主题、图标及预览文件标记缓存一份投影；选择、展开状态、滚动、焦点和几何不变时，再复用一份可见视口 Buffer。仍由 tui-tree-widget 维护树状态与鼠标命中映射。键鼠输入及模型变化使对应缓存失效，替换投影前释放旧值，不为每个目录保存绘图副本。
2. 搜索着色：原实现对每个可见字素遍历全局命中列表，滚动到末段时尤其昂贵。现在对按行排序的结果二分定位当前行，再随字素前进；无选择且无本行命中时直接跳过着色，超出可见宽度后停止。保留重叠匹配、组合字素与宽字符选择语义。
3. 编辑导航：方向键等不修改文本，直接调用编辑组件导航，无须克隆文档及撤销历史或重新验证文本预算。真实修改仍使用候选缓冲区校验。测试发现原有 `input_without_shortcuts` 不处理方向键，本轮同时修复；因此旧“导航耗时”实际测到的是无效操作，不能用它计算同功能加速比。

## Release 测量

| 负载 | 优化前均值 ms | 优化后均值 ms | 优化前 p95 ms | 优化后 p95 ms |
| --- | ---: | ---: | ---: | ---: |
| 1 万节点重复绘制（含首次冷帧） | 16.653 | 0.188 | 22.634 | 0.060 |
| 密集搜索着色：开头 | 1.154 | 0.251 | 1.278 | 0.379 |
| 密集搜索着色：末段 | 51.161 | 0.267 | 59.519 | 0.512 |
| 全量查找 8 万处（单次） | 5.951 | 5.554 | 5.951 | 5.554 |
| 无搜索命中的预览绘制 | 0.221 | 0.171 | 0.223 | 0.176 |
| 508,000 字节编辑器输入 | 2.443 | 2.479 | 3.639 | 2.873 |

100 次树绘制中，`tree.build_items` 与 `tree.widget_render` 的调用次数均从 **100 次降为 1 次**；其余帧仅复制可见单元格。末段搜索着色避免逐字素扫描之前约 76,000 个命中。

修复后的导航均值 0.000113 ms，不与旧无效操作计算加速比。编辑文本仍保留原有预算与候选提交机制。

| 规模负载 | 0.3.0 默认 | 0.3.1 默认 | 0.3.1 开启统计 |
| --- | ---: | ---: | ---: |
| 空闲整机 CPU | 0.50% | 0.22% | 0.27% |
| 基础工作集总和 | 81.42 MiB | 81.56 MiB | 82.03 MiB |
| 预览与编辑工作集总和 | 93.82 MiB | 94.46 MiB | 95.31 MiB |
| 保存采样峰值工作集总和 | 97.35 MiB | 97.91 MiB | 98.81 MiB |

本次开启与关闭统计均满足现有整机空闲 CPU < 1%、DevTabs 保存采样峰值 < 100 MiB 目标。工作集较旧版本略升，缓存并非免费；两次运行的差异包含系统调度与工作集波动，不能据此声称统计开销为零。


同机、release、开启统计。1 万节点使用合成文件元数据，排除批量创建磁盘文件；100 次重复绘制包含第一次冷构建。预览使用 1,000 行 × 80 次命中；末段从第 950 行开始绘制，重复 10 次。这里证明的是这些负载下的局部优化，不代表整个应用同比加速。一次运行存在调度及缓存波动；未针对文本输入、全量查找或脏比较宣称收益。

真实目录测试另创建 30,000 文件，仍限制最多缓存 10,000 节点，验证批量加载与冷渲染。规模测试运行 100 项目、8 会话、6 个 DevTabs 进程，覆盖打开预览、508,000 字节编辑、撤销及保存。内存为这 6 个进程之和，排除 PowerShell、Agent、WT/OpenConsole、编译器及测试运行器；CPU 为 20 逻辑处理器整机百分比。保存峰值是采样值。开启/关闭统计各一次实测只用于验证目标，不作为严格统计开销估计。

## 回归与证据

普通测试 **51 项**通过：18 单元、13 编辑、1 统计进程隔离、5 PTY、7 侧栏、7 宿主；fmt 和 Clippy（warnings denied）通过。额外验证热点负载、开启/关闭统计的规模负载、Codex/Claude 原生路径输入、实际 Windows Terminal 布局及大目录。统计子进程夹具在普通测试中标记 ignored，由其父测试显式执行，不算缺失验收。

新增测试将缓存与完整组件绘制逐格比较，核对选择/滚动/展开/焦点/主题/宽度/预览标记与鼠标命中；核对重叠搜索、组合字素和宽字符；验证真实导航、Shift 选区、修订号、撤销历史；验证默认关闭、周期覆盖、退出补写、报告不含路径及 I/O 错误不破坏进程。既有保存、冲突、编码、预算与进程生命周期测试继续通过。

- [优化前源码及原验收](../artifacts/performance-baseline/source/)、[加入统计但未优化的源码](../artifacts/performance-baseline/instrumented-source/)。
- [优化前热点](../artifacts/performance-baseline/profile-final/hotspots.json)、[优化后热点](../artifacts/performance-verification/hotspots-profile/hotspots.json)。
- [优化前规模](../artifacts/performance-baseline/scale-release.json)、[默认模式规模](../artifacts/performance-verification/scale-default.json)、[开启统计规模](../artifacts/performance-verification/scale-profile.json)、[多进程原始报告](../artifacts/performance-verification/live-profile/)。
- [最终命令、退出码及耗时](../artifacts/performance-verification/checks.json)，同目录保留每项完整日志。
- [交付构建信息与源码哈希](../artifacts/DevTabs-0.3.1/build.json)、[可执行文件 SHA-256](../artifacts/DevTabs-0.3.1/SHA256SUMS)。

复现热点测试：

```powershell
$env:DEVTABS_PERF_DIR = Join-Path $PWD 'artifacts\perf-reproduce'
cargo test --release --locked --target-dir target/perf --test performance release_hotspots -- --ignored --nocapture --test-threads=1
Remove-Item Env:DEVTABS_PERF_DIR
```

完整验收命令和隔离环境见 [执行脚本](../artifacts/run-performance-verification.py) 与 [补充回归脚本](../artifacts/run-performance-regressions.py)。历史 0.3.0 验证见 [08](08-File-Editor-Verification.md)。

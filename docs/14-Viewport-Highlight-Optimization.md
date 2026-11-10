# Viewport Highlight Optimization

日期：2026-11-10。第一项优化；基线提交 `78e2b15`。测量数据：[01-Viewport-Highlight.json](performance/01-Viewport-Highlight.json)。

## 决策与实现

绘制当前文本不应等待语法着色。代码预览与编辑器只为可视行及上下各 16 行预读生成彩色 Span；缓存命中直接绘制，未命中显示当前位置原文。首次打开立即计算；滚动停止 80 ms 后提交最终需求。每个文档最多一个滚动高亮任务，旧任务逐行检查代次并退出，避免积压中间位置。原文与行号持续更新。

正确显示多行注释、嵌套注释或原始字符串需要前文的词法状态。每 128 行保存解析状态与完整 ScopeStack；首次跳到深处仍解析尚未建立检查点的前缀，但不为前缀计算主题样式或分配彩色 Span。后续跳转复用检查点。更换内容目前重建检查点；增量失效留给第三项优化。

Oniguruma 的解析状态不能跨线程发送，因此保留总计两个后台线程：一个处理文件操作，一个拥有渲染器与词法检查点。UI 只发送共享源码快照、视口与取消代次，接收可发送的渲染结果与统计，不通过锁访问解析器。文件大小、行长与 Span 预算继续有效。

全文件的逻辑纯文本、搜索与选择映射仍保留，避免改变跨屏搜索、复制和源文件坐标。公开的完整高亮函数继续用于参照测试及 Markdown 代码块；真实源码预览和编辑器使用可视区路径。本项不修改 Markdown 排版，快速滚动时不会在 Markdown 排版与源码间切换。

## 前后数据

相同 182000 字节、7000 行 Rust 文件，100×40 终端，Release 构建，共享语法与主题提前初始化，连续五次打开取中位数。只测文件操作至首屏高亮及 TestBackend 绘制，不含物理终端输出。

| 指标 | 优化前 | 优化后 |
| --- | ---: | ---: |
| 打开至首屏高亮 | 166.41 ms | 9.92 ms |
| 保留高亮行 | 7000 | 52 |
| 彩色 Span | 63000 | 468 |
| 进程工作集 | 22.25 MiB | 17.45 MiB |
| 私有内存 | 13.28 MiB | 8.25 MiB |
| 单次绘制均值的中位数 | 0.273 ms | 0.365 ms |

打开耗时减少约 94%，工作集减少约 21.6%。绘制微基准增加约 0.09 ms，未实现绘制加速；低于 1 ms，保留本项优化的依据是明显改善打开耗时与保留内存。

一次真实 Sidebar 的 Release 快速滚动负载发送 60 次滚动更新，中间不新增高亮任务，最后停留处新增一次。tick 与绘制耗时中位数 0.360 ms、最大 0.688 ms；最终位置原文约 0.309 ms 可见，颜色约 207.44 ms 完成，后者包含 80 ms 等待与首次解析 7000 行前文。总共解析 7053 行，但只着色 69 行，建立 55 个检查点。80 ms 并不代表所有深处跳转能在 80 ms 内完成高亮。

六个 DevTabs 进程的既有规模负载通过：预览/编辑工作集总和 93.79 MiB，保存采样峰值 96.51 MiB，空闲整机 CPU 0.227%。这些是指定负载的实测结果，不能推广为所有输入的资源保证。

## 验证与复现

升级前新增“7000 行文件不应保留全部彩色行”测试，确认旧实现以 7000 行失败。样本最初触发旧 Span 上限、退化成纯文本，已丢弃该样本的数据；正式比较确认旧实现实际保留 63000 个彩色 Span。

升级后 97 项普通工作区测试通过，包含四项新增回归：多行/嵌套注释与原始字符串在双主题和双向跳转下逐行等同完整高亮；源码变化、控制字符、未知语言、预算与取消；实际 Sidebar 快速滚动原文即时可见、停稳仅新增一个任务且跨屏复制正确；真实 UI 保留多种代码颜色且彩色行数有界。另通过 fmt、Clippy（拒绝警告）、Release 热点、大目录与规模测试，实际 PTY/进程负载串行执行。

```powershell
cargo test --workspace --all-targets --locked -- --test-threads=1
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:DEVTABS_VIEWPORT_REPORT = 'artifacts/viewport-after.json'
cargo test --release --locked --test viewport_highlight source_viewport_workload -- --ignored --test-threads=1
$env:DEVTABS_SCROLL_REPORT = 'artifacts/scroll-after.json'
cargo test --release --locked --test viewport_highlight fast_scrolling -- --test-threads=1
```

统计默认不写日志。`DEVTABS_PERF_DIR` 启用聚合计时，其中 `preview.viewport_highlight` 覆盖实际 worker 计算。`Document::highlight_stats()` 提供请求、缓存命中、取消、解析行、着色行、保留行和检查点数量，不记录路径或文件内容。

结论：收益得到数据支持，保留、提交并推送第一项。后续依次评估 Markdown 按块布局、编辑增量失效和流式保存，不把这些后续实现混入本项提交。

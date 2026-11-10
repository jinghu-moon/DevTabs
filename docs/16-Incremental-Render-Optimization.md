# Incremental Render Optimization

日期：2026-11-10。第三项优化，基线 `6f90999`。数据：[03-Incremental-Render.json](performance/03-Incremental-Render.json)。

## 实现与边界

代码内容变化后比较显示文本的公共前后行。修改前的词法检查点继续有效；修改后的检查点暂存，只有重新解析得到完全相同的 ParseState 和 ScopeStack、且后续文本未变时才能恢复。状态恢复后可跳到更靠近视口的检查点，跳过确定未变的中间区域。增删行会调整后缀检查点行号；未闭合注释等状态改变仍可能传播至文件末尾，不假定“只改一行就只解析一行”。

Markdown 保留全局事件和引用解析，避免引用定义改变时复用错误链接。只有布局宽度、主题、已解析事件、原始源字节及绝对源坐标都相同的缓存块才复用。存在渲染警告时保守重建；插入引起后续源坐标移动时保守失效，不新增复杂坐标重映射。样式缓存预算沿用第二项。

编辑器同一 revision 的滚动/预览请求复用共享源码快照，实际修改才构造新快照。UI 收到结果仍校验当前 revision，旧颜色不得应用到新文本。现阶段编辑器自身的候选缓冲复制、历史、校验等开销没有被消除；本项针对解析与布局重复计算。

## 对比

Release，同一 7000 行 Rust 文件，在第 6501 行修改一条语句，可视区 30 行，五次独立缓存负载取中位数：

| 指标 | 优化前 | 优化后 |
| --- | ---: | ---: |
| 编辑后的高亮计算 | 101.61 ms | 3.39 ms |
| 重新解析行 | 6546 | 146 |
| Markdown 局部标题修改后的首屏处理 | 1.823 ms | 1.387 ms |
| Markdown 重排块 | 26 | 1（复用 25） |

代码计算减少约 96.7%，Markdown 样本约 23.9%。这是实际生产 worker 使用的渲染器调用测量，包含快照转 Arc 的调用参数开销，未包含编辑输入分发、快照文本拼接、终端输出及滚动等待，不能声称用户按键完整延迟为 3.39 ms。

升级前新增测试确认局部代码编辑重解析 6546 行、Markdown 未复用块，均失败；升级后通过。108 项普通工作区测试通过，新增六项覆盖：深处局部编辑、Markdown 未变块复用、多行词法改变与增删行、状态恢复后的下游检查点复用、引用/公式变化和移动源坐标、真实 Sidebar 编辑器的当前版本与颜色。fmt、Clippy（拒绝警告）及既有 Release 六进程规模测试通过。本项不声明普遍内存降幅，规模数据随结果归档。

收益得到数据支持，保留、提交并推送第三项。

## 复现

```powershell
cargo test --test incremental_render -- --test-threads=1
$env:DEVTABS_INCREMENTAL_REPORT = 'artifacts/incremental-after.json'
cargo test --release --locked --test incremental_render incremental_edit_workload -- --ignored --test-threads=1
```

`HighlightStats::reused_checkpoints` 记录恢复的检查点；`MarkdownStats::reused_blocks` 记录当前快照复用的块。统计不记录文件内容、路径，原有默认关闭的聚合计时继续覆盖实际渲染路径。

# Markdown Renderer Implementation

日期：2026-11-10。版本：**DevTabs 0.4.0**。实施依据：[11-Codex-Markdown-Rendering-Research.md](11-Codex-Markdown-Rendering-Research.md)。

## 完成范围

调研建议的六个阶段均已落地：共享主题、块级排版、响应式表格、来源映射与资源预算、Mermaid 子集、数学子集。文件读取、后台任务、编辑缓冲区和现有性能统计继续复用。

| 阶段 | 最终行为 | 主要代码 |
| --- | --- | --- |
| 1. 共享主题 | 深色 Catppuccin Mocha / 浅色 Latte；Markdown、文件高亮、编辑器使用同一语法主题；标题、强调、链接、引用分层 | [theme.rs](../src/preview/theme.rs)、[preview.rs](../src/preview.rs) |
| 2. 块级排版 | 正文按词换行；中文与字素簇按显示列计算；列表续行对齐；代码保留缩进和长行，使用已有左右键横向滚动 | [markdown.rs](../src/preview/markdown.rs)、[layout.rs](../src/preview/layout.rs) |
| 3. 表格 | 按列内容分配宽度，优先收缩路径等长 token，保留短值；无法满足列宽时改为字段/值记录 | [table.rs](../src/preview/markdown/table.rs) |
| 4. 来源与预算 | 显示行保存原文字节范围；复制源码可按选择取回公式、实体、图形等来源；高亮前检查预算；主题参与缓存失效；重排清除旧选择 | [preview.rs](../src/preview.rs)、[sidebar.rs](../src/sidebar.rs) |
| 5. Mermaid | 已闭合围栏中的受支持图形转成带主题的 Unicode 图；不支持、超宽、超限时保留源码并提示原因 | [devtabs-mermaid](../crates/devtabs-mermaid/README.md) |
| 6. 数学 | 支持美元及反斜线定界符的有限 TeX 子集；分数、上下标、矩阵等排版；代码/链接受保护；无法转换时恢复原文 | [math.rs](../src/preview/math.rs)、[math/render.rs](../src/preview/math/render.rs) |

本次直接消费 `pulldown-cmark` 事件，不再使用 `tui-markdown` 的围栏标记及二次扫描。Markdown 解析仍由成熟库承担；本地模块只负责展示、布局和来源映射。`tui-syntax-highlight` 也已移除，统一直接使用 Syntect。

直接依赖锁定到 pulldown-cmark **0.13.6**、textwrap **0.16.4**、unicode-linebreak **0.1.5**。延续 Ratatui 0.30.2、Syntect 5.3.0、two-face 0.5.2+bat-0.26.1。textwrap 使用自定义 Unicode 断行入口与已有字素处理，关闭会额外引入 ICU 的默认断行功能，减少此处不必要的依赖和运行时数据。

## 交互与显示约定

- 点击 `.md` 文件进入阅读模式；预览焦点下按 **m / M**，或点击 `[M]`，切换阅读/源码。
- 阅读模式保留 DevTabs 隐藏标题标记的选择，通过字重、颜色和留白区分标题级别。字体大小由终端决定。
- 代码块不按正文重排。长代码可用左右方向键横向阅读；围栏语言支持 `rust,no_run`、`rust title=demo` 等元数据。
- Markdown 的字面代码示例保持代码；不会套用 Codex Agent 消息入口展开 Markdown 围栏的特殊处理。
- **Ctrl+C** 复制当前显示选择；F2 菜单“Copy source”在有选择时使用来源映射，无选择时复制完整源码。公式或图形可取回原始表达式/围栏，合成列表缩进不混入来源。
- 图形等转换以完整原文单元复制；普通字面文本支持精确片段。Tab 展开有独立范围，选择后面的词不会复制整段代码。来源映射不是完整 Markdown AST 反序列化；例如加粗文字的普通文本范围不自动重建外侧 `**`，完整源码仍可直接复制。
- 宽度、模式或源内容变化时清除旧视觉选择；搜索按新显示行重新计算。主题切换后重算，异步旧结果不能替代当前请求。
- 编辑器 `[P]` 预览当前未保存缓冲区；显示转换不修改缓冲区或磁盘内容。
- 图片显示替代文字和地址，HTML 保留文本。Agent PTY 输出仍由 Agent 自己排版，文件预览渲染器只处理本项目拥有的预览内容。

## 支持边界和资源约束

Mermaid 沿用 Codex 模块明确列出的流程图、时序、状态、类、ER 子集。输入最多 16 KiB；图结构最多 16 节点 / 24 边；画布 65,536 单元。完整语法列表和时序图独立限制见其 [上游说明](../refer/codex-main/codex-rs/mermaid/README.md)。未闭合围栏不尝试转换。

数学沿用有限字符排版模块：识别 `$…$`、`$$…$$`、`\(…\)`、`\[…\]`，每式转换预算小于 4,096 字节，最多 16 行、256 列。代码、链接、HTML 受保护，货币与未知命令有回退。空间布局必须完整容纳；嵌套结构采用更保守的转换条件。这些能力不等于完整 Mermaid 或 LaTeX 支持。

高亮前检查 512 KiB、10,000 行、任意单行 4 KiB 上限；超限显示纯文本，并由预览/编辑器结果提示预算。保留 100,000 Span 预算。Markdown 最多生成 30,000 显示行，达到预算时提示剩余内容可在源码模式查看。极窄窗口中无法容纳的双列字素用 `�` 表示并保留来源，不输出半个字素；缩进占满视口时显示可容纳前缀。

扩展语法集仍只在默认集合缺失语言时加载；主题及其正文 Palette 分别延迟缓存。完成后的文档只保留当前布局结果，不为每个历史宽度累积缓存。文件预览没有引入聊天流式状态机或通用插件系统。

读取结果、只读渲染任务与来源映射通过 `Arc<str>` 共享原文。正常读取直接使用解码字符串，只有实际超出行/列预算时才构造截断副本；无需替换控制字符的内容借用原文，纯文本回退直接构造显示行，避免先复制整个字符串再拆行。编辑器仍拥有自己的可修改缓冲区，保存冲突所需的快照继续保留。

## 升级前后测试

升级前保留 [0.3.2 源码与锁文件](../artifacts/markdown-upgrade-baseline/)，原 57 项普通测试通过。新增五项行为测试先在旧实现上运行，**全部按预期失败**，见 [new-regressions.log](../artifacts/markdown-upgrade-baseline/new-regressions.log)：按词换行、窄表记录、围栏元数据、长行高亮预算、图表和公式转换。

升级后，workspace 串行普通回归 **93 项全部通过**：原有 57 项 + 新增 15 项渲染/交互测试 + Mermaid 上游 21 项。另有一个按需运行的发布版 Markdown 性能负载。检查记录见 [checks.json](../artifacts/markdown-upgrade-verification/checks.json)，全量日志见 [tests-serial.log](../artifacts/markdown-upgrade-verification/tests-serial.log)。fmt / Clippy warnings denied 通过。

本轮测试覆盖：

- 实际 Ratatui Buffer 的明暗主题、粗斜体、删除线、下划线、代码词法颜色、嵌套样式。
- 1 / 2 / 3 / 5 / 20 / 40 / 80 / 120 列边界、中文、emoji ZWJ、组合字符、URL、悬挂缩进。
- 窄表记录、单元格样式、左右对齐和选择来源；未知语言、多行语法、Tab 与空行、字面 Markdown。
- 公式与图形的显示和原文选择；代码/链接/金额保护；未知、超宽、未闭合输入回退。
- 渲染扩展预算、终端控制字符清理、高亮前预算。
- 侧栏异步读取、resize 与主题切换、未保存缓冲区预览、源码/阅读往返、搜索与选择。
- 原有真实嵌套 PTY、输入、编辑保存、外部冲突、宿主会话和进程生命周期回归。

首次全量回归与发布版负载同时运行时，两项 PTY 测试出现启动超时和输出进程退出，记录在 [tests-final.log](../artifacts/markdown-upgrade-verification/tests-final.log)。没有放宽断言或增加超时；后续在独立串行运行中全部通过。该结果表明串行复测通过，不能单凭这次复测确定早先失败的根因。

初次新增 Unicode 测试也检出一列宽视口替换字素后来源坐标未同步的问题；已修复并纳入回归。

## 性能验证

新增 `markdown.parse_layout`、`markdown.table_layout`、`markdown.code_layout`、`markdown.mermaid`、`markdown.math` 统计点，沿用 `preview.highlight`。统计不记录文档内容和路径。

发布版负载比较 100 组正文、100 行表格、200 行 Rust 代码，在 20 / 40 / 80 / 120 列各运行 12 次，分别记录首次耗时和后 11 次均值。升级前后文件见 [baseline](../artifacts/markdown-upgrade-baseline/render-workload.json) 与 [final](../artifacts/markdown-upgrade-verification/render-workload-final.json)。新布局及来源信息不同，显示行数也会变化；这些数字是本机端到端测量，不是对解析器的孤立排名。

热点分析显示代码负载主要耗时在共享高亮器。已去掉取切片时复制整个代码 Piece 的开销，只复制所需片段；主题 Palette 也缓存复用。窄表降级减少了大量碎行，但新主题高亮比旧主题耗时更长，不能将本次改造概括为所有场景均加速。

一次规模测试先以 99.33 MiB 峰值通过，最终复测却达到 **100.23 MiB**，超过原目标；[失败数据](../artifacts/markdown-upgrade-verification/scale-before-source-sharing.json) 和日志保留。随后修正新增来源映射的原文重复持有，并消除正常读取、无控制字符预览和纯文本回退中的整段冗余复制。共享所有权有指针一致性测试，实际保存行为继续接受全量回归；没有提高内存上限。

最终数值见下方测量摘要及 JSON。真实规模测试继续执行原有 100 MiB 工作集、空闲 CPU 和交互延迟断言，统计排除 PowerShell、Agent、Windows Terminal/OpenConsole、编译器和测试运行器。该负载的通过不代表任意多种语法和任意扩展内容组合都已测量。

| 负载 / 宽度 | 0.3.2 热均值 | 0.4.0 热均值 |
| --- | ---: | ---: |
| prose / 20 | 0.865 ms | 1.467 ms |
| prose / 40 | 0.757 ms | 1.227 ms |
| prose / 80 | 0.709 ms | 0.944 ms |
| prose / 120 | 0.914 ms | 0.984 ms |
| table / 20 | 4.947 ms | 0.974 ms |
| table / 40 | 1.625 ms | 1.322 ms |
| table / 80 | 0.556 ms | 0.700 ms |
| table / 120 | 0.474 ms | 0.699 ms |
| code / 20 | 9.195 ms | 15.303 ms |
| code / 40 | 8.618 ms | 15.691 ms |
| code / 80 | 8.834 ms | 15.187 ms |
| code / 120 | 8.666 ms | 15.515 ms |

| 最终规模与交互负载 | 结果 |
| --- | ---: |
| 6 个 DevTabs 进程空闲整机 CPU | 0.14% |
| 预览/编辑工作集总和 | 95.80 MiB |
| 保存采样工作集峰值 | 99.63 MiB |
| 30,000 目录项负载 Markdown 冷 / 热渲染 | 6.69 / 4.86 ms |
| tree_10000_repaint 均值 | 0.178 ms |
| preview_draw_dense_matches_near_end 均值 | 0.256 ms |

[规模数据](../artifacts/markdown-upgrade-verification/scale.json)、[热点数据](../artifacts/markdown-upgrade-verification/hotspots/hotspots.json)、[大目录数据](../artifacts/markdown-upgrade-verification/large-directory.json)。

## 构建和手工验收

可执行文件：[artifacts/DevTabs-0.4.0/devtabs.exe](../artifacts/DevTabs-0.4.0/devtabs.exe)。构建命令、哈希及源码指纹见 [build.json](../artifacts/DevTabs-0.4.0/build.json)。

从项目根目录运行：

```powershell
.\artifacts\DevTabs-0.4.0\devtabs.exe --project 'D:\100_Projects\110_Daily\new-project' --layout split
```

在文件树打开 [tests/fixtures/markdown-reading.md](../tests/fixtures/markdown-reading.md)，检查标题、引用、任务列表、代码颜色、窄表、流程图、分数与矩阵；缩放窗口后切换 m / M，再进入编辑并使用 `[P]` 查看未保存内容。

测试后端还导出了 8 组宽度/主题组合的 [Buffer JSON](../artifacts/markdown-upgrade-verification/buffers.json) 和方便浏览的 [HTML 预览](../artifacts/markdown-upgrade-verification/buffer-preview.html)。它们反映实际单元格颜色及修饰，**不是操作系统截图**；字体、斜体和 emoji 外观仍以真实终端为准。

自动回归命令：

```powershell
cargo test --workspace --all-targets --locked -- --test-threads=1
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
```

## 第三方源码与复现

Mermaid 源码位于 `crates/devtabs-mermaid`，保留上游测试，快照文件名前缀适配本地 crate 名称，期望内容不变。数学预处理与布局位于 `src/preview/math*`，按 pulldown-cmark 0.13、标准迭代器和本地宽度函数作适配。

许可、NOTICE 和原始/本地文件哈希见 [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md)、[licenses/codex](../licenses/codex/)。运行与构建无需 `refer/`。本次没有编译完整 Codex，也没有将其内部聊天、超链接输出或流式状态机整体引入。

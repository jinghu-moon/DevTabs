# DevTabs File Tree Library Research

调研日期：2026-11-10。现有架构：Ratatui 0.30.2、Crossterm 0.29.0、原生 ConPTY Agent 会话。本次仅调研，没有变更依赖或实现功能。

后续方案见 [侧边栏功能实现文档](06-File-Sidebar-Implementation.md)：补充 `tui-markdown` 阅读模式，并明确布局、焦点、拖放、实施顺序和升级前后测试要求。本文保留初次调研结论；实施以该功能文档为准。

## 需求与推荐

侧边栏整体可收起；目录可展开/折叠；点击文件显示只读预览；将侧边栏文件拖到同一 DevTabs 界面的 Agent 区域后，插入该文件的绝对路径，不自动提交。

推荐 **tui-tree-widget 0.24.1 + tui-syntax-highlight 0.2.0 / syntect 5.3.0**。按需添加 ignore 0.4.34 处理忽略规则，notify 8.2.2 处理文件变化。继续使用现有 Ratatui 布局、Crossterm 鼠标事件和会话输入链路，不引入另一套 GUI、编辑器或拖拽框架。

版本来自当日 crates.io API 的最新稳定版，并核对发布包源码及依赖。Context7 用于查阅树组件与 Syntect API，网页检索补充候选和项目维护信息。以下为调研判断，尚未进行新增依赖的编译或实际交互验证。

## 库比较

| 库 | 已核对的能力和兼容性 | 结论 |
| --- | --- | --- |
| [tui-tree-widget 0.24.1](https://crates.io/crates/tui-tree-widget) | 展开、折叠、选择、滚动；`rendered_at` 做坐标命中，`click_at` 选择/切换；依赖 ratatui-core ^0.1、ratatui-widgets ^0.3，与当前 0.30 系列匹配 | 首选树视图。应用提供文件节点和按需目录读取，库负责树状态、绘制及命中 |
| [ratatree 0.4.0](https://crates.io/crates/ratatree) | 文件列表/真正树形模式、原地展开、鼠标选择、模糊搜索、多选、隐藏文件、路径；依赖 ratatui ^0.30 | 一体化备选，做原型更快；作为常驻项目侧栏需要调整选择器交互 |
| [ratatui-explorer 0.3.0](https://crates.io/crates/ratatui-explorer) | 文件浏览、当前路径、主题及预览示例；依赖 ratatui ^0.30 | 更适合单目录进入/返回的文件选择器，当前实现不是所需的多层原地展开侧栏树，不列首选 |
| [tui-syntax-highlight 0.2.0](https://crates.io/crates/tui-syntax-highlight) | Syntect 高亮直接转换成 Ratatui Text，支持行号与显示样式；依赖 ratatui-core ^0.1、syntect ^5 | 首选代码预览适配层，避免自行转换高亮样式 |
| [syntect 5.3.0](https://crates.io/crates/syntect) | 代码语法检测、高亮及主题，支持逐行处理 | 高亮引擎；配合上面适配层使用。Windows 可选纯 Rust fancy-regex 后端，但需要实测性能 |
| [ignore 0.4.34](https://crates.io/crates/ignore) | ripgrep 使用的目录遍历和 .gitignore/.ignore 匹配 | 需要忽略规则时采用，不自行实现 glob 和层级规则 |
| [notify 8.2.2](https://crates.io/crates/notify) | 跨平台文件系统监听，提供 Windows 后端 | 需要自动更新文件树/预览时采用；合并事件后刷新受影响目录 |

前三个树/浏览器项目均为 MIT；高亮适配层为 MIT OR Apache-2.0，Syntect 为 MIT，ignore 为 Unlicense OR MIT，notify 为 CC0-1.0。

另外检查了 ratatui-interact 0.5.4 和 ratatui-toolkit 的文件树文档。前者提供大量组件及焦点体系，当前只增加一个侧栏没有必要引入整套交互体系；后者文档使用的 `ratkit-file-system-tree` 包名在当日 crates.io API 查询返回 404，不能仅凭文档将其列为可直接采用的依赖。

## 为什么首选通用树组件

项目侧栏需要长期保留展开状态、按需加载大目录、监听刷新和与 Agent 共用焦点规则。tui-tree-widget 专注这部分交互，应用只需把文件系统映射为节点；无需自己实现树的可见行计算、选择、展开状态与鼠标命中。

ratatree 的已发布源码确认支持 Tree 模式，不只是目录列表。但还存在以下适配工作：

- 默认鼠标处理仅选择和滚动，点击目录展开、点击文件预览仍要接业务行为；没有文件到 Agent 的拖放操作。
- 它的 `filter` 明确不传入目录，所有目录保持可见，不能直接用该回调剪除被忽略的 node_modules/target 等目录。
- 展开/刷新在状态方法内同步读取目录。若用于大仓库，要验证 UI 阻塞和异步加载方案；不能把“现成文件选择器”当成已经满足性能约束。
- Enter/Esc 默认确认或取消选择器；常驻侧栏需要接管这些按键，防止与 Agent 区域冲突。

因此 ratatree 是值得验证的一体化备选，tui-tree-widget 更适合当前明确的常驻侧栏交互。二者都没有包含预览及 Agent 输入的完整端到端功能。

## 拖到 Agent 的边界

当前需求是同一 TUI 内的拖动，现有 [Crossterm MouseEventKind](https://docs.rs/crossterm/0.29.0/crossterm/event/enum.MouseEventKind.html) 已有 Down、Drag、Up，无需新增桌面拖放库：

1. 左键按下文件行时，记录命中的 PathBuf。
2. 移动超过阈值后进入拖动状态；拖动期间由 DevTabs 捕获事件，不转发成 Agent 鼠标拖选。
3. 在 Agent 区域松开后，规范化绝对路径，针对中文、空格和引号生成待插入文本。
4. 通过现有 Agent Session 输入通道按其当前粘贴模式发送，只插入文字，不发送 Enter。
5. 普通点击在松开时触发预览，避免拖动起点先切换布局；在其他区域松开则取消拖动。

这需要少量应用内的交互状态，属于组件之间的连接逻辑。树库不认识 DevTabs 的 Agent 会话，不可能直接替代它。侧栏整体收起/展开同样只需当前 Ratatui Layout 和一个可见状态。

DevTabs 嵌入的是原生 Agent CLI，没有其输入框对象；向会话发送文字不等于保证该 CLI 当前一定处于聊天编辑状态。必须验证真实 Codex/Claude 正在编辑提示词时的效果，并定义菜单、运行任务、Agent 已退出等状态下的行为。不能靠拖动坐标推测 Agent 内部输入框位置，也不自动添加 @ 或伪造模型请求。

本结论仅覆盖同一 DevTabs 视图内的拖动。Windows Explorer 拖入、跨独立 WT 窗口拖放属于另一种系统交互，不由这里的鼠标事件方案保证。

## 预览范围和接入验证

建议先实现只读文本/代码预览：用现有 Ratatui Paragraph/滚动布局承载高亮 Text，不增加文本编辑器。Markdown 可先显示源码。图片、PDF、Office 和二进制预览需要另外的解码/渲染方案，不能由 Syntect 包办。

目录只在展开时读取；预览按点击加载并缓存，限制读取字节、行数及异常长行。大文件、未知编码、权限错误、符号链接循环和文件已删除必须有明确反馈。ignore 的隐藏规则要允许用户切换显示，避免配置文件被意外隐藏。

后续实施应验证：

- 侧栏整体收起、目录展开/折叠、滚动后坐标命中、窗口缩放和切换项目后状态。
- 单击预览与拖动互不误触；预览关闭后恢复原 Agent 视图，进程与输入内容保持。
- 中文/空格/引号路径拖放，取消拖动，连续拖动不重复插入，不自动提交。
- 原生 Codex/Claude 提示词编辑及菜单/运行状态；仅 Agent 收到路径，Debug 不受影响。
- 大仓库、文件变化突发、限长预览，以及既有 100 项目/8 会话性能回归。

## 原始资料

- [tui-tree-widget TreeState API](https://docs.rs/tui-tree-widget/0.24.1/tui_tree_widget/struct.TreeState.html)
- [tui-tree-widget 仓库](https://github.com/EdJoPaTo/tui-rs-tree-widget)
- [ratatree 文档](https://docs.rs/ratatree/0.4.0/ratatree/)
- [ratatree 发布包事件源码](https://docs.rs/crate/ratatree/0.4.0/source/src/event.rs)
- [ratatree 发布包目录读取源码](https://docs.rs/crate/ratatree/0.4.0/source/src/entry.rs)
- [ratatui-explorer 预览示例](https://github.com/tatounee/ratatui-explorer/blob/master/examples/file_preview.rs)
- [tui-syntax-highlight 文档](https://docs.rs/tui-syntax-highlight/0.2.0/tui_syntax_highlight/)
- [Syntect](https://github.com/trishume/syntect)
- [ignore WalkBuilder](https://docs.rs/ignore/0.4.34/ignore/struct.WalkBuilder.html)
- [notify](https://docs.rs/notify/8.2.2/notify/)

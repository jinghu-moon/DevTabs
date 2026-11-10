# DevTabs Preview Rendering Fixes

日期：2026-11-10。版本 **0.3.2**，修复用户截图中 Cargo.toml 单色以及 Markdown 阅读与源码模式差异不明显的问题。

## 根因与修复

Rust / JSON 已有高亮，说明终端颜色及 Span 绘制正常。Syntect 5.3.0 默认语法集不包含 TOML、PowerShell 等语言，因此先前会静默回退到纯文本。新增 **two-face 0.5.2+bat-0.26.1** 的现成语法集合，以同一 Oniguruma 后端补齐缺失语言；优先沿用较小的默认语法集，仅缺失时按需初始化扩展集。支持按文件全名、扩展名和语言名识别，未知语言继续显示可读文本。

Markdown 的标题、引用等样式存在于 Ratatui 的 Line 上；原有转为静态文本及预换行只复制 Span，丢失了行级样式。同时 tui-markdown 默认保留标题 `#` 与代码围栏，导致看起来接近源码。

现在保留 Text → Line → Span 的样式继承与覆盖，换行后继续保留颜色、粗体、斜体等；阅读模式隐藏标题标记与围栏。标题使用青蓝色与粗体，一级标题加下划线；行内代码和代码块使用可辨背景。链接保留下划线、引用保留颜色、表格继续由原库绘制。终端字体大小保持一致，通过颜色、字重和留白区分层级。

文件预览、编辑器和 Markdown 代码块共用高亮函数。关闭 tui-markdown 自带的独立代码高亮，避免它只能识别默认语言的问题；通过它公开的 code_block_fence 样式接口标记已解析代码块，再替换为共享高亮结果。标记每次生成且保证不出现在输入中，展示前移除；Markdown 语法、嵌套、列表及表格仍交给现有库解析，不另写 Markdown 解析器。测试覆盖嵌套引用/列表中的代码、空行、空代码块、未知语言与字面 Markdown，防止误删代码内容。

## 使用与预期

点击 `.md` 默认阅读模式；焦点在预览时按 **m / M**，或点击标题 `[M]`，切换阅读/源码。编辑时 m / M 输入普通字符，点击 `[P]` 预览当前未保存缓冲区。

阅读模式应呈现以下效果：

### 标题、行内样式与链接

这是 **粗体**、*斜体*、~~删除线~~、`行内代码`，以及 [项目文档](https://example.com/docs)。链接显示文本和目标，不自动打开。

> 引用文本应带颜色，窄窗口换行后继续保持样式。

### TOML 与 PowerShell 代码块

```toml
[package]
name = "devtabs"
version = "0.3.2"
enabled = true
# 字符串、布尔值、键和注释应有颜色差异
```

```powershell
$answer = 42
Write-Host "Hello DevTabs"
# Markdown 中的 PowerShell 与 .ps1 文件使用同一套语法
```

| 模式 | 标记 | 代码 |
| --- | --- | --- |
| 阅读 | 隐藏标题 # 与围栏 | 带语法颜色和背景 |
| 源码 | 显示 Markdown 原文 | 保留源码语法高亮与行号 |

HTML、图片、Mermaid 和数学公式仍遵循现有终端文本预览边界，不执行或绘制浏览器内容。读取、编辑、行数及高亮 Span 预算没有扩大。浅色主题也有独立的可读配色。

## 验证

新增渲染测试先在 0.3.1 上运行，确认 TOML 颜色、标题样式和阅读标记测试失败：[升级前输出](../artifacts/rendering-baseline/regression-before.log)。[原源码](../artifacts/rendering-baseline/src/) 与 Cargo.lock 已保存。

新增 **6 项**普通测试，检查实际 Ratatui Buffer 中的颜色和字体修饰，不以“文本中存在标题”替代样式断言。覆盖：

- TOML / Rust / JSON / PowerShell 至少三类词法颜色。
- 阅读标记隐藏、源码保留、标题及行内样式、窄窗口换行。
- 明暗主题代码块、嵌套代码字面内容与空行。
- 实际 Sidebar 异步读取、模式往返、搜索高亮、TOML 编辑器颜色。
- Windows 短暂共享冲突后完整读取，以及持续锁定明确返回错误。

全量回归还发现既有 Windows ReplaceFileW 读者测试偶发共享冲突 32；此前仅重试 NotFound。现在两种瞬态错误都使用原来的最多 4 × 5 ms 后台重试，永久删除/持续占用仍返回错误；保存冲突协议和写入权限不变。[失败记录](../artifacts/rendering-verification/tests-before-sharing-retry.log) 保留。

最终普通测试 **57 项全部通过**（原 51 项 + 6 项渲染/读取回归），fmt / Clippy warnings denied 通过。真实嵌套 PTY 的预览、编辑和保存测试包含在全量测试内。另完成 release 热点、大目录和 100 项目 / 8 会话规模测试。

| 最终测量 | 结果 |
| --- | ---: |
| 1 万节点重复绘制均值（含首次冷帧） | 0.194 ms |
| 密集搜索末段绘制均值 | 0.257 ms |
| 6 个 DevTabs 进程空闲整机 CPU | 0.29% |
| 预览与编辑工作集总和 | 94.88 MiB |
| 保存采样峰值工作集总和 | 98.19 MiB |

规模测试排除 PowerShell、Agent、WT/OpenConsole、编译器和测试运行器，CPU 按本机 20 逻辑处理器整机比例计算。


首次直接使用完整扩展集时，原规模负载保存采样峰值为 **101.06 MiB**，超过 100 MiB 目标；[原结果](../artifacts/rendering-verification/scale-before-lazy-syntax.json) 和失败日志保留。改为缺失语言按需加载后复测，结果如下。该规模负载包含 Rust、Markdown 内 Rust、纯文本编辑及大目录，尚不能把它的内存数字套用到同时加载多种扩展语言的场景；扩展语法集在进程内缓存，不按文件重复初始化。

最终命令、退出码、耗时及日志见 [checks.json](../artifacts/rendering-verification/checks.json)。真实侧栏 Buffer 的 JSON 验证样本见 [snapshots](../artifacts/rendering-verification/snapshots/)，它们不是操作系统截图。构建及哈希见 [0.3.2 交付信息](../artifacts/DevTabs-0.3.2/build.json)。

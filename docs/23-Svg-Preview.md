# SVG Preview

日期：2026-11-11。承接 [终端图片设计](21-Terminal-Image-Preview-Design.md) 与 [实现验证](22-Terminal-Image-Preview-Verification.md)，将静态 SVG 纳入独立图片标签和 Markdown 本地图片。

## 1. 行为与范围

单击文件树中的 `.svg`，进入现有只读图片预览；Markdown `![说明](diagram.svg)` 使用相同后台 worker、缓存、懒加载与切片。没有扩展名时，前 8 KiB 包含可识别 SVG 根标签也能转入图片管线。现有 0 适应、1 原比例、+/- 缩放、平移、信息、外部打开、历史、标签及停滚恢复全部复用。SVG 的 100% 指一个 CSS 像素对应一个终端图形像素，物理单位按 96 DPI 解析。

采用最新稳定版 `resvg 0.49.0`（Apache-2.0 OR MIT），只启用 `text`、`system-fonts`，不启用 SVGZ、图片解码及内存映射字体等默认 features。使用其重导出的 usvg、tiny-skia 和 roxmltree，不自研 SVG 解析、文字排版或光栅器。

支持静态路径、形状、渐变、透明、裁剪、遮罩及渲染器支持的滤镜。`<text>` 使用已安装系统字体；第一次遇到文字时在图片线程加载字体库，之后全前端复用，中文等字符的覆盖取决于本机字体。缺失字体可能替换，系统外部打开仍可用于对照。

首期要求 UTF-8、SVG 命名空间和有效尺寸/viewBox；支持 BOM、XML 声明及注释。DTD/实体声明、SVGZ 不支持；脚本、事件、动画和交互不执行，foreignObject 不作为浏览器页面渲染。SVG 内 `<image>` 的本地文件、URL 与 data image 引用统一省略，不自动读取文件或下载内容；信息面板明确显示此范围。`defs` 与 `use` 的文档内部引用由 usvg 处理。Markdown 引用 SVG 文件本身仍遵守既有本地路径规则。

## 2. 管线与资源

读取和文件版本检查复用原管线，格式类型明确区分 PNG/JPEG/WebP/SVG。源数据改为互斥的位图或 SVG 树；SVG 不先生成原尺寸 RGBA，更不生成放大后的整张图。

SVG 解析后记录 CSS 尺寸，使用现有 geometry 计算适应、缩放和归一化中心，再分配最大 2 MiPixel 的可见区域 Pixmap。根变换同时执行缩放与可见原点平移；放大时重新从矢量绘制，避免放大固定像素快照。输出背景先设为当前预览实际背景，tiny-skia 负责预乘 Alpha 合成，然后交给现有 Sixel/Half-block 编码路径。滤镜等场景可能仍遍历整棵树或使用额外临时图层，不能将“仅分配可见目标像素”解释为只计算相交路径。

SVG 文本最大 4 MiB，XML 最多 50,000 节点，嵌套深度限制约 128；现有单边 32,768、名义源尺寸 32 MiPixel、目标 2 MiPixel、Sixel 8 MiB 与呈现缓存 32 MiB 继续生效。尺寸检查可以解析 XML/矢量树，但不加载系统字体或分配目标栅格；它不是常数时间文件头读取。

`image.svg_render` 统计实际矢量绘制时间，原 `image.decode_orient` 对 SVG 表示解析。SVG 的 `image.source_bytes/source_pixels` 为 0，因为没有源 RGBA 像素；这些 gauge 不计矢量树和字体库。第三方解析对象、字体、路径和滤镜图层用进程工作集辅助观察，预算不构成进程硬内存上限。

## 3. 验证与样例

新增 `tests/svg_preview.rs`，六项覆盖：

- 路径/透明/viewBox、格式签名、大小写扩展名及无扩展名、双协议。
- 800% 直接矢量绘制时的硬边界与左右平移裁剪，避免低分辨率放大。
- 错误 XML、非 SVG 根、零尺寸、超限尺寸/像素/字节、DTD、过深与过多节点。
- 本地文件/远程/data image 引用不加载。
- SVG 独立标签、Markdown 内联以及历史返回。
- 系统字体文字确实产生图形像素。

位图既有测试同步适配明确的格式枚举，其余图片、Markdown、编辑保存与工作区回归继续运行。升级前基线为 199 通过、0 失败、19 忽略，证据为 `artifacts/image-fixes-full.log`。本轮全量 **205 通过、0 失败、19 忽略**；SVG 专项六项全部通过，fmt、Clippy `-D warnings` 及 release 构建通过。日志：`artifacts/svg-full.log`、`svg-special.log`、`svg-clippy.log`、`svg-release.log`、`svg-features.log`。

可提交样例：[tests/fixtures/svg-preview.svg](../tests/fixtures/svg-preview.svg)，包含渐变、透明、裁剪、路径和文字；由本项目编写，遵循 MIT。

```powershell
cargo test --locked --test svg_preview -- --test-threads=1
cargo test --workspace --locked -- --test-threads=1
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo build --release --locked
```

真机测试由用户执行：从文件树打开样例，检查比例、透明留白、文字和边界；按 1、+/- 与方向键观察放大/平移；在含 SVG 的 Markdown 中滚动、停滚、点图和返回，确认不影响 Agent/Debug。Sixel 与 `--image-protocol halfblock` 分别验证；不自动操作用户终端 UI。

## 4. 独立测试产物

程序：[artifacts/DevTabs-0.4.0-svg-preview/devtabs.exe](../artifacts/DevTabs-0.4.0-svg-preview/devtabs.exe)，15,083,008 字节。SHA-256：`F13D504D463CD2777E092FDEEF1FA4629E145AA75BA9026D23B48E6A93426130`。

同目录有 `svg-preview.svg` 与 `svg-preview.md`，包含独立文件、重复引用和折叠引用入口；旧测试程序保留。文件树打开样例即可验收，使用 Shell 模式可避免启动 Agent：

```powershell
& ".\artifacts\DevTabs-0.4.0-svg-preview\devtabs.exe" --project "$PWD\artifacts\DevTabs-0.4.0-svg-preview" --agent shell --layout split --new-instance
```

在当前项目与 Codex/Debug 同时测试：

```powershell
& ".\artifacts\DevTabs-0.4.0-svg-preview\devtabs.exe" --project "$PWD" --agent codex --layout split --new-instance
```

从文件树打开 `tests/fixtures/svg-preview.svg`；`artifacts` 默认被忽略，可按 I 显示后打开产物目录中的 Markdown。真机效果等待用户反馈；本轮未提交、推送。

来源：[resvg 0.49.0](https://docs.rs/resvg/0.49.0/resvg/)、[usvg 0.49.0 Options](https://docs.rs/usvg/0.49.0/usvg/struct.Options.html)、[上游最小示例](https://github.com/linebender/resvg/blob/v0.49.0/crates/resvg/examples/minimal.rs)。依赖由 Cargo.lock 固定。

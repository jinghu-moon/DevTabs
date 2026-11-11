# Terminal Image Preview Verification

日期：2026-11-10。设计：[21](21-Terminal-Image-Preview-Design.md)。基线：`26d8370`。本轮完成实现与自动化验证；Windows Terminal 真机 UI、画质和残影由用户验收，尚未提交或推送。

## 1. 实现范围

文件树支持 PNG、JPEG、静态 WebP 图片标签，以及无扩展名但具有支持格式签名的文件。提供适应窗口、100%、12.5%–800% 手动缩放、以指针为中心的滚轮缩放、平移、滚动条、信息、重试、外部打开、协议与 Photo/Screenshot/Mixed 切换。图片只读，拖到 Agent 仍插入绝对路径。文本快照、渲染结果和编辑器由 `Content::Text` 持有，与 `Content::Image` 互斥；历史位置也区分文本与图片。

Markdown 本地图片独立成块，保留说明和源映射，初始占位三行，最终最大 48 行。只对可见图片解码像素；附近已排版图片可以预取尺寸，不扩大全篇排版。尺寸预取仍有界读取压缩文件、检查版本与动画，不解码栅格。重复路径共享呈现，尺寸到达后按源码位置恢复阅读。滚动后暂隐图片，静止 150 ms 后恢复最终视口。点击进入图片标签，Alt+Left 返回。

默认 auto 在当前前端确认 Sixel 能力和单元像素尺寸后使用 Sixel，否则退回 Half-block；后者是字符预览。探测与正常输入复用唯一 Windows Events 读取者，启动等待上限 500 ms，窗口或焦点变化后重查。原生响应分片、超时、过长、迟到及物理键交错有专项测试；未知来源的输入仍优先保留。没有调用 Picker 的独占 stdin 查询函数。

一个串行图片 worker，最多一个执行、一个可替换待执行和一个结果槽；阶段间检查取消，过期结果不恢复。关闭、隐藏、非活动标签释放像素与呈现。图片处理不占用文本保存 worker。Sixel 区域、呈现代次或切片偏移改变时局部重绘旧图片暴露区域；浮层期间抑制图片，关闭后重发缓存。静止图片不随 Agent 输出重新编码或发送。真机反馈后的 Fit/滚动修复见第 6 节。

## 2. 资源与依赖

压缩文件 32 MiB；单边 32,768；源像素 32 MiPixel；decoder 分配提示 192 MiB；目标 2 MiPixel；单份 Sixel 8 MiB；前端呈现缓存合计 32 MiB。缓存包含字符串、字符像素和切片索引的 allocation capacity。原始像素仅保留当前一个文件；放大只生成可见裁剪，不构造 800% 整图。

这些是应用预算，不能等同于整个进程的硬内存限制：第三方编码工作区、临时转换、Ratatui 双缓冲和终端自身内存另计。性能报告提供缓冲近似值及 Windows 进程当前/峰值工作集。Sixel writer 记录跨 write 的实际 DCS 次数、字节数和写入/flush 时间，不把写入完成称为终端显示完成。

| 组件 | 实际版本与处理 |
| --- | --- |
| ratatui-image | 11.1.0，关闭默认 features，只启用 crossterm |
| image | 0.25.10，只启用 PNG/JPEG/WebP |
| icy_sixel | 生产 0.5.1；0.7.0 仅 dev 基准候选 |
| fast_image_resize | 6.1.0，生产使用 Bicubic |
| 本地补丁 | `vendor/ratatui-image`，MIT 许可证保留 |

补丁只开放实际编码参数、复用编码后的切片、修正顶部裁剪 band 数量及报告持有字节；组件布局与协议绘制仍复用上游。撤除条件见 [补丁说明](../vendor/ratatui-image/DEVTABS-PATCH.md)。生产依赖树未启用 Chafa，不要求安装额外动态库。

## 3. 自动化验证

| 检查 | 结果 / 证据 |
| --- | --- |
| 升级前全量 | 172 通过，0 失败，14 忽略；`artifacts/image-preview-before.log` |
| 升级后全量 | 196 通过，0 失败，17 忽略；`artifacts/image-preview-after.log` |
| 图片专项 | `image_preview`：17 通过、1 子进程 fixture 忽略；全量测试实际调用该 fixture |
| Markdown 图片 | `markdown_images`：7 通过，含附近尺寸预取时 decode/encode 均为 0 |
| 格式 / lint | `cargo fmt --all -- --check`、Clippy `-D warnings` 通过 |
| 发行构建 / CLI | release 构建与独立 exe 的 `--help` 通过 |
| 依赖核查 | `artifacts/image-features.log`、`artifacts/image-duplicates.log` |

新增覆盖格式/签名、动画拒绝、EXIF、Alpha、预算、裁剪边界、profile 实际参数、失败重试、最新任务优先、未保存取消、图片/文本历史、隐藏释放、菜单命中、探测输入过滤，以及 Markdown 折叠、远程拒绝、重复引用、尺寸预取、快速滚动和非 6 倍数单元高度切片。既有文本高亮、阅读、搜索、编辑、保存、书签、工作区与生命周期测试仍运行。

静止测试以实际 Crossterm 后端写入字节缓冲，模拟至少 10 秒、1,000 次 Agent 内容更新，断言 decode、encode、图像传输次数及图像字节的增量全部为 0。覆盖恢复会再发送；普通文本更新不重发。这里验证实际序列和状态，不能替代 Windows Terminal 像素观察。

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked -- --test-threads=1
cargo test --locked --test image_preview --test markdown_images -- --test-threads=1
cargo build --release --locked
cargo test --release --locked --test image_benchmark -- --ignored --nocapture --test-threads=1
```

## 4. 性能对照与保留决定

环境：Windows 11 IoT Enterprise LTSC 10.0.26100、Intel i5-13600KF、Rust 1.98.1、release。素材由程序生成，MIT，六类 3840×2160 图缩至 640×360，各 16 次；先合成到相同背景，避免 Alpha 策略污染对照。固定素材及哈希：`artifacts/image-benchmark/sha256.json`；完整原始数据：`comparison.json`。未控制 OS 文件缓存，也不包含真实 WT 绘制时间。

| 素材 | image Bicubic P50/P95 ms | FIR Bicubic P50/P95 ms | 两缩放器 RGB MAE（0–255） |
| --- | --- | --- | --- |
| gradient | 31.28 / 42.20 | 14.38 / 16.00 | 0.019 |
| texture | 31.55 / 38.30 | 14.21 / 19.76 | 0.118 |
| code-dark | 41.17 / 50.04 | 18.04 / 37.54 | 0.040 |
| code-light | 37.59 / 68.44 | 20.49 / 45.96 | 0.047 |
| mixed | 42.02 / 55.11 | 20.05 / 32.21 | 0.061 |
| alpha | 31.06 / 41.00 | 13.77 / 18.89 | 0.039 |

FIR 的 P50 加速约 1.83–2.28 倍，因此保留；MAE 只表示两种缩放实现的数值差异，不代表人工画质已经验收。0.5.1/0.7.0 及 Photo/Screenshot/Mixed 的时间、输出体积、解码尺寸和误差均有对照；0.7 没有稳定耗时优势，未进入生产。生产仍采用 Wu256，扩散值分别为 0.875 / 0 / 0.5，默认 Mixed，等待用户用真实照片和密集截图确认。

补充的应用管线基准见 `workload.json`：生成 4K 纹理，目标 640×360、诊断单元 8×18。尺寸预取、冷加载、连续裁剪各 16 次，缓存重绘 1,000 次。这里“冷”指 DevTabs 没有解码/呈现缓存，不指磁盘冷缓存；字节 sink 不包含终端回压。

| 场景 | P50 / P95 ms |
| --- | --- |
| 尺寸检查，无像素解码 | 19.35 / 20.56 |
| 读取、解码、缩放、编码 | 110.04 / 120.98 |
| 缓存源图，连续 100% 可见裁剪 | 9.43 / 11.23 |
| 缓存图片，Agent 内容变化时重绘 | 0.183 / 0.324 |

初次传输一次、363,739 Sixel 字节；后续 1,000 帧额外图像传输为 0。库比较进程峰值工作集 84,795,392 字节，管线基准进程为 84,201,472 字节；这是各自受控进程的数据，不能外推到真实多会话 DevTabs/WT 的总内存。

统计通过 `DEVTABS_PERF_DIR` 开启，新增 `image.*` timings/counts/gauges，包括尺寸检查、请求等待、读取、解码方向、裁剪/Alpha/缩放、量化编码、切片准备、源缓存命中、取消/替换、输出字节、清理、降级及进程工作集。常规报告不包含路径或用户输入。直方图分位数仍为桶上界；基准文件中的分位值来自单次计时样本。

## 5. 独立产物和用户验收

程序：[artifacts/DevTabs-0.4.0-terminal-images/devtabs.exe](../artifacts/DevTabs-0.4.0-terminal-images/devtabs.exe)，11,821,056 字节。

SHA-256：`CE816732EC374A7DCCD449D43F7E0666A283A7C15307273AF1936FD7C727D83A`。

同目录提供 `image-preview.md`、七张生成 PNG 和 `help.txt`；未覆盖旧测试程序。产物与本地性能日志由现有 `.gitignore` 排除。

从项目根目录启动图片测试目录，Shell 模式无需启动 Agent：

```powershell
& ".\artifacts\DevTabs-0.4.0-terminal-images\devtabs.exe" --project "$PWD\artifacts\DevTabs-0.4.0-terminal-images" --agent shell --layout split --new-instance
```

文件树单击 `image-preview.md` 和 PNG。按 0/1/+/-、方向键、滚轮、滚动条检查；打开 I、More、路径提示后关闭，观察恢复；切文本、关闭图片、最大化、收起预览、改变窗口/字体和退出，观察越界与残影。快速滚动 Markdown，检查停滚后高度与阅读位置。用相同命令加 `--image-protocol halfblock` 对照。

已有项目内测试 Codex/Debug 共存：

```powershell
& ".\artifacts\DevTabs-0.4.0-terminal-images\devtabs.exe" --project "$PWD" --agent codex --layout split --new-instance
```

`artifacts` 默认被忽略，可在文件树按 I 显示后进入产物目录。需要性能报告时，启动前设置 `$env:DEVTABS_PERF_DIR = "$PWD\artifacts\image-live-perf"`；完成后 `Remove-Item Env:DEVTABS_PERF_DIR`。宿主完整统计需要独立 `--data-dir`；本轮新增图片统计由新前端记录。

真机尚待确认：WT 版本/字体/DPI、真实照片与小字截图画质、实际输出回压、浮层覆盖和退出残影、窗口/字体变化探测，以及连续外部文件更新。本节记录的位图交付不包含 SVG/PDF；后续 SVG 支持见 [23](23-Svg-Preview.md)。GIF/APNG/动态 WebP、PDF、ICC/HDR 色彩管理、远程图片自动下载和 Agent 图形协议透传仍不在支持范围内。Sixel 按六像素 band 保守裁剪，非整除字体高度下可能省略最多五个边缘像素，仍需真机观察。

保留用户已有 README、Markdown 测试样例及设计文档的工作区改动。依据设计的真机画质/清理门槛，当前提供可测产物与自动化证据；真机验收通过后再提交、推送。

## 6. 真机反馈：Fit 超限与 Markdown 滚动闪烁

用户反馈后确认两个实现问题：Fit 只计算窗口限制，算完再检查 2 MiPixel，较大的窗口中连 Fit 自己也会失败；App 对每次 Sixel 切片/位置变化调用 `terminal.clear()`，导致 Agent、Debug 和预览一起被清空再绘制。

修复后，Fit 同时满足窗口、源图尺寸和最终向上取整后的目标像素预算。100%/手动缩放继续保持真实比例，超过预算时明确拒绝，不暗中降采样。使用用户提供的 2560×1440 JPEG，在诊断单元 16×32、200×70 内容区复现；原实现的新增测试失败记录为 `artifacts/image-fit-before.log`。修复后完整管线在 Sixel 与 Half-block 下均成功，目标 1931×1086，缩放 75.417%；结果见 `artifacts/image-real-photo.log`。照片文件不加入仓库，也不擅自视为 MIT 基准素材。

图片变化改用 `PixelDamage`：只强制重绘旧图片矩形中暴露的文本/空白单元，保留新图的协议锚点和 Skip 单元。图片清理与新帧放在 DEC 2026 同步输出事务中，正常滚动不执行整屏 clear；退出时结束同步模式。Markdown 第一次滚动便暂隐图像并保留缓存/高度，停滚 150 ms 后只恢复最终可见切片，避免较慢的连续滚轮绕过原“三次/100 ms”门槛。

新增回归验证普通空白单元也会清除旧像素、其他区域不被强制刷新、第一次滚动即抑制传图、停滚复用相同编码代次、More 覆盖恢复，以及同步输出始终配对。至少 10 秒/1,000 次 Agent 更新的零额外解码、编码和图像字节测试已经改为运行新的输出路径。

当前全量结果：**199 通过，0 失败，19 忽略**，见 `artifacts/image-fixes-full.log`。额外忽略项为真实用户图片专项及 release 输出基准；这两项已显式执行通过。格式检查、Clippy `-D warnings`、release 构建通过，日志分别为 `image-fixes-clippy.log`、`image-fixes-release.log`。

为分离“局部清理”和“滚动抑制”两个因素，release 基准对同样 61 帧、每帧移动一张图片的序列，分别运行旧整屏清屏与新局部更新（120×30、字节 sink）。数据见 `artifacts/image-benchmark/scroll-output.json`：

| 指标 | 旧整屏清理 | 新局部更新 |
| --- | --- | --- |
| 整屏清屏次数 | 60 | 0 |
| 不变 Agent 文本写入次数（含首帧） | 61 | 1 |
| Sixel 传输次数 | 61 | 61 |
| 总输出字节 | 973,527 | 974,361 |
| 输出合成 P50 / P95 ms | 0.1262 / 0.1817 | 0.0639 / 0.1713 |

局部更新消除了整屏清理并保留其他分区，P50 下降约 49%；该对照中图像仍每帧移动，传输次数相同，字节增加约 0.086%，不宣称降低了这项负载的图像带宽。实际 Markdown 滚动专项另验证：连续六次滚动期间额外传图为 0，停下只恢复一次。以上均为字节/状态证据，真实 WT 像素清除和视觉闪烁仍交由用户复测。

新版程序位于 [artifacts/DevTabs-0.4.0-image-preview-fix/devtabs.exe](../artifacts/DevTabs-0.4.0-image-preview-fix/devtabs.exe)，11,822,080 字节。SHA-256：`3CB1A039173D2596BB394B46A64A0EC8EE38C8910FB05F7B044BF9BDCDDD0060`。这是新目录，旧测试程序保留。

```powershell
& ".\artifacts\DevTabs-0.4.0-image-preview-fix\devtabs.exe" --project "$PWD" --agent codex --layout split --new-instance
```

在文件树打开用户测试照片，按 0；随后打开含图 Markdown 连续滚动，观察 Agent/Debug 是否保持稳定。新产物目录提供 `image-preview.md`，仅以相对路径引用原照片和上轮生成素材；根目录中 `artifacts` 被忽略时按 I 显示，或使用该产物目录作为 `--project`。

```powershell
# 真实图片专项：仅后台处理，不打开终端 UI
$env:DEVTABS_IMAGE_TEST_FILE = Join-Path $PWD 'Dragoon X (6).jpg'
cargo test --release --locked --test image_preview real_image_fit_at_large_viewport -- --ignored --exact --nocapture
Remove-Item Env:DEVTABS_IMAGE_TEST_FILE

# 输出清理前后对照：不启动 DevTabs/WT UI
cargo test --release --locked --test image_benchmark image_scroll_output_comparison -- --ignored --exact --nocapture
```

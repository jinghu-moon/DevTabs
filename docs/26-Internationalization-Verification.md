# 26-Internationalization-Verification

## 库与职责

使用 [rust-i18n 4.2.4](https://docs.rs/rust-i18n/4.2.4/rust_i18n/)，关闭默认功能，依赖由 Cargo.lock 固定。`src/lib.rs` 通过库的 `i18n!` 宏加载 `src/i18n/en-US.json` 和 `zh-CN.json`，英语作为回退语言。编译期生成翻译数据，运行时不读取或解析 JSON。

`src/i18n.rs` 仅保留 DevTabs 的语言偏好、系统语言识别，以及调用库的适配接口。查询使用 `rust_i18n::t!`，语言切换使用 `set_locale`，命名参数使用库的 `replace_patterns`。不维护另一套翻译查找表、回退引擎、占位符解析器或按英文原文查找的字典。

选择此库是因为当前需求是 Rust TUI 中英文资源、运行时切换和命名参数。暂不增加 Fluent、ICU 或另一套并行国际化系统。当前没有承诺多语言复数、日期/数字地区格式化；未来出现这类实际需求时再评估相应库。

## 使用与资源规则

- 语言偏好为 `auto`、`en-us`、`zh-cn`，实际资源 locale 为 `en-US` 和 `zh-CN`。
- Auto 优先读取非空 LC_ALL、LC_MESSAGES、LANG，再读取 Windows UI 语言；中文选择简体中文，其余回退英语。
- 总览 Ctrl+L 或命令面板切换，工作区 Ctrl+G → U 或 F2 菜单切换，配置后台原子保存。
- `devtabs config language zh-cn` 持久设置；`--language en-us` 仅覆盖当前前端。
- 只翻译 DevTabs 自己的界面。路径、文件内容、项目便签、命令、Git 提交信息、Agent/Debug 输出和日志保持原文。
- 使用稳定语义 key 和 `%{name}` 命名参数。中英文 key、非空值、命名参数必须一致，禁止重复 JSON key。
- 静态查询保持库返回的 `Cow<str>`，在需要持有字符串的边界转成 String；按钮绘制与点击范围使用终端字符单元宽度。

总览及侧栏菜单、关键提示、预览/文件信息使用同一套资源。Clap 命令行帮助、系统/第三方错误与部分旧诊断文案仍为英文；技术格式名和协议名称保留原名。

## 自动验证

`tests/i18n.rs` 验证中英文资源契约、逐项库查询与借用返回值、实际库 locale 切换、Auto 偏好保留、配置往返、命名参数不递归解释、缺失 key/参数行为，以及界面显式 key 的资源存在性。

总览测试覆盖中英文不同宽度绘制、中文搜索、弹窗层级与点击区域、设置失败保留输入、过期结果丢弃、批量重试复用工作区和原生历史回退。已有文件树、预览、编辑、图片、保存与隔离 ConPTY 测试继续运行。依赖机器上 Agent 或真实 Windows Terminal 的测试维持显式忽略，由用户真机验收。

```powershell
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets --target-dir target/perf -- -D warnings
cargo test --locked --workspace --all-targets --target-dir target/perf -- --test-threads=1

$env:DEVTABS_I18N_REPORT = "$PWD/artifacts/rust-i18n-benchmark.json"
cargo test --release --locked --test i18n --target-dir target/perf i18n_lookup_cost -- --ignored --nocapture
Remove-Item Env:DEVTABS_I18N_REPORT
```

日志为 `artifacts/overview-fmt.log`、`overview-clippy.log`、`overview-full.log` 和 `rust-i18n-benchmark.log`；性能原始数据保存在 `artifacts/rust-i18n-benchmark.json`。

最终结果：工作区 **255 通过、0 失败、21 项默认忽略**；fmt、Clippy（`-D warnings`）、Release 构建和独立产物 `--help` 检查通过。Clippy 本轮使用独立缓存目录 `target/i18n-check`，其余完整回归与构建使用 `target/perf`。终端屏幕断言明确指定英语，避免受系统 Auto 语言影响；退出状态断言已随本地化后的 `Exited` 文案更新。另一次并发终端启动超时在单独复测与最终串行完整回归中均通过，未删除或忽略该测试。

测试程序：`artifacts/DevTabs-0.4.0-project-overview/devtabs.exe`，16,575,488 bytes，SHA256 `2FE52E4544461FD1B7C93B6D9707711F19304FFBCEFCC6423E0FD9D27DBA68B9`。同目录 `build.json` 记录构建与验证命令。直接运行进入总览，Ctrl+L 切语言；进入工作区后使用 Ctrl+G → U。真机 UI 验收由用户执行。

## 性能口径

Release 基准分别测量首次库查询、固定语言查询、当前 locale 查询及两个命名参数的完整格式化成本。固定语言查询运行五轮，每轮 500,000 次取中位数；当前语言查询 500,000 次，格式化 100,000 次。使用 black_box 避免常量折叠。

资源 JSON 字节数不等于内存占用或二进制增量。Cargo.lock 新增依赖中包含过程宏和构建期解析依赖，不能直接用依赖包总数判断运行时重量。普通静态查询的借用返回值已经逐项验证；格式化会分配字符串。首次查询也有库初始化成本，不能按预热后的纳秒结果宣称没有启动开销。

本机 Release 实测（2026-11-11，364 条中英文资源）：

| 指标 | 结果 |
| --- | ---: |
| 首次中英文库查询合计（含初始化） | 138.7 μs |
| 固定英文查询中位数 | 13.75 ns/次 |
| 固定中文查询中位数 | 13.86 ns/次 |
| 当前语言查询（实际界面调用路径） | 20.48 ns/次 |
| 两个命名参数的完整格式化 | 173.43 ns/次 |
| 对照：直接 Rust `format!` | 47.09 ns/次 |
| 两份资源 JSON 原始字节数 | 40,105 bytes |

按该微基准估算，500 次当前语言查询约 10 μs；这不代表一帧完整 UI 的耗时。命名参数格式化比固定 Rust 格式化慢，但本次测量仍小于 0.2 μs，满足当前菜单、标签和状态文案需求。这里验证的是采用专用库的成本，没有测量并宣称相对已删除自研实现的加速比。资源规模、机器和编译选项变化后应重新测量。

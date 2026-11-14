# 统一设置实施与验证记录

本记录区分已执行验证与计划；忽略和真机未运行均不计入通过。

## 基线

- 日期：2026-11-14；提交：61fecb7c434e54f903284703ded0ebf523b256c9。
- Windows x86_64-pc-windows-msvc；Rust/Cargo 1.98.1；隔离 DEVTABS_HOME，未修改个人配置。
- `cargo test --workspace --all-targets`：库测试 158 通过、3 失败、7 忽略；默认并发运行中主题全局状态相互干扰。失败为 statistics_modal_covers_background_and_restores_it_after_mouse_close、pulse_heatmap_hotspots_and_budget_labels_remain_readable_at_all_widths、tree_view_cache_keeps_selection_scroll_focus_badges_and_hit_map。
- `cargo test --workspace --all-targets -- --test-threads=1`：退出 0，43 个套件，474 通过、0 失败、33 忽略。后续同口径执行完整回归；未移除上述断言。
- 原始日志：本地忽略目录 artifacts/settings-refactor/before 下 tests.log、tests-serial.log、toolchain.txt、commit.txt。

## 阶段 1：模型与所有者

新增 settings::model：稀疏偏好覆盖、显式启动快照、继承来源、路径身份、范围与业务校验。此阶段尚未切换运行时格式，不提供旧格式回退读取。

| 数据 | 唯一持久所有者 | 现有入口及接入阶段 |
| --- | --- | --- |
| 全局/项目启动、外观、默认视图、发现、预设、通知偏好、快捷键 | SettingsStore / config.toml | config、cli、overview/worker、sidebar/preferences、host；阶段 2–4 |
| 便签/书签内容 | UserDataStore / user-data.json | overview/worker、bookmarks；阶段 2 |
| 最近打开、组折叠、已确认终端浮层矩形 | UiStateStore / ui-state.json | host/open、overview、app/popup；阶段 2 |
| 任务、运行身份、准备快照、检查结果、通知任务策略 | 原业务存储 / runtime | tasks、checks、host、notifications；阶段 2 |
| 统计等可重算信息 | cache | 原缓存接口；阶段 2/4 |
| 搜索、当前焦点、临时视图、未保存缓冲、预览覆盖 | 当前前端内存 | overview、sidebar、editor、settings；阶段 2/4 |

明确了临时视图与默认值、预设脏草稿应用、docs/52 与 docs/53 保存权限的边界，见设计 §9。

已运行 `cargo test --lib settings::model -- --test-threads=1`：6 通过。覆盖 C01 继承/显式空值、C02 必填快照与可选标题、C03 快照字段及字面参数的模型层；运行队列、持久化及 picker 交互仍待后续阶段。另覆盖文档 TOML 示例、比例/路径、检查环境变量与超时校验。

完整阶段回归：`cargo test --workspace --all-targets -- --test-threads=1` 退出 0，480 通过、0 失败、33 忽略；日志 artifacts/settings-refactor/stage1/tests.log。`cargo fmt --all -- --check`、`git diff --check` 通过。阶段 1 完成，运行时格式接入属于阶段 2。

阶段 1 提交并推送：48e17b3。

## 阶段 2：统一存储与业务接入

SettingsStore 保留原文、注释和未知项，使用语义身份三方合并与字段错误隔离；保存失败不发布内存状态。UserData 管理便签与书签，UiState 管理最近时间、折叠与浮层。原 Config 仅作为现有总览的内存投影，不再作为文件格式。CLI 用 clap ValueSource 识别显式参数；布尔参数支持显式 false。运行记录位于 runtime，可重算统计位于 cache；项目模板不再自动读取或回写。

准备步骤在 Task/Plan 捕获，检查每次读取；空列表也为确定快照，缺少快照字段报错。预设持久结构只保存完整启动值与续接策略。通知全局偏好与任务策略分离，发送前复核当前偏好，宿主诊断报告检查/采用摘要。

有意变化：总览和文件树就地视图改动不自动保存默认；有额外会话不再隐式启用 Custom；代码默认 Agent 为 Codex；取消项目文件自动覆盖。原测试更新格式/路径/上述行为断言，保留 PTY、租约、重连、字面参数、恢复、文件保护等业务断言。

验证：`cargo test --workspace --all-targets --no-fail-fast -- --test-threads=1`，44 套件，491 通过、0 失败、33 忽略；日志 artifacts/settings-refactor/stage2/tests-confirmed.log。前两轮失败与修正日志同目录保留。补充配置服务/辅助存储单测、书签冲突与 Remap 合并、通知交付策略，以及 tests/unified_configuration.rs 的三项端到端配置验证。提交并推送：9587fbb。

阶段 2 完成配置/存储层的 C01、C02、C04、C05、C07–C10；C03 的运行时快照覆盖，具体会话转策略 UI 待阶段 4。含界面的验收 ID 仍需阶段 4 执行，不能以本轮全套通过代替。阶段 3、4：待实施；Windows 外部热键、输入法与真实 Agent 场景尚未运行。


## 阶段 3：命令索引与输入保真

新增 shortcuts 注册表、精确 chord、最多两步序列、按命令/区域的增删差异与首选显示。默认 F3/Ctrl+F3 并存，Ctrl+F3 首选；不机械新增 Ctrl+Fn。终端默认只占用 Ctrl+G，双按前缀执行 terminal.send_prefix；Ctrl+P/F3 等默认透传。工作区菜单直接执行命令 ID；总览、编辑器、文件树与预览主要快捷命令使用同一索引，控件内部文本/导航/确认保持独占。

原生输入记录包含独立修饰键。账本按 (vk, scan, enhanced) 记录所有者和原会话连接，重复/松开沿原所有者处理；失焦补发已交付 Win32 按下的松开并抑制迟到 release，重连不会继承旧 release。VT 模式记录原 application-cursor 编码。队列接受为交付边界，IPC/连接失败计入未平衡诊断；不能保证已经断开的外部进程收到补发。没有全局钩子/占用探测。

有意变化：按键修饰符精确匹配；子区域停用阻断父绑定；大写/额外 Alt/Shift 不再偶然执行小写命令。Shift+加号/感叹号作为实际键盘布局需要的显式别名登记。独立修饰键不消耗前缀等待。原 ConPTY 测试的 command 帮助函数缺 Enter 松开，现补足真实按下/松开对，保留全部两个 shell、焦点、磁盘输出、中文编辑及未保存保护断言。

键位审计：docs/shortcut-key-audit.tsv 为精确源行白名单，tests/shortcut_audit.rs 遍历所有 src Rust 文件；新增/修改/删除 KeyCode 行均需审查归类，否则测试失败。含测试夹具与协议行，避免新分支躲过扫描。独占控件操作与业务命令仍通过各自明确路径处理，最终管理 UI/动态提示验收属于阶段 4。

验证：隔离 DEVTABS_HOME 下 cargo test --workspace --all-targets --no-fail-fast -- --test-threads=1 退出 0，503 通过、33 忽略、0 失败；日志 artifacts/settings-refactor/stage3-final.log。包括两个真实嵌套 ConPTY 测试、工作区新任务表单、默认透传穷举、子作用域停用、配置错误隔离、独立修饰记录、重复/失焦/重连账本、改键后旧键不执行，以及键位审计。失败调查日志保留；Windows 截图软件/输入法/真实 Agent 的人工环境矩阵仍未运行，不能据此声称外部热键已解决。

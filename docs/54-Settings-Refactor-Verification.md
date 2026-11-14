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

阶段 2、3、4：待实施。C04–C20 尚未验收；Windows 外部热键、输入法与真实 Agent 场景尚未运行。

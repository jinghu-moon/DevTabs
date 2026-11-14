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

阶段 2 完成配置/存储层的 C01、C02、C04、C05、C07–C10；C03 的运行时快照覆盖，具体会话转策略 UI 待阶段 4。含界面的验收 ID 仍需阶段 4 执行，不能以本轮全套通过代替。在该提交时阶段 3、4 待实施；当前进度见后文。Windows 外部热键、输入法与真实 Agent 场景尚未运行。


## 阶段 3：命令索引与输入保真

新增 shortcuts 注册表、精确 chord、最多两步序列、按命令/区域的增删差异与首选显示。默认 F3/Ctrl+F3 并存，Ctrl+F3 首选；不机械新增 Ctrl+Fn。终端默认只占用 Ctrl+G，双按前缀执行 terminal.send_prefix；Ctrl+P/F3 等默认透传。工作区菜单直接执行命令 ID；总览、编辑器、文件树与预览主要快捷命令使用同一索引，控件内部文本/导航/确认保持独占。

原生输入记录包含独立修饰键。账本按 (vk, scan, enhanced) 记录所有者和原会话连接，重复/松开沿原所有者处理；失焦补发已交付 Win32 按下的松开并抑制迟到 release，重连不会继承旧 release。VT 模式记录原 application-cursor 编码。队列接受为交付边界，IPC/连接失败计入未平衡诊断；不能保证已经断开的外部进程收到补发。没有全局钩子/占用探测。

有意变化：按键修饰符精确匹配；子区域停用阻断父绑定；大写/额外 Alt/Shift 不再偶然执行小写命令。Shift+加号/感叹号作为实际键盘布局需要的显式别名登记。独立修饰键不消耗前缀等待。原 ConPTY 测试的 command 帮助函数缺 Enter 松开，现补足真实按下/松开对，保留全部两个 shell、焦点、磁盘输出、中文编辑及未保存保护断言。

键位审计：docs/shortcut-key-audit.tsv 为精确源行白名单，tests/shortcut_audit.rs 遍历所有 src Rust 文件；新增/修改/删除 KeyCode 行均需审查归类，否则测试失败。含测试夹具与协议行，避免新分支躲过扫描。独占控件操作与业务命令仍通过各自明确路径处理，最终管理 UI/动态提示验收属于阶段 4。

验证：隔离 DEVTABS_HOME 下 cargo test --workspace --all-targets --no-fail-fast -- --test-threads=1 退出 0，503 通过、33 忽略、0 失败；日志 artifacts/settings-refactor/stage3-final.log。包括两个真实嵌套 ConPTY 测试、工作区新任务表单、默认透传穷举、子作用域停用、配置错误隔离、独立修饰记录、重复/失焦/重连账本、改键后旧键不执行，以及键位审计。失败调查日志保留；Windows 截图软件/输入法/真实 Agent 的人工环境矩阵仍未运行，不能据此声称外部热键已解决。


阶段 3 提交并推送：ddd8d36。

## 阶段 4：统一设置、管理器与交互交付

本阶段将旧项目表单接入同一个 SettingsUi / Draft，不再由子编辑器单独写配置。13 个分类是设计八组内容的导航拆分，配置所有者没有增加。总览 F4（包括无项目）、命令面板/鼠标菜单、工作区 Ctrl+G → `,` 均进入同一设置系统；Ctrl+G → `*` 直达快捷键。

交付范围：稀疏继承/覆盖、语言主题预览、短选项平铺/长列表弹层、数值精调/百分比滑块、有序字面参数/会话/检查/预设子编辑器、项目选择/后台路径探测、错误定位/三方冲突、全窗口保存/放弃。快捷键页包含作用域、筛选、别名、录制/文本输入、停用/恢复、删除备注、首选、冲突替换、正式/草稿两个路由器的独占测试、异步导入和已提交配置导出。原始物理诊断有界保存八条事件，测试不执行命令也不写 PTY。

保存成功后更新有效索引和未来视图默认；当前编辑缓冲、换行、阅读位置、已运行 PTY 与任务身份保留。预设相同路径多条可分别排队、调序和编辑；具体会话 ID 必须先选择 latest/fresh/picker 再保存。内容归档恢复先预览，冲突默认保留当前，确认时锁内复查基值；即时操作和取消设置草稿相互独立。

补齐的故障保护包括：百分比负数/解析溢出保留输入；已删除父实体只有明确 Keep mine 才重新新增完整实体；便签归档路径归一、重复 JSON/规范路径身份拒绝；缺失配置导出为合法 schema 文档而不创建生效文件；输出路径不能覆盖生效配置、内容、记忆、runtime 或 cache。普通浏览、取消新增条目、主题放弃、导入完成均不会暗中提交。

### 命令和证据分类

自动完整命令（PowerShell，独立数据目录）：

```powershell
$env:DEVTABS_HOME = Join-Path (Get-Location) 'artifacts/settings-refactor/stage4-home'
$env:DEVTABS_SETTINGS_FRAMES = Join-Path (Get-Location) 'artifacts/settings-refactor/stage4-frames-final'
cargo test --workspace --all-targets --no-fail-fast -- --test-threads=1
cargo fmt --all -- --check
git diff --check
```

正式审计不设置 DEVTABS_AUDIT_EXPORT。`tests/shortcut_audit.rs` 审查 629 个精确源码指纹，新增修饰键/别名/限定路径/宏/guard 覆盖；真正 cfg(test) 被结构性排除，名为 tests.rs 的生产模块仍被扫描。`.github/workflows/ci.yml` 在 Windows runner 中执行同一串行完整测试和格式检查，不启用导出绕过。

`tests/shortcut_regression.rs` 自动生成 artifacts/shortcuts 下默认绑定/终端占用清单和 19 个固定场景的前后差异。旧结果来自对 61fecb7 源码的核对，不是本轮重跑旧构建。精确化有意移除多余 Alt/Shift 的偶然命中；F3/Ctrl+F3 并存且后者首选；普通终端 Ctrl+P 透传。菜单、详情和状态栏读取有效绑定，停用命令不再展示旧键；独占表单的导航/确认键有明确控件分类。

首轮完整日志 stage4-final.log 保留 550 通过、1 失败、33 忽略；唯一失败为新增通知测试过早记录 shell_pid=None，后续合法启动变为 Some 被误报。修正为等待 shell 启动后记录 PID，保留“偏好重载不重启 shell”的断言。其他调查日志同目录保留。检查生命周期新增测试最初因无控制台 PowerShell 输出采用 OEM 编码而在中文路径断言失败；最终夹具显式输出 UTF-8，保持精确中文路径断言，生产代码不重写字面 argv、不猜任意程序编码。

第二轮完整日志 stage4-confirmed.log 的唯一失败来自预览菜单测试按旧固定文案查找条目；菜单已经展示有效作用域绑定，现改用稳定命令 ID 选择，保留原文件差异/未保存内容断言，目标 28 项均通过。补强真实宿主恢复测试时，修正夹具的 Git HEAD/运行数据隔离后发现原子替换的长路径缺陷：Rust 文件 API 支持长路径，直接调用 MoveFileExW 却缺少扩展绝对路径；同时临时清理覆盖了 GetLastError。已修正路径和错误码保真，并以超过 260 字符、真实锁定目标的 access/sharing 错误、清理与重试测试覆盖。调查日志 stage4-prepare-restart*.log / stage4-long-path*.log 保留。另修正检查指纹误用文件树目录徽标：现在枚举实际变更/未跟踪文件，忽略目录不作为文件读取；tests/task_checks.rs::revision_hash_reads_nested_untracked_files_and_ignores_ignored_tree_badges 覆盖嵌套文件修改、忽略目录、暂存重命名和未暂存内容。真实宿主恢复用例最终目标通过日志为 stage4-prepare-restart7.log。

补强设置详情前的完整回归：46 套件，560 通过、0 失败、33 忽略，日志 stage4-complete-before-shortcut-details.log。随后补齐快捷键单项修改、重复默认键去重、窄屏详情分页及默认/来源/状态/允许区域/删除备注展示；10 项管理器目标测试通过，包含失败/取消原绑定不丢、显式冲突替换只移除选中旧键、其他别名与首选保留。M / PageUp / PageDown 三个新控件分支逐条归为 widget。

最终完整回归（2026-11-15）：命令退出 0，46 套件，563 通过、0 失败、33 忽略。正式键位审计未设置导出环境变量；忽略项均保留原有条件/原因，不计通过。cargo fmt --all -- --check 与 git diff --check 通过。日志 artifacts/settings-refactor/stage4-complete.log。

### C01–C20 自动验收

下表的“通过”仅表示列明的自动/ConPTY 断言执行成功。`ui::` 指 src/settings/ui/tests.rs；`store::`、`data::`、`model::`、`manager::`、`operations::` 分别指 settings 对应模块的测试。未运行的真实外部环境列在后表，不能用这些自动结果替代。

| ID | 分类 / 结果 | 实际测试入口与结论 |
| --- | --- | --- |
| C01 | 自动 / 通过 | model::inheritance_preserves_false_empty_collections_and_argument_boundaries；ui::inherited_lists_require_override_and_empty_is_distinct_from_inherit。缺失、false、[]、空参数保持不同，标题/颜色范围受校验。 |
| C02 | 自动 / 通过 | model::snapshot_requires_complete_values_and_keeps_launch_capabilities、example_is_valid_and_optional_titles_do_not_inherit_into_snapshots。覆盖可缺失，快照必填值缺失拒绝，快照不继承后续标题。 |
| C03 | 自动 / 通过 | tests/reference_parity.rs::presets_capture_every_launch_option_and_rename_delete_without_losing_order；overview::tests::preset_apply_stages_all_options_without_launch_and_options_can_be_adjusted、saving_concrete_conversations_requires_explicit_policy_and_retains_failed_form；ui::preset_same_project_entries_reorder_and_path_decisions_preserve_or_recapture。保留 frozen 参数/42 行/新实例/新窗口/Profile/策略，同路径条目可独立操作，只入队不启动。策略三项各有键盘和鼠标断言。 |
| C04 | 自动 / 通过 | tests/unified_configuration.rs::plans_freeze_prepare_and_new_plans_capture_current_definitions、frozen_prepare_capacity_failure_precedes_resource_and_command_side_effects；tests/task_checks.rs::initialization_retry_skips_successful_steps_and_does_not_initialize_other_tasks；tests/workspace_windows.rs::host_restart_preserves_empty_and_populated_prepare_snapshots_and_successful_steps。真实宿主结束/重启并恢复两个工作区，原列表和空列表都保持，成功步骤不重跑，新任务采用最新定义。Task/Plan 缺失快照拒绝，超 Catalog 预算在资源/命令副作用前失败。 |
| C05 | 自动 / 通过 | tests/workspace_windows.rs::checks_resolve_per_request_from_original_project_and_keep_running_spec_and_task_cwd；tests/task_checks.rs。原 project、子 checkout、module cwd 均不同；检查相对 checkout，额外会话相对 Task.cwd；旧检查保持原 Spec，下次用新定义，显式 Spec 优先，task/run 不串。 |
| C06 | 自动 / 通过 | data::notes_conflict_ui_events_merge_and_neither_writes_preferences、note_clears_conflict_and_independent_projects_and_whole_rectangles_are_preserved；overview::tests::failed_persistence_restores_form_with_all_entered_values。便签基值冲突和清空都受保护，不同项目合并，失败表单保留。 |
| C07 | 自动 / 通过 | bookmarks::tests::concurrent_writers_merge_without_lost_bookmarks、note_edit_merges_with_remap_but_conflicts_with_other_notes_or_deleted_entries；tests/preview_workspace.rs。备注/Remap 合并，删除/修改冲突，ID 不复活。 |
| C08 | 自动 / 通过 | data::note_clears_conflict_and_independent_projects_and_whole_rectangles_are_preserved、notes_conflict_ui_events_merge_and_neither_writes_preferences；app::popup::tests::cell_geometry_is_bounded_and_moves_without_resizing。最近时间单调、组修改合并、矩形整体替换，窄屏适配与取消几何保护。 |
| C09 | 自动 / 通过 | store::disjoint_fields_merge_preserving_comments_unknowns_and_invalid_values、shortcut_dependency_failure_is_isolated_but_draft_is_atomic、missing_read_does_not_write_and_broken_files_are_preserved；ui::all_drafts_are_atomic_and_invalid_edits_are_recoverable。既存错误段隔离，新脏草稿整批校验，坏语法原文不覆盖。 |
| C10 | 自动 / 通过 | store::concurrent_disjoint_saves_merge_under_a_deterministic_barrier、rename_reorder_and_parent_deletion_require_explicit_conflict_resolution、same_field_and_parent_deletion_conflict_without_writing；ui::conflicts_and_io_failures_keep_all_drafts_and_never_overwrite_source、lock_timeout_and_live_file_replace_failure_preserve_source_and_draft_for_retry。config::tests::atomic_replace_supports_long_paths_and_preserves_the_original_win32_error。同步屏障、同值/不同值/排序/父删除、备份写入故障、真实 Windows 5 秒锁超时和拒绝替换文件句柄，失败原文/草稿不丢，解除故障后重试成功；超过 260 字符路径正常写入与替换，失败错误码保真。 |
| C11 | 自动 / 通过 | ui::dropdown_cancel_does_not_change_theme_and_discard_uses_latest_commit、conflicts_and_io_failures_keep_all_drafts_and_never_overwrite_source；store 的坏文件保护。预览放弃读最新 committed，冲突和坏文件不覆盖原文。 |
| C12 | 自动 / 通过 | tests/workspace_windows.rs::notification_adoption_reports_failure_and_retry_without_rolling_back_preferences；tests/notification_policy.rs、notification_queue.rs、activity_history.rs。采用失败不回滚偏好，保留上次合法值、显式诊断重试，任务静音不写全局偏好，送出前复核策略；测试没有注册或发送系统通知。 |
| C13 | 自动＋真实 ConPTY / 通过 | ui::browsing_every_category_and_f4_placeholder_is_read_only、inherited_lists_require_override_and_empty_is_distinct_from_inherit；app::terminals::tests::settings_capture_paste_keys_mouse_and_release_without_terminal_side_effects；tests/pty_windows.rs::unified_settings_f4_is_reachable_in_an_empty_real_overview、unified_settings_in_real_terminal_save_defaults_and_return_without_input_leaks。空总览 F4、当前项目、子编辑/粘贴、保存/关闭和返回 shell 真实执行。 |
| C14 | 自动渲染/交互 / 通过 | ui::responsive_forms_and_exclusive_popups_keep_controls_reachable、acceptance_frames_include_both_languages_themes_and_narrow_action_menu、disabled_danger_hover_and_primary_states_are_distinct_and_non_mutating、every_numeric_preference_accepts_its_endpoints_and_rejects_outside_and_overflow_input。两种语言、深浅主题、极窄尺寸、边界/禁用/焦点/悬停/错误，禁用不触发且不穿透。 |
| C15 | 自动 / 通过 | data::whole_content_budget_is_independent_and_a_full_file_can_still_be_shrunk、note_archive_paths_normalize_and_duplicate_identities_never_silently_overwrite；bookmarks 的损坏/128 项容量测试；operations::exports_and_cleanup_respect_storage_owners_and_committed_values。16 MiB 精确读边界、超限不写、满文件可缩减，书签与总内容预算独立，清理不删内容/任务。 |
| C16 | 自动 / 通过 | fields::catalogue_covers_real_model_fields_and_preserves_scalar_types；ui::every_numeric_preference_accepts_its_endpoints_and_rejects_outside_and_overflow_input、custom_colors_shell_paths_and_default_layout_are_real_values、preset_same_project_entries_reorder_and_path_decisions_preserve_or_recapture。模型叶字段与组件表一致，数值无损、Profile 正向标签正确反向映射 no_profile；schema 只读。 |
| C17 | 自动 / 通过 | ui::child_cancel_and_ctrl_s_preserve_literal_arguments_and_current_input、environment_errors_keep_input_and_milliseconds_and_program_are_literal、preset_same_project_entries_reorder_and_path_decisions_preserve_or_recapture。空参数、引号、中文空格、环境变量空值/大小写重复、毫秒精度、子编辑完成/取消、保存未完成输入均覆盖。 |
| C18 | 自动 / 通过 | manager::modifying_one_binding_is_atomic_preserves_aliases_and_never_duplicates_defaults、explicit_conflict_replacement_also_removes_only_the_original_selected_alias、narrow_details_scroll_to_defaults_sources_allowed_scopes_and_deletion_notes、aliases_preference_disable_delete_notes_and_restore_are_only_drafts、conflicting_defaults_require_explicit_replacement_and_preserve_other_aliases、test_mode_resolves_sequences_without_executing_and_can_prefer_tested_alias、asynchronous_import_is_only_a_draft_and_export_uses_committed_values；ui 的整批草稿事务。没有第二条管理器落盘路径。 |
| C19 | 自动 / 通过 | ui::stale_path_probe_does_not_overwrite_new_input、project_picker_is_explicit_background_work_and_preserves_cancel；tests/unified_configuration.rs::discovery_scans_only_direct_children_and_excludes_exact_project_identities；sidebar::performance_tests::refreshed_defaults_preserve_the_live_editor_and_apply_to_new_documents。探测核对 path/value，项目扫描仅直接子目录，未知图片/宿主状态保留为未知，新图片协议不修改当前文档选择。 |
| C20 | 自动 / 通过 | ui::immediate_cache_operation_survives_draft_discard、responsive_forms_and_exclusive_popups_keep_controls_reachable；operations::restore_preview_is_read_only_default_keeps_conflicts_and_commit_checks_its_base、corrupt_and_over_capacity_archives_never_change_user_content；tests/agent_hook_install.rs 在临时配置验证 ownership/安装/移除。恢复和清理不影响偏好草稿/任务，窄屏可返回和滚动。 |

### 真实终端、视觉与未运行环境

真实 ConPTY 测试启动本次编译的 devtabs.exe 和无个人 Profile 的 shell，不是 TestBackend 录屏替代：原有双 shell、侧栏源码编辑、输入隔离、未保存保护，以及两条新增设置流程都纳入完整回归。原始物理输入账本的 release/repeat/失焦/重连由单测轨迹与 ConPTY 一起验证；自动测试不能证明截图软件或 Terminal 没有截走事件。

视觉生成包含中文/英文 × terminal/light × 120/48 列 × 主题/项目/操作/快捷键四场景，共 32 个 TestBackend 字符帧，路径 artifacts/settings-refactor/stage4-frames-final。另有 2×2 极窄布局和错误/焦点/禁用断言。人工检查过最终帧的宽屏项目、窄屏操作与快捷键 PNG 合成预览；本地 Pillow 对 Microsoft YaHei 的栅格化异常另用 SimHei 检查，原字符帧没有缺字；这是字符帧的辅助展示，不是 Windows Terminal/IME 真机截图，字体栅格化不作为产品字体结论。

| 项目 | 分类 / 结果 | 边界 |
| --- | --- | --- |
| 外部截图软件 F3、其 Ctrl+F3 组合与切出 DevTabs | 人工 / 未运行 | 没有改个人全局热键，也没有注册/抢占或探测软件名。 |
| Windows Terminal 用户自定义 actions 与真实 Microsoft IME 候选/切换 | 人工 / 未运行 | 自动原生事件/Unicode 测试不替代用户桌面候选框；未改宿主或输入法配置。 |
| C12/C20 的真实通知注册、展示/声音与个人 Hooks 配置 | 人工 / 未运行 | 安装 ownership 用临时文件测试；真实注册/测试通知只提供明确入口，不在个人环境自动执行。 |
| 个人 PowerShell Profile、已安装 Codex/Claude 输入界面 | ignore / 未运行 | 仍保留原有忽略测试，不计通过；真实 Agent 行为不由 fixture 推断。 |
| release-only 性能/大量文件/图片质量比较工作负载 | ignore / 未运行 | 完整回归中的忽略项逐条显示原因；与设置正确性自动测试分开。 |

代码与自动验收完成后按用户授权提交并推送；外部人工环境矩阵保持未运行状态，正式发布前单独完成。没有新增兼容读取、迁移器、真实配置自动搬运或真实全局热键改写。

# DevTabs Sidebar Explorer

日期：2026-11-11。侧边栏功能与交互升级，适用于 Windows TUI。

## 实现范围

文件树负责浏览、选择和文件操作；预览区域负责阅读与编辑；Agent 区域接收路径。搜索、Git、文件管理和配置写入由独立项目线程处理，不占用预览与保存线程。

| 项目 | 实现 |
| --- | --- |
| 全项目查找 | Ctrl+P 弹窗；文件名、相对路径模糊匹配；覆盖未展开目录；显示相对路径 |
| 当前文件定位 | 默认自动跟随；历史、书签、Markdown 文件链接复用同一打开入口；可关闭 |
| Git 状态 | M 修改、A 新增、? 未跟踪、! 忽略、D 删除、R 重命名、U 冲突；目录汇总；仅已修改筛选 |
| 文件管理 | 新建文件/目录、重命名、移至系统回收站；未保存保护 |
| 多选 | Ctrl 点选、Shift 范围选择；批量复制、预览、外部打开、拖入 Agent |
| 顶部与状态 | 项目名称、搜索、更多菜单；隐藏/忽略、筛选、选择数量、Git 状态 |
| 行布局 | 箭头、图标、名称、右侧状态；目录折叠显示闭合文件夹，展开显示打开文件夹；文件名优先，长名称保留扩展名 |
| 视觉区分 | `>` 焦点行、`*` 多选、`[P]` 当前预览；Agent 获得焦点后保留标记 |
| 路径辨认 | 树层级显示目录关系，文件名不追加灰色父目录提示；悬停绝对路径；搜索显示相对路径 |
| 紧凑目录 | 已加载单子目录链压缩为 `src / preview / image`；可关闭 |
| 宽度 | 拖动分隔线；按项目记忆；保留 Agent 最小宽度 |
| 加载与刷新 | 目录行 Loading、错误、重试；刷新保留展开、选择及滚动位置 |
| 鼠标与退出 | 文件单击预览；目录名称选择、箭头展开；右键菜单；Esc 按层退出 |
| 大仓库 | 有界索引、后台任务、可取消搜索、可见区域绘制缓存 |

## 操作

| 操作 | 入口 |
| --- | --- |
| 全项目查找 | Ctrl+P 或树标题 `[P]`；Agent/预览/编辑器焦点下均可使用 |
| 查找弹窗 | 输入过滤；↑↓、PgUp/PgDn 选择；Enter/单击打开；F5 重建索引；Esc 关闭 |
| 清空弹窗输入 | Ctrl+U |
| 已加载节点查找 | 树内 `/`；Ctrl+P 才是全项目查找 |
| 更多菜单 | F2、标题 `[···]`、右键；包含全部新增操作 |
| 多选 | Ctrl+单击切换；Shift+单击选择连续可见行；Shift+↑↓ 扩展 |
| 批量预览 | Enter 或 Preview selected files；最多 8 个文件 |
| 批量复制 | 树内 Ctrl+C 复制绝对路径；菜单支持相对路径；每项一行 |
| 发送给 Agent | 拖动所选文件到 Agent 内容区；菜单 Insert path into Agent；Ctrl+G I |
| 新建文件 / 目录 | 树内 Ctrl+N / Ctrl+Shift+N 或菜单 |
| 重命名 | 树内 F4 或菜单；默认原名称，Ctrl+U 清空 |
| 删除 | Delete 或菜单；弹窗 Enter 确认移至回收站，Esc 取消 |
| 仅已修改 | 树内 G 或菜单；包含未跟踪，排除纯忽略文件 |
| 隐藏 / 忽略 | 树内 `.` / I；H/I 表示显示，h/i 表示隐藏 |
| 调宽 | 拖动分隔线的 Agent 一侧边框，或树右边框的顶部/底部端点；也可 Alt+拖动树右边框 |
| 重试/刷新 | 目录错误行 Enter；F5 刷新树和预览 |
| 退出层级 | 关闭弹窗/取消拖拽，清除多选，再 Esc 返回 Agent |

Windows Terminal 截获快捷键时使用鼠标和 F2。目录不会递归加入批量预览或 Agent 路径，只处理直接选中的文件。

## 状态与文件操作

宽度、自动跟随、紧凑目录存入现有 `config.toml` 的项目配置 `sidebar`。后台读取，修改后延迟 500 ms 写入，沿用配置锁与原子写入。

重命名不覆盖目标：Windows 使用无替换语义的 MoveFileW，新文件使用 create_new，新目录使用 create_dir。拒绝分隔符、控制字符、Windows 保留名称、末尾点/空格。项目根目录和 `.git` 受保护。逐段检查重解析点及项目边界，不穿越链接/目录联接。

影响当前未保存文档的重命名和删除复用 Save / Discard / Cancel；保存失败保留缓冲区。文件操作执行期间阻止继续操作和编辑。批量删除先校验路径，再逐项调用系统回收站；中途失败时更新已经成功的项目并报告错误。

重命名目录同步标签、阅读历史和持久书签；相关干净快照重新读取。删除关闭受影响标签并移除相关历史、书签。书签同步失败独立报告，已完成的磁盘操作不会伪装成未执行。

Agent 路径先在后台整批验证，全部通过才组合为一次粘贴，沿用输入链路回执。路径双引号包围，不发送 Enter。最多 128 项、32 KiB；任意文件消失则整批不发送。链路接受不代表 Agent 已读取文件。

## 搜索与性能边界

- 复用 nucleo-matcher 0.3.1、trash 5.3.0 和已有 ignore。
- 输入防抖 120 ms；按代次取消旧任务、丢弃过期结果。输入过程复用索引；重新打开弹窗重建索引，覆盖未展开目录内的新增文件。
- 遵循当前忽略开关、`.gitignore`、`.ignore` 和 Windows 隐藏属性；始终跳过 `.git` 内容，不跟随目录链接。
- 索引最多 100,000 个条目、深度 64；路径标签最多 4096 字节；显示最佳 200 项与总匹配数。达到边界或跳过不可访问目录时提示。
- Git 通过系统 git porcelain v1 NUL 输出读取，支持仓库子目录；禁用可选锁。活跃项目每 3 秒检查，文件变化后提前刷新，非仓库不阻止浏览。
- 每个 Git 子进程限时 5 秒、输出限 8 MiB。忽略/未跟踪目录的继承与普通目录汇总分开，避免污染干净文件。
- 树维持 10,000 节点限制；刷新请求分批提交，保留已加载节点与展开/滚动状态。
- 结构、宽度、Git、主题、预览标记变化才重建树投影；多选星号只叠绘可见行，选择变化不重建标签；Agent 输出更新只复制缓存的可见单元。
- 文件树不再构建同名文件集合或追加父目录文字，原预留宽度全部供文件名使用。`wide_tree_gives_duplicate_names_full_width_and_resize_escape_restores_width` 验证两份同名文件均无灰色后缀，文件名使用可用宽度；原有悬停路径、搜索相对路径和预览标签的路径辨认保持各自行为。
- 文件夹图标跟随 `TreeState::opened`：折叠使用 Nerd Font `U+F114`，展开使用 `U+F115`，与展开箭头状态一致。仅在可见视口缓存重建时替换目录图标，不因展开状态变化另行重建全部标签；根目录、合并目录均适用，关闭图标选项时沿用纯文本。回归 `folder_icons_follow_expansion_without_rebuilding_labels` 覆盖展开／折叠、合并目录、根目录、缓存重复绘制和主题切换。修改前闭合图标断言失败，见 `artifacts/folder-icons-before.log`；修改后相关 4 个测试套件共 113 项通过、0 失败、2 项默认忽略，见 `artifacts/folder-icons-regressions.log`。
- 新增聚合统计 `project.index`、`project.fuzzy_search`、`project.git_status`、`project.file_operation`，沿用 `--profile`，不记录路径和输入内容。

## 验证

升级前 SVG 阶段基线：205 通过、0 失败、19 忽略，日志 `artifacts/svg-full.log`。

新增 `tests/explorer_windows.rs`：真实 Git 临时仓库、真实回收站恢复、后台检索、TestBackend 事件、多选、未保存文件的保存/放弃/取消、目录重命名后标签/历史/书签同步、宽度记忆、刷新位置、目录压缩。配置写入使用独立子进程与临时 DEVTABS_HOME；回收站只操作唯一生成文件，恢复后清理。

原滚动条测试显式从首行开始，避免自动跟随改变初始位置后影响其原本测试目标。

```powershell
cargo fmt --all --check
cargo clippy --workspace --locked --all-targets -- -D warnings
cargo test --workspace --locked --all-targets
cargo test --release --locked --test explorer_windows project_search_benchmark -- --ignored --nocapture
cargo build --release --locked
```

性能对照：同一 4000 文件数据集、同一查询与结果，比较逐次索引和复用索引。数据写入 `artifacts/explorer-search-benchmark.json`。此测量只衡量索引复用，不代表终端刷新或 Agent 性能。

本次结果：224 通过、0 失败、20 默认忽略（根包全部目标和 Mermaid 包全部目标分别执行，合计包含新增 19 项通过；另单独执行并通过 Release 基准）。fmt、工作区 Clippy 均通过。回归日志为 `artifacts/explorer-full.log`，静态检查日志为 `artifacts/explorer-clippy.log`。早期一轮 PTY 保存测试发生超时，单独复查及最终完整回归均通过；自动测试无法替代实际 Windows Terminal 的输入与显示验收。

| 搜索场景 | 中位耗时 |
| --- | ---: |
| 每次重新索引并查找 | 10.1245 ms |
| 复用索引查找 | 0.3592 ms |
| 首次索引并查找 | 11.6725 ms（单次） |

重复查询约快 28.2 倍；全部返回 4000 个总匹配、前 200 个显示结果。选择标记测试还验证了 40 次方向键移动及 Ctrl 切换标记时，`tree.build_items` 次数保持不变，且星号正确显示/清除。

真机由用户验收：标题搜索/菜单、窄窗口、目录箭头、多选拖入 Agent、滚动条与调宽命中范围、未保存提示、Git 标记及重启后的设置。自动验证不启动交互终端。

独立构建路径：`artifacts/DevTabs-0.4.0-sidebar-explorer/devtabs.exe`。

Release 构建与产物 `--help` 检查通过；大小 15,642,624 字节。SHA-256：`E4E876EEE4C97EC90A50CBBB59235CD4CFD84089BCF3FAE21FB67F3E8E6EB857`，同目录 `build.json` 保存校验信息。

```powershell
.\artifacts\DevTabs-0.4.0-sidebar-explorer\devtabs.exe --project . --agent codex --layout split
```

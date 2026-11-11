# 后台宿主按构建隔离

## 问题

2026-11-11 启动 `DevTabs-0.4.0-folder-icons` 报配置错误：`unknown field agent_ratio, expected width, follow, compact`。该构建本身已包含 `agent_ratio`，用它执行只读 `config show` 能正常读取用户原始配置。进程检查发现此前的 `DevTabs-0.4.0-inline-terminal-panels/devtabs.exe __serve` 仍在运行。

原 IPC 名称仅包含 `DevTabs-v4`、数据目录和 `DEVTABS_INSTANCE`，所有 0.4.0 开发构建共享一个后台宿主。新前端的 `ensure()` 收到旧宿主 Ping 后直接复用，随后 `Open` 要更新项目最近打开时间，旧宿主在解析含新字段的配置时失败。`--new-instance` 仅创建独立工作区，无法隔离宿主版本。

配置内容有效；删除 `agent_ratio` 会丢失布局偏好，不能解决后续升级仍连接旧程序的问题。本次保留配置和正在运行的会话。

## 实现

- 根目录 `build.rs` 在编译时对源码、工作区模块、vendored 依赖、Cargo.toml、Cargo.lock、构建脚本本身，以及目标、Profile、Feature、Rust flags 和编译器身份生成 16 位构建标识。无需新依赖，不在帧循环或 RPC 时扫描文件。该标识用于本机开发版本区分，不作为安全校验。
- IPC 使用 `DevTabs-v5-{build_id}-{data_and_instance_hash}`。不同构建使用各自宿主，同一构建的复制件、同项目重连仍复用现有宿主。
- `--version` 显示包版本、构建标识和 IPC 版本，便于确认实际程序。
- 所有新版构建对同一数据目录使用一个 `ConfigLock`，锁名称只由数据目录决定，不含构建标识或宿主实例。共享配置的并发写入仍串行执行。已发布的旧程序不能被此次修复改写，其锁和配置语义不迁移。
- 保留配置的严格字段校验，避免拼写错误被静默吞掉；旧设置缺失 `agent_ratio` 时默认 5000，新增设置保存和重新读取时保留原树宽度及比例。

新版不会停止旧宿主或接管其 Agent／Debug 进程。跨构建的会话列表和共享浮层目标各自独立；旧会话需要通过原程序继续操作。此选择符合开发期无需兼容旧进程的要求。

## 验证

`tests/workspace_windows.rs::legacy_host_cannot_receive_new_build_requests_or_reject_new_sidebar_settings` 在临时数据目录启动精确旧 v4 名称的监听器：Ping 成功，Open 返回相同的 `agent_ratio` 解析错误。修复前启动测试真实失败，见 `artifacts/host-build-before.log`。

修复后验证新工作区正常创建，旧监听器收到的请求数为 0，配置中的 `width = 46`、`agent_ratio = 6250` 被保留，最近打开时间正常更新；把同一构建复制为另一个路径仍看到相同工作区。该测试只启动隔离 PowerShell，无真实 WT 或已安装 Agent。

单元回归同时验证构建、数据目录和实例的隔离关系，以及默认配置、新字段往返与错误拼写拒绝。既有真实宿主生命周期、重连、会话身份、Float 布局、拖动及配置并发保存测试保留。

用户原配置仅使用只读 `config show` 验证，不修改、重置或迁移。新版独立产物和完整检查记录见 `artifacts/DevTabs-0.4.0-host-build-isolation/build.json`。

完整工作区自动测试 **301 项通过、0 失败、22 项默认忽略，26 个测试套件**。Clippy `-D warnings` 与格式检查通过；具体日志为 `artifacts/host-build-all.log`、`artifacts/host-build-clippy.log` 和 `artifacts/host-build-fmt.log`。

```powershell
.\artifacts\DevTabs-0.4.0-host-build-isolation\devtabs.exe --version
.\artifacts\DevTabs-0.4.0-host-build-isolation\devtabs.exe --project . --agent codex --layout float --new-instance
```

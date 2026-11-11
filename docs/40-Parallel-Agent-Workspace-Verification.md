# 并行 Agent 工作区实施记录

本记录关联 [设计](37-Parallel-Agent-Workspace-Design.md) 和 [TDD 任务](39-Parallel-Agent-Workspace-TDD-Implementation.md)。自动化不启动真实 Agent/WT 或发送系统通知；真机项单独记录。

## 阶段 0：基线和测试地基

2026-11-12 完成 T00–T02。升级前完整 workspace 360 通过、0 失败、27 忽略；升级前 Clippy 和 fmt 通过。保留旧项目脉搏可执行文件及本次源码 SHA-256 清单，输出在本机 `artifacts/parallel-workspace/stage-0/`，不提交构建产物。

RED：`parallel_foundations` 中超预算接收与旧 generation 覆盖两个行为断言实际失败；GREEN：3 通过、0 失败、1 子进程夹具忽略。截止时间测试确认清理自己的子进程不影响其他 fixture；配额在更新记账前拒绝，revision 只接受相同 generation 的递增结果。重构后 Clippy/fmt 通过，未改现有用户运行配置。

当前 Git 基线包含此前图片/SVG、侧栏、总览/项目脉搏等已在工作树中的实现及必要 vendor patch；不包含散落根目录的个人测试图片、参考 PowerShell 脚本和原始参考/访谈。它们没有被删除。新能力阶段 A–E 和用户真机验收尚待实施。

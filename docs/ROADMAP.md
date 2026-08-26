# repo-tui 实现路线图

- 文档状态：Active
- 设计依据：[DESIGN.md](./DESIGN.md)
- 当前版本目标：`0.3.0` M2 + M3
- 状态更新时间：2026-08-25

## 1. 路线图原则

1. 先建立可运行的纵向切片，再扩展操作宽度；每个里程碑都必须能独立演示和验证。
2. Git 与 Repo 系统 CLI 是行为真相，解析机器可读输出，不复制其完整语义。
3. UI 线程不执行阻塞命令；所有扫描、历史加载和写操作通过任务层运行。
4. 先保证状态准确和终端可恢复，再增加写操作和跨仓库能力。
5. 批量操作按项目报告结果，不提供虚假的跨仓库事务语义。
6. 危险能力必须在通用执行、安全和测试机制完成后进入产品。

## 2. 状态与优先级

状态：

- `Planned`：已定义，尚未开始。
- `In progress`：当前正在实现。
- `Done`：代码、测试和验收项均已完成。
- `Blocked`：存在明确外部阻塞，需要记录解除条件。
- `Deferred`：不属于当前发布目标。

优先级：

- `P0`：里程碑不可缺少，失败会阻止发布。
- `P1`：核心体验，允许在不影响数据安全时分批交付。
- `P2`：增强能力。

## 3. 依赖关系

```text
M0 工程基础
 └─ M1 只读纵向切片
     ├─ M2 单仓库变更与提交
     │   └─ M3 分支整合与远端
     ├─ M4 Repo 批量工作流
     └─ M5 命令面板与终端接管
          └─ M6 高级 Git 与扩展
M2 + M3 + M4 + M5
 └─ M7 性能、兼容与发布
```

M1 是所有后续里程碑的共同数据与交互基础。M4 可以在 M2 后半段并行开发，但跨仓库写操作必须复用 M2 的风险确认和实时前置检查。

## 4. 当前交付范围

当前实现已完成 `M0 + M1 + M2 + M3`：

- 从任意子目录识别 Repo 工作区或单 Git 仓库，并并发扫描状态。
- Workspace、完整 all-refs Graph、Changes 和 Repository 管理页面均已可用。
- Graph 支持两级上下文操作：选中节点后选择 commit/HEAD/本地分支/远端分支/tag/stash，再选择固定动作或 typed form。
- Graph 内可发现并执行 commit/amend、stash 创建、branch/tag 创建、merge/rebase/cherry-pick/revert 和 stash 操作；本地与远端分支动作集合明确隔离。
- 文件、hunk 与 changed-line stage/unstage/discard，commit/amend/sign-off/signing 和 stash 全流程。
- merge/rebase/cherry-pick/revert 冲突状态、ours/theirs/mark-resolved 与合法 continue/skip/abort。
- branch/tag/remotes 管理，以及 fetch/pull/push/upstream/prune 和 `--force-with-lease`。
- 所有写操作复用 project lock、index lock、实时 snapshot/token 前置检查、确认和 generation 结果隔离。
- Graph overlay、confirmation 和结果状态已覆盖 80x24 与 120x40 TestBackend 渲染。

下一执行点是 M4 Repo 批量工作流；外部 editor/mergetool 和任意受控命令的 PTY takeover 属于 M5。

## 5. M0：工程基础

- 状态：`Done`
- 优先级：`P0`
- 目标版本：`0.1.0`

### M0.1 工程与 CLI

交付物：

- Rust package、锁文件和最小依赖集合。
- `repo-tui [PATH]` 启动 TUI。
- `repo-tui doctor [PATH]` 输出 Git、Repo、终端和工作区诊断。
- `--scan-concurrency`、`--log-file` 等基础参数留出稳定接口。

验收：

```bash
cargo build
cargo run -- --help
cargo run -- doctor .
```

### M0.2 终端生命周期

交付物：

- Crossterm raw mode、alternate screen 和光标恢复 RAII。
- 正常退出和错误路径都恢复终端。
- 输入、resize 和 tick 事件进入统一循环。

验收：

- `q`/`Esc` 正常退出，shell echo 和光标恢复。
- 终端 resize 后 UI 重新布局且不 panic。

### M0.3 分层与任务边界

交付物：

- `domain` 不依赖 UI。
- `git`/`repo` adapter 负责命令和解析。
- `services` 负责发现、扫描和 graph 加载。
- `app` reducer 持有 UI 状态；worker 只通过消息返回结果。

验收：

- parser 可用 fixture 独立测试。
- Ratatui widget 不直接调用外部命令。

## 6. M1：只读纵向切片

- 状态：`Done`
- 优先级：`P0`
- 目标版本：`0.1.0`
- 依赖：M0

### M1.1 工作区发现

交付物：

- 向上寻找 `.repo` 并确定 Repo root。
- 能力允许时通过 `repo list` 获取当前 manifest projects。
- 解析 project path/name；缺失路径保留为错误项目。
- 非 Repo 场景通过 `git rev-parse --show-toplevel` 打开单仓库。

验收：

- 从 Repo 子目录启动仍定位到 client root。
- 从 Git 子目录启动仍只创建一个稳定 project。
- 普通非仓库目录返回可读错误，不进入损坏终端状态。

### M1.2 Git 状态协议

交付物：

- 解析 `git status --porcelain=v2 --branch -z`。
- 支持 branch、detached、unborn、upstream、ahead/behind。
- 聚合 staged、unstaged、untracked、conflicted。
- 命令失败和解析失败明确呈现，不误报 clean。

验收：

- fixture 覆盖 clean、dirty、rename、untracked、conflict、detached 和 unborn。
- 包含空格和非 ASCII 路径的记录不会破坏解析边界。

### M1.3 并发扫描与状态刷新

交付物：

- 有界 Tokio worker 并发扫描不同 project。
- 每次刷新使用 generation；过期结果不覆盖新状态。
- UI 在扫描时保持可输入和可绘制。
- 支持手动刷新；完成后更新聚合统计。

验收：

- 任一项目失败不阻止其他项目更新。
- 连续刷新时最终 UI 只展示最新 generation。
- 并发上限可配置且至少为 1。

### M1.4 Workspace 页面

交付物：

- 表格展示状态、project/path、HEAD、upstream 和错误。
- Summary 展示总数、dirty、conflict、ahead、behind、error。
- `j/k`、方向键、`g/G` 导航。
- `/` 搜索 project/path/branch，`Esc` 清空搜索或退出。
- `r` 刷新，`Enter` 打开选中仓库，`?` 显示帮助。
- 宽屏显示检查器，窄屏保持主列表可用。

验收：

- selection 绑定 project identity，过滤后不会索引越界。
- 80x24 与 120x40 TestBackend 渲染不 panic。
- 扫描中、空列表和错误状态都有明确界面。

### M1.5 Commit graph 页面

交付物：

- 使用显式 NUL 字段协议读取 `git log --topo-order --all` 的完整可达历史。
- 独立解析本地分支、远端分支、annotated/lightweight tag、HEAD 和每条 stash reflog entry。
- 展示不限四路的多色 topology lane、OID、refs、subject、author 和时间。
- 分支从共同祖先分出、merge parent 展开以及重新汇入共同祖先时，显示方向明确的连接线。
- HEAD/local/remote/tag/stash 使用独立颜色 badge，右侧检查器保持相同语义配色。
- 空仓库、unborn HEAD 和 log 错误可恢复。

验收：

- parser 覆盖普通 commit、merge commit、全部 ref 类型、stash 和含换行 message。
- topology fixture 覆盖无 merge commit 的分支分叉、双亲 merge、octopus merge 和多 lane 连接。
- 进入/返回不会丢失 Workspace selection 和搜索。
- graph 加载不阻塞 event loop。

### M1.6 测试与文档

交付物：

- 单元测试、临时真实 Git 仓库集成测试、TestBackend UI 测试。
- README 包含安装、运行、按键和当前范围。
- 路线图状态按实际结果回填。

发布门禁：

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo run -- doctor .
```

## 7. M2：单仓库变更与提交

- 状态：`Done`
- 优先级：`P0`
- 目标版本：`0.2.0`
- 依赖：M1

范围：

- Changes 文件树和 diff 检查器。
- 文件、hunk、changed-line stage/unstage/discard。
- commit/amend/signoff/signing，hook 输出和 message 恢复。
- stash list/show/push/apply/pop/drop。
- operation state、冲突列表、ours/theirs/mark-resolved 和 continue/skip/abort。

完成证据：

- File/Hunk/Line 三态导航和高亮，逐行 patch 从锁内当前 diff 重建。
- `OperationSpec`、风险等级、per-project 写锁、worktree-aware index lock 检查。
- diff token、hunk/line fingerprint 和 `git apply --check --unidiff-zero` 拒绝陈旧目标。
- destructive file/hunk/line discard 确认；失败保留选择与错误，成功自动刷新。
- commit hook 失败保留 message/选项；stash 和 conflict 操作均有真实临时仓库测试。
- merge conflict 下 ours/theirs、mark-resolved、continue/abort，以及 cherry-pick skip 已验证。

关键基础设施：

- 通用 `OperationSpec`、风险等级、预览和实时前置检查。
- 同 project 写锁、执行审计和完成后刷新。
- path 原始字节处理及 NUL 协议。

验收：

- edit-to-commit 流程不离开 TUI。
- destructive 操作不会跳过确认或使用旧快照执行。
- hook 失败不会丢失提交消息。

## 8. M3：分支整合与远端

- 状态：`Done`
- 优先级：`P0`
- 目标版本：`0.3.0`
- 依赖：M2

范围：

- Repository 页 Status、Stashes、Branches & Tags、Remotes 四个 tab。
- branch/tag create/switch/rename/delete，merge/rebase/cherry-pick/revert。
- fetch、pull/rebase、push、set upstream、prune 和 remote add/set-url/remove。
- conflict resolver 与合法 continue/skip/abort。
- remote-write 精确预览和 `force-with-lease`；不提供裸 `--force`。

验收证据：

- 真实临时仓库完成 branch/tag、merge/rebase/cherry-pick/revert 和 operation control 矩阵。
- workspace-local seed/client/bare remote 完成 fetch/pull/push/upstream/prune 与 remote 管理闭环。
- remote write 显示准确 `branch:branch` refspec、OID range、upstream 和 lease 状态，URL userinfo 脱敏。
- repository snapshot token 在锁内拒绝确认后发生的 refs/stash/conflict/remote 状态变化。
- 外部 mergetool/editor 依赖 M5 PTY takeover，明确不在 M3 后台任务中启动。

## 9. M4：Repo 批量工作流

- 状态：`Planned`
- 优先级：`P0`
- 目标版本：`0.4.0`
- 依赖：M1、M2 安全执行层

范围：

- 项目多选和命名过滤视图。
- `repo sync/start/checkout/abandon/prune/rebase`。
- `repo upload/download` 和 manifest 导出。
- workspace exclusive lock、逐项目进度和结果。
- 取消、部分失败、只重试失败项。

验收：

- 每次批量写操作显示目标项目和参数。
- 取消后不宣称回滚，实际状态通过复扫确认。
- Repo 输出无法结构化时仍保存日志并给出最终逐项目状态。

## 10. M5：命令面板与终端接管

- 状态：`Planned`
- 优先级：`P1`
- 目标版本：`0.5.0`
- 依赖：M1；写命令依赖 M2

范围：

- 页面、project、ref 和领域动作 fuzzy palette。
- `git`/`repo` argv 命令执行，不经 shell 拼接。
- Capture 和 PTY takeover 两种模式。
- `$EDITOR`、认证、interactive rebase、difftool/mergetool。
- 敏感参数脱敏和非敏感历史。

验收：

- 外部程序退出、失败和信号路径均恢复终端。
- 命令历史不保存 token/password/credential。
- takeover 返回后按作用域刷新真实状态。

## 11. M6：高级 Git 与扩展

- 状态：`Planned`
- 优先级：`P1/P2`
- 目标版本：`0.6.x`
- 依赖：M3、M5

范围：

- reflog、bisect、blame、range-diff。
- worktree、submodule、sparse-checkout。
- format-patch/apply/am。
- maintenance/gc/fsck。
- Git LFS 和配置化外部动作。
- 保存视图、主题和快捷键 preset。

验收按功能独立定义；任何扩展都必须复用统一任务、安全和日志机制。

## 12. M7：性能、兼容与发布

- 状态：`Planned`
- 优先级：`P0`
- 目标版本：`1.0.0`
- 依赖：M2、M3、M4、M5

范围：

- 1,000 project 和大型 commit history benchmark。
- 可见行优先、缓存上限、任务日志 ring buffer、可选 watcher。
- Linux/macOS、主流终端、16/256/true color、Unicode/ASCII 矩阵。
- parser fuzz、PTY 信号测试和安全审查。
- 安装包、man page、completion、配置迁移和发布文档。

1.0 门禁：

- 设计文档 MVP 验收项全部通过。
- 无已知 P0 数据损坏或终端恢复问题。
- 支持版本与性能基准有可重复证据。

## 13. 代码演进约束

- 不在 widget 中调用 `std::process::Command`。
- 不解析本地化的人类输出作为核心状态。
- 不使用 shell 字符串拼接路径、ref 或用户输入。
- 不将扫描错误转换为 clean。
- 不以列表行号作为 project identity。
- 不让旧 generation 结果覆盖新状态。
- 不在安全层完成前加入临时写命令。
- 不为了抽象而提前拆分多 crate；单 package 达到明确编译或 ownership 压力后再拆。

## 14. 当前进度

| 工作项 | 状态 | 验证 |
| --- | --- | --- |
| 设计文档 | Done | `docs/DESIGN.md` 已完成静态检查 |
| 实现路线图 | Done | M0-M7 依赖、交付物和验收门禁已定义 |
| M0 工程与 CLI | Done | `cargo build`、`--help`、`doctor` 通过 |
| M0 终端与事件循环 | Done | TestBackend + 真实 PTY 退出恢复通过 |
| M1 工作区发现 | Done | 单 Git + fake Repo client 端到端通过 |
| M1 状态扫描 | Done | porcelain fixture + 真实 Git + 缺失项目隔离通过 |
| M1 Workspace UI | Done | 80x24/120x40 渲染和真实导航通过 |
| M2 Changes/commit | Done | file/hunk/line、hook failure、stale/index lock 和真实 Git 通过 |
| M2 stash/conflict | Done | stash 全流程、ours/theirs/resolved、continue/skip/abort 通过 |
| M3 refs/integration | Done | branch/tag、merge/rebase/cherry-pick/revert 真实矩阵通过 |
| M3 remotes | Done | bare remote fetch/pull/push/upstream/prune 与 remote 管理通过 |
| M4-M7 | Planned | 下一步从 M4 Repo 批量工作流开始 |

M0-M3 最终验证（Rust 1.98、Git 2.43.0、Repo launcher 2.54）：

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
./target/debug/repo-tui doctor /path/to/git-or-repo-workspace
```

已验证边界：

- Graph 加载所有本地分支、远端分支、tag、HEAD 和每条 stash 的完整可达历史；大型仓库流式虚拟化仍是后续优化。
- Changes、commit、stash、冲突、refs/integration 和 remotes 均使用统一锁、snapshot/token、确认和 generation 机制。
- 当前无任务中心、Repo sync/upload、命令面板或 PTY takeover；外部 editor/mergetool 明确依赖 M5。
- Repo 端到端测试使用本地 fake `repo list/version` 和真实 Git project，不依赖公网；大型 Repo client 性能基准属于 M7。

## 15. 下一执行点

1. 实现 M4 项目多选、workspace exclusive lock 和 Repo 批量任务模型。
2. 实现 `repo sync/start/checkout/abandon/prune/rebase` 的逐项目进度、取消和失败重试。
3. 为大型 all-refs graph 设计保持全局拓扑和 ref 可见性的流式虚拟化。

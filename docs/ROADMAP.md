# trepo 实现路线图

- 文档状态：Active
- 设计依据：[DESIGN.md](./DESIGN.md)
- 当前版本目标：`0.4.0` M4 + 跨平台发布/更新
- 状态更新时间：2026-08-31

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

当前实现已完成 `M0 + M1 + M2 + M3 + M4`：

- 从任意子目录识别 Repo 工作区或单 Git 仓库，并并发扫描状态。
- Workspace、完整 all-refs Graph、Changes 和 Repository 管理页面均已可用。
- Workspace 支持稳定 ProjectId 多选、命名搜索；`d` 独立切换全部、仅改动、改动仓库及文件三种范围，`t` 独立切换每种范围的列表/树形布局并分别记忆。宽屏 Inspector 和 `S`/`Z`/`D` 冻结仓库 Stage/Stash/Discard 保持可用。
- Graph 支持 commit/HEAD/local branch/remote branch/tag/stash 两级上下文操作及 typed form；Subject 按显示列宽多行渲染，Inspector 保留 body 原始换行，本地分支直接提供普通 Push 与 Force push with lease。
- Changes 支持文件多选批量 Stage/Unstage、selected-path Stash 和完整 Discard；文件/hunk/changed-line、commit/stash/conflict、refs/integration 和 remotes 写操作受锁、token 和 generation 保护。
- Repo `sync/start/checkout/abandon/prune/rebase/upload/download` 和 pinned manifest export 具有 workspace lock、逐项目结果、流式日志、取消后复扫与失败重试。
- Graph、Changes、Workspace Git 与 Repo overlay、confirmation 和结果状态均覆盖 80x24 与 120x40 TestBackend 渲染；四个主页面的数据行选中态另有 cell 前景、背景和粗体断言。
- UI 默认英文，`-zh`/`--zh` 与 `-en`/`--en` 以实例级语言状态覆盖主要页面；长路径、diff 和外部文本按终端列宽安全处理，控制字符不能污染终端布局。选中行使用暗蓝灰色 `#262e3a` 背景并保留原有文本前景色，状态仍由颜色和字符或符号共同表达。
- GitHub Actions 已支持 `v<semver>` tag 自动校验 Cargo 版本，构建 Linux x86_64、macOS Intel/Apple Silicon 的 tar.gz 与 Windows x86_64 MSVC zip，生成 `SHA256SUMS`，并发布含一键安装、自更新、compare 链接和提交列表的 Release。
- `install.sh`、`install.ps1` 和 `trepo update [--check]` 共用版本化资产与 SHA-256 信任链；Windows 更新由 self-replace 辅助进程处理运行中 exe。

下一执行点是 M5 命令面板与终端接管；交互认证、外部 editor/mergetool 和任意受控命令不在 M4 后台任务中启动。

## 5. M0：工程基础

- 状态：`Done`
- 优先级：`P0`
- 目标版本：`0.1.0`

### M0.1 工程与 CLI

交付物：

- Rust package、锁文件和最小依赖集合。
- `trepo [PATH]` 启动 TUI。
- `trepo doctor [PATH]` 输出 Git、Repo、终端和工作区诊断。
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
- `/` 搜索 project/path/branch，`Esc` 清空搜索或退出；`d` 按“全部 → 仅改动 → 改动与文件”循环数据范围，两个改动范围与搜索按 AND 组合。
- `t` 切换当前范围的 List/Tree，三个范围分别记忆布局；前两个范围的 Tree 展示仓库相对路径树，第三个范围的 Tree/List 展示文件目录树/完整路径列表。
- `r` 刷新，`Enter` 打开选中仓库，`?` 显示帮助。
- 仓库树目录行和文件行不可独立选择；`j/k`、Enter、Space、Workspace Git 和 Repo 批量操作始终解析到稳定 `ProjectId`。宽屏 Inspector 继续显示完整仓库详情。

验收：

- selection 绑定 project identity，搜索、范围/布局切换和扫描增量后不会索引越界或跳到错误项目。
- 80x24 与 120x40 TestBackend 渲染不 panic；六种范围/布局组合均可见，仓库树目录行不可选择，改动文件可显示目录树或完整路径列表。
- 扫描中、空列表和错误状态都有明确界面。

### M1.5 Commit graph 页面

交付物：

- 使用显式 NUL 字段协议读取纯 `git log --topo-order --all` 的完整可达历史，禁止再叠加 date-order 打散平行开发线。
- 独立解析本地分支、远端分支、annotated/lightweight tag、HEAD 和每条 stash reflog entry。
- 展示多色 pipe topology、OID、按 Subject 列显示宽度换行的 subject 和响应式 metadata；可变高度 viewport 保持选中 OID 可见，Graph/Subject/重要 refs 优先于 Date、Author、Age。
- 主列表完整优先展示 HEAD/local/stash，remote/tag 使用有界 badge 和 `R:+N`/`T:+N`；Inspector 与对象菜单保留全部 refs。
- `graph_layout` 将 Direct/Indirect/Missing edge、Starts/Continues/Terminates pipe 与 Ratatui 渲染分离，支持 continuing lane 向左压缩。
- lane 超过紧凑模式上限时显示 `~N`，missing parent 使用 `◉`，不静默裁剪或伪造直接连接。
- Graph 支持 Branch、Query、Author、Since、Until 组合过滤；过滤保留完整 all-refs topology DAG 和稳定 commit OID。
- 空仓库、unborn HEAD 和 log 错误可恢复。

验收：

- parser 覆盖普通 commit、merge commit、全部 ref 类型、stash 和含换行 message。
- topology fixture 覆盖无 merge commit 的分叉、双亲/octopus merge、多 lane、continuing lane 左移、missing parent 和 lane cap。
- 真实 Git 日期交错双分支 fixture 证明加载顺序等于纯 topo-order 且不同于 date-order。
- Graph 加载不阻塞 event loop；进入/返回不丢失 Workspace selection 和搜索。
- 过滤语义覆盖分支可达闭包、文本/作者/日期 AND、无效日期、稳定选择和零匹配安全状态。
- 80x24 与 120x40 TestBackend 覆盖响应式列、Subject 多行、可变行高 viewport、refs 摘要、保留 body 空行的完整 Inspector、过滤和零匹配详情。

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

- Workspace Inspector 与 Changes 共享展开式文件树，目录连接符不改变稳定文件身份。
- Changes 文件多选和批量 Stage/Unstage、selected-path Stash、完整 Discard；文件、hunk、changed-line 单目标 Stage/Unstage/Discard。
- 多行 commit/amend 编辑器支持光标导航、当前位置输入与粘贴、signoff/signing、hook 输出和 message/cursor 恢复。
- stash list/show/push/apply/pop/branch/drop/clear；push 支持 include-untracked、keep-index、staged-only，apply/pop 支持恢复 index。
- operation state、冲突列表、ours/theirs/mark-resolved 和 continue/skip/abort。

完成证据：

- File/Hunk/Line 三态导航和高亮，逐行 patch 从锁内当前 diff 重建。
- `OperationSpec`、风险等级、per-project 写锁、worktree-aware index lock 检查。
- diff token、hunk/line fingerprint 和 `git apply --check --unidiff-zero` 拒绝陈旧目标。
- destructive file/hunk/line discard 确认；失败保留选择与错误，成功自动刷新。
- 文件批次使用稳定 `PathBuf` 集合，在写入前验证全部 diff token；Stash 保存 selected tracked/untracked，Discard 清理 tracked index/worktree、staged-added、untracked 和 rename 新旧路径，未选路径保持不变。
- Workspace `S`/`Z`/`D` 在显式选择为空时冻结光标仓库，非空时仅冻结 stable multi-select，并在确认框展示最终仓库范围与改动统计；全批路径/token/index-lock 预检失败时零写入，Stage 暂存完整 tracked/untracked 改动并拒绝冲突仓库，执行结果按仓库保留且不承诺跨仓库回滚。
- bracketed paste 保留提交正文换行；Unicode 光标移动、中间插入/删除、行首尾和跨行移动有状态测试覆盖。
- 80x24/120x40 TestBackend 验证 Changes/Workspace Git 确认与结果、Message 边框、Options/Keys 分隔区和真实 cursor；Changes 文件名 cell 直接覆盖 staged、unstaged、mixed、untracked、conflict 状态色及选中态覆盖。
- 真实临时仓库覆盖 selected-path Stash、完整 Discard、双仓库 Stage/Stash/Discard、冲突拒绝、stale 全批零写入，以及高级 stash 与 conflict 工作流。

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
- fetch、pull/rebase、普通 push、force-with-lease push、set upstream、prune 和 remote add/set-url/remove；Repository 与 Graph 本地分支均有独立 Push/Force Push 入口。
- conflict resolver 与合法 continue/skip/abort。
- remote-write 精确预览、Force Push 历史重写警告和 `force-with-lease`；不提供裸 `--force`。

验收证据：

- 真实临时仓库完成 branch/tag、merge/rebase/cherry-pick/revert 和 operation control 矩阵。
- workspace-local seed/client/peer/bare remote 完成 fetch/pull/push/upstream/prune 与 remote 管理闭环，并验证普通非快进拒绝、陈旧 lease 拒绝和更新 tracking ref 后 lease 强推成功。
- remote write 显示准确 `branch:branch` refspec、OID range、upstream 和 lease 状态，Force Push 显示历史重写警告，URL userinfo 脱敏。
- repository snapshot token 在锁内拒绝确认后发生的 refs/stash/conflict/remote 状态变化。
- 外部 mergetool/editor 依赖 M5 PTY takeover，明确不在 M3 后台任务中启动。

## 9. M4：Repo 批量工作流

- 状态：`Done`
- 优先级：`P0`
- 目标版本：`0.4.0`
- 依赖：M1、M2 安全执行层

已交付：

- ProjectId 稳定多选、命名/path 过滤和过滤集合全选。
- `repo sync/start/checkout/abandon/prune/rebase/upload/download` 固定 argv 工作流；空选择 sync 确认后执行整个 workspace 的 `repo sync -c -j8`，有选择时聚合执行 `repo sync -c -j8 -- <projects...>`。
- workspace 级 pinned manifest 导出，并验证输出目录不通过 symlink 逃逸。
- workspace exclusive lock 与现有 project Git 写锁协调，锁内重验路径、目录和 index lock。
- workspace 或逐项目 pending/running/success/failed/cancelled 显示与有界、凭据脱敏的 stdout/stderr 逐行日志。
- 聚合 sync 按整批退出状态更新参与作用域，不解析人类日志；结束后复扫真实状态。
- SIGINT 后 grace period 终止进程组；取消明确不回滚并触发真实状态复扫。
- 非 sync 动作保持逐项目部分失败隔离；sync 失败时按原 workspace 或冻结项目作用域重试，结果使用 generation 隔离。

验收证据：

- 每次执行前展示冻结的目标作用域、参数和完整 `repo` argv；整个 workspace Sync 也必须显式确认。
- fake Repo 真实子进程证明 workspace 和多项目 sync 都只调用一次，argv 分别为 `sync -c -j8` 与 `sync -c -j8 -- <projects...>`。
- App state 覆盖空选择确认、workspace/聚合 started event、失败重试、稳定多选与 stale batch event；Workspace overlay 覆盖 80x24/120x40。
- 后台 upload 固定使用确认页展示的 `--current-branch --yes`；交互认证和高级参数属于 M5 PTY takeover。

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
- Linux/macOS/Windows、主流终端、16/256/true color、Unicode/ASCII 矩阵。
- parser fuzz、PTY 信号测试和安全审查。
- 已交付单命令安装、SHA-256 自更新和发布文档；后续补充系统安装包、man page、completion 与配置迁移。

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
| M2 Changes/commit | Done | 文件树、批量 Stage/Unstage/Stash/Discard、可导航多行 editor、file/hunk/line、stale/index lock 和真实 Git 通过 |
| M2 stash/conflict | Done | staged/keep-index/index restore/branch/clear、非法组合拒绝、ours/theirs/resolved、continue/skip/abort 通过 |
| M3 refs/integration | Done | branch/tag、merge/rebase/cherry-pick/revert 与 Graph 本地分支 Push/Force Push 矩阵通过 |
| M3 remotes | Done | bare remote 普通 push、非快进拒绝、陈旧/当前 lease、fetch/pull/upstream/prune 与 remote 管理通过 |
| M4 Repo/Workspace batch | Done | Repo batch 与 Workspace Git Stage/Stash/Discard 的 stable multi-select、全批预检、逐项结果和 80x24/120x40 通过 |
| UI 文本/语言/配色 | Done | 默认英文、-zh/-en、stash“储藏”/stage“暂存”术语、Workspace 三范围与独立 List/Tree、显示列宽/控制字符/重绘回归，以及四页面暗蓝灰选中态和双语言 80x24/120x40 通过 |
| GitHub Release 与更新链路 | Done | tag/Cargo 校验、Linux/macOS tar.gz、Windows MSVC zip、SHA256SUMS、安装脚本、update 单测和双 Windows target check 通过 |
| M5-M7 | Planned | 下一步从 M5 命令面板与终端接管开始 |

M0-M4 最终验证（Rust 1.98、Git 2.43.0、Repo launcher 2.54）：

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
./target/debug/trepo doctor /path/to/git-or-repo-workspace
```

已验证边界：

- Graph 加载所有本地分支、远端分支、tag、HEAD 和每条 stash 的完整可达历史；大型仓库流式虚拟化仍是后续优化。
- Graph 的 Branch/Query/Author/Since/Until 过滤在内存中组合执行，保留完整 all-refs topology，并按稳定 OID 维护选择和对象操作。
- Graph 使用纯 topo-order 和独立 pipe 布局；continuing lane 可向左收缩，missing parent 与 lane cap 分别使用 `◉`/`~N`，高密度 remote/tag 在列表汇总但 Inspector/对象菜单保留完整 refs。
- Workspace Inspector 与 Changes 共享稳定路径文件树；Changes 批量 Stage/Unstage/Stash/Discard 在全量预检 token 后写入，多行提交编辑器支持 bracketed paste、Unicode 光标导航和清晰分区。
- Git 与 Repo 写操作协调 workspace/project 锁、实时前置检查、确认和 generation。
- Repository 与 Graph 的普通 Push/Force Push 使用固定 `branch:branch` refspec；裸 `--force` 不可达，force-with-lease 并发推进场景由真实 peer/bare remote 覆盖。
- Stash 高级模式、index 恢复、branch/clear，以及 selected-file/selected-repository Stash 和整仓 Stage 的领域映射、范围确认和 80x24/120x40 可见性均有测试覆盖。
- Repo 批处理保留凭据脱敏日志；Workspace Git 批任务保留逐仓库 pending/running/success/failure。两者均不承诺跨仓库回滚并在结束后复扫事实状态。
- Graph Subject 和 Workspace 展开仓库使用真实视觉行高；Changes diff 每个源行固定一行，显示列宽安全层已覆盖中文宽字符、控制字符与长转短重绘残留。
- Language 注入 App，默认 English；精确 `-zh`/`-en` 在 Clap 前规范化，标准 `--zh`/`--en` 同时受支持且互斥。
- 当前无任意命令面板或 PTY takeover；交互认证、外部 editor/mergetool 明确依赖 M5。

## 15. 下一执行点

1. 实现 M5 页面/project/ref/领域动作 fuzzy palette 与安全 argv 命令输入。
2. 实现 Capture 和 PTY takeover，覆盖交互认证、editor、difftool 与 mergetool 后的终端恢复。
3. 为大型 all-refs graph 设计保持全局拓扑和 ref 可见性的流式虚拟化。

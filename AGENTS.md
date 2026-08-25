# repo-tui Agent 指南

本文档面向在本仓库中执行设计、实现、测试、审查和文档维护的开发 Agent。目标是让新 Agent 快速建立正确上下文，并在不破坏 Git 数据、异步状态或终端生命周期的前提下交付可验证改动。

## 1. 开始之前

按以下顺序建立上下文，不要只阅读单个源文件后直接修改：

1. 阅读 `README.md`，了解当前用户可见能力、按键和运行方式。
2. 阅读 `docs/ROADMAP.md`，确认当前里程碑、已完成切片和下一执行点。
3. 按需阅读 `docs/DESIGN.md`，确认产品方向、架构、安全与交互约束。
4. 检查 `git status --short`，区分已有用户改动与本次任务改动，不得覆盖或回退无关工作。
5. 定位相关模块和既有测试，优先沿用当前模型、消息、generation 和 adapter 模式。

`docs/DESIGN.md` 包含长期目标，其中部分目录结构和能力是目标状态，不一定已经实现。判断当前代码事实时，以实际源码和 `docs/ROADMAP.md` 的当前进度为准；发现文档过期时必须同步修正。

## 2. 项目速览

`repo-tui` 是用于管理 Android Repo 工作区及其 Git 仓库的 Rust TUI。Git 和 Repo 系统 CLI 是行为真相，程序负责安全地构造 argv、解析机器协议、调度异步任务并渲染状态。

当前核心能力包括：

- Repo client 与单 Git 仓库发现。
- 多项目并发状态扫描和 Workspace 汇总。
- 包含本地分支、远端分支、tag、HEAD 与每条 stash 的全引用 commit graph。
- staged、worktree、untracked diff 预览。
- 文件与 hunk 级 stage、unstage、discard。
- commit/amend、sign-off、signing 与 hook 失败消息恢复。
- 项目写锁、index lock 检查、陈旧状态拒绝和破坏性确认。

当前仍在推进 M2，stash 操作和 conflict/operation state 尚未完成。任务已明确要求完成 M2/M3，因此后续应先闭合 M2，再实现 M3 的分支、整合与远端工作流。

## 3. 实际代码结构

本仓库当前是单 Rust package：

- `src/main.rs`
  - Clap CLI、日志初始化、终端 RAII、Crossterm 事件循环和按键分发。
- `src/domain.rs`
  - 与 UI 解耦的 workspace、project、status、commit、change、hunk 和 operation 模型。
- `src/adapters/git.rs`
  - Git argv、机器可读协议、路径处理、diff/hunk 解析和受控 `git apply`。
- `src/adapters/repo.rs`
  - Repo 版本、project 列表和能力相关适配。
- `src/services/discovery.rs`
  - Repo root 与单 Git 仓库发现。
- `src/services/scanner.rs`
  - 有界并发状态扫描。
- `src/services/operations.rs`
  - 项目写锁、实时前置检查和受保护写操作。
- `src/app/state.rs`
  - 页面状态、selection、generation、异步结果归并和 operation 流程。
- `src/ui/*.rs`
  - Ratatui 纯渲染与 TestBackend 测试，不执行外部命令。

新增功能应放在拥有该行为的现有层中。只有当重复和 ownership 压力真实存在时才增加抽象或拆分模块，不为单次调用创建框架。

## 4. 快速构建与运行

环境要求：Rust 1.81+、Git porcelain v2、Linux/macOS 终端；Repo workspace 模式还需要 Google/Android `repo`。

```bash
cargo build
cargo test
cargo run -- /path/to/git-or-repo-workspace
cargo run -- doctor /path/to/git-or-repo-workspace
```

项目根目录已经是 Git 仓库，因此也可以运行：

```bash
cargo run -- doctor .
```

`.local-rust/`、`target/`、`.me/` 和 `*.log` 已被忽略，不得提交这些本地工具链、构建产物、Agent 运行数据或临时验证仓库。

## 5. 不可破坏的工程规则

### 5.1 外部命令与协议

- 不在 widget 或渲染函数中创建进程。
- Git/Repo 路径、ref 和用户输入必须作为独立 argv 传递，禁止拼接 shell 命令。
- 核心状态只解析机器可读协议，如 porcelain v2、NUL 分隔字段和显式格式串。
- 不依赖本地化的人类输出，不把解析失败或命令失败转换为 clean/empty 成功状态。
- Git/Repo CLI 仍是配置、凭据、hooks、签名、LFS、worktree 和版本行为的权威后端。

### 5.2 路径与不可信数据

- 仓库路径使用 `PathBuf`/`OsString`，不要无故转换为 UTF-8 `String`。
- Git 路径参数放在 `--` 后面，并拒绝绝对路径和 `..` 逃逸。
- Unix NUL 协议解析应保留原始路径字节；lossy 转换只用于显示。
- 外部命令输出、commit message、ref、文件名和 diff 都是不可信输入；解析器必须返回错误而不是 panic。

### 5.3 异步状态

- 长任务不得阻塞输入和绘制循环。
- scan、graph、changes、preview 和 operation 结果必须携带并校验 generation。
- 过期结果不得覆盖新页面、新 project、新 path 或新 generation。
- selection 应绑定稳定 identity；列表刷新后必须 clamp，不能依赖旧行号代表同一对象。

### 5.4 Git 写操作

- 同一 project 的写操作必须经过 `OperationRunner` 和项目锁。
- 执行前检查 worktree-aware index lock，并在锁内重新读取当前状态。
- 文件操作必须验证 preview token；hunk 操作必须重新解析当前 diff，以 source + fingerprint 唯一定位。
- UI 不得提交任意 patch 字节作为执行输入；patch 必须由 adapter 从当前 Git 输出重建。
- hunk apply 先执行对应的 `git apply --check`，检查成功后才真正 apply。
- 外部变化、hunk 移动、内容变化或不唯一匹配必须返回 precondition/stale 错误。
- discard、删除、reset、force 等破坏性行为必须有明确作用域预览和显式确认。
- 操作失败应尽量保留用户选择、输入和原始错误；成功后按影响范围刷新真实状态。

### 5.5 终端与 UI

- raw mode、alternate screen 和光标必须由 RAII 保证恢复。
- 正常退出、错误、外部程序返回和信号路径都不能留下损坏终端。
- UI 需要在至少 80x24 与 120x40 下可用，不得出现文本重叠或动态内容导致布局跳动。
- 颜色需要表达稳定语义，不能作为唯一信息来源；同时保留符号或文字标签。

## 6. 标准开发流程

1. 明确任务属于哪个 roadmap 里程碑和模块边界。
2. 读取现有实现、调用点和测试，不基于猜测设计接口。
3. 先定义领域模型和 adapter 协议，再接 service/app，最后接 UI。
4. parser 或安全写操作优先补测试，再连接更大的交互流程。
5. 保持改动聚焦，不混入无关重构、格式噪音或依赖升级。
6. 完成后运行与风险相称的验证，并检查 `git diff --check`。
7. 同步相关文档，确认 roadmap 状态与真实实现一致。
8. 提交前检查 staged 文件、diff、测试结果和提交消息格式。

## 7. 测试与验证门禁

所有正常交付至少运行：

```bash
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo build
git diff --check
```

根据改动类型追加验证：

- parser/协议：fixture 覆盖正常、畸形、空输出、非 UTF-8/空格路径和边界记录。
- Git 行为：使用 `tempfile` 创建真实临时仓库，验证 index、worktree、refs 和 stash 的实际结果。
- app state：覆盖 stale generation、selection clamp、失败状态保留和成功刷新。
- UI：使用 Ratatui TestBackend 覆盖 80x24、120x40、空数据、错误、加载和确认状态。
- 终端交互：使用真实 PTY 验证按键流程、外部状态变化以及退出后的 echo、光标和 shell prompt。
- 写操作：证明只改变目标文件/hunk/ref，并明确断言未选择对象保持不变。

不能运行某项验证时，在交付说明中写明原因和残余风险，不得声称通过。

## 8. 文档同步规则

代码与文档必须在同一提交中保持一致。以下变化需要同步更新：

- 用户可见能力、按键、运行方式、依赖或限制：更新 `README.md`。
- 产品语义、交互、架构、安全模型或长期技术方向：更新 `docs/DESIGN.md`。
- 里程碑状态、已完成切片、验收证据或下一执行点：更新 `docs/ROADMAP.md`。
- Agent 工作流、工程约束、测试门禁或提交规范：更新 `AGENTS.md`。

禁止出现以下状态：

- 代码已经实现，但 roadmap 仍标记 Planned。
- README 宣称支持尚未实现的行为。
- DESIGN 的安全模型与实际写路径不一致。
- 完成新切片后仍保留已经过期的“下一执行点”。

修改文档时应描述可观察事实和验证证据，不写无法执行的泛泛要求。

## 9. Git 提交规范

### 9.1 提交格式

提交消息使用以下结构：

```text
<emoji> <type>(<scope>): <中文摘要>

1. <具体改动或用户行为>。
2. <关键实现或安全约束>。
3. <测试、文档或兼容性结果>。
```

要求：

- header 必须包含 emoji、type、scope 和简洁中文摘要。
- header 与正文之间只保留一个空行；正文内部除非确实需要区分不同语义段落，否则不要插入空行。
- 正文使用连续的 `1. 2. 3.` 编号条目，条目之间默认不留空行，说明结果，不记录流水账。
- 不要为每个正文条目分别传入一个 `git commit -m`；多个 `-m` 参数会被 Git 视为多个独立段落并自动插入空行。
- `git commit -n` 等同于 `--no-verify`，只用于明确跳过 hooks，不能代替提交消息参数。
- 多行提交消息优先使用单个完整消息源，例如 `git commit -F - <<'EOF'`，以稳定保留正文格式。
- scope 使用稳定模块名，如 `graph`、`changes`、`git`、`repo`、`workspace`、`ui`、`docs`、`build`。
- 一个提交只包含一个连贯目的；不要把无关修复、重构和格式调整混在一起。
- 提交前确认工作树、staged diff、测试和文档同步状态。

推荐命令：

```bash
git commit -F - <<'EOF'
📝 docs(agent): 收紧提交正文格式

1. 规定编号正文默认连续排列，不在条目之间插入空行。
2. 说明多个 `-m` 会生成独立段落，`-n` 仅用于跳过 hooks。
3. 增加 heredoc 提交示例并同步 Agent 开发约定。
EOF
```

推荐类型：

- `✨ feat`：新增用户可见能力。
- `🐛 fix`：修复缺陷或回归。
- `♻️ refactor`：不改变外部行为的结构调整。
- `✅ test`：新增或强化测试。
- `📝 docs`：文档和开发约定。
- `⚡ perf`：性能优化。
- `🔧 chore`：构建、工具或维护任务。
- `🔒 security`：安全边界、校验或风险控制。

示例：

```text
✨ feat(window): 统一标题栏主题配色

1. Windows 窗口标题栏使用对应界面的深色主题背景。
2. 保留原生窗口按钮并支持标题区域拖动。
3. 同步动态窗口标题并增加结构测试。
```

本项目示例：

```text
✨ feat(graph): 展示完整分支与 stash 拓扑

1. Commit graph 加载本地分支、远端分支、标签、HEAD 和每条 stash。
2. 使用结构化 ref 类型和多色 lane 保持分支语义清晰。
3. 增加真实复杂仓库、TestBackend 和文档同步验证。
```

### 9.2 提交前检查

```bash
git status --short
git diff --check
git diff --cached --stat
git diff --cached
```

不得提交：

- `.local-rust/`、`target/`、`.me/`、日志和临时测试仓库。
- 密钥、token、凭据、私有 URL 或用户数据。
- 未理解来源的已有改动。
- 未通过必要验证且没有明确说明风险的改动。

## 10. Agent 交付说明

最终回复应简洁包含：

- 完成了什么，以及关键行为变化。
- 主要文件或模块。
- 实际运行的验证及结果。
- 未完成项、限制或风险。
- 如果创建了提交，给出 commit ID 和提交标题。

不要只说“已完成”，也不要把长篇原始日志粘贴给用户。
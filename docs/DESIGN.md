# repo-tui 设计文档

- 文档状态：Draft
- 产品名称：`repo-tui`
- 目标平台：Linux、macOS；Windows/WSL 后续评估
- 主要技术栈：Rust、Ratatui、Crossterm、Tokio

## 1. 背景与术语

大型 Android、嵌入式或平台工程通常通过 Google/Android `repo` 管理数十到数千个 Git 仓库。现有命令行工具功能完整，但用户需要频繁组合 `repo`、`git`、过滤器和脚本，才能回答以下问题：

- 哪些仓库有未提交、未跟踪或冲突文件？
- 各仓库当前位于哪个分支或 detached HEAD，HEAD 对应什么提交？
- 哪些仓库领先或落后上游？
- 一次 `repo sync`、批量分支操作或上传中，哪些项目成功、失败或被跳过？
- 某仓库的提交拓扑、分支、标签和远端引用之间是什么关系？
- 如何在不离开当前上下文的情况下完成 stage、commit、rebase、push 等 Git 操作？

`repo-tui` 是一个面向终端的多仓库工作台。它以 Repo 工作区为顶层对象，以各 Git project 为主要管理单元，把状态观察、历史浏览、单仓库操作和跨仓库工作流统一到同一个 TUI 中。

本文中：

- **Repo**：Google/Android 的 `repo` 多仓库命令行工具。
- **Repo 工作区**：包含 `.repo/` 的 client checkout 根目录。
- **项目/仓库**：manifest 中的一个 project 及其 Git worktree。
- **引用**：Git branch、tag、remote-tracking ref 等 ref。
- **任务**：一个可观察、可取消并产生结构化结果的后台命令。

如果启动目录不是 Repo 工作区，`repo-tui` 可以把它作为单个 Git 仓库打开；可选的普通目录扫描属于兼容模式，不取代 Repo 作为核心场景。

## 2. 产品目标与边界

### 2.1 产品目标

1. 在一个屏幕中快速掌握整个 Repo 工作区的健康状态。
2. 让仓库列表、工作区变更、HEAD、上游关系和错误状态可搜索、排序、过滤和批量处理。
3. 为单仓库提供清晰、可分页、可交互的 commit graph，并呈现本地分支、远端分支和标签。
4. 为高频 Repo/Git 操作提供结构化界面、执行前预览、执行中进度和执行后结果。
5. 保留 Git 和 Repo 的完整表达能力，使低频或版本相关命令仍可从 TUI 安全触达。
6. 对数百到数千个项目保持响应；任何外部命令都不能阻塞输入和绘制循环。
7. 尊重用户已有 Git 配置、凭据、hooks、签名、LFS、submodule 和 Repo 版本行为。

### 2.2 “完整操作”的定义

Git 与 Repo 的参数面很大，而且会随版本、插件和服务端扩展变化。`repo-tui` 采用两层覆盖模型：

- **一等工作流**：高频命令拥有专用页面或弹窗、参数校验、影响范围预览和结构化结果。
- **受控命令面板**：所有其余 `git`/`repo` 子命令可通过 argv 形式执行；不会经 shell 拼接。需要编辑器、认证、交互式 rebase 或复杂 prompt 时，临时退出 alternate screen，将终端控制权交给子进程，结束后恢复 TUI 并刷新状态。

因此，“完整”指所有合法命令都能在产品内触达，不代表首版为每个参数设计独立表单。命令面板不是规避安全规则的任意 shell；默认只允许 `git`、`repo` 和配置白名单中的工具。

### 2.3 非目标

- 不重新实现 Git 对象数据库、网络协议或 Repo manifest 解析器的全部语义。
- 不代替终端编辑器、merge tool、diff tool 或代码 IDE。
- 不提供 GitHub/GitLab/Gerrit 的完整 Web 管理后台；服务端集成作为扩展能力。
- 不承诺跨多个 Git 仓库的原子事务，因为底层 Git/Repo 本身不具备该保证。
- 首版不提供守护进程、远程 Web UI 或多人共享状态。

## 3. 用户与核心场景

### 3.1 目标用户

- 维护 Android/AOSP、BSP、嵌入式平台或大型单产品代码树的开发者。
- 需要批量同步、建分支、上传、检查状态的集成工程师。
- 需要定位跨项目提交、分支或同步问题的发布和构建工程师。
- 偏好键盘工作流，同时希望获得比命令输出更稳定信息结构的 Git 用户。

### 3.2 核心场景

1. 启动工具后，优先看到 dirty、conflict、ahead/behind、detached、缺失或扫描失败的仓库。
2. 搜索 project name/path/group，筛选修改仓库，批量 stash、discard、建分支或执行 Repo 操作。
3. 进入一个仓库查看提交图，定位某个 commit，查看详情和 diff，并对它执行 branch、tag、cherry-pick、revert 等动作。
4. 在 Changes 页面逐文件或逐 hunk stage/unstage，提交并 push。
5. 执行 `repo sync`、`repo start`、`repo upload` 等操作，观察每个项目的独立进度和错误，失败后只重试失败项。
6. 冲突发生后，从任务结果直接跳到冲突文件列表，解决后继续 merge/rebase/cherry-pick。
7. 通过命令面板执行未被专用 UI 覆盖的命令，并在返回后自动刷新受影响数据。

## 4. 信息架构

### 4.1 一级页面

| 页面 | 作用 |
| --- | --- |
| Workspace | Repo 工作区主页，展示所有仓库及聚合状态 |
| Repository | 单仓库容器，包含 Graph、Changes、Branches、Tags、Remotes、Stashes 等标签页 |
| Repo Actions | Repo 级 init/sync/start/upload/download/manifest/forall 等工作流 |
| Tasks | 正在执行和历史任务、逐项目结果、日志、取消与重试 |
| Command Palette | 搜索页面、动作、仓库、引用，以及执行受控 Git/Repo 命令 |
| Settings | 显示、快捷键、扫描并发、默认命令参数、确认策略和日志设置 |
| Help | 当前上下文可用快捷键、版本和诊断信息 |

### 4.2 导航模型

- 页面使用栈式路由：`Workspace -> Repository -> Commit detail -> Diff`。
- `Esc` 关闭最上层弹窗或返回上一级；根页面上触发退出确认。
- `Tab`/`Shift+Tab` 在同一页面的区域间切换，Repository 内用 `[`/`]` 切换标签页。
- `/` 打开当前数据集搜索，`f` 打开结构化过滤器，`s` 打开排序菜单。
- `Space` 切换当前行选择，`v` 进入范围选择；批量动作只作用于已选择项。
- `Ctrl+P` 打开命令面板，`Ctrl+R` 刷新当前范围，`?` 打开上下文帮助。
- 底部状态栏只展示当前上下文最重要的动作、任务状态和错误计数；完整快捷键位于 Help。
- 快捷键可配置；文本输入态、列表态和弹窗态有独立 key scope，避免同键歧义。

## 5. Workspace 主页

### 5.1 布局

宽终端采用“列表 + 检查器”，窄终端退化为单栏，并可打开详情抽屉。

```text
┌ repo-tui  workspace: android-main  manifest: default.xml  436 projects ┐
│ Filter: dirty,conflict      Search: camera                 Tasks 2/1 ! │
├──┬ Status ┬ Project/Path             ┬ HEAD       ┬ Upstream   ┬ Age ─┤
│  │ ● M?   │ platform/camera          │ feature/x  │ +2 -1      │ 2h  │
│✓ │ ! UU   │ vendor/acme/display      │ a18bd72    │ detached   │ 1d  │
│  │ ✓      │ frameworks/base          │ main       │ =          │ 5m  │
│  │ ×      │ device/acme/product      │ scan error │ -          │ -   │
├───────────────────────────────────────┬───────────────────────────────┤
│ Summary: 12 dirty / 1 conflict        │ Selected repository inspector │
│ 4 ahead / 7 behind / 2 detached       │ branch, commit, file summary  │
└ Enter Open  Space Select  / Search  f Filter  a Actions  ? Help ─────┘
```

### 5.2 仓库列与状态

默认列：

- 选择标记与任务占用标记。
- 工作区状态：staged、modified、untracked、renamed、deleted、conflicted。
- project name 与相对路径；两者不同时可切换主显示字段。
- HEAD：本地分支名；detached 时显示短 OID；unborn branch 单独标记。
- 上游：upstream 名和 ahead/behind 数。
- HEAD subject、author、相对时间，可按宽度动态显示。
- manifest revision、remote、groups，可作为可选列。
- 操作进行中、Git index lock、仓库缺失、权限或扫描错误。

状态不能压缩成一个含糊的 dirty 布尔值。领域模型至少保留 staged、unstaged、untracked、conflicted 的数量以及 merge/rebase/cherry-pick/bisect 等进行中状态。

颜色与符号同时传递信息，确保无颜色终端和色觉差异用户仍能判断状态。主题支持 16 色、256 色和 true color 降级。

### 5.3 搜索、过滤与排序

搜索范围包括 project name、path、branch、HEAD OID/subject、manifest group 和错误文本。默认使用大小写不敏感的 fuzzy match，并支持切换 literal/regex。

内置过滤器：

- dirty、clean、staged、unstaged、untracked、conflict。
- ahead、behind、diverged、no-upstream。
- detached、unborn、operation-in-progress、scan-error、missing。
- manifest remote/revision/group。
- selected、task-running、task-failed。

多个过滤器采用可见的 AND/OR 规则。过滤条件可保存为命名视图，例如“待上传”“有冲突”“vendor 修改”。

默认排序优先级为：错误/冲突、dirty、diverged、ahead/behind、detached、clean，再按 path。还支持 path、project、branch、最近提交、变更数和任务结果排序。扫描结果到达时保持当前 selection 绑定到稳定 `ProjectId`，不能因行重排跳到其他仓库。

### 5.4 聚合与批量动作

主页顶部和底部显示可点击/可聚焦的聚合指标：总项目、dirty、conflict、ahead、behind、detached、扫描失败。选择指标可快速应用对应过滤器。

批量动作包括：

- 刷新、fetch、pull/rebase、push。
- 创建/切换/删除分支，统一设置 upstream。
- stash push/pop/apply。
- restore/clean/reset 等破坏性操作。
- Repo start/checkout/abandon/prune/sync/upload。
- 执行白名单 Git/Repo 命令。

批量写操作必须先展示目标项目、前置检查、预计命令和风险。执行结果按项目记录 `success/failed/skipped/cancelled`；部分失败不伪装成整体成功。

## 6. Repository 页面

### 6.1 Graph 标签页

Graph 是进入仓库后的默认页，由提交列表和详情检查器组成：

```text
┌ platform/camera  Graph  Changes(5)  Branches  Tags  Remotes  Stashes ┐
├ Graph ── Commit ─────────── Refs ───────────── Author ──────── Time ┤
│ ●  91f2c7a Fix frame ownership      (HEAD -> feature/x)       2h   │
│ │\                                                                    │
│ │ ● 5aa82e1 Tune exposure           (origin/feature/x)        4h   │
│ ● │ e30d991 Add stream metrics      (tag: v2.4-rc1)           1d   │
│ ●─┘ 0c671b0 Merge camera HAL update                             2d   │
├───────────────────────────────────┬─────────────────────────────────┤
│ Commit message / metadata         │ Files / stats / selected diff    │
└ j/k Move  Enter Detail  d Diff  b Branch  t Tag  c Cherry-pick ─────┘
```

提交行展示：

- ASCII/Unicode 拓扑线、commit 符号和 merge 连接。
- 短 OID、subject、refs decoration。
- author/committer、绝对或相对时间。
- 签名状态、GPG/SSH signer、父提交数量等可选字段。

交互能力：

- 默认展示所有本地分支、远端分支、标签、HEAD 和每条 stash entry 的完整可达历史。
- 搜索 message、author、OID、path；后续可选择 first parent 或指定 ref 范围。
- 查看完整 commit metadata、message、parents、notes、签名和变更统计。
- 查看 commit、commit range 或单文件 diff。
- 从提交创建 branch/tag、checkout、reset、revert、cherry-pick、rebase onto。
- 复制 OID、subject 或生成可执行命令；从父/子提交间跳转。
- 过滤本地分支、远端分支、标签、stash 和 manifest revision。

### 6.2 Commit graph 布局算法

Git 提供提交及 parent 关系，`repo-tui` 负责视觉 lane 分配：

1. 按 `--topo-order` 获取提交，保证父提交不会出现在子提交之前。
2. 为当前 commit 复用等待它的 lane；若不存在则分配最左可用 lane。
3. 先按当前 lanes 绘制节点，再计算提交后的 parent lanes，避免 lane 插入掩盖来源位置。
4. first parent 延续当前 lane，其余 parent 分配或复用其他 lane；每个 cell 根据上、下、左、右连接位动态选择 `─`、`│`、`├`、`┤`、`┬`、`┴`、`┼`、`┌`、`┐`、`└`、`┘` 等 box-drawing 字符。
5. merge 节点使用菱形标记，多个 parent 连接从该节点展开；共同祖先重新进入同一 lane 时保留反向连接。
6. octopus merge、boundary commit 和 shallow history 使用独立标记。
7. ref 变化后使当前 graph generation 失效，保留选中 OID；如果 OID 不再可达，给出提示而不是静默跳行。

内部数据以 OID 和 parents 构成 DAG，不把 `git log --graph` 的终端文本当数据源。渲染算法需覆盖直线、分叉、双亲 merge、octopus merge、交叉 lane、shallow boundary 和 replace/graft 等 fixture。

当前实现一次加载 all-refs 完整可达历史，优先保证分支树和 stash 不被分页边界隐藏。大型仓库的后续优化应采用保持全局拓扑与 ref 可见性的虚拟化或流式加载，不能退化为仅显示 HEAD 或静默截断其他 refs。

### 6.3 Changes 标签页

Changes 页面分为文件树、hunk 列表、diff 检查器和提交对话框：

- 分类展示 staged、unstaged、untracked、conflicted、ignored（按需）。
- 文件级 stage/unstage/restore/delete/intent-to-add。
- hunk 与 changed-line 级 stage、unstage、discard；行操作使用当前 diff 重建零上下文 patch，执行 `git apply --check --unidiff-zero` 后写入。
- diff 模式支持 unified、side-by-side、word diff、忽略空白。
- 二进制、重命名、submodule、mode change、大文件和不可解码文件明确降级。
- commit 对话框支持 message、amend、signoff 和 GPG/SSH signing；当前 UI 通过独立开关控制后三项，默认不跳过 hooks。
- 提交使用 project 级写锁并在锁内检查 `index.lock`；普通 commit 要求存在 staged 内容，amend 遵循 Git 当前 HEAD 语义。
- commit 失败时合并 hook stdout/stderr，保留用户输入、选项和错误状态，允许修正后重试；成功后刷新 Changes 与 Workspace。

文件名按原始字节保存，显示层才做可逆转义或 lossy 展示。Git 数据读取尽量使用 `-z`，正确处理空格、制表符、换行及非 UTF-8 路径。

### 6.4 Conflict Resolver

- 当前实现识别 merge、rebase、cherry-pick 和 revert 冲突状态，列出 conflicted paths。
- 提供 take ours、take theirs 和 mark resolved；文本三方合并视图不手写复杂语义。
- 显示 continue/skip/abort，并由 Git operation 类型限制合法动作；merge 不提供 skip。
- abort、ours/theirs 覆盖等操作显示风险并二次确认，执行前用 snapshot token 复核 refs、stash、conflicts 和 remotes。
- 外部 mergetool/editor 需要离开 alternate screen 并接管 PTY，明确归入 M5；M3 后台任务不启动交互程序。

### 6.5 Branches、Tags 与 Remotes

Branches：

- 当前 Repository 页面列出本地 branch OID、upstream、ahead/behind，并以固定表单提供 create、switch、rename、set upstream、merge、rebase 和 delete。
- 强制 branch 删除和所有 ref 删除进入危险确认流程；执行时在 project 锁内重新核对 repository snapshot token。

Tags：

- 当前支持 lightweight tag create/delete；annotated/signed tag、verify 和独立 tag push 属于后续增强。

Remotes：

- 当前列出 fetch/push URL，并提供 add、set-url、remove、fetch、prune、pull、pull-rebase、push 和 set-upstream。
- RemoteWrite 确认展示 remote、`branch:branch` refspec、远端到本地 OID range、set-upstream 和 force-with-lease 状态。
- URL 只在显示层隐藏 `scheme://userinfo@host` 的 userinfo；真实 URL 仍作为单独 argv 传给 Git。
- UI 不提供裸 `--force`，只提供 `--force-with-lease`；确认后任何 ref/remote 状态变化都会使 snapshot token 失效并拒绝执行。

### 6.6 其他 Git 工作流

一等或半结构化支持：

| 类别 | 操作 |
| --- | --- |
| 工作区/index | status、diff、add、restore、rm、mv、clean、commit、amend |
| stash | list、show、push、apply、pop、branch、drop、clear |
| 历史 | log、show、reflog、blame、shortlog、range-diff |
| 分支整合 | merge、rebase、interactive rebase、cherry-pick、revert |
| 远端 | remote、fetch、pull、push、prune、set-upstream |
| 对象与引用 | branch、tag、notes、replace、rev-parse、for-each-ref |
| 补丁 | apply、am、format-patch、request-pull |
| 调试恢复 | bisect、reset、reflog restore、fsck |
| 扩展工作树 | worktree、submodule、sparse-checkout |
| 维护 | gc、maintenance、repack、count-objects |
| 配置 | 作用域明确的 config 查看和编辑、credential 诊断 |
| 扩展 | Git LFS 等已安装扩展通过命令面板或插件式动作进入 |

interactive rebase、`git add -p`、外部 mergetool/difftool、credential prompt 等交互式能力通过终端接管实现。TUI 恢复后重新扫描仓库，而不猜测外部命令修改了什么。

## 7. Repo 工作流

### 7.1 工作区发现与 Manifest

启动顺序：

1. 解析 `repo-tui [PATH]`，未提供时使用当前目录。
2. 向上查找有效 Repo client 根目录，并验证 `repo` 命令可用。
3. 通过 `repo list` 获取当前 manifest 生效的项目集合；不要只递归查找 `.git`。
4. 读取 manifest 输出补充 project name、path、remote、revision、groups、upstream、dest-branch 等信息。
5. 对缺失 project、同步中间态和嵌套 project 保留记录并标记状态。
6. 若不是 Repo 工作区，再尝试单 Git 仓库模式；普通目录递归发现必须由用户显式启用。

Repo manifest 的 include、extend-project、remove-project、local manifests 和版本差异较复杂。首选调用当前环境中的 `repo manifest`/`repo list` 获取展开后的真实视图，不自行复制 Repo 的完整合并语义。XML 解析用于展示和校验，不作为执行真相的唯一来源。

Manifest 页面支持：

- 查看当前 manifest URL、branch、manifest name、groups、mirror/reference 等 client 信息。
- 查看展开后的 project 列表和 revision，比较当前 HEAD 与 manifest revision。
- 导出 pinned manifest（revision-as-HEAD）及选择输出位置。
- 查看 manifest diff；编辑 manifest 文件时打开用户配置的 `$EDITOR`，返回后重新验证。
- 管理 local manifest 属于后续一等工作流，首版通过编辑器和命令面板完成。

### 7.2 Repo 命令覆盖

| 工作流 | 专用 UI 方向 |
| --- | --- |
| init | URL、manifest branch/name、groups、depth、reference 等表单，执行前显示目标目录 |
| list/status/info/diff/branches/overview | 结构化列表、过滤器和跳转 |
| sync | 项目范围、jobs、current-branch、detach、force-sync、prune 等参数与逐项目进度 |
| start/checkout/abandon/prune | 分支名、项目范围、前置状态检查和批量结果 |
| upload | 待上传分支/commit、目标 review branch、topic、reviewer/cc、dry-run；认证交给 Repo |
| download | change/patchset 输入、目标项目和执行结果 |
| rebase | 项目范围、是否 auto-stash、失败后跳冲突页 |
| forall | 显式命令 argv/受控 shell 模式、环境变量说明、并行度和逐项目输出 |
| grep | 关键字、项目/组范围和跳转文件位置 |
| manifest | 查看、导出 pinned manifest、保存文件 |
| selfupdate/version/help | 诊断页；selfupdate 必须显式确认 |

不同 Repo 版本的子命令和参数并不完全一致。启动时记录 `repo version` 和 `repo help` 能力，UI 仅启用确认支持的选项；未知能力仍可通过命令面板执行。不能为了统一界面悄悄改写用户的 Repo 参数。

### 7.3 Sync 与 Upload 体验

`repo sync` 是长时间、并发、可部分失败的操作。任务页面需要展示：

- 当前阶段、总项目数、完成/失败/跳过/运行中数量。
- 可解析时显示每个项目的状态；无法稳定解析的 Repo 版本至少保留原始流式日志和最终项目复扫结果。
- 取消时先发送温和中断，超时后再允许强制终止；明确提示子进程可能已完成部分修改。
- 结束后优先刷新受影响项目，并把冲突、detached、dirty 或失败项目形成临时过滤视图。

Upload 执行前展示 project、local branch、目标 branch、commit range 和工作区状态。提供 dry-run 时优先使用；不解析或存储密码/token。若 Repo/Gerrit 需要交互式认证，切换到终端接管。

## 8. 命令面板与终端接管

命令面板有三类条目：页面跳转、领域动作和原生命令。

原生命令输入解析为程序加参数数组，不经 `/bin/sh -c`。历史记录保存前执行凭据脱敏；带有 token/password/credential 等敏感参数的命令默认不持久化。用户可以查看准确的 cwd、argv 和环境覆盖。

执行模式：

- **Capture**：非交互命令在后台执行，stdout/stderr 分流或合并进入任务日志。
- **PTY takeover**：挂起 TUI alternate screen 和 raw mode，子进程继承真实终端；完成后恢复终端并全量刷新当前作用域。
- **External editor/tool**：为 `$EDITOR`、sequence editor、difftool、mergetool 使用 PTY takeover。

恢复逻辑必须使用 RAII guard 和 panic hook，确保正常退出、错误、panic、`SIGINT`、`SIGTSTP/SIGCONT` 后尽量恢复光标、raw mode 和 alternate screen。

## 9. 操作安全模型

### 9.1 风险等级

- **ReadOnly**：status、log、show、diff、list；直接执行。
- **ReversibleWrite**：stage、commit、stash、create branch；显示目标并允许执行。
- **RemoteWrite**：push、upload、delete remote ref；展示远端、refspec、commit range，并确认。
- **Destructive**：clean、hard reset、force checkout、drop/clear stash、force delete、abort with local changes；要求二次确认。
- **CrossRepoDestructive**：跨仓库 restore/clean/reset/abandon；展示项目和文件统计，要求输入确认词或使用可配置强确认。

### 9.2 统一前置检查

写操作执行前重新读取而不是依赖页面旧快照：

- 目标 project 是否仍存在且路径未变化。
- HEAD、branch/upstream 是否与预览时一致。
- 是否存在 index.lock、其他写任务或 operation in progress。
- 工作区变更是否会被覆盖。
- remote URL/refspec 和 push lease 是否仍一致。

检查失败则停止该项目并返回 stale/precondition failed，不自动用新状态继续危险操作。

### 9.3 执行约束

- 使用 `Command` 与 `Vec<OsString>` 传参，禁止拼接 shell 字符串。
- cwd 必须是发现阶段确认过的工作区或 project 路径；对 symlink 规范化并限制到工作区边界。
- 结构化后台任务默认关闭终端 prompt；检测到认证需求后提示用 PTY 模式重试。
- 不自动添加 `--no-verify`、`--force`、`--ignore-errors` 等绕过选项。
- 日志对 URL userinfo、token、authorization header、credential helper 输出做脱敏。
- 跨项目操作记录逐项结果和实际 argv，支持只重试失败项，但不宣称事务性回滚。

## 10. 技术方案

### 10.1 技术选型

推荐 Rust stable：

| 能力 | 建议依赖 | 用途 |
| --- | --- | --- |
| TUI | `ratatui` | 布局、widget、TestBackend |
| 终端 | `crossterm` | 输入、raw mode、alternate screen、跨平台终端控制 |
| 异步 | `tokio`、`tokio-util` | 任务、进程、channel、取消令牌 |
| CLI | `clap` | 启动参数和非交互诊断子命令 |
| 序列化 | `serde`、`toml` | 配置、用户视图和会话信息 |
| XML | `quick-xml` | manifest 展示数据解析 |
| 错误 | `thiserror`、可选 `anyhow` | 分层错误与应用边界上下文 |
| 日志 | `tracing`、`tracing-subscriber` | 结构化诊断日志 |
| 文本 | `unicode-width`、`unicode-segmentation` | 正确截断和光标宽度 |
| 匹配 | `nucleo-matcher` 或同类库 | fuzzy search |
| 标识 | `git2` 的 OID 类型或自定义 validated OID | 仅作数据类型时可避免绑定 libgit2 |

依赖版本在实现阶段按当前 Rust MSRV 和维护状态锁定；本文不固定具体版本号。

### 10.2 为什么以系统 CLI 为权威后端

`git` CLI 与用户环境在以下方面保持一致：credential helper、SSH、GPG/SSH signing、hooks、attributes、filters、LFS、submodule、worktree、版本特性和全局配置。Repo 本身也是外部 Python CLI，且没有稳定的公共 Rust API。

因此：

- Git/Repo 的读取和写入均优先走系统 CLI。
- `libgit2` 不作为首版主执行后端，避免与 CLI 语义和配置出现双轨。
- 可在性能数据证明必要后，使用 `gix`/`git2` 加速纯读取，但结果必须与 CLI 校验，写操作仍走 CLI。
- 所有解析器基于明确版本和机器可读协议，不解析本地化的人类输出。

### 10.3 机器可读数据来源

| 数据 | 建议命令/协议 |
| --- | --- |
| 仓库状态、branch、ahead/behind | `git status --porcelain=v2 --branch -z` |
| Git dir/worktree/common dir | `git rev-parse --path-format=absolute ...` |
| refs 与 upstream | `git for-each-ref` + 自定义 NUL/字段格式 |
| commit DAG | `git log --topo-order --parents` + 显式记录/字段分隔符 |
| commit detail | `git show --no-patch` + 显式格式 |
| diff/name status | `git diff --raw/-z`、`--numstat -z`、`--patch` |
| unmerged stages | `git ls-files -u -z` |
| operation state | Git path 查询与 MERGE_HEAD/rebase/cherry-pick 等状态文件存在性 |
| Repo projects | `repo list` 的当前版本支持格式；能力探测后选择参数 |
| 展开 manifest | `repo manifest` 输出，再由 XML parser 读取 |
| Repo version/capability | `repo version`、`repo help <command>` |

命令环境设置 `LC_ALL=C` 只用于确需稳定英文诊断的子进程；核心解析不能依赖诊断文本。字段分隔符使用 NUL 或不会出现在字段中的显式控制分隔符，并为异常输出保留 parser error 和原始字节摘要。

### 10.4 进程与架构

```text
Keyboard / Tick / Resize / Process events
                  │
                  v
        ┌──────────────────┐
        │ App event loop   │  单线程 reducer，拥有 UI 状态
        └───────┬──────────┘
                │ Effect / Command
                v
        ┌──────────────────┐
        │ Task supervisor  │  取消、并发、锁、进度、任务历史
        └───┬───────────┬──┘
            │           │
       Git adapter   Repo adapter
            │           │
            └──── Process runner / PTY runner
                         │
                         v
                  git / repo processes
```

核心原则：

- UI event loop 不执行文件扫描、Git 命令或阻塞等待。
- reducer 根据 `AppEvent` 生成新状态和 `Effect`；副作用由 service/task 层执行。
- worker 只返回领域事件和不可变 snapshot，不直接修改 widget 状态。
- 每次刷新带 `generation`；旧 generation 晚到的结果被丢弃。
- 列表 identity 使用稳定 `ProjectId`/canonical path，不能使用行号。
- 绘制按终端事件和状态变化触发，并限制最高帧率，避免空闲 busy loop。

### 10.5 模块划分

```text
src/
  main.rs                 # CLI、日志、终端生命周期
  app/
    state.rs              # 全局 UI/导航/选择状态
    event.rs              # 输入、领域、任务事件
    reducer.rs            # 纯状态转换和 effects
  ui/
    workspace.rs
    repository/
      graph.rs
      changes.rs
      branches.rs
    repo_actions.rs
    tasks.rs
    command_palette.rs
    widgets/
  domain/
    workspace.rs
    repository.rs
    status.rs
    commit.rs
    refs.rs
    operation.rs
    task.rs
  adapters/
    git_cli/
      command.rs
      status.rs
      log.rs
      refs.rs
      diff.rs
    repo_cli/
      command.rs
      capability.rs
      projects.rs
      manifest.rs
  services/
    discovery.rs
    scanner.rs
    graph_loader.rs
    operation_runner.rs
    task_supervisor.rs
  infra/
    process.rs
    pty.rs
    terminal.rs
    config.rs
    logging.rs
```

解析器和领域模型不依赖 Ratatui；widget 不直接创建进程。这样可以使用 fixture 对命令输出做完整测试，并用 fake adapter 驱动 UI 测试。

### 10.6 核心领域模型

```rust
struct Workspace {
    root: PathBuf,
    kind: WorkspaceKind,
    manifest: Option<ManifestSummary>,
    projects: Vec<ProjectId>,
}

struct ProjectSnapshot {
    id: ProjectId,
    name: OsString,
    path: PathBuf,
    manifest: Option<ManifestProject>,
    head: HeadState,
    upstream: Option<UpstreamState>,
    worktree: WorktreeSummary,
    operation: Option<OperationState>,
    last_commit: Option<CommitSummary>,
    health: ScanHealth,
    generation: u64,
}

enum HeadState {
    Branch { name: Vec<u8>, oid: ObjectId },
    Detached { oid: ObjectId },
    Unborn { name: Vec<u8> },
}

struct WorktreeSummary {
    staged: usize,
    unstaged: usize,
    untracked: usize,
    conflicted: usize,
}
```

`ProjectSnapshot` 表示某一时刻的只读事实；正在执行的任务、UI selection 和过滤条件不混入该结构。完整文件状态、refs 和 commit pages 按需加载，不复制到主页每一行。

### 10.7 任务、并发与锁

- 扫描采用有界 worker pool；默认并发度取 CPU、磁盘和配置的保守值。
- 首屏优先：先发现项目并绘制 skeleton，再扫描可见行、dirty/error 候选和其余项目。
- 不同项目的只读任务可并行。
- 同一项目的写任务串行，并阻止与它冲突的扫描；任务结束后触发一次合并刷新。
- `repo sync`、manifest 变更等全工作区写操作获取 workspace exclusive lock。
- 用户仍可能从外部终端修改仓库，因此锁只约束本进程；每个写操作必须做实时前置检查。
- 取消通过 `CancellationToken` 传播。子进程先收中断，等待 grace period 后再允许 kill process group。
- 任务日志使用有界 ring buffer 并可选落盘，避免无限 stdout 耗尽内存。

锁的粒度是领域资源，而不是 UI 页面。任务中心显示等待锁、运行、取消中、完成和失败状态。

### 10.8 缓存与刷新

首版只持久化用户配置、保存视图和非敏感命令历史，不持久化 Git 状态作为真相。运行时缓存：

- Workspace project/manifest snapshot。
- 每个 project 的轻量状态和 refs。
- 当前仓库最近访问的 commit pages/diff。
- Repo capability probe 结果。

刷新来源：

- 用户手动刷新。
- 内部写任务完成。
- 页面 focus/终端接管返回。
- 可选文件 watcher 对 `.git/HEAD`、index、refs、operation state 和 manifest 发出 debounce invalidation。
- 低频兜底轮询；默认关闭或使用较长间隔，避免数千仓库 watcher/scan 压力。

文件事件只表示“可能过期”，最终状态仍由 Git 命令确认。

### 10.9 错误模型与恢复

错误至少包含：kind、operation、project、cwd、exit status/signal、sanitized argv、stderr 摘要、是否可重试和用户建议。分类包括：

- executable/version/capability 缺失。
- invalid workspace/project missing。
- parse/protocol mismatch。
- permission/index lock。
- authentication/network/remote rejected。
- conflict/precondition stale。
- cancelled/terminated。
- internal invariant/terminal restore failure。

单个仓库扫描失败不阻止主页加载；该行保留最后一次成功 snapshot（明确标记 stale）和当前错误。解析失败不能静默退化为 clean。

## 11. 配置、诊断与可扩展性

配置遵循 XDG 目录，使用 TOML。建议层级：内置默认值、用户配置、工作区本地配置、CLI 参数；工作区配置默认不执行其中的命令，避免打开不可信仓库即运行代码。

可配置项：

- 主题、符号集、日期格式、列和布局。
- 快捷键映射。
- 扫描/网络任务并发度、超时和刷新策略。
- 默认 sync/pull/rebase/push 参数。
- `$EDITOR`、difftool、mergetool 和 pager 行为。
- 危险操作确认等级。
- 命令白名单和任务日志保留策略。

诊断页显示 repo-tui、Git、Repo 版本，工作区根，终端能力，配置来源和最近 parser/process 错误。`repo-tui doctor` 提供非交互诊断，便于在 TUI 无法启动时排查。

首版不承诺动态插件 ABI。可扩展动作先使用配置化外部命令模板，但变量必须作为独立 argv 展开，并明确标注是否需要 shell。真正插件系统应在领域动作和安全模型稳定后另行设计。

## 12. 性能与兼容性目标

基准环境需在实现阶段固定硬件、Git/Repo 版本和冷/热缓存条件。初始目标：

- 命令校验后 150 ms 内绘制首个可交互 frame。
- 1,000 个 project 的列表发现目标小于 1 秒；状态逐步填充，不阻塞交互。
- SSD 热缓存下 1,000 个 project 全状态扫描 p95 小于 10 秒，可见行优先在 500 ms 内更新。
- 输入到下一次绘制 p95 小于 50 ms。
- Graph 首 200 个 commit 在普通本地仓库中目标小于 300 ms。
- 10,000 个已加载 commit 场景内存目标小于 200 MiB；diff、日志和任务输出均有上限。

兼容策略：

- Linux、macOS 为首发平台；Repo 在原生 Windows 上的可用性和 PTY 差异使其不进入首版承诺，WSL 可按 Linux 路径测试。
- 支持当前仍受上游维护的 Git 版本范围；最低版本根据 porcelain v2 等依赖确定并在构建时写入文档。
- Repo 使用能力探测而非只比较版本字符串。
- 终端最小建议 80x24；过小时显示明确占位提示，但保持退出/帮助可用。
- 处理 resize、无颜色、16 色、Unicode/ASCII symbols、宽字符和组合字符。

## 13. 测试策略

### 13.1 单元测试

- porcelain v2 `-z` 状态解析，包括 rename、untracked、conflict、submodule、unborn、detached。
- refs、log、diff raw/numstat、Repo list/manifest 解析。
- 非 UTF-8、含换行文件名、超长 subject、空仓库、shallow clone。
- commit lane 算法的 golden fixtures。
- reducer、过滤/排序、selection identity 和 generation stale result。
- argv 构造、路径边界、凭据脱敏和风险分类。

### 13.2 集成测试

使用临时目录和真实 `git` 构造：

- clean/dirty/staged/untracked/conflict 状态。
- branch/tag/remote/ahead/behind/diverged/detached/unborn。
- merge、rebase、cherry-pick 进行中及 continue/abort。
- worktree、submodule、LFS（环境可用时）、hooks 失败。
- 命令取消、index lock、进程崩溃、网络/认证 fake remote。

Repo 集成测试使用小型本地 manifest 和本地 bare remotes，避免依赖公网。针对多个 Repo 版本保留输出 fixture 和最小端到端矩阵。

### 13.3 UI 与端到端测试

- Ratatui `TestBackend` 对 80x24、120x40、窄屏和宽屏做 snapshot/golden 测试。
- 测试键盘 focus、modal stack、搜索过滤、批量选择、resize 和帮助。
- PTY 端到端测试启动真实程序，验证 raw mode/alternate screen 在退出、panic、外部编辑器和信号后恢复。
- 大规模 synthetic adapter 模拟 1,000/10,000 project，测量渲染和 reducer 延迟。
- fuzz parser 和 commit graph；外部命令输出属于不可信输入，解析器不得 panic。

## 14. 可观测性与隐私

- 默认日志写到 XDG state 目录，不污染工作区；支持 `RUST_LOG` 和 `--log-file`。
- 日志使用 task/project/operation/generation span，记录耗时、退出码和解析版本。
- 默认不记录完整 diff、commit message、文件内容、环境变量或凭据。
- 原始 stderr 进入受限任务日志前先脱敏；诊断导出需展示将包含的文件并由用户确认。
- 可选匿名遥测默认关闭；若未来引入，需独立设计和明确 opt-in。

## 15. 分阶段实施

### Phase 0：工程基础

- Rust workspace、CLI、日志、终端 RAII、event loop、配置。
- ProcessRunner/GitAdapter/RepoAdapter 接口和 fake 实现。
- Repo/Git 能力检测、统一错误和任务模型。

完成标准：能够安全进入/退出空 TUI，`doctor` 能报告 Git/Repo/终端信息，异常退出测试能恢复终端。

### Phase 1：只读 MVP

- Repo 工作区发现和 manifest project 列表。
- 并发解析 `git status --porcelain=v2 -z`。
- Workspace 状态、HEAD、ahead/behind、搜索/过滤/排序。
- Repository Graph、commit detail 和只读 diff。
- 刷新、任务中心、错误行和基本帮助。

完成标准：在真实 100+ project 工作区中稳定定位 dirty/conflict 仓库并查看提交图；扫描失败不阻塞其他仓库。

### Phase 2：单仓库核心写操作

- file/hunk stage/unstage/restore、commit/amend、stash。
- branch/tag、fetch/pull/push、merge/rebase/cherry-pick/revert。
- 风险确认、实时前置检查、冲突状态与 continue/abort。
- PTY takeover 支持编辑器、认证和交互式 rebase。

完成标准：典型 edit-to-push 流程无需离开 repo-tui，失败时状态和用户输入可恢复。

### Phase 3：Repo 一等工作流

- sync/start/checkout/abandon/prune/rebase。
- upload/download、manifest 导出和逐项目结果。
- 批量选择、workspace lock、失败项重试。

完成标准：多项目操作具备范围预览、进度、取消、部分失败报告和结束后增量刷新。

### Phase 4：完整覆盖与高级能力

- 命令面板原生命令、Repo capability-aware actions。
- reflog/bisect/worktree/submodule/sparse checkout/maintenance 等页面增强。
- 外部 diff/merge tools、保存视图、自定义动作。
- 性能优化、文件 watcher 和更完整的兼容矩阵。

完成标准：未有专用 UI 的合法 Git/Repo 命令可从受控命令面板或 PTY 执行，执行上下文、日志和刷新行为一致。

### Phase 5：发布质量

- 安装包、shell completion、man page、升级与配置迁移。
- 大规模 benchmark、fuzz、跨终端/平台测试。
- 用户文档、故障诊断、隐私与安全审查。

## 16. MVP 验收清单

- 能从任意工作区子目录启动并识别 Repo 根目录。
- 主页列出 manifest 中所有 project，并区分 missing/scan error。
- 每行准确显示 staged、unstaged、untracked、conflict、HEAD 和 upstream ahead/behind。
- 支持搜索、组合过滤、排序和稳定 selection。
- 状态扫描期间键盘和绘制无明显卡顿，可取消或手动刷新。
- 进入仓库后可查看拓扑正确的 commit graph、refs、commit detail 和 diff。
- detached、unborn、shallow、merge commit、非 UTF-8 路径有明确处理。
- 外部命令失败不会导致 TUI 崩溃，错误能关联到对应 project/task。
- 正常退出、panic 和终端接管返回后终端状态可恢复。
- 单元、集成和 TestBackend UI 测试在 CI 中运行。

## 17. 风险与待确认事项

### 17.1 主要风险

1. **Repo 输出稳定性**：不同 Repo 版本的进度和错误输出不完全机器可读。方案是能力探测、保留原始日志、操作后按 Git 真相复扫，而不是脆弱地解析所有显示文本。
2. **数千仓库 I/O 放大**：无界并发会争用磁盘并降低整体速度。需要可调有界并发、可见行优先、增量刷新和基准测试。
3. **跨仓库部分失败**：不能提供虚假的事务语义。必须将逐项目状态和失败重试作为一等体验。
4. **交互命令与终端恢复**：编辑器、认证和信号处理容易破坏 raw mode。需要统一 PTY runner、RAII 和端到端信号测试。
5. **Git 边缘语义**：worktree、submodule、filters、hooks、签名等很难由库完全复刻。以系统 CLI 为权威可降低但不能消除风险。
6. **危险批量操作**：错误范围可能造成大面积数据丢失。需要 stale 检查、影响预览、强确认和默认保守参数。

### 17.2 实现前需要确认的产品决策

- 首版是否只支持 Repo 工作区和单 Git 仓库，还是必须同时支持普通目录递归发现多个 Git 仓库。
- 首发 Git/Repo 最低版本及主要目标发行版。
- `repo upload` 的首要服务端是否为标准 Gerrit；是否需要额外 Gerrit REST 集成。
- 默认快捷键更接近 Vim、Lazygit，还是提供两套 preset。
- MVP 是否包含写操作；本文建议 Phase 1 先交付可靠只读体验，再进入写操作。
- 是否允许 workspace 本地配置定义自定义命令；若允许，必须设计 trust/allow 机制。

## 18. 推荐的首个实现切片

第一个可演示切片应严格纵向打通：

1. 识别 Repo 工作区并通过 `repo list` 获得项目。
2. 后台并发执行 Git status/HEAD 扫描。
3. Workspace 表格展示状态、HEAD 和错误，并支持搜索过滤。
4. Enter 进入仓库，加载前 200 个 commit 的 DAG。
5. Graph 渲染 refs，选择 commit 后显示 metadata 和变更统计。
6. 任务中心可看到扫描任务，失败可重试。

该切片直接验证产品最核心的价值、CLI 数据协议、异步架构和提交图算法，同时不引入写操作风险。通过真实大型 Repo 工作区的性能与可用性验证后，再扩展 Changes 和 Repo 写工作流。

# Git 面板目录树与 diff 浮层 · 设计文档

- 日期：2026-11-10
- 分支：`feat/sidebar_tree_v232`
- 状态：已实现（Task 1-11 落地；2026-11-11 对齐实际触碰面与实现裁定）

## 1. 背景与目标

右侧 Git 抽屉（`PanelView::Git`）目前以平铺列表展示变更文件（文件名 + 灰色父目录），查看单个文件改动需要双击打开 Document 标签页——左侧任务栏会多出一个条目，且终端区被切换走。本设计解决两件事：

1. **Git 目录树**：变更文件按真实目录层级树化展示（本地与远端 SSH Git 同构）。
2. **diff 浮层**：单击文件即在中间终端区上方弹出全幅 diff 对比面板，不新增任务条目、不打断终端任务。

## 2. 需求决策记录

| 决策点 | 结论 |
|---|---|
| 目录树形态 | 保留「暂存的更改 / 更改」分组标题，组内按真实目录层级嵌套展开（可折叠） |
| 树化范围 | 本地 Git 与远端 SSH Git 同构覆盖；Changes 与 Conflicts 视图树化；History 视图、SVN 维持平铺 |
| 点击行为 | 单击文件行 = 弹 diff 浮层（并选中）；双击文件行 = 现有行为不变（本地开文档 / 冲突开 merge editor / 远端开远端文档）；单击目录行 = 折叠/展开 |
| diff 布局 | 默认 side-by-side（左右对比），头部可切换 unified（上下统一）；切换状态跨文件保留 |
| 浮层几何 | 无遮罩；铺满中间区域（中心卡 `absolute().inset_0()`，随卡片圆角裁剪）；顶部标题栏、左侧任务栏、右侧 Git 面板保持可见；终端视图保持挂载 |
| 关闭路径 | ✕ 按钮 / Esc / active tab 变化或 settings 打开（即「点左侧任务栏关闭」） |
| 未跟踪文件 | 全文按新增（绿色）显示；超过 2000 行截断并提示 |
| 冲突文件 | 取「工作区 vs HEAD」，冲突标记原样渲染；双击仍走 merge editor |
| 实现约束 | 增量优先、防上游合并冲突：主体逻辑放新增文件，上游代码文件做最小插入（落地实际：workspace.rs 净零、render.rs +4/−1、vcs_panel.rs +4、list_model.rs +105/−38、rows.rs +47/−1、remote_git.rs +27，合计 +187/−40；i18n 两表各 +13 行） |

## 3. 总体架构

### 3.1 新增文件（fork 侧资产，承载全部主体逻辑）

```
nebula_app/src/gpui_shell/workspace/vcs_panel/
├── diff_overlay.rs            # 浮层视图：头部、布局切换、键盘/Esc、关闭路径、状态机
├── diff_overlay/
│   ├── fetch.rs               # 本地/远端 diff 取数，generation 防串
│   ├── fetch_tests.rs
│   ├── pair.rs                # Patch/Hunk → side-by-side / unified 行配对（适配层）
│   └── tests.rs
├── tree_rows.rs               # GitInfo 变更列表 → 目录树行构建 + 展开状态
└── tree_rows_tests.rs
```

落地形态与最初草案的差别：浮层模块挂在 `vcs_panel/` 下而非 `workspace/` 顶层——架构棘轮不允许 `workspace.rs` 超基线行数，`DiffOverlayState` 由 `VcsList` 持有（浮层是抽屉行的详情面，生命周期与抽屉视图一致），`workspace.rs` 保持净零。

### 3.2 上游文件触碰清单（落地实际规模；树化接入集中在 list_model.rs，非纯 append）

| 文件 | 改动 | 规模 |
|---|---|---|
| `workspace.rs` | 无（浮层模块挂 vcs_panel、状态入 VcsList，行数棘轮净零） | +0/−0 |
| `workspace/render.rs`（中心卡 `.children(content)` 后） | `when_some` 追加浮层 child | +4/−1 |
| `vcs_panel.rs` | 追加 `mod diff_overlay;` 等模块挂载 | +4/−0 |
| `vcs_panel/list_model.rs` | `VcsRow` 枚举追加 `Dir` 变体；`VcsList` 追加 `collapsed_dirs` 与 `diff_overlay` 字段；`sync()` 行构建委托 `tree_rows.rs` | +105/−38 |
| `vcs_panel/rows.rs`（Change 行 `on_click`） | 追加「弹 diff」调用（site/semantics/remote 解析）；原选中行为保留 | +47/−1 |
| `remote_git.rs` | 远端 diff/未跟踪取数命令支持 | +27/−0 |
| i18n（`vcs.diff` 词组） | 追加 11 个词条（纯 append） | 两表各 +13 行 |

防冲突策略：

- 所有插入点选择「行尾追加」或独立 listener/child 挂载，不修改上游既有行。
- diff 取数**不复用/重构** `runtime_api/git.rs`（上游文件）：在 `fetch.rs` 用 `git_worktree` 的 `hidden_git_command` 直接实现，接受约几十行逻辑重复换取零冲突。
- diff 解析**只调用不修改** `native_result_model/diff.rs`；side-by-side 配对放 `pair.rs`。

## 4. Git 目录树

- **数据源**：`GitInfo`（`staged` / `unstaged` / `conflicts` 均为 `(status char, 相对路径)` 列表）。本地来自 `read_git`，远端来自 `remote_git_info`——同一载体，`tree_rows.rs` 不区分来源；`VcsList.remote` 标志继续只影响行内操作（远端 exec）与双击目标。
- **行结构**：每个 section 内，目录行（▾/▸ + 本级目录名 + 文件计数，可折叠）在前，文件行（状态字母 + 文件名）随深度缩进。
- **展开状态**：存于 `VcsList` 内新增 `collapsed_dirs: HashSet<String>`（key = 目录相对路径，同名目录视为同一折叠意图），默认全部展开；不新增 workspace 字段。
- **排序**：目录行在前、同级按名称排序；文件行保持 git status 输出语义。
- **行内操作**：stage / unstage / discard / resolve 等 hover 按钮原样保留在文件行；目录行不带操作按钮。
- **SVN 与 History**：行模型走原 `VcsRow::Change` 平铺路径，不经过 `tree_rows.rs`。

## 5. diff 浮层

### 5.1 状态机（`DiffOverlayState`）

```
Closed
  └─ 单击文件行 → Loading(path, semantics, generation)
        ├─ fetch 成功 → Loaded(rows, mode)
        ├─ fetch 失败/超时 → Error(原因, 重试)
        └─ 再次单击其他文件 → Loading(新 path, generation+1)（旧结果作废）
Loaded/Error 期间单击其他文件 → 同上；单击同一文件 → 无操作
任意态：Esc / ✕ / tab 指纹变化 → Closed（焦点回终端）
```

- `mode: Split | Unified` 属于浮层持久偏好，跨文件、跨会话内保留。
- 语义（semantics）由行来源决定：`unstaged` → 工作区 vs index（`git diff -- <path>`）；`staged` → index vs HEAD（`git diff --cached -- <path>`）；`?` → 未跟踪全文；冲突 → `git diff HEAD -- <path>`。

实现裁定的边界行为：

- 快照 Some→None 直接关闭。
- 文件转入冲突段/离开所在段即关闭（含 untracked 被 add 后）。
- diff 语义冻结在点击时刻（`?`→`M` 后仍为全文视图直至重开）。
- 快照驱动重取会回到 Loading 并重置滚动（后续切片可考虑行数不变时保位）。

### 5.2 视图

- **头部**：状态字母徽标（沿用现有颜色映射）+ 相对路径 + 语义标签 + `SegmentedRow` 布局切换（对比/统一）+ ✕ 关闭按钮。
- **内容区**：`gpui::list` 虚拟列表，固定行高（等宽字体）。Split 模式左右两列（各自独立行号槽，删除/新增行背景着色，上下文行两列同现）；Unified 模式单列 `+/-` 行。左右行数由 `pair.rs` 配对保证一致，单滚动驱动，无独立滚动条同步问题。
- **键盘/焦点**：打开时焦点进浮层 `FocusHandle`；Esc 关闭；↑↓ / PgUp / PgDn / Home / End 滚动；内容支持文本选择复制；关闭后焦点归还终端 pane。

### 5.3 关闭路径实现

- Esc：注册在浮层 focus handle 上，不动全局键表。
- ✕：头部按钮。
- 「点左侧任务栏关闭」：浮层 render 入口比较 tab 指纹（`(active tab index, settings_open)`），变化即自关——sidebar 代码零触碰。

## 6. 数据获取与刷新（`fetch.rs`）

- **本地**：后台线程跑 `git diff --no-color [--cached | HEAD] -- <path>`（`hidden_git_command`）；已跟踪二进制文件由 git diff 输出的 `Binary files … differ` 行识别，显示占位提示；未跟踪文件直接读盘（≤2000 行按新增渲染，超出截断并提示），其二进制判定 = 前 8KB 含 NUL 字节。
- **远端**：`ssh_session::exec_capture` 运行同款 git 命令，预算常量沿用 `remote_git.rs` 的 `STATUS_BUDGET` 模式；远端未跟踪文件改用 `head -c` 经 exec 通道读原文（避开 `git diff --no-index` 退出码 1 丢 stdout 的陷阱），远端 root 冻结在点击时刻为已知局限。
- **超时**：本地/远端均 15s 上限，超时进 Error 态可重试。
- **刷新**：浮层 render 入口用 `Arc::ptr_eq` 比较 Git 快照（与 `VcsList::sync` 同款检测）——快照更新且文件仍在对应列表 → 自动重取；文件已不在 → 自动关闭浮层。
- **生命周期**：generation 归 `DiffOverlayState` 所有，workspace entity 销毁即回收；旧 generation 结果一律丢弃（不覆盖新状态）。

## 7. 国际化

新增 Message ID（落地命名，均纯 append 进枚举与各语言表，回退合同遵循 `docs/internationalization.md`）：

`VcsDiffWorktreeUnstaged`（工作区 · 未暂存）、`VcsDiffStagedHead`（已暂存 · vs HEAD）、`VcsDiffUntracked`（未跟踪）、`VcsDiffConflictHead`（冲突 · vs HEAD）、`VcsDiffModeSplit`（拆分）、`VcsDiffModeUnified`（统一）、`VcsDiffClose`（关闭差异）、`VcsDiffBinary`（二进制文件，不显示差异）、`VcsDiffTooLarge`（内容过大，已截断显示 / "Content too large; display truncated"，无占位符）、`VcsDiffFailed`（无法加载差异）、`VcsDiffNoChanges`（无文本改动）。重试按钮复用现有 `Message::CommonRetry`（与 `common.retry` 同源，避免重复英文源串触发 source_id 歧义）。

## 8. 测试策略

按 `nebula_app/AGENTS.md` 要求覆盖真实控件与布局路径（编译/状态单测不算数）：

- **tree_rows 单测**：树构建、折叠计数、section 隔离、冲突排除、深层路径、排序稳定性；同一 `GitInfo` 同时驱动本地与远端 `VcsList::sync` 验证同构。
- **浮层 workspace 级测试**（沿用 `workspace/tests.rs` 模式）：单击文件行弹层、Loading→Loaded、Esc/✕/切 tab 三路径关闭、布局切换与跨文件保留、面板开着点其他文件原地换内容、快照更新自动重取/文件消失自动关闭、二进制与超大文件占位、错误态重试。
- **fetch 测试**：临时 git 仓库覆盖 staged/unstaged/untracked/binary/deleted 形态；远端用假 exec 通道注入慢响应验证 generation 防串与超时。
- **pair 测试**：hunk → 左右行配对一致性（两列行数相等、行号正确）、unified 行序。
- **视觉验收**：hover 行内操作仍可用、键盘焦点环、Split 对齐。

## 9. 错误处理

- git 缺失 / 仓库消失 / 本地进程失败：Error 面板 + 重试按钮。
- 远端 exec 失败 / 超时：Error 面板 + 重试（预算与错误文案沿用 remote git 现有合同）。
- diff 文本解析失败：降级为 unified 原文显示（不白屏）。

## 10. 非目标（本期不做）

- SVN 的目录树与 diff 浮层。
- History 视图树化。
- 字符级 intra-line 高亮。
- 目录级整组 stage/unstage/discard 操作。
- 浮层内编辑文件、多文件 tab 式 diff 队列。
- WSL 仓库的 diff 取数（本地 pane 命中 WSL 仓库时浮层走原生命令，预期落错误态；后续单独切片）。

## 11. 行数预算与风险

- 单文件控制在 800 行提示线内（超出拆子模块），`scripts/check_architecture.py --base <PR-base>` 验证。已接受的例外：`vcs_panel/diff_overlay.rs` 814 行（迁入 vcs_panel 后路径变长引发的 fmt 拆行所致，非逻辑膨胀）与 `diff_overlay/tests.rs`——均为 guidance 级提示、不阻断门禁；渲染拆分（`diff_overlay/view.rs`）留待该文件下次实质功能改动时顺带处理。
- 主要合并风险集中在上游同期重构 `vcs_panel/list_model.rs` 或中心卡结构；两者均为单点小插入，冲突易手工解决。
- `native_result_model/diff.rs` 解析器若配对信息不足（如缺逐行行号对），`pair.rs` 从 hunk 头行号推导，不修改解析器。

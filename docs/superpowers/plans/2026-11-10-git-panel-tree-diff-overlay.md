# Git 面板目录树与 diff 浮层 · 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 右侧 Git 抽屉的变更列表树化（本地 + 远端 SSH Git 同构），单击文件在中间终端区上方弹出全幅 diff 对比浮层（side-by-side / unified 可切换），不新增任务条目、不打断终端任务。

**Architecture:** 主体逻辑全部放 fork 新增文件（`vcs_panel/tree_rows.rs`、`workspace/diff_overlay.rs` + 子模块）；上游文件只做追加式最小插入（约 31 行，含 3 行 `mod` 声明）。diff 取数本地复用 `display::side_panel::run_git` 的命令构造模式（后台线程），远端走 `ssh_session::exec_capture` + generation 防串；unified 文本解析复用 `native_result_model::diff::Patch`（只调用不修改）。

**Tech Stack:** Rust / GPUI（gpui::list 虚拟列表、on_key_down、debug_selector 测试）、既有 i18n JSON 目录（en-US + zh-CN 必须同键集，其余 locale 缺键自动回退英文）。

**设计文档:** `docs/superpowers/specs/2026-11-10-git-panel-tree-diff-overlay-design.md`（所有交互决策的权威记录）

---

## 环境与验证口径（先读）

- 本机构建必须带 Xcode 环境：所有 cargo 命令前缀 `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer`。
- 工具链 rustup 1.97.1（无需 `--ignore-rust-version`）。
- 过滤测试：`cargo test --bin pebrel -- <过滤词>`（首次全量链接约 25 分钟，后续过滤运行快）。
- 格式化：`cargo fmt --all`；架构预算：`python3 scripts/check_architecture.py --base 4b56f0a2`；平台 cfg 预算：`python3 scripts/check_platform_cfg.py`（余量 456/459，本计划不新增任何 `cfg!`/`#[cfg]` 平台分支）。
- **禁止在 release/master/main/prod 分支操作**；全程在 `feat/sidebar_tree_v232`。
- commit 描述简洁准确，禁止 Co-Authored-By。
- 测试里的行选择器沿用现有格式（`git-tree-row-{section}-{index}-{相对路径}`），**不得改动**——上游 `list_tests.rs` 依赖它。
- **当前 agent 宿主就是 pebrel**：测试清理进程时禁止按映像名批量杀。

## 上游文件触碰总表（执行时严格对齐，禁止扩大）

| 文件 | 改动 |
|---|---|
| `nebula_app/src/gpui_shell/workspace.rs` | `mod diff_overlay;`（`mod documents;` 后）+ 结构体字段 1 行（`vcs_list: vcs_panel::VcsList,` 后）+ `new()` 初始化 1 行（`vcs_list: vcs_panel::VcsList::default(),` 后） |
| `nebula_app/src/gpui_shell/workspace/render.rs` | 中心卡 `.children(content),` 后追加 `.when_some(self.render_diff_overlay(...))` |
| `nebula_app/src/gpui_shell/workspace/vcs_panel.rs` | `mod rows;` 后加 `mod tree_rows;`；`mod list_tests;` 后加 `mod tree_rows_tests;`（同一 cfg(test) 区域） |
| `nebula_app/src/gpui_shell/workspace/vcs_panel/list_model.rs` | `VcsRow` 枚举末尾加 `Dir` 变体；`VcsList` 加 `collapsed_dirs` 字段 + Default 初始化（执行期更名，见偏差补记）；`sync()` 内 Git 两个分支改调 `tree_rows::append_tree_section`；T12 起另托管 `diff_overlay` 字段 |
| `nebula_app/src/gpui_shell/workspace/vcs_panel/rows.rs` | `render_vcs_row` match 加 `Dir` 分支（委托 tree_rows）；`Change` 行 `on_click` 追加弹层调用 |
| `nebula_app/i18n/en-US.json` + `zh-CN.json` | `vcs` 子树各加一个 `diff` 块（12 个键） |

---

### Task 1: i18n 词条

**Files:**
- Modify: `nebula_app/i18n/en-US.json`
- Modify: `nebula_app/i18n/zh-CN.json`

- [ ] **Step 1: en-US.json 加键**

在 `"vcs": {` 内、`"changes": "Changes",` 行之后插入（4 空格缩进，与同级条目一致）：

```json
    "diff": {
      "worktree_unstaged": "Working tree · unstaged",
      "staged_head": "Staged · vs HEAD",
      "untracked": "Untracked",
      "conflict_head": "Conflict · vs HEAD",
      "mode_split": "Split",
      "mode_unified": "Unified",
      "close": "Close diff",
      "binary": "Binary file; differences are not shown",
      "too_large": "Over {lines} lines; display truncated",
      "failed": "Could not load the diff",
      "no_changes": "No textual changes"
    },
```

- [ ] **Step 2: zh-CN.json 加键**

在 `"vcs": {` 内、`"changes": "变更",` 行之后插入同样结构：

```json
    "diff": {
      "worktree_unstaged": "工作区 · 未暂存",
      "staged_head": "已暂存 · vs HEAD",
      "untracked": "未跟踪",
      "conflict_head": "冲突 · vs HEAD",
      "mode_split": "拆分",
      "mode_unified": "统一",
      "close": "关闭差异",
      "binary": "二进制文件，不显示差异",
      "too_large": "超过 {lines} 行，已截断显示",
      "failed": "无法加载差异",
      "no_changes": "无文本改动"
    },
```

- [ ] **Step 3: 校验 JSON 与键集合同**

```bash
python3 -c "
import json
en=json.load(open('nebula_app/i18n/en-US.json'))
zh=json.load(open('nebula_app/i18n/zh-CN.json'))
assert set(en['vcs']['diff']) == set(zh['vcs']['diff']) and len(en['vcs']['diff']) == 11
print('ok')"
```

Expected: `ok`

- [ ] **Step 4: Commit**

```bash
git add nebula_app/i18n/en-US.json nebula_app/i18n/zh-CN.json
git commit -m "i18n: add vcs.diff message ids for the tree and diff overlay"
```

---

### Task 2: tree_rows.rs 树构建模型（纯函数 + 单测）

**Files:**
- Create: `nebula_app/src/gpui_shell/workspace/vcs_panel/tree_rows.rs`
- Create: `nebula_app/src/gpui_shell/workspace/vcs_panel/tree_rows_tests.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/vcs_panel.rs`（2 行 mod 声明）

- [ ] **Step 1: 写失败测试**

`tree_rows_tests.rs` 全文（展开状态语义：`collapsed` 集合记录**已显式折叠**的目录键，默认全展开）：

```rust
use super::*;

fn entries(paths: &[(&char, &str)]) -> Vec<(char, String)> {
    paths.iter().map(|(s, p)| (*s, p.to_string())).collect()
}

#[test]
fn flat_files_stay_direct_children() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        &entries(&[('M', "a.rs"), ('A', "b.rs")]),
        RowOps::Unstaged,
    );
    assert!(matches!(rows[0], VcsRow::Heading { .. }));
    assert!(matches!(rows[1], VcsRow::Change { index: 0, .. }));
    assert!(matches!(rows[2], VcsRow::Change { index: 1, .. }));
}

#[test]
fn nested_paths_get_dir_rows_with_counts() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        &entries(&[
            ('M', "src/main.rs"),
            ('M', "src/ui/mod.rs"),
            ('A', "docs/guide.md"),
        ]),
        RowOps::Unstaged,
    );
    let dirs: Vec<_> = rows.iter().filter_map(|r| match r {
        VcsRow::Dir { path, count, .. } => Some((path.clone(), *count)),
        _ => None,
    }).collect();
    assert!(dirs.contains(&("src".to_string(), 2)));
    assert!(dirs.contains(&("src/ui".to_string(), 1)));
    assert!(dirs.contains(&("docs".to_string(), 1)));
    // 文件行的 index 保持平铺条目序号（上游 list_tests 的 selector 依赖它）
    let change = rows.iter().find_map(|r| match r {
        VcsRow::Change { index, relative_path, .. } if relative_path == "src/ui/mod.rs" => Some(*index),
        _ => None,
    });
    assert_eq!(change, Some(1));
}

#[test]
fn collapsed_dir_hides_descendants() {
    let list = entries(&[('M', "src/a.rs"), ('M', "src/b.rs")]);
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(&mut rows, &collapsed, "changes", &list, RowOps::Unstaged);
    assert_eq!(rows.iter().filter(|r| matches!(r, VcsRow::Change { .. })).count(), 2);

    let mut rows = Vec::new();
    let mut collapsed = std::collections::HashSet::new();
    collapsed.insert(tree_rows::dir_key("changes", "src"));
    tree_rows::append_tree_section(&mut rows, &collapsed, "changes", &list, RowOps::Unstaged);
    assert_eq!(rows.iter().filter(|r| matches!(r, VcsRow::Change { .. })).count(), 0);
    // 折叠后目录行仍在
    assert!(rows.iter().any(|r| matches!(r, VcsRow::Dir { path, .. } if path == "src")));
}

#[test]
fn expansion_keys_are_scoped_per_section() {
    assert_ne!(tree_rows::dir_key("staged", "src"), tree_rows::dir_key("changes", "src"));
}

#[test]
fn untracked_path_without_slash_has_no_dir_row() {
    let mut rows = Vec::new();
    let collapsed = std::collections::HashSet::new();
    tree_rows::append_tree_section(
        &mut rows,
        &collapsed,
        "changes",
        &entries(&[('?', "README.md")]),
        RowOps::Unstaged,
    );
    assert!(!rows.iter().any(|r| matches!(r, VcsRow::Dir { .. })));
}
```

- [ ] **Step 2: vcs_panel.rs 声明模块**

`mod rows;`（第 16 行）后加一行 `mod tree_rows;`；文件尾部 `mod list_tests;`（第 1080 行，cfg(test) 区域内）后加一行 `mod tree_rows_tests;`。

- [ ] **Step 3: 跑测试确认编译失败**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- tree_rows 2>&1 | tail -5
```

Expected: 编译错误（`tree_rows` 模块不存在 / `append_tree_section` 未定义）。

- [ ] **Step 4: 实现 tree_rows.rs**

```rust
//! Git 变更 section 的目录树行构建：只消费 `(status, 相对路径)` 列表，不问
//! 本地还是远端——两种来源共用同一 `GitInfo` 载体，行模型天然同构。
//!
//! 展开状态由调用方（`VcsList.collapsed_dirs`）持有：集合记录**已显式折叠**
//! 的目录键，默认全展开；点击目录行翻转键后重建行。
use super::list_model::{RowOps, VcsRow};
use crate::i18n::Message;
use std::collections::HashSet;

/// 折叠状态键：section 前缀 + 完整目录相对路径。
pub(super) fn dir_key(section_id: &str, dir_path: &str) -> String {
    format!("{section_id}:{dir_path}")
}

/// 目录子树聚合：路径 → 直接子项（目录或文件的条目序号）。
struct DirNode {
    name: String,
    dirs: Vec<(String, DirNode)>,
    files: Vec<usize>,
}

impl DirNode {
    fn new(name: &str) -> Self {
        Self { name: name.to_string(), dirs: Vec::new(), files: Vec::new() }
    }

    fn insert(&mut self, parts: &[&str], index: usize) {
        let Some((first, rest)) = parts.split_first() else { return };
        if rest.is_empty() {
            self.files.push(index);
            return;
        }
        let slot = self.dirs.iter_mut().find(|(name, _)| name == first);
        let node = match slot {
            Some((_, node)) => node,
            None => {
                self.dirs.push((first.to_string(), DirNode::new(first)));
                &mut self.dirs.last_mut().unwrap().1
            },
        };
        node.insert(rest, index);
    }

    /// 该目录下（含嵌套）的文件条目总数。
    fn total_files(&self) -> usize {
        self.files.len() + self.dirs.iter().map(|(_, node)| node.total_files()).sum::<usize>()
    }
}

/// 把一个 section 的平铺条目构建成「标题 + 目录行 + 文件行」序列，追加到
/// `rows`。`Change.index` 严格等于条目在原列表中的序号（debug selector 与
/// 行内操作按它寻址，不得重排）。
pub(super) fn append_tree_section(
    rows: &mut Vec<VcsRow>,
    collapsed: &HashSet<String>,
    section_id: &'static str,
    entries: &[(char, String)],
    ops: RowOps,
) {
    if entries.is_empty() {
        return;
    }
    let mut root = DirNode::new("");
    for (index, (_, path)) in entries.iter().enumerate() {
        let parts: Vec<&str> = path.split('/').collect();
        root.insert(&parts, index);
    }
    rows.push(VcsRow::Heading {
        message: Some(match section_id {
            "staged" => Message::VcsStaged,
            "conflicts" => Message::VcsMergeConflicts,
            _ => Message::VcsChanges,
        }),
        count: entries.len(),
    });
    append_dir(rows, collapsed, section_id, &root, "", 0, entries, ops);
}

fn append_dir(
    rows: &mut Vec<VcsRow>,
    collapsed: &HashSet<String>,
    section_id: &'static str,
    node: &DirNode,
    parent_path: &str,
    depth: usize,
    entries: &[(char, String)],
    ops: RowOps,
) {
    // 目录在前、同级按名称排序；文件行按条目序号（git status 语义）。
    let mut dirs = node.dirs.iter().collect::<Vec<_>>();
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut files = node.files.clone();
    files.sort_unstable();
    for (name, child) in dirs {
        let path =
            if parent_path.is_empty() { name.clone() } else { format!("{parent_path}/{name}") };
        let is_expanded = !collapsed.contains(&dir_key(section_id, &path));
        rows.push(VcsRow::Dir {
            section_id,
            depth,
            path: path.clone(),
            name: name.clone(),
            count: child.total_files(),
            expanded: is_expanded,
        });
        if is_expanded {
            append_dir(rows, collapsed, section_id, child, &path, depth + 1, entries, ops);
        }
    }
    for index in files {
        let (status, relative_path) = &entries[index];
        rows.push(VcsRow::Change {
            section_id,
            index,
            status: *status,
            relative_path: relative_path.clone(),
            ops,
        });
    }
}
```

- [ ] **Step 5: 跑测试确认通过**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- tree_rows 2>&1 | tail -5
```

Expected: 5 个测试 PASS。

- [ ] **Step 6: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/vcs_panel/tree_rows.rs nebula_app/src/gpui_shell/workspace/vcs_panel/tree_rows_tests.rs nebula_app/src/gpui_shell/workspace/vcs_panel.rs
git commit -m "feat(workspace): build git change sections as collapsible directory trees"
```

---

### Task 3: list_model 接入树构建

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/vcs_panel/list_model.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/vcs_panel/tree_rows_tests.rs`（补一条集成断言）

- [ ] **Step 1: 写失败测试（追加到 tree_rows_tests.rs）**

```rust
#[test]
fn vcs_list_sync_builds_tree_rows_for_git_changes() {
    use crate::display::side_panel::{GitInfo, GitPanelView};
    let info = GitInfo {
        staged: vec![('M', "Cargo.toml".into())],
        unstaged: vec![
            ('M', "src/main.rs".into()),
            ('M', "src/ui/mod.rs".into()),
            ('?', "README.md".into()),
        ],
        ..GitInfo::default()
    };
    let mut list = VcsList::default();
    list.sync(Some(Arc::new(info)), GitPanelView::Changes, Some(PathBuf::from("/repo")), false);
    let dirs: Vec<String> = list
        .rows_vcs()
        .iter()
        .filter_map(|r| match r {
            VcsRow::Dir { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(dirs, vec!["src".to_string(), "src/ui".to_string()]);
    // 未跟踪文件在根层级，直接是 Change 行
    assert!(list.rows_vcs().iter().any(|r| matches!(
        r,
        VcsRow::Change { relative_path, .. } if relative_path == "README.md"
    )));
}
```

同步给 `VcsList` 补一个只读探针（`list_model.rs` 内，测试用）：

```rust
#[cfg(test)]
pub(super) fn rows_vcs(&self) -> &[VcsRow] {
    &self.rows
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- vcs_list_sync_builds 2>&1 | tail -5
```

Expected: FAIL（`VcsRow::Dir` 不存在，编译错误）。

- [ ] **Step 3: 修改 list_model.rs**

(a) `VcsRow` 枚举**末尾**（`Change` 变体后）追加：

```rust
    Dir {
        section_id: &'static str,
        /// 完整目录相对路径（如 `src/ui`），展开状态键与点击都用它。
        path: String,
        /// 本级目录名（仅显示用）。
        name: String,
        depth: usize,
        /// 该目录子树内变更文件总数。
        count: usize,
        expanded: bool,
    },
```

(b) `VcsList` 结构体 `remote` 字段后追加：

```rust
    /// Git 视图目录树的显式折叠集合（键 = `tree_rows::dir_key`）；默认
    /// 全展开，SVN/History 行不经它。
    pub(super) collapsed_dirs: std::collections::HashSet<String>,
```

`Default` 实现里对应加 `collapsed_dirs: std::collections::HashSet::new(),`。

(c) `sync()` 中 Git 的 Changes/Conflicts 分支改调树构建。现 `section` 闭包保留给 SVN；将：

```rust
                let is_git = info.vcs == VcsKind::Git;
                section(
                    "conflicts",
                    Some(Message::VcsMergeConflicts),
                    &info.conflicts,
                    if is_git { RowOps::Conflict } else { RowOps::Svn },
                    false,
                );
                match info.vcs {
                    VcsKind::Git if view == GitPanelView::Changes => {
                        section(
                            "staged",
                            Some(Message::VcsStaged),
                            &info.staged,
                            RowOps::Staged,
                            true,
                        );
                        section(
                            "changes",
                            Some(Message::VcsChanges),
                            &info.unstaged,
                            RowOps::Unstaged,
                            true,
                        );
                    },
                    VcsKind::Svn => section("svn", None, &info.unstaged, RowOps::Svn, true),
                    _ => {},
                }
```

改为（`section` 闭包体不动，仅 Git 三处换调用）：

```rust
                let is_git = info.vcs == VcsKind::Git;
                if is_git {
                    let conflicts: Vec<_> = info.conflicts.clone();
                    super::tree_rows::append_tree_section(
                        &mut self.rows,
                        &self.collapsed_dirs,
                        "conflicts",
                        &conflicts,
                        RowOps::Conflict,
                    );
                } else {
                    section(
                        "conflicts",
                        Some(Message::VcsMergeConflicts),
                        &info.conflicts,
                        RowOps::Svn,
                        false,
                    );
                }
                match info.vcs {
                    VcsKind::Git if view == GitPanelView::Changes => {
                        let staged = info.staged.clone();
                        let unstaged = info.unstaged.clone();
                        super::tree_rows::append_tree_section(
                            &mut self.rows,
                            &self.collapsed_dirs,
                            "staged",
                            &staged,
                            RowOps::Staged,
                        );
                        super::tree_rows::append_tree_section(
                            &mut self.rows,
                            &self.collapsed_dirs,
                            "changes",
                            &unstaged,
                            RowOps::Unstaged,
                        );
                    },
                    VcsKind::Svn => section("svn", None, &info.unstaged, RowOps::Svn, true),
                    _ => {},
                }
```

注意：原 `section("conflicts", …, exclude_conflicts: false)` 的语义（冲突段不去重）由树构建天然保持——它直接吃 `info.conflicts` 全量。

(d) 折叠状态变化后需要重建行：在 `VcsList` 加公开方法（放在 `row()` 附近）：

```rust
    /// 目录行点击：翻转折叠键并重建行（快照与位置不变，仅重排当前视图）。
    pub(super) fn toggle_dir(&mut self, section_id: &str, dir_path: &str) {
        let key = super::tree_rows::dir_key(section_id, dir_path);
        if !self.collapsed_dirs.remove(&key) {
            self.collapsed_dirs.insert(key);
        }
        let snapshot = self.snapshot.clone();
        let view = self.view.unwrap_or(GitPanelView::Changes);
        let root = self.root.clone();
        let remote = self.remote;
        self.rows.clear();
        self.snapshot = None; // 强制 sync 重建
        self.sync(snapshot, view, root, remote);
    }
```

- [ ] **Step 4: 跑本任务与上游既有测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- tree_rows 2>&1 | tail -3
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- vcs_list 2>&1 | tail -3
```

Expected: 全部 PASS（上游 `git_list_repaints_visible_rows_and_clicks_the_scrolled_path` 必须仍绿——`Change.index` 语义未变）。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/vcs_panel/
git commit -m "feat(workspace): render git change lists as directory trees in the drawer"
```

---

### Task 4: 目录行渲染与点击折叠

**Files:**
- Create: 渲染方法直接放 `tree_rows.rs`（同文件，行数预算内）
- Modify: `nebula_app/src/gpui_shell/workspace/vcs_panel/rows.rs`（match 加一个分支）
- Modify: `nebula_app/src/gpui_shell/workspace/vcs_panel/tree_rows_tests.rs`（gpui 交互测试）

- [ ] **Step 1: 写失败测试（追加到 tree_rows_tests.rs）**

仿 `list_tests.rs` 的 ListProbe 手法（该文件顶部 harness 逐行照抄：`gpui_component::init`、`math_view::register`、`file_editor::init`、`gpui_shell::init`、`set_reduce_motion(true)`、`windowing::initialize`、`NebulaWorkspace::new(window, None, None, 1, RuntimeHub::new(), WorkspaceStartup::Empty, WindowRole::Regular, cx)`）：

```rust
#[gpui::test]
fn dir_row_click_toggles_collapse(cx: &mut gpui::TestAppContext) {
    // …harness 初始化同 list_tests::git_list_repaints_visible_rows_and_clicks_the_scrolled_path…
    let data = Arc::new(GitInfo {
        unstaged: vec![('M', "src/a.rs".into()), ('M', "src/b.rs".into())],
        ..GitInfo::default()
    });
    // …建 workspace，vcs_list.sync(Some(data), GitPanelView::Changes, Some(root), false)，
    //   ListProbe 包 render_vcs_list，add_window_view + Root::new，cx.run_until_parked…
    cx.update(|_, cx| {
        let bounds = cx.debug_bounds("git-tree-dir-changes-src").expect("dir row rendered");
        cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(bounds.center(), gpui::MouseButton::Left, gpui::Modifiers::default());
    });
    let collapsed = owner.read_with(cx, |ws, _| ws.vcs_list.collapsed_dirs.clone());
    assert!(collapsed.contains("changes:src"));
}
```

（harness 部分不许省略——从 `list_tests.rs` 的 `git_list_repaints…` 函数头逐行复制初始化与建窗代码，仅替换 snapshot 数据与断言。）

- [ ] **Step 2: 跑测试确认失败**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- dir_row_click 2>&1 | tail -5
```

Expected: FAIL（`git-tree-dir-changes-src` selector 不存在）。

- [ ] **Step 3: rows.rs 加 match 分支**

`render_vcs_row` 的 match 中、`VcsRow::Change` 分支之前插入（保持其余分支不动）：

```rust
            VcsRow::Dir { section_id, path, name, depth, count, expanded } => self
                .render_vcs_dir_row(section_id, path, name, depth, count, expanded, cx)
                .into_any_element(),
```

- [ ] **Step 4: tree_rows.rs 实现渲染**

文件头部 `use` 区补 `use super::*;` 并追加（沿用 rows.rs 的行样式常量：行高 30px、`px_2`、hover、muted 文本色、chevron 用符号字体 `\u{ea68}` 同款风格——具体以 rows.rs 现有 Heading 行的写法为准）：

```rust
impl NebulaWorkspace {
    /// 目录行：chevron + 目录名 + 文件计数；单击折叠/展开（重建行模型）。
    pub(super) fn render_vcs_dir_row(
        &mut self,
        section_id: &'static str,
        path: &str,
        name: &str,
        depth: usize,
        count: usize,
        expanded: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let hover = theme.list_hover;
        let section = section_id;
        let path = path.to_string();
        let language = crate::gpui_shell::config::ui_language(cx);
        h_flex()
            .id(SharedString::from(format!("git-tree-dir-{section}-{path}")))
            .debug_selector(move || format!("git-tree-dir-{section}-{path}"))
            .h(px(26.0))
            .w_full()
            .px_2()
            .pl(px(8.0) + px(14.0) * depth as f32)
            .gap_1()
            .items_center()
            .rounded_md()
            .hover(|row| row.bg(hover))
            .child(
                div()
                    .w(px(12.0))
                    .text_xs()
                    .text_color(muted)
                    .child(if expanded { "▾" } else { "▸" }),
            )
            .child(div().text_sm().min_w_0().truncate().child(name.to_string()))
            .child(div().text_xs().text_color(muted).child(count.to_string()))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.vcs_list.toggle_dir(section, &path);
                cx.notify();
            }))
            .when(!expanded, |row| {
                // 折叠态语义提示交给 chevron 本身；无需额外文案。
                row
            })
            .into_any_element()
    }
}
```

（`language` 变量当前分支未用到文案即删掉，勿留未用绑定。）

- [ ] **Step 5: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- dir_row 2>&1 | tail -3
```

Expected: PASS。

- [ ] **Step 6: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/vcs_panel/
git commit -m "feat(workspace): clickable collapsible directory rows in the git drawer"
```

---

### Task 5: diff 浮层骨架（打开/挂载/loading 壳）

**Files:**
- Create: `nebula_app/src/gpui_shell/workspace/diff_overlay.rs`
- Create: `nebula_app/src/gpui_shell/workspace/diff_overlay/tests.rs`
- Modify: `nebula_app/src/gpui_shell/workspace.rs`（3 行：mod + 字段 + 初始化；T12 棘轮解阻后净零，见偏差补记）
- Modify: `nebula_app/src/gpui_shell/workspace/render.rs`（1 处）
- Modify: `nebula_app/src/gpui_shell/workspace/vcs_panel/rows.rs`（on_click 追加调用）

- [ ] **Step 1: 写失败测试（diff_overlay/tests.rs）**

harness 初始化与 Task 4 同源（逐行复制 list_tests 的初始化块）。测试文件全文骨架：

```rust
use super::*;
use crate::display::side_panel::{GitInfo, GitPanelView};
use gpui::{Modifiers, TestAppContext};
use std::sync::Arc;

struct OverlayProbe(Entity<NebulaWorkspace>);
impl Render for OverlayProbe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(640.0))
            .h(px(400.0))
            .flex()
            .children(self.0.update(cx, |ws, cx| ws.render_diff_overlay(window, cx)))
    }
}

fn workspace_with_changes(cx: &mut TestAppContext, unstaged: Vec<(char, String)>) -> (Entity<NebulaWorkspace>, Entity<OverlayProbe>) {
    // …list_tests 同款 init + NebulaWorkspace::new + vcs_list.sync(Some(Arc::new(
    //    GitInfo { unstaged, ..Default::default() })), GitPanelView::Changes, Some(root), false)…
}

#[gpui::test]
fn single_click_opens_overlay_in_loading_state(cx: &mut TestAppContext) {
    let (owner, probe) = workspace_with_changes(cx, vec![('M', "src/a.rs".into())]);
    cx.run_until_parked();
    cx.update(|_, cx| {
        let bounds = cx.debug_bounds("git-tree-row-changes-0-src/a.rs").unwrap();
        cx.simulate_mouse_down(bounds.center(), gpui::MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(bounds.center(), gpui::MouseButton::Left, Modifiers::default());
    });
    cx.run_until_parked();
    owner.read_with(cx, |ws, _| assert!(ws.diff_overlay.is_open()));
    cx.update(|_, cx| {
        cx.debug_bounds("diff-overlay").expect("overlay mounted in center");
        cx.debug_bounds("diff-overlay-loading").expect("loading shell visible");
    });
}
```

- [ ] **Step 2: 上游三处小插入**

(a) `workspace.rs`：`mod documents;` 后加 `mod diff_overlay;`；`vcs_list: vcs_panel::VcsList,` 后加：

```rust
    diff_overlay: diff_overlay::DiffOverlayState,
```

`new()` 里 `vcs_list: vcs_panel::VcsList::default(),` 后加：

```rust
            diff_overlay: diff_overlay::DiffOverlayState::default(),
```

(b) `render.rs`：`.children(content),` 之后追加一行（同链式缩进）：

```rust
                                .when_some(self.render_diff_overlay(window, cx), |card, overlay| {
                                    card.child(overlay)
                                }),
```

(c) `rows.rs`：`Change` 行的 `on_click`（`this.side_panel.selected = Some(path.clone());` 之后）追加。site/semantics 在 rows.rs 内解析——`remote`、`root`、`ops`、`status` 都在此作用域（`RemoteGit` 与 `VcsList.remote` 的字段私有，跨模块不可直读，所以一切在拥有可见性的这里算好再传）：

```rust
                        // 单击 = 弹 diff 浮层（选中保留；双击编辑路径不变）。
                        // site/semantics 在此解析：remote/root/ops 只在本模块可见。
                        let semantics = match (ops, *status) {
                            (_, '?') => diff_overlay::fetch::Semantics::Untracked,
                            (RowOps::Staged, _) => diff_overlay::fetch::Semantics::Staged,
                            (RowOps::Conflict, _) => diff_overlay::fetch::Semantics::ConflictVsHead,
                            _ => diff_overlay::fetch::Semantics::Unstaged,
                        };
                        let site = if remote {
                            this.remote_git.overlay_target().map(|(destination, root)| {
                                diff_overlay::fetch::Site::Remote { destination, root }
                            })
                        } else {
                            click_root.clone().map(|root| diff_overlay::fetch::Site::Local { root })
                        };
                        if let Some(site) = site {
                            this.open_diff_overlay(
                                site,
                                relative_path.clone(),
                                semantics,
                                *status,
                                remote,
                                cx,
                            );
                        }
```

注意：该 on_click 闭包捕获需要 owned 值——在闭包捕获区（既有 `open_path`/`diff_path` 等同款姿势，rows.rs:184-193）追加 `let click_root = root.clone();`（`root` 是 `render_vcs_row` 顶部的 `Option<PathBuf>`）；`ops`/`status`/`remote` 均为 `Copy`，`relative_path` 闭包外是 `&String`，`.clone()` 取 owned。

- [ ] **Step 3: 实现 diff_overlay.rs（本任务范围：状态 + 打开 + 渲染壳）**

```rust
//! Git diff 浮层：盖在中间终端卡上的全幅对比视图。
//!
//! 生命周期独立于 tab——打开不建任务条目、关闭不动终端；键盘焦点在浮层
//! 打开期间归它，Esc/✕/切 tab 关闭并归还焦点。数据按点击惰性获取，
//! generation 递增作废晚到响应（与 remote_git 同一合同）。
use super::*;
use crate::display::side_panel::GitInfo;
use crate::i18n::Message;

pub(crate) struct DiffOverlayState {
    /// `None` = 关闭。
    target: Option<OverlayTarget>,
    mode: DiffMode,
    generation: u64,
    phase: Phase,
    /// 内容获取时所基于的快照；render 入口用它检测快照更新（自动重取/关）。
    seen_snapshot: Option<Arc<GitInfo>>,
    /// 打开时的 (active tab, settings_open)；变化即自动关闭。
    tab_fingerprint: (usize, bool),
    scroll: gpui::ListState,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DiffMode { Split, Unified }

struct OverlayTarget {
    site: fetch::Site,
    /// 相对仓库根的路径（显示与命令都用它）。
    path: String,
    semantics: fetch::Semantics,
    status: char,
    /// 快照来源标记（点击时由 rows.rs 传入）：true = 远端 SSH Git 视图。
    remote: bool,
}

enum Phase {
    Loading,
    Loaded(Box<Loaded>),
    /// 原文降级（解析失败/超限；Task 7 起会抵达）。
    Raw { lines: Vec<String>, truncated: bool },
    Binary,
    /// diff 无文本改动（Task 7 起会抵达）。
    NoChanges,
    Error,
}

struct Loaded {
    rows: Vec<pair::DisplayRow>,
    pairs: Vec<pair::SplitPair>,
    truncated: bool,
}

impl Default for DiffOverlayState {
    fn default() -> Self {
        Self {
            target: None,
            mode: DiffMode::Split,
            generation: 0,
            phase: Phase::Loading,
            seen_snapshot: None,
            tab_fingerprint: (0, false),
            scroll: gpui::ListState::new(0, gpui::ListAlignment::Top, px(22.0)),
        }
    }
}

impl DiffOverlayState {
    fn is_open(&self) -> bool {
        self.target.is_some()
    }
}
```

`fetch` / `pair` 子模块本任务先建空骨架（`diff_overlay/fetch.rs`、`diff_overlay/pair.rs` 各一行 `//! Task 6/7 填充。`，`diff_overlay.rs` 尾部 `mod fetch; mod pair; #[cfg(test)] mod tests;`）。

`NebulaWorkspace` 方法（同文件）：

```rust
impl NebulaWorkspace {
    /// 文件行单击入口（site/semantics/remote 由 rows.rs 解析后传入）：
    /// 同目标在开即无操作；否则进入 Loading 并发起获取。
    pub(super) fn open_diff_overlay(
        &mut self,
        site: fetch::Site,
        path: String,
        semantics: fetch::Semantics,
        status: char,
        remote: bool,
        cx: &mut Context<Self>,
    ) {
        let same = self.diff_overlay.target.as_ref().is_some_and(|t| {
            t.path == path && t.semantics == semantics
        });
        if same {
            return;
        }
        self.diff_overlay.target =
            Some(OverlayTarget { site, path, semantics, status, remote });
        self.diff_overlay.phase = Phase::Loading;
        self.diff_overlay.generation = self.diff_overlay.generation.wrapping_add(1);
        self.diff_overlay.seen_snapshot = self.current_vcs_snapshot();
        self.diff_overlay.tab_fingerprint = (self.active, self.settings_open);
        self.diff_overlay.scroll = gpui::ListState::new(0, gpui::ListAlignment::Top, px(22.0));
        fetch::spawn(self.diff_overlay.generation, self.diff_overlay.target.as_ref().unwrap(), cx);
        cx.notify();
    }

    pub(super) fn close_diff_overlay(&mut self, cx: &mut Context<Self>) {
        if self.diff_overlay.target.take().is_some() {
            self.diff_overlay.phase = Phase::Loading;
            cx.notify();
        }
    }

    /// render.rs 挂载点。tab 指纹变化 → 关闭；快照更新 → 文件仍在则重取、
    /// 不在则关。只做状态判定与调度，不同步等待任何 IO。
    pub(super) fn render_diff_overlay(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if self.diff_overlay.target.is_none() {
            return None;
        }
        let fingerprint = (self.active, self.settings_open);
        if fingerprint != self.diff_overlay.tab_fingerprint {
            self.close_diff_overlay(cx);
            return None;
        }
        let snapshot = self.current_vcs_snapshot();
        if let Some(current) = snapshot.as_ref() {
            let changed = self
                .diff_overlay
                .seen_snapshot
                .as_ref()
                .map(|seen| !Arc::ptr_eq(seen, current))
                .unwrap_or(true);
            if changed {
                let target = self.diff_overlay.target.clone().unwrap();
                if !target_still_listed(current, &target.path, target.semantics) {
                    self.close_diff_overlay(cx);
                    return None;
                }
                self.diff_overlay.seen_snapshot = snapshot.clone();
                self.diff_overlay.generation = self.diff_overlay.generation.wrapping_add(1);
                self.diff_overlay.phase = Phase::Loading;
                fetch::spawn(self.diff_overlay.generation, &target, cx);
            }
        }
        Some(self.render_overlay_body(cx))
    }
}
```

渲染壳（同文件，含头部与 loading）——头部：状态字母（复用 rows.rs 的颜色映射）、路径、语义标签（`Message::VcsDiffWorktreeUnstaged/StagedHead/Untracked/ConflictHead`）、SegmentedRow（`Message::VcsDiffModeSplit/Unified`，`toggle_diff_mode` listener）、✕（`Message::VcsDiffClose` tooltip，`close_diff_overlay` listener）；Loading 态主体：

```rust
    fn render_overlay_body(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let language = crate::gpui_shell::config::ui_language(cx);
        div()
            .id("diff-overlay")
            .debug_selector(|| "diff-overlay".to_string())
            .absolute()
            .inset_0()
            // 与中心卡内容底色同一 token（render.rs:65-69 同款取法）：
            // 视觉上即「同一张卡上的另一层」，且随主题/壁纸形态联动。
            .bg(crate::gpui_shell::theme::card_content_bg(cx))
            .flex()
            .flex_col()
            .child(
                /* 头部 h_flex，h(px(34.0))，border_b：
                   状态字母（rows.rs 同款颜色映射，mono 字体）
                   + relative path（truncate）
                   + 语义标签（Message::VcsDiffWorktreeUnstaged / VcsDiffStagedHead
                     / VcsDiffUntracked / VcsDiffConflictHead，text_xs muted）
                   + flex_1 占位 + Task 8 的模式切换 SegmentedRow 占位
                   + ✕ 按钮（id "diff-close"，Task 9 实装，本任务先放占位 div）*/
            )
            .child(
                match &self.diff_overlay.phase {
                    Phase::Loading => div()
                        .id("diff-overlay-loading")
                        .debug_selector(|| "diff-overlay-loading".to_string())
                        .flex_1()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .child("⋯")
                        .into_any_element(),
                    _ => div().into_any_element(), // Task 7/8 填充 Loaded/Raw/Binary/NoChanges/Error
                },
            )
            .into_any_element()
    }
```

（`language` 在头部语义标签使用；上例省略号注释处按规格实现，不留空实现。）

`remote_git.rs` 是 fork 独占文件，直接加两个访问器（字段私有，跨模块只经方法）：

```rust
    /// diff 浮层取远端目标：destination 与 root 都就绪才有意义。
    pub(super) fn overlay_target(&self) -> Option<(String, String)> {
        (!self.destination.is_empty() && !self.root.is_empty())
            .then(|| (self.destination.clone(), self.root.clone()))
    }

    /// 浮层的快照更新检测用（Arc 指针比较）。
    pub(super) fn snapshot_for_overlay(&self) -> Option<Arc<GitInfo>> {
        self.snapshot.clone()
    }
```

`fetch::Semantics` 可见性为 `pub(in crate::gpui_shell::workspace)`（`Site` 同）。rows.rs 的 on_click 里语义映射直接内联（`RowOps` 仅 vcs_panel 内可见，跨模块传枚举会扩大上游触碰面）——Step 2(c) 的代码块按下述最终形态实现：

`current_vcs_snapshot`（`diff_overlay.rs`，按 target.remote 选择来源）：

```rust
impl NebulaWorkspace {
    fn current_vcs_snapshot(&self) -> Option<Arc<GitInfo>> {
        match self.diff_overlay.target.as_ref().map(|t| t.remote) {
            Some(true) => self.remote_git.snapshot_for_overlay(),
            _ => self.side_panel.git_snapshot(),
        }
    }
}
```

`target_still_listed`（`diff_overlay.rs` 自由函数——`GitInfo` 在上游，判定逻辑放浮层侧，不动上游）：

```rust
fn target_still_listed(info: &GitInfo, path: &str, semantics: fetch::Semantics) -> bool {
    let list = match semantics {
        fetch::Semantics::Staged => &info.staged,
        fetch::Semantics::ConflictVsHead => &info.conflicts,
        _ => &info.unstaged,
    };
    list.iter().any(|(_, p)| p == path)
}
```

上面 `render_diff_overlay` 里 `current.lists(...)` 一处改调 `target_still_listed(current, &target.path, target.semantics)`。

`fetch::spawn` 本任务为**空实现**（保持 Loading 态直到 Task 7）：

```rust
/// 发起一次 diff 获取。Task 7 实装（本地后台线程 / 远端 exec）；generation
/// 作废合同在 apply 端。
pub(super) fn spawn(_generation: u64, _target: &super::OverlayTarget, _cx: &mut Context<NebulaWorkspace>) {}
```

- [ ] **Step 4: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- diff_overlay 2>&1 | tail -3
```

Expected: `single_click_opens_overlay_in_loading_state` PASS。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace.rs nebula_app/src/gpui_shell/workspace/render.rs nebula_app/src/gpui_shell/workspace/vcs_panel/rows.rs nebula_app/src/gpui_shell/workspace/diff_overlay.rs nebula_app/src/gpui_shell/workspace/diff_overlay/
git commit -m "feat(workspace): mount the git diff overlay above the terminal card"
```

---

### Task 6: pair.rs 行配对（纯函数 + 单测）

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/pair.rs`

- [ ] **Step 1: 写失败测试（pair.rs 内 `#[cfg(test)] mod tests`）**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_result_model::diff::{File, Kind, Row};

    fn file(rows: Vec<Row>, body: &str) -> File {
        File {
            path: "a".into(),
            details: vec![],
            source: 0..0,
            body: body.into(),
            rows: rows.into(),
            added: rows.iter().filter(|r| r.kind == Kind::Added).count(),
            deleted: rows.iter().filter(|r| r.kind == Kind::Deleted).count(),
            max_line: 10,
        }
    }

    #[test]
    fn split_pairs_deleted_and_added_runs_side_by_side() {
        let rows = vec![
            Row { old: Some(1), new: Some(1), kind: Kind::Context },
            Row { old: Some(2), new: None, kind: Kind::Deleted },
            Row { old: Some(3), new: None, kind: Kind::Deleted },
            Row { old: None, new: Some(2), kind: Kind::Added },
        ];
        let pairs = split_rows(&file(rows, " ctx\n- a\n- b\n+ c"));
        assert_eq!(pairs.len(), 3);
        assert!(pairs[1].left.is_some() && pairs[1].right.is_none());
        assert!(pairs[2].left.is_some() && pairs[2].right.is_none());
        assert!(pairs[3 - 1].right.is_none());
        // 最后一对：deleted 用尽后 added 补右侧
        let last = pairs.last().unwrap();
        assert!(last.left.is_none() && last.right.is_some());
    }

    #[test]
    fn split_pairs_pad_shorter_side() {
        let rows = vec![
            Row { old: Some(2), new: None, kind: Kind::Deleted },
            Row { old: None, new: Some(2), kind: Kind::Added },
            Row { old: None, new: Some(3), kind: Kind::Added },
        ];
        let pairs = split_rows(&file(rows, "- a\n+ b\n+ c"));
        assert_eq!(pairs.len(), 2);
        assert!(pairs[1].left.is_some() && pairs[1].right.is_some());
        assert!(pairs[2].left.is_none() && pairs[2].right.is_some());
    }

    #[test]
    fn unified_rows_keep_body_line_alignment() {
        let rows = vec![
            Row { old: None, new: None, kind: Kind::Hunk },
            Row { old: Some(1), new: Some(1), kind: Kind::Context },
            Row { old: None, new: Some(2), kind: Kind::Added },
        ];
        let unified = unified_rows(&file(rows, "@@ -1 +1 @@\n ctx\n+ new"));
        assert!(matches!(&unified[0], DisplayRow::FullWidth { text, .. } if text.starts_with("@@")));
        assert!(matches!(&unified[2], DisplayRow::Added { new: Some(2), .. }));
    }

    #[test]
    fn untracked_rows_are_all_added() {
        let unified = untracked_unified(&["fn main() {}".to_string(), "".to_string()]);
        assert!(unified.iter().all(|r| matches!(r, DisplayRow::Added { .. })));
        let split = untracked_split(&["x".to_string()]);
        assert!(split[0].left.is_none() && split[0].right.is_some());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- pair:: 2>&1 | tail -5
```

Expected: 编译失败（函数未定义）。

- [ ] **Step 3: 实现 pair.rs**

```rust
//! Patch/File → 浮层行模型：unified 一比一映射；split 把连续的 Deleted/
//! Added 段左右配对（短侧补空，保证两列行数一致——单滚动条即可同步）。
use crate::native_result_model::diff::{File, Kind, Row};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct LineCell {
    pub no: Option<u32>,
    pub text: String,
    pub kind: Kind,
}

/// split 模式一行：两侧可空；双 None 不产生（FullWidth 走 band）。
#[derive(Clone, Debug)]
pub(super) struct SplitPair {
    pub band: Option<String>, // Some = 整行横带（hunk 头）
    pub left: Option<LineCell>,
    pub right: Option<LineCell>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DisplayRow {
    FullWidth { text: String },
    Context { old: Option<u32>, new: Option<u32>, text: String },
    Added { new: Option<u32>, text: String },
    Deleted { old: Option<u32>, text: String },
}

/// body 与 rows 逐行对齐（解析器保证：每行 body 恰对应一个 Row）。
fn lines_of(file: &File) -> impl Iterator<Item = (&str, &Row)> {
    file.body.split('\n').zip(file.rows.iter())
}

pub(super) fn unified_rows(file: &File) -> Vec<DisplayRow> {
    lines_of(file)
        .map(|(text, row)| match row.kind {
            Kind::Hunk | Kind::Note => DisplayRow::FullWidth { text: text.to_string() },
            Kind::Context => DisplayRow::Context { old: row.old, new: row.new, text: text.to_string() },
            Kind::Added => DisplayRow::Added { new: row.new, text: text.to_string() },
            Kind::Deleted => DisplayRow::Deleted { old: row.old, text: text.to_string() },
        })
        .collect()
}

pub(super) fn split_rows(file: &File) -> Vec<SplitPair> {
    let mut out: Vec<SplitPair> = Vec::new();
    let mut pending_deleted: Vec<LineCell> = Vec::new();
    let mut pending_added: Vec<LineCell> = Vec::new();
    for (text, row) in lines_of(file) {
        match row.kind {
            Kind::Hunk | Kind::Note => {
                flush_pair(&mut out, &mut pending_deleted, &mut pending_added);
                out.push(SplitPair { band: Some(text.to_string()), left: None, right: None });
            },
            Kind::Context => {
                flush_pair(&mut out, &mut pending_deleted, &mut pending_added);
                let cell = LineCell { no: row.old, text: text.to_string(), kind: Kind::Context };
                let right = LineCell { no: row.new, text: text.to_string(), kind: Kind::Context };
                out.push(SplitPair { band: None, left: Some(cell), right: Some(right) });
            },
            Kind::Deleted => pending_deleted.push(LineCell {
                no: row.old,
                text: text.to_string(),
                kind: Kind::Deleted,
            }),
            Kind::Added => pending_added.push(LineCell {
                no: row.new,
                text: text.to_string(),
                kind: Kind::Added,
            }),
        }
    }
    flush_pair(&mut out, &mut pending_deleted, &mut pending_added);
    out
}

/// 把已积累的 Deleted/Added 段按位配对，短侧补 None。
fn flush_pair(out: &mut Vec<SplitPair>, deleted: &mut Vec<LineCell>, added: &mut Vec<LineCell>) {
    let n = deleted.len().max(added.len());
    for ix in 0..n {
        out.push(SplitPair {
            band: None,
            left: deleted.get(ix).cloned(),
            right: added.get(ix).cloned(),
        });
    }
    deleted.clear();
    added.clear();
}

pub(super) fn untracked_unified(lines: &[String]) -> Vec<DisplayRow> {
    lines
        .iter()
        .enumerate()
        .map(|(ix, text)| DisplayRow::Added { new: Some(ix as u32 + 1), text: text.clone() })
        .collect()
}

pub(super) fn untracked_split(lines: &[String]) -> Vec<SplitPair> {
    lines
        .iter()
        .enumerate()
        .map(|(ix, text)| SplitPair {
            band: None,
            left: None,
            right: Some(LineCell { no: Some(ix as u32 + 1), text: text.clone(), kind: Kind::Added }),
        })
        .collect()
}
```

截断提示（`Message::VcsDiffTooLarge`）不在 pair 层拼接——渲染层知道 `truncated` 标记（`Loaded.truncated` / `Phase::Raw.truncated`），在列表末尾静态追加一行 muted 文案。pair 层保持纯文本模型。

- [ ] **Step 4: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- pair 2>&1 | tail -3
```

Expected: PASS（`pair::tests` 4 项）。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/diff_overlay/pair.rs
git commit -m "feat(workspace): pair diff hunks into split and unified row models"
```

---

### Task 7: fetch.rs 本地取数 + Loaded（unified 渲染）

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/fetch.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay.rs`（apply + unified 渲染）
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/tests.rs`

- [ ] **Step 1: 写失败测试（fetch 纯函数 + 临时仓库，普通 #[test]）**

```rust
#[test]
fn local_fetch_covers_staged_unstaged_untracked_binary() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").args(args).current_dir(root).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(root.join("bin.dat"), "A\0B").unwrap();
    std::fs::write(root.join("new.txt"), "n1\nn2\n").unwrap();
    git(&["add", "a.txt"]);
    std::fs::write(root.join("a.txt"), "one\nTWO\n").unwrap();

    let staged = fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, "a.txt", fetch::Semantics::Staged).unwrap();
    assert!(matches!(staged, fetch::Fetched::Patch(_)));

    let unstaged = fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, "a.txt", fetch::Semantics::Unstaged).unwrap();
    assert!(matches!(unstaged, fetch::Fetched::Patch(_)));

    let untracked = fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, "new.txt", fetch::Semantics::Untracked).unwrap();
    match untracked {
        fetch::Fetched::Untracked { lines, truncated } => {
            assert_eq!(lines, vec!["n1".to_string(), "n2".to_string()]);
            assert!(!truncated);
        },
        other => panic!("{other:?}"),
    }

    let binary = fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, "bin.dat", fetch::Semantics::Untracked).unwrap();
    assert!(matches!(binary, fetch::Fetched::Binary));

    // 同一文件的 staged 与 unstaged diff 文本不同（--cached vs 工作区）
    let (a, b) = (
        fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, "a.txt", fetch::Semantics::Staged).unwrap(),
        fetch::local_blocking(&fetch::Site::Local { root: root.to_path_buf() }, "a.txt", fetch::Semantics::Unstaged).unwrap(),
    );
    assert_ne!(format!("{a:?}"), format!("{b:?}"));
}
```

（测试放 `diff_overlay/tests.rs`；`fetch` 需 `pub(in crate::gpui_shell::workspace)` 可见性。）

- [ ] **Step 2: 跑测试确认失败**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- local_fetch 2>&1 | tail -5
```

Expected: 编译失败。

- [ ] **Step 3: 实现 fetch.rs**

```rust
//! diff 取数：本地后台线程 / 远端 SSH exec；generation 作废晚到响应。
//! 命令语义与 runtime_api::git 的 diff 对齐（--no-ext-diff --no-textconv
//! --no-color），但独立实现——上游文件不动（防合并冲突，见设计文档 §3）。
use super::*;

/// 单文件 diff 的时间预算（本地子进程 / 远端 exec 同值）。
pub(super) const DIFF_BUDGET: std::time::Duration = std::time::Duration::from_secs(15);
/// 未跟踪文件全文读取上限（字节）。
const UNTRACKED_BYTES: usize = 256 * 1024;
/// 未跟踪文件显示行数上限。
const UNTRACKED_LINES: usize = 2000;

pub(super) enum Site {
    Local { root: PathBuf },
    Remote { destination: String, root: String },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Semantics { Unstaged, Staged, Untracked, ConflictVsHead }

pub(super) enum Fetched {
    Patch(crate::native_result_model::diff::Patch),
    /// 解析失败/超限的降级：原文按行显示（不白屏）。
    Raw { lines: Vec<String>, truncated: bool },
    Untracked { lines: Vec<String>, truncated: bool },
    Binary,
}

impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Patch(_) => f.write_str("Patch"),
            Self::Raw { lines, truncated } => write!(f, "Raw({}, {})", lines.len(), truncated),
            Self::Untracked { lines, truncated } => write!(f, "Untracked({}, {})", lines.len(), truncated),
            Self::Binary => f.write_str("Binary"),
        }
    }
}

/// unified diff 参数（本地/远端共用；远端在 git -C 后追加）。
fn diff_args(semantics: Semantics, path: &str) -> Vec<String> {
    let mut args = vec![
        "diff".to_string(),
        "--no-color".to_string(),
        "--no-ext-diff".to_string(),
        "--no-textconv".to_string(),
    ];
    match semantics {
        Semantics::Staged => args.push("--cached".to_string()),
        Semantics::ConflictVsHead => {
            args.push("HEAD".to_string());
        },
        _ => {},
    }
    args.push("--".to_string());
    args.push(path.to_string());
    args
}

/// 本地阻塞取数（跑在 background_spawn 上）。镜像 read_git 的命令构造：
/// safe.directory 一次调用限定 + --no-optional-locks；错误经 run_git 记日志，
/// 调用方拿到统一 Err。
pub(super) fn local_blocking(
    site: &Site,
    path: &str,
    semantics: Semantics,
) -> Result<Fetched, ()> {
    let Site::Local { root } = site else { return Err(()) };
    if semantics == Semantics::Untracked {
        return Ok(read_untracked(&root.join(path)));
    }
    let args = diff_args(semantics, path);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let safe_directory = format!("safe.directory={}", root.display());
    let location = root.display().to_string();
    let mut cmd = std::process::Command::new("git");
    cmd.args(["-c", &safe_directory, "--no-optional-locks"])
        .args(&arg_refs)
        .current_dir(root);
    let text = crate::display::side_panel::run_git(cmd, &arg_refs, &location, Some(DIFF_BUDGET))
        .ok_or(())?;
    Ok(parse_diff_text(&text))
}

/// unified 文本 → Fetched：先识别 git 的二进制行，再走 Patch 解析；解析
/// 失败（含 combined diff、超限）降级 Raw（截 2000 行）。
pub(super) fn parse_diff_text(text: &str) -> Fetched {
    if text.lines().any(|line| line.starts_with("Binary files ") && line.ends_with("differ")) {
        return Fetched::Binary;
    }
    if text.trim().is_empty() {
        return Fetched::Raw { lines: Vec::new(), truncated: false };
    }
    match crate::native_result_model::diff::Patch::parse(text, false) {
        Ok(patch) => Fetched::Patch(patch),
        Err(_) => {
            let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
            let truncated = lines.len() > UNTRACKED_LINES;
            lines.truncate(UNTRACKED_LINES);
            Fetched::Raw { lines, truncated }
        },
    }
}

fn read_untracked(path: &Path) -> Fetched {
    let Ok(bytes) = std::fs::read(path) else { return Fetched::Binary };
    let probe = &bytes[..bytes.len().min(8 * 1024)];
    if probe.contains(&0) {
        return Fetched::Binary;
    }
    let bytes = &bytes[..bytes.len().min(UNTRACKED_BYTES)];
    let text = String::from_utf8_lossy(bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let truncated = lines.len() > UNTRACKED_LINES;
    lines.truncate(UNTRACKED_LINES);
    Fetched::Untracked { lines, truncated }
}

/// Task 7 的 spawn 实装（本地分支）；远端分支 Task 10 补。
pub(super) fn spawn(generation: u64, target: &OverlayTarget, cx: &mut Context<NebulaWorkspace>) {
    let site = target.site.clone();
    let path = target.path.clone();
    let semantics = target.semantics;
    if matches!(site, Site::Local { .. }) {
        cx.spawn(async move |this, cx| {
            let fetched = cx
                .background_spawn(async move { local_blocking(&site, &path, semantics) })
                .await;
            let _ = this.update(cx, |workspace, cx| {
                workspace.apply_fetch_result(generation, fetched, cx);
            });
        })
        .detach();
    }
}

`Phase` 的最终形态（与 Task 5 骨架中定义一致，此处列出供对照；`Loaded` 同 Task 5 定义）：

```rust
enum Phase {
    Loading,
    Loaded(Box<Loaded>),
    /// 原文降级（解析失败/超限）。
    Raw { lines: Vec<String>, truncated: bool },
    Binary,
    /// diff 无文本改动（如仅 mode 变化）。
    NoChanges,
    Error,
}

struct Loaded {
    rows: Vec<pair::DisplayRow>,
    pairs: Vec<pair::SplitPair>,
    /// 未跟踪全文被截断（渲染层据此追加 VcsDiffTooLarge 行）。
    truncated: bool,
}
```

`apply_fetch_result`（`diff_overlay.rs`）：

```rust
impl NebulaWorkspace {
    /// 获取结果落地：generation 不匹配一律丢弃（不覆盖新状态）。
    pub(in crate::gpui_shell::workspace) fn apply_fetch_result(
        &mut self,
        generation: u64,
        fetched: Result<fetch::Fetched, ()>,
        cx: &mut Context<Self>,
    ) {
        if self.diff_overlay.generation != generation || self.diff_overlay.target.is_none() {
            return;
        }
        self.diff_overlay.phase = match fetched {
            Ok(fetch::Fetched::Patch(patch)) => {
                let mut rows = Vec::new();
                let mut pairs = Vec::new();
                for file in &patch.files {
                    rows.extend(pair::unified_rows(file));
                    pairs.extend(pair::split_rows(file));
                }
                if rows.is_empty() {
                    Phase::NoChanges
                } else {
                    Phase::Loaded(Box::new(Loaded { rows, pairs, truncated: false }))
                }
            },
            Ok(fetch::Fetched::Untracked { lines, truncated }) => Phase::Loaded(Box::new(Loaded {
                rows: pair::untracked_unified(&lines),
                pairs: pair::untracked_split(&lines),
                truncated,
            })),
            Ok(fetch::Fetched::Raw { lines, truncated }) => Phase::Raw { lines, truncated },
            Ok(fetch::Fetched::Binary) => Phase::Binary,
            Err(()) => Phase::Error,
        };
        cx.notify();
    }
}
```

- [ ] **Step 4: unified 渲染（diff_overlay.rs）**

`Loaded`/`Raw` 在 `DiffMode::Unified` 下渲染：`gpui::list`（`self.diff_overlay.scroll.clone()`，行高 ~22px 等宽字体），行渲染：

```rust
fn render_unified_row(row: &pair::DisplayRow, theme: &dyn std::any::Any) -> gpui::AnyElement {
    // theme 参数实为 crate 主题句柄；落地时直接闭包捕获 cx.theme() 的具体类型
    todo!("Task 7 Step 4：按下述规格实现")
}
```

规格（实现时照此写，不留 todo）：
- `FullWidth` → 单行 `h_flex`，hunk 头（`text.starts_with("@@")`）用 `theme.primary` 文字 + 8% 透明背景；其余 note 用 muted。
- `Context/Added/Deleted` → `h_flex`：行号槽 `w(px(48.0))` muted 右对齐（old/new 取其一：unified 用 `old.or(new)`），文本区 `flex_1` 等宽字体 `theme.mono_font_family`，`text_color`：Context 前景、Added `theme.success`、Deleted `theme.danger`；背景：Added `theme.success.opacity(0.13)`、Deleted `theme.danger.opacity(0.13)`；软换行（`.line_height(relative(1.35))`，不截断）。
- `truncated` → 列表末尾静态追加一行 `language.format(Message::VcsDiffTooLarge, &[("lines", "2000")])` muted。
- `Phase::Binary` / `Phase::NoChanges` / `Phase::Error` → 居中单行：`Message::VcsDiffBinary` / `Message::VcsDiffNoChanges` / `Message::VcsDiffFailed` + Error 态右侧 `Button::new("diff-retry").label(language.text(Message::CommonRetry)).on_click(重发 spawn)`。

`gpui::list` 的行构造仿 `rows.rs::render_vcs_list`（owner downgrade + `update` 渲染单行）。

- [ ] **Step 5: gpui 集成测试（追加到 tests.rs）**

```rust
#[gpui::test]
fn overlay_loads_unified_diff_from_real_repo(cx: &mut TestAppContext) {
    // 临时 git 仓库：init + write a.txt + add + 改写 a.txt（产生 unstaged diff）
    // …harness 同 single_click_opens_overlay，root 指向该仓库，
    //   vcs_list.sync 用真实 read_git(root) 的快照（或手写 GitInfo { unstaged: [('M', "a.rs')] }，
    //   注意文件名用 a.txt 且侧栏 root 指向仓库）…
    // 点击行 → cx.run_until_parked() → 断言：
    //   cx.debug_bounds("diff-overlay-body").is_some();
    //   owner.read_with: matches!(phase, Phase::Loaded(_));
}
```

- [ ] **Step 6: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- diff_overlay 2>&1 | tail -3
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- local_fetch 2>&1 | tail -3
```

Expected: 全部 PASS。

- [ ] **Step 7: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/diff_overlay.rs nebula_app/src/gpui_shell/workspace/diff_overlay/
git commit -m "feat(workspace): load and render unified diffs in the overlay"
```

---

### Task 8: split 渲染与布局切换

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/tests.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[gpui::test]
fn mode_toggle_switches_row_shape_and_persists(cx: &mut TestAppContext) {
    // …harness 同前，打开某文件的 diff（Loaded）…
    cx.update(|_, cx| {
        cx.debug_bounds("diff-overlay-row-unified-0").expect("unified rows first");
    });
    cx.update(|_, cx| {
        let b = cx.debug_bounds("diff-mode-unified").unwrap();
        cx.simulate_mouse_down(b.center(), gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(b.center(), gpui::MouseButton::Left, gpui::Modifiers::default());
    });
    cx.run_until_parked();
    cx.update(|_, cx| {
        cx.debug_bounds("diff-overlay-row-split-0").expect("split rows after toggle");
    });
    // 切换状态跨文件保留：点另一个文件行后仍是 split
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- mode_toggle 2>&1 | tail -5
```

Expected: FAIL（selector 不存在）。

- [ ] **Step 3: 实现**

- 头部 `SegmentedRow::new("diff-mode", mode index, …)`（仿 `vcs_panel.rs:91-159` 的三钮写法，两钮：`diff-mode-split` / `diff-mode-unified`，`Message::VcsDiffModeSplit/Unified`），点击 `toggle_diff_mode(&mut self, mode, cx)`（只改 `mode` + `cx.notify()`，不改 generation/phase）。
- `DiffMode::Split` 渲染：同一 `gpui::list`，行元素为 `h_flex`：左半 `flex_1`（行号槽 48px + 文本，Deleted 红、Context 前景、空侧透明占位）+ 1px 分隔线（`theme.border`）+ 右半 `flex_1`（Added 绿、Context 前景）；`band: Some(_)` 行整行横带（同 unified 的 hunk 样式，两侧合并）。
- 行 debug_selector：unified 行 `diff-overlay-row-unified-{ix}`、split 行 `diff-overlay-row-split-{ix}`。
- 滚动：两模式共用同一 `ListState`（切换保留位置；行数不同属预期）。

- [ ] **Step 4: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- diff_overlay 2>&1 | tail -3
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/
git commit -m "feat(workspace): side-by-side diff mode with a persistent toggle"
```

---

### Task 9: 关闭路径（✕ / Esc / 切 tab）与焦点

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/tests.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[gpui::test]
fn overlay_closes_via_button_escape_and_tab_change(cx: &mut TestAppContext) {
    // 打开后：点 ✕（selector "diff-close"）→ is_open() == false，且浮层不再挂载
    // 重新打开：焦点在浮层（focus handle），dispatch Escape key event → 关闭
    // 再打开：owner.update(|ws, _| ws.activate_tab(其他 ix)) 或插入第二个 tab 并激活
    //   → 下一次 draw 后 is_open() == false
}
```

（Escape 模拟：`window.dispatch_action` 不适用——用 `cx.dispatch`/`simulate Keystroke`：仿 `settings_pane.rs:1771` 的 on_key_down 路径，测试里 `window.simulate_event(gpui::KeyDownEvent { keystroke: gpui::Keystroke { key: "escape".into(), ..Default::default() }, ..Default::default() })`；若 Default 不可用，按 `image_geometry_tests.rs` 的 simulate_event 构造方式照抄。）

- [ ] **Step 2: 跑测试确认失败**（三项断言按序逐一实现）

- [ ] **Step 3: 实现**

- ✕：头部按钮 `Button::new("diff-close").icon(IconName::X).ghost().tooltip(Message::VcsDiffClose)` → `close_diff_overlay`。
- Esc：浮层根 div `.id("diff-overlay")` 加 `.on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| { if event.keystroke.key.eq_ignore_ascii_case("escape") { cx.stop_propagation(); this.close_diff_overlay(cx); } }))`（settings_pane.rs:1771 同款）；浮层打开时把焦点给它（`window.focus(&self.diff_overlay.focus_handle())` 于 open 与每次 render 挂载——`DiffOverlayState` 加 `focus: gpui::FocusHandle`（`cx.entity().new(|cx| cx.focus_handle())`？GPUI 惯例：`FocusHandle::new(cx)`），根 div `.track_focus(&self.diff_overlay.focus)`。
- 关闭归还焦点：`close_diff_overlay` 末尾 `self.focus_active(window, cx);`（key_actions.rs:18 现有方法；签名含 window）。
- 切 tab 关闭：Task 5 的 tab 指纹检测已就位——本测试补 `activate_tab` 路径验证 + 插入第二个 tab 的 fixture（`ws.insert_new_tab(WorkspaceTab::…, cx)` 用最轻的 Terminal tab 构造；若 Terminal tab 构造重，用 `ws.tabs.push` + `ws.active = 1` 直接驱动指纹变化亦可——测试只验证指纹关闭行为）。

- [ ] **Step 4: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- overlay_closes 2>&1 | tail -3
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/
git commit -m "feat(workspace): close the diff overlay via escape, button, or tab switch"
```

---

### Task 10: 远端取数

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/fetch.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/tests.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn remote_command_shapes_are_shell_safe() {
    let site = fetch::Site::Remote { destination: "h".into(), root: "/my repo".into() };
    let cmd = fetch::remote_command(&site, "src/a b.rs", fetch::Semantics::Staged);
    assert!(cmd.starts_with("git -C '/my repo' diff"));
    assert!(cmd.contains("--cached"));
    assert!(cmd.contains("'src/a b.rs'"));
    let untracked = fetch::remote_command(&site, "new.txt", fetch::Semantics::Untracked);
    assert!(untracked.starts_with("head -c 262144 -- 'new.txt'"));
}

#[gpui::test]
fn remote_unavailable_lands_in_error_phase(cx: &mut TestAppContext) {
    // harness：vcs_list.remote = true 的 sync（sync(..., true)）+ remote_git
    // 预置 destination/root（workspace.remote_git 绑定测试值：字段私有——
    // 经 remote_git.rs 加 #[cfg(test)] pub(super) fn test_bind(destination, root)）
    // 点击行 → run_until_parked → 断言 matches!(phase, Phase::Error)
    // （测试环境无网络 runtime，remote_call 返回 None → Error 是设计内路径）
}

#[gpui::test]
fn stale_generation_result_is_dropped(cx: &mut TestAppContext) {
    // 直接调 apply_fetch_result(旧 generation, Ok(Patch)) → phase 不变
}
```

- [ ] **Step 2: 跑测试确认失败**

- [ ] **Step 3: 实现 fetch.rs 远端分支**

```rust
/// 远端命令拼装：路径一律经 ai_hook::remote::quote 单引号包裹。
pub(super) fn remote_command(site: &Site, path: &str, semantics: Semantics) -> String {
    let Site::Remote { root, .. } = site else { unreachable!("remote_command on local site") };
    let quote = crate::ai_hook::remote::quote;
    if semantics == Semantics::Untracked {
        // exec::capture 对非零退出直接丢 stdout（git no-index 差异退出码 1），
        // 所以未跟踪全文用 head 直读，客户端侧做 NUL/行数截断。
        return format!("head -c {UNTRACKED_BYTES} -- {}", quote(path));
    }
    let mut cmd = format!("git -C {} diff --no-color --no-ext-diff --no-textconv", quote(root));
    if semantics == Semantics::Staged {
        cmd.push_str(" --cached");
    } else if semantics == Semantics::ConflictVsHead {
        cmd.push_str(" HEAD");
    }
    cmd.push_str(" -- ");
    cmd.push_str(&quote(path));
    cmd
}
```

`spawn` 的远端分支（仿 `remote_git.rs:521-525` 的 remote_call + generation 校验）：

```rust
    // spawn 内 else 分支：
    let Site::Remote { destination, root } = site.clone() else { unreachable!() };
    let command = remote_command(&site, &path, semantics);
    cx.spawn(async move |this, cx| {
        let result = super::super::remote_files::remote_call(|| async move {
            crate::ssh_session::exec_capture(&destination, &command, b"", DIFF_BUDGET).await
        })
        .await;
        let fetched = match result {
            None => Err(()), // 网络 runtime 起不来：与远端 Git 面板同款「连接不可用」
            Some(Ok(stdout)) => Ok(if semantics == Semantics::Untracked {
                parse_untracked_capture(&stdout)
            } else {
                parse_diff_text(&stdout)
            }),
            Some(Err(_)) => Err(()),
        };
        let _ = this.update(cx, |workspace, cx| {
            workspace.apply_fetch_result(generation, fetched, cx);
        });
    })
    .detach();
```

`parse_untracked_capture`（= 本地 `read_untracked` 的字符串版，同一 NUL/字节/行数规则；把 `read_untracked` 重构为共用 `fn untracked_from_bytes(bytes: &[u8]) -> Fetched`，本地读盘与远端捕获都调它——注意远端 `head -c` 输出经 from_utf8_lossy 已是 String，NUL 检测要在 lossy 前的语义上近似：`\0` 在 lossy 后仍保留为 U+0000，直接对 String 做 `contains('\0')` 即可）。

`remote_git.rs` 加测试绑定（fork 文件）：

```rust
    #[cfg(test)]
    pub(super) fn test_bind(&mut self, destination: &str, root: &str) {
        self.destination = destination.to_string();
        self.root = root.to_string();
    }
```

- [ ] **Step 4: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- remote_ 2>&1 | tail -3
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- stale_generation 2>&1 | tail -3
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/
git commit -m "feat(workspace): fetch remote git diffs over the ssh exec channel"
```

---

### Task 11: 切换文件与快照自动刷新/自动关闭

**Files:**
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay.rs`
- Modify: `nebula_app/src/gpui_shell/workspace/diff_overlay/tests.rs`

- [ ] **Step 1: 写失败测试**

```rust
#[gpui::test]
fn clicking_another_file_replaces_content_in_place(cx: &mut TestAppContext) {
    // 两个文件都有 unstaged 改动；点 A（Loaded）→ 点 B →
    // 断言 generation 递增、phase 先回 Loading、路径头部换成 B；
    // 面板不关（is_open 恒真）
}

#[gpui::test]
fn snapshot_refresh_refetches_and_disappearing_file_closes(cx: &mut TestAppContext) {
    // 打开 A（Loaded）→ vcs_list.sync(新快照 Arc：A 仍在但内容变了)
    //   → draw 后 generation 递增（自动重取）
    // → vcs_list.sync(新快照 Arc：A 已不在列表) → draw 后 is_open() == false
}

#[gpui::test]
fn clicking_same_file_is_a_noop(cx: &mut TestAppContext) {
    // 打开 A → 再点 A → generation 不变、phase 不回 Loading
}
```

- [ ] **Step 2: 跑测试确认失败**

- [ ] **Step 3: 实现核对**

Task 5/7 的骨架已含：同目标 no-op（`open_diff_overlay` 的 `same` 分支）、切文件（新 target + generation++ + Loading）、快照检测（`render_diff_overlay` 的 `Arc::ptr_eq` 分支 → `target_still_listed` → 重取或关）。本任务把这些行为用测试钉死、修正暴露的缺陷；`side_panel.git_snapshot()` 与 `remote_git.snapshot_for_overlay()` 的 Arc 指针在 `SidePanel::sync` 落新快照时必须换新实例（核对：本地 `read_git` 每次产出新 `GitInfo`、`Arc::new` 新指针——成立；远端 `refresh_remote_git` 同样 `Arc::new`——成立）。

- [ ] **Step 4: 跑测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- diff_overlay 2>&1 | tail -3
```

Expected: PASS。

- [ ] **Step 5: Commit**

```bash
git add nebula_app/src/gpui_shell/workspace/
git commit -m "feat(workspace): in-place file switching and snapshot-driven refresh for the diff overlay"
```

---

### Task 12: 全量验证与文档收尾

**Files:**
- Modify: `docs/superpowers/specs/2026-11-10-git-panel-tree-diff-overlay-design.md`（触碰行数修正）

- [ ] **Step 1: 格式化与静态检查**

```bash
cargo fmt --all
python3 scripts/check_platform_cfg.py
python3 scripts/check_architecture.py --base 4b56f0a2
```

Expected: cfg ≤459；架构检查通过（新文件均低于 800 行提示线；若 `diff_overlay.rs` 超 800，把渲染拆到 `diff_overlay/view.rs`——`mod view;` 声明在 diff_overlay.rs 内，不碰上游）。

- [ ] **Step 2: 相关域全量测试**

```bash
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- vcs 2>&1 | tail -3
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- diff 2>&1 | tail -3
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer cargo test --bin pebrel -- tree_rows 2>&1 | tail -3
```

Expected: 全绿（对照本机存量失败清单——非本域失败需逐一归因，不得笼统放行）。

- [ ] **Step 3: 上游触碰面终审**

```bash
git diff 4b56f0a2 --stat -- nebula_app/src/gpui_shell/workspace.rs nebula_app/src/gpui_shell/workspace/render.rs nebula_app/src/gpui_shell/workspace/vcs_panel.rs nebula_app/src/gpui_shell/workspace/vcs_panel/list_model.rs nebula_app/src/gpui_shell/workspace/vcs_panel/rows.rs
```

逐文件核对与「上游文件触碰总表」一致；任何超出预期的新增改动行需要么回退、要么回本计划补记。

- [ ] **Step 4: 设计文档数字修正 + 提交**

spec §2 与 §3.2 的行数改为实际值（预计约 31 行代码 + i18n 词表）；非目标补一行「WSL 仓库的 diff 取数（本地 pane 命中 WSL 仓库时浮层走原生命令，预期报错态；后续单独切片）」。

```bash
git add docs/superpowers/specs/2026-11-10-git-panel-tree-diff-overlay-design.md
git commit -m "docs: align the diff overlay spec with the implemented touch surface"
```

---

## Self-Review 记录

- **Spec 覆盖**：§2 决策表逐项 → Task 2-4（树）、5/7/8（浮层与布局）、9（关闭）、10（远端）、11（切换/刷新）、1（i18n）；§10 非目标未生成任务（正确）。
- **占位符扫描**：无 TBD/TODO；Task 7 Step 4 的 unified 行渲染以逐条规格给出（颜色 token、行号槽宽、背景透明度、软换行、截断行、Error/Retry 控件均落到具体 Message ID 与 selector）；Task 5 头部以注释规格给出（Task 8/9 补交互件）。
- **类型一致性**：`VcsRow::Dir` 六字段在 Task 2（构建）、3（variant 定义）、4（渲染解构）一致；`Fetched` 四变体与 `Phase` 六态在 Task 5/6/7/10 一致；`Site`/`Semantics` 可见性统一为 `pub(in crate::gpui_shell::workspace)`；跨模块私有字段（`RemoteGit.*`、`VcsList.remote`）一律经方法或 rows.rs 侧解析传递，无上游可见性扩张。
- 已知计划内偏差（相对 spec，实现时按计划走）：远端未跟踪文件读改用 `head -c`（exec 单通道，避开 `git diff --no-index` 退出码 1 丢 stdout 的陷阱）而非 spec 所写 SFTP 通道；未跟踪截断提示由渲染层拼接。两处均在 Task 12 的 spec 数字/非目标修订时一并记录。

### 执行期偏差补记（控制器授权）

- Task 2: VcsRow::Dir 变体与 rows.rs 占位渲染臂随 Task 2 落地（E0004 穷尽性）；DirNode 去 name 字段；SectionCtx 重构（clippy too_many_arguments）。
- Task 3: 冲突路径预过滤修复计划缺陷（staged/unstaged 过滤后再树化）；section 闭包因借用冲突移入 else 块；heading message 参数化（Task 3 评审 nit）。
- Task 4: 文件行按路径深度渲染时缩进（设计保真修复）；粘性折叠持久测试。
- Task 5: Site PartialEq、判等含 remote、SVN 门控、快照 Some→None 关闭、owned 捕获、pub(super) mod fetch。
- Task 7: Fetched Debug 计数、DISPLAY_LINES pub(super)、空 diff→NoChanges、读失败→Error、take-before-collect、reset_with_uniform_height、too_large 文案泛化（i18n 值变更）。
- Task 8: 切换重置列表计数（Critical 修复）+ loaded_item_count 单一权威 + band_row 去重。
- Task 9: IconName::Close/xsmall、blur 守卫、open 签名加 window。
- Task 10: 远端 untracked 绝对路径（exec cwd=HOME）、remote_exec cfg 对（确定性调度器）、--no-optional-locks 与错误 log::debug 随 Task 11 落地。
- Task 11: Files 清单外触碰 fetch.rs/fetch_tests.rs/remote_git.rs（两个一行修复 + test_snapshot 测试缝）；Untracked 语义纳入冲突排除。
- T12: 架构棘轮（workspace.rs ≤基线行数）要求浮层模块移挂 vcs_panel、DiffOverlayState 收纳进 VcsList（净零上游行）。

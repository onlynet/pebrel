//! diff 取数：本地走后台线程，远端走 SSH exec 通道（remote_call 桥回 GPUI
//! 执行器，与远端 Git 面板同一模式）。命令语义与 runtime_api::git 的 diff
//! 对齐（--no-ext-diff --no-textconv --no-color），但独立实现——上游文件
//! 不动（防合并冲突，设计文档 §3）。
use super::*;
use crate::ai_hook::remote::quote;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// 单文件 diff 的时间预算（本地子进程；远端同值）。
pub(super) const DIFF_BUDGET: Duration = Duration::from_secs(15);
/// 未跟踪文件全文读取上限（字节）。
const UNTRACKED_BYTES: usize = 256 * 1024;
/// 未跟踪/降级原文显示行数上限。
pub(super) const DISPLAY_LINES: usize = 2000;

#[derive(Clone, PartialEq, Debug)]
pub(in crate::gpui_shell::workspace) enum Site {
    Local { root: PathBuf },
    Remote { destination: String, root: String },
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(in crate::gpui_shell::workspace) enum Semantics {
    Unstaged,
    Staged,
    Untracked,
    ConflictVsHead,
}

/// 与 Site/Semantics 同级可见：apply_fetch_result（pub(in workspace)）签名
/// 里出现，不能比函数更窄（否则私有类型泄漏进公共接口）。
pub(in crate::gpui_shell::workspace) enum Fetched {
    Patch(crate::native_result_model::diff::Patch),
    /// 解析失败/超限的降级：原文按行显示（不白屏）。
    Raw {
        lines: Vec<String>,
        truncated: bool,
    },
    Untracked {
        lines: Vec<String>,
        truncated: bool,
    },
    Binary,
}

impl std::fmt::Debug for Fetched {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // 行数/增删计数入 Debug：测试用 Debug 区分 staged 与 unstaged
            // 两种 Patch，只打印 "Patch" 会把它们判等。
            Self::Patch(patch) => write!(
                f,
                "Patch(files={}, added={}, deleted={})",
                patch.files.len(),
                patch.files.iter().map(|file| file.added).sum::<usize>(),
                patch.files.iter().map(|file| file.deleted).sum::<usize>(),
            ),
            Self::Raw { lines, truncated } => write!(f, "Raw({}, {})", lines.len(), truncated),
            Self::Untracked { lines, truncated } => {
                write!(f, "Untracked({}, {})", lines.len(), truncated)
            },
            Self::Binary => f.write_str("Binary"),
        }
    }
}

/// 远端命令拼装：路径一律经 ai_hook::remote::quote 单引号包裹（与远端
/// Git 面板同一转义规则，空格、`$` 都安全到达远端 shell）；
/// `--no-optional-locks` 与本地孪生（local_blocking）同款——后台读操作
/// 不落 index.lock。未跟踪文件
/// 不走 git diff——捕获层对非零退出直接丢 stdout，而
/// `git diff --no-index` 有差异时退出码为 1，改用 `head -c` 按字节上限
/// 直读全文。远端通道的工作目录是远端 HOME 而非仓库根，路径必须与根
/// 拼成绝对形态（document_target 同款字符串拼接，不经 PathBuf 本地化）。
pub(super) fn remote_command(site: &Site, path: &str, semantics: Semantics) -> String {
    let Site::Remote { root, .. } = site else { unreachable!("remote_command on local site") };
    if semantics == Semantics::Untracked {
        return format!("head -c {UNTRACKED_BYTES} -- {}", quote(&format!("{root}/{path}")));
    }
    let mut cmd = format!(
        "git -C {} --no-optional-locks diff --no-color --no-ext-diff --no-textconv",
        quote(root)
    );
    if semantics == Semantics::Staged {
        cmd.push_str(" --cached");
    } else if semantics == Semantics::ConflictVsHead {
        cmd.push_str(" HEAD");
    }
    cmd.push_str(" -- ");
    cmd.push_str(&quote(path));
    cmd
}

/// 远端结果桥：生产路径经 remote_call 上网络 runtime 跑远端命令捕获
/// （`None` = runtime 起不来，调用方按「连接不可用」处理）。测试环境不
/// 发起远端命令——完成的 tokio 任务会从工作线程唤醒 GPUI 确定性测试
/// 调度器（probe_ssh_group_head 同款取舍），返回 None 走同一错误合同。
#[cfg(not(test))]
async fn remote_exec(destination: &str, command: &str) -> Option<Result<String, ()>> {
    let destination = destination.to_owned();
    let command = command.to_owned();
    // 模块现挂 vcs_panel/diff_overlay 下：remote_files 经 `use super::*` 链
    // （diff_overlay → vcs_panel → workspace）按名可见，无需写全路径。
    remote_files::remote_call(|| async move {
        crate::ssh_session::exec_capture(&destination, &command, b"", DIFF_BUDGET).await.map_err(
            |err| {
                // 折叠进统一 Err 合同前保留原因（run_git 失败日志同款）。
                log::debug!("git diff exec failed in {destination}: {err}");
            },
        )
    })
    .await
}

#[cfg(test)]
async fn remote_exec(_destination: &str, _command: &str) -> Option<Result<String, ()>> {
    None
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
        Semantics::ConflictVsHead => args.push("HEAD".to_string()),
        _ => {},
    }
    args.push("--".to_string());
    args.push(path.to_string());
    args
}

/// 本地阻塞取数（跑在 background_spawn 上）。镜像 read_git 的命令构造：
/// safe.directory 一次调用限定 + --no-optional-locks；错误经 run_git 记日志，
/// 调用方拿到统一 Err。
pub(super) fn local_blocking(site: &Site, path: &str, semantics: Semantics) -> Result<Fetched, ()> {
    let Site::Local { root } = site else { return Err(()) };
    if semantics == Semantics::Untracked {
        return read_untracked(&root.join(path));
    }
    let args = diff_args(semantics, path);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let safe_directory = format!("safe.directory={}", root.display());
    let location = root.display().to_string();
    let mut cmd = std::process::Command::new("git");
    cmd.args(["-c", &safe_directory, "--no-optional-locks"]).args(&arg_refs).current_dir(root);
    let text = crate::display::side_panel::run_git(cmd, &arg_refs, &location, Some(DIFF_BUDGET))
        .ok_or(())?;
    Ok(parse_diff_text(&text))
}

/// unified 文本 → Fetched：先识别 git 的二进制行，再走 Patch 解析；解析
/// 失败（含 combined diff、超限）降级 Raw（截 2000 行）。空输出（无改动）
/// 走空 Patch，落地端归入「无文本改动」而不是空白原文。
pub(super) fn parse_diff_text(text: &str) -> Fetched {
    if text.lines().any(|line| line.starts_with("Binary files ") && line.ends_with("differ")) {
        return Fetched::Binary;
    }
    if text.trim().is_empty() {
        return Fetched::Patch(empty_patch());
    }
    match crate::native_result_model::diff::Patch::parse(text, false) {
        Ok(patch) => Fetched::Patch(patch),
        Err(_) => {
            // 先 take 再 collect：多 MB 的原文不做全量物化，只留上限 + 1 行
            // 供 truncated 判定。
            let mut lines: Vec<String> =
                text.lines().take(DISPLAY_LINES + 1).map(str::to_string).collect();
            let truncated = lines.len() > DISPLAY_LINES;
            lines.truncate(DISPLAY_LINES);
            Fetched::Raw { lines, truncated }
        },
    }
}

fn empty_patch() -> crate::native_result_model::diff::Patch {
    crate::native_result_model::diff::Patch {
        preamble: std::sync::Arc::from(""),
        files: Vec::new(),
    }
}

/// 未跟踪文件全文读取。读失败（文件已消失等）走 Err → Error 态可重试，
/// 不误报「二进制」；先按原字节做 8KB NUL 探针（避免全文扫描），再交给
/// 本地/远端共用的 [`untracked_from_lossy`]。
fn read_untracked(path: &Path) -> Result<Fetched, ()> {
    let bytes = std::fs::read(path).map_err(|_| ())?;
    let full_len = bytes.len();
    let probe = &bytes[..full_len.min(8 * 1024)];
    if probe.contains(&0) {
        return Ok(Fetched::Binary);
    }
    let bytes = &bytes[..full_len.min(UNTRACKED_BYTES)];
    let text = String::from_utf8_lossy(bytes);
    Ok(untracked_from_lossy(&text, full_len))
}

/// 未跟踪全文 → 展示模型（本地字节与远端文本共用）：NUL 判二进制
/// （from_utf8_lossy 保留 NUL 为 U+0000），行数上限截断。`full_len` 是
/// 截断前的完整长度：本地传原始字节长；远端 head -c 已按字节上限截断，
/// 真实长度不可知，输出顶到上限时按「可能还有更多」保守标记。
fn untracked_from_lossy(text: &str, full_len: usize) -> Fetched {
    if text.contains('\0') {
        return Fetched::Binary;
    }
    let mut lines: Vec<String> = text.lines().take(DISPLAY_LINES + 1).map(str::to_string).collect();
    let truncated = lines.len() > DISPLAY_LINES || full_len > UNTRACKED_BYTES;
    lines.truncate(DISPLAY_LINES);
    Fetched::Untracked { lines, truncated }
}

/// 发起一次 diff 获取（本地后台线程 / 远端 SSH 通道）。generation 作废
/// 合同在 apply 端。
pub(super) fn spawn(
    generation: u64,
    target: &super::OverlayTarget,
    cx: &mut Context<NebulaWorkspace>,
) {
    let site = target.site.clone();
    let path = target.path.clone();
    let semantics = target.semantics;
    match site {
        Site::Local { .. } => {
            cx.spawn(async move |this, cx| {
                let fetched = cx
                    .background_spawn(async move { local_blocking(&site, &path, semantics) })
                    .await;
                let _ = this.update(cx, |workspace, cx| {
                    workspace.apply_fetch_result(generation, fetched, cx);
                });
            })
            .detach();
        },
        Site::Remote { .. } => {
            let command = remote_command(&site, &path, semantics);
            let Site::Remote { destination, .. } = site else { unreachable!() };
            cx.spawn(async move |this, cx| {
                let result = remote_exec(&destination, &command).await;
                let fetched = match result {
                    // 网络 runtime 起不来：与远端 Git 面板同款「连接不可用」。
                    None => Err(()),
                    Some(Ok(stdout)) => Ok(if semantics == Semantics::Untracked {
                        // head -c 已按 UNTRACKED_BYTES 截断：输出顶到上限即按
                        // 「还有更多」保守标记（恰好等于上限的文件会多一个
                        // 截断提示，无害）。
                        let full_len = if stdout.len() >= UNTRACKED_BYTES {
                            UNTRACKED_BYTES + 1
                        } else {
                            stdout.len()
                        };
                        untracked_from_lossy(&stdout, full_len)
                    } else {
                        parse_diff_text(&stdout)
                    }),
                    Some(Err(())) => Err(()),
                };
                let _ = this.update(cx, |workspace, cx| {
                    workspace.apply_fetch_result(generation, fetched, cx);
                });
            })
            .detach();
        },
    }
}

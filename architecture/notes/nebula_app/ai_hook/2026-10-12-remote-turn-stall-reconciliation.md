# 远端回合停滞对账（Agent 状态第二重证据）

- Status: Accepted
- Context: SSH 主机上的 AI Agent 回答完成后，侧栏状态停在 Running（转圈
  不熄）。本地 pane 有两重证据：hook + 进程树对账
  （`reconcile_shell_activity` → `apply_activity_evidence`，发现 agent 进程
  消失即收边）；远端 pane 两者皆缺——进程树对 `!is_this_machine()` 早退，
  全生命周期 hook 在场时 `AgentActivity::allows_screen()` 又完全禁用屏幕
  对账。任一 Stop(TurnDone) 丢失即把回合永久钉在 Working。
- Evidence: 远端 hook 的真实丢失路径（`res/hooks/remote_bridge.py`）：通道
  锁 0.3s busy 即丢（`TimeoutError("hook channel is busy")`）、claude hook
  安装 `timeout:3`（`ai_hook/remote.rs`，python3 冷启动慢即超时）、envelope
  超 64KB 丢弃；claude hooks 的覆盖是 Lifecycle+attention
  （`ai_hook/event.rs::capabilities_for`），`allows_screen()` 回合中恒 false。
  `runtime_task_state()` 中 agent status Working 恒映射 Running，远端没有
  任何证据能纠正它。
- Decision: 给远端补**屏幕证据的第二重对账**，语义对齐本地进程树对账的
  「进程消失 ≠ 失败，只收边」：
  1. `AgentActivity` 记录 `last_hook_at`（`apply_hook` 成功路径），新增
     `reconcile_stalled_turn(detection, stalled)`：仅当 `!allows_screen()`
     （正常路径仍走 `observe_screen`）、hook 已静默 ≥ `REMOTE_TURN_STALL`
     （20s）、且屏幕**连续两拍**是 agent 自家 idle 提示符时，按
     `turn_observed` 语义收敛为 Done/Idle。非 idle 检测、未停滞、
     `pending_submit` 都归零连续计数。
  2. 接线在 `refresh_agent_screen_state`（1Hz 看门狗）：远端 pane
     （`!is_this_machine()`）即使 `allows_screen()` 为假也取一份屏幕快照，
     只喂对账、不喂 `observe_screen`。
  3. **不调 `finish_foreground_command`**：那是「agent 进程已消失」的命令
     边界语义（清 `running_program`/会话身份），而远端 CLI 还活着；状态
     迁移留在 `AgentActivity` 内，身份归命令边界（133;D / SessionEnd）管。
  4. 不发完成通知（迟到的真 Stop 会照常到达并补发，`accepts_hook` 的同
     turn 去重兜底）；下一回合 `PromptSubmit` 照常把状态拉回 Working。
- Rejected alternatives:
  - 放开 `allows_screen()` 让屏幕全面参与远端对账：破坏「hook 权威不被屏幕
    改写」的既有合同（本地测试
    `hook_lifecycle_cannot_be_rewritten_by_screen_words_or_idle_samples`
    钉住的行为）；权限询问屏/陪同文本会误判。
  - 提高远端 hook `timeout:3` 与放宽 bridge 锁：改远端安装面（manifest
    资产哈希全体滚动、所有主机要重推），治标不治本；客户端对账兜住后
    不再必须。
  - WSL 走 `for_wsl` 快照的进程树穿透：`\\wsl.localhost` 三条路径实测失败
    （见 `SuggestEnv::Wsl` 的记录），不可行。
- Consequences: 本地 pane 语义零变化（`SuggestEnv::Local` 不进对账分支，
  进程树仍是第二重证据）；WSL pane 与 SSH 同受益（`is_this_machine()` 对
  Wsl/Ssh/Shell 皆 false）。误判防线是「屏幕连续两拍 agent 自家的 idle
  提示符」而非纯计时——长思考/工具执行期间屏幕是转圈（Working 检测），
  权限等待是 Blocked 状态，都不在收敛范围。Blocked 卡死（NeedsAttention
  丢失）不在本期：Working-only。
- Validation: `ai_hook/lifecycle/tests.rs`（`reconcile_stalled_turn` 状态机：
  两拍阈值/停滞门/`pending_submit`/让位屏幕权威）；`terminal/view/
  activity_tests.rs`（`remote_lost_stop_is_reconciled_by_stalled_idle_screen`
  复现丢失 Stop → 停滞 + 两拍 idle 收敛 Done 且身份保留；
  `a_working_screen_resets_the_stall_reconciliation_samples`；
  `local_hook_authority_is_never_eroded_by_stall_reconciliation`）。
- Supersedes: None
- Revisit when: 远端 bridge 改为可靠传输（排队/重试）后，阈值可以收紧或
  退役这条对账；或把 Blocked 卡死也纳入对账时重新评估误判面。
- Superseded by: 2026-10-16-remote-turn-reignite-reconciliation.md（收敛
  方向保留，反方向「两拍转圈重新点火」由后续 note 接管）。

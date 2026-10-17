# 远端回合停滞对账双向化（丢点火事件的重新点火）

- Status: Accepted
- Context: 2026-10-15 用户报告：SSH 主机上的 Claude Code 第一回合状态完整
  （UserPromptSubmit→Running、Stop→ready），从第二回合起侧栏永久钉在
  ready，而屏幕上回合明明在转圈（摘要行 spinner）。上一轮对账
  （2026-10-12 note）只覆盖「丢 Stop」方向；丢 UserPromptSubmit/权限答复
  时没有任何证据能把回合拉回 Working——Lifecycle 覆盖下 `allows_screen()`
  禁用屏幕，`reconcile_stalled_turn` 旧实现只认 idle 形态，Working 检测与
  无匹配走同一条清零路径。
- Evidence: 远端 bridge 的丢失路径同前 note（通道锁 0.3s busy 即丢、hook
  `timeout:3`、envelope 超限）。Claude 回合的点火事件是 UserPromptSubmit
  （→Working）与权限答复 PostToolUse/PermissionResult（Blocked→Working）；
  两者走同一条有损信道。1Hz 看门狗在 `allows_screen()` 为假时本就每秒做
  一次 `detect` 只喂单向收敛（`refresh_agent_screen_state`），采样与计时
  都已存在，缺的只是状态机的反方向分支。
- Decision: 对称补全，零新增采样/计时器/IO：
  1. `reconcile_stalled_turn` 双向化：停滞与 `pending_submit` 前置共用；
     idle 两拍 → Done/Idle（原语义不动）；working 两拍 → 重新点火
     Working 并补记 `turn_observed`（迟到的 Stop 收敛为 Done 而非 Idle）。
     两个方向各自计数（`idle_samples`/`working_samples`），任一拍出现
     对方形态即归零——交替形态不构成「稳定形态」。Blocked/Done/Unknown
     检测两向都归零：权限屏是 attention 语义，归 Notification hook 与
     `observe_screen` 的权威判定管。
  2. 调用点去掉 `status == Working` 单向门：Done/Idle/Blocked 残留同样可
     被两拍转圈拉回 Working（丢权限答复是丢 UserPromptSubmit 的镜像场景）；
     `next == status` 由状态机内部短路，无转换不产生日志或 notify。
  3. `apply_hook`/`input_sent` 同时清两向计数：权威事件回归或用户键入都
     使屏幕对账证据作废。
  4. 身份与完成通知语义不变（同前 note 第 3、4 条：不清 `running_program`
     /会话身份；不发完成通知，迟到的真 hook 照常到达并被同 turn 去重）。
- Rejected alternatives:
  - 只修 Done→Working、不动 Blocked：权限答复与 UserPromptSubmit 走同一
    条有损信道，堵一半等于留着另一半钉死面。
  - 放宽停滞阈值或一拍点火：单帧转圈可能是重绘间隙/滚动残留；两拍 + 20s
    hook 静默是与收敛方向同一量级的误判防线。
  - 修远端安装面（timeout/锁加宽）：见前 note 同款否决理由——manifest
    资产哈希全体滚动、所有主机重推，且治不了已经发生的丢失。
- Consequences: 本地 pane 语义零变化（`reconcile_remote` 门未动）。远端
  回合的 UI 可见性不再依赖单个 hook 事件的存活：屏幕点火 Working 后，迟
  到的真 UserPromptSubmit 到达时 `apply_hook` 照常权威覆盖（同为 Working，
  无害）。残余误判面：Claude 启动动画若持续 >20s 且连续两拍命中 working
  规则，会把 SessionStart-only 会话点火为 Working——启动动画实测秒级消失，
  且第一个真事件到达即被权威覆盖。
- Validation: `ai_hook/lifecycle/tests.rs`
  （`stalled_turn_reconciliation_reignites_a_lost_prompt_submit` 含双向
  闭环与交替形态归零、
  `stalled_turn_reconciliation_reignites_a_blocked_turn_after_the_answer`、
  扩展后的 `stall_reconciliation_honors_the_pending_submit_barrier`）；
  `terminal/view/activity_tests.rs`
  （`remote_lost_prompt_submit_is_reignited_by_stalled_working_screen`：
  完整第一回合 → 丢点火 → hook 时间线未停滞不得点火 → 两拍转圈点火
  Running 且身份保留）。
- Supersedes: 2026-10-12-remote-turn-stall-reconciliation.md（收敛方向与
  全部防线保留，本 note 补齐反方向并接管其语义）。
- Revisit when: 远端 bridge 改为可靠传输（排队/重试）后，两向对账可一起
  退役或收紧阈值；working 检测规则若出现新误判形态（如 IDE 集成屏），优
  先加固规则而不是加计时。

//! Incremental terminal damage tracking.

use std::cmp;
use std::slice;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::index::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineDamageBounds {
    pub line: usize,
    pub left: usize,
    pub right: usize,
}

impl LineDamageBounds {
    #[inline]
    pub fn new(line: usize, left: usize, right: usize) -> Self {
        Self { line, left, right }
    }

    #[inline]
    pub fn undamaged(line: usize, num_cols: usize) -> Self {
        Self { line, left: num_cols, right: 0 }
    }

    #[inline]
    pub fn reset(&mut self, num_cols: usize) {
        *self = Self::undamaged(self.line, num_cols);
    }

    #[inline]
    pub fn expand(&mut self, left: usize, right: usize) {
        self.left = cmp::min(self.left, left);
        self.right = cmp::max(self.right, right);
    }

    #[inline]
    pub fn is_damaged(&self) -> bool {
        self.left <= self.right
    }
}

/// Terminal damage information collected since the last reset.
#[derive(Debug)]
pub enum TermDamage<'a> {
    Full,
    Partial(TermDamageIterator<'a>),
}

/// Iterator over damaged lines that are currently visible in the viewport.
#[derive(Clone, Debug)]
pub struct TermDamageIterator<'a> {
    line_damage: slice::Iter<'a, LineDamageBounds>,
    display_offset: usize,
}

impl<'a> TermDamageIterator<'a> {
    pub fn new(line_damage: &'a [LineDamageBounds], display_offset: usize) -> Self {
        let num_lines = line_damage.len();
        let line_damage = &line_damage[..num_lines.saturating_sub(display_offset)];
        Self { display_offset, line_damage: line_damage.iter() }
    }
}

impl Iterator for TermDamageIterator<'_> {
    type Item = LineDamageBounds;

    fn next(&mut self) -> Option<Self::Item> {
        self.line_damage.find_map(|line| {
            line.is_damaged().then_some(LineDamageBounds::new(
                line.line + self.display_offset,
                line.left,
                line.right,
            ))
        })
    }
}

/// Mutable damage bookkeeping owned by a terminal.
pub(super) struct TermDamageState {
    pub(super) full: bool,
    pub(super) lines: Vec<LineDamageBounds>,
    pub(super) last_cursor: Point,
    /// 渲染世代：**内容突变**单调递增——可打印写入（`write_at_cursor`）、
    /// damage 标记、整屏标记、resize；`reset`（消费）不递增。原子计数让
    /// 渲染端免持 `Term` 锁读「网格变没变」：GPUI 元素路径不消费 damage
    /// 区间，靠世代号做快照缓存键（设计见 nebula_app 侧
    /// `2026-10-12-render-snapshot-generation-cache.md`）。普通字符写入
    /// **不标 damage**（旧壳靠 `damage()` 读取时的光标兜底），世代递增点
    /// 必须直接放 `write_at_cursor`，不能只放 damage 标记——这是 Fleet
    /// 移植评估钉死的回归教训。
    generation: AtomicU64,
}

impl TermDamageState {
    pub(super) fn new(num_cols: usize, num_lines: usize) -> Self {
        let lines =
            (0..num_lines).map(|line| LineDamageBounds::undamaged(line, num_cols)).collect();
        Self {
            full: true,
            lines,
            last_cursor: Default::default(),
            generation: AtomicU64::new(0),
        }
    }

    /// 当前渲染世代（纯读取）。
    #[inline]
    pub(super) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Relaxed)
    }

    /// 世代递增（内容突变）。取 `&self`：`write_at_cursor` 在格子可变借用
    /// 存续期间也能调用。
    #[inline]
    pub(super) fn bump_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub(super) fn resize(&mut self, num_cols: usize, num_lines: usize) {
        self.last_cursor = Default::default();
        self.full = true;
        self.bump_generation();
        self.lines.clear();
        self.lines.reserve(num_lines);
        for line in 0..num_lines {
            self.lines.push(LineDamageBounds::undamaged(line, num_cols));
        }
    }

    /// 整屏标记（含 alt-screen 互换、滚动）：内容世界换了，世代必须走。
    #[inline]
    pub(super) fn mark_full(&mut self) {
        self.full = true;
        self.bump_generation();
    }

    #[inline]
    pub(super) fn damage_point(&mut self, point: Point<usize>) {
        self.damage_line(point.line, point.column.0, point.column.0);
    }

    #[inline]
    pub(super) fn damage_line(&mut self, line: usize, left: usize, right: usize) {
        self.lines[line].expand(left, right);
        self.bump_generation();
    }

    pub(super) fn reset(&mut self, num_cols: usize) {
        // 消费不是突变：世代不变，缓存结论仍然有效。
        self.full = false;
        self.lines.iter_mut().for_each(|line| line.reset(num_cols));
    }
}

impl<T> super::Term<T> {
    /// 网格内容的渲染世代：可打印写入、damage 标记、整屏标记（滚动/换屏）
    /// 与 resize 后单调递增；纯读取与 `reset_damage` 不变。渲染端用它做
    /// 快照缓存键——免持 `Term` 锁即可判断「网格变没变」。
    pub fn render_generation(&self) -> u64 {
        self.damage.generation()
    }
}

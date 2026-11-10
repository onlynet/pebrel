//! Patch/File → 浮层行模型（Task 6 实装）。
//!
//! unified 一比一映射；split 把连续的 Deleted/Added 段左右配对——短侧补空，
//! 两列行数由此恒等，单滚动条即可同步（设计 §5.2）。
use crate::native_result_model::diff::{File, Kind, Row};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum DisplayRow {
    FullWidth { text: String },
    Context { old: Option<u32>, new: Option<u32>, text: String },
    Added { new: Option<u32>, text: String },
    Deleted { old: Option<u32>, text: String },
}

#[derive(Clone, Debug)]
pub(super) struct LineCell {
    pub no: Option<u32>,
    pub text: String,
    pub kind: crate::native_result_model::diff::Kind,
}

#[derive(Clone, Debug)]
pub(super) struct SplitPair {
    pub band: Option<String>,
    pub left: Option<LineCell>,
    pub right: Option<LineCell>,
}

/// body 与 rows 逐行对齐（解析器保证：每行 body 恰对应一个 Row）。
fn lines_of(file: &File) -> impl Iterator<Item = (&str, &Row)> {
    file.body.split('\n').zip(file.rows.iter())
}

pub(super) fn unified_rows(file: &File) -> Vec<DisplayRow> {
    lines_of(file)
        .map(|(text, row)| match row.kind {
            Kind::Hunk | Kind::Note => DisplayRow::FullWidth { text: text.to_string() },
            Kind::Context => {
                DisplayRow::Context { old: row.old, new: row.new, text: text.to_string() }
            },
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
                out.push(SplitPair {
                    band: None,
                    left: Some(LineCell {
                        no: row.old,
                        text: text.to_string(),
                        kind: Kind::Context,
                    }),
                    right: Some(LineCell {
                        no: row.new,
                        text: text.to_string(),
                        kind: Kind::Context,
                    }),
                });
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

/// 未跟踪文件全文按新增行（行号从 1 起，右侧列）。截断提示由渲染层追加。
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
            right: Some(LineCell {
                no: Some(ix as u32 + 1),
                text: text.clone(),
                kind: Kind::Added,
            }),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_result_model::diff::{File, Kind, Row};

    fn file(rows: Vec<Row>, body: &str) -> File {
        let added = rows.iter().filter(|r| r.kind == Kind::Added).count();
        let deleted = rows.iter().filter(|r| r.kind == Kind::Deleted).count();
        File {
            path: "a".into(),
            details: vec![],
            source: 0..0,
            body: body.into(),
            rows: rows.into(),
            added,
            deleted,
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
        // context 行两侧同现
        assert!(pairs[0].left.is_some() && pairs[0].right.is_some());
        // deleted/added 段按位配对：第一对两侧都有
        assert!(pairs[1].left.is_some() && pairs[1].right.is_some());
        // deleted 段更长：末对右侧补空
        assert!(pairs[2].left.is_some() && pairs[2].right.is_none());
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
        assert!(pairs[0].left.is_some() && pairs[0].right.is_some());
        // added 段更长：左侧补空
        assert!(pairs[1].left.is_none() && pairs[1].right.is_some());
    }

    #[test]
    fn hunk_header_flushes_pending_runs_before_band() {
        let rows = vec![
            Row { old: Some(1), new: None, kind: Kind::Deleted },
            Row { old: None, new: None, kind: Kind::Hunk },
            Row { old: None, new: Some(1), kind: Kind::Added },
        ];
        let pairs = split_rows(&file(rows, "- a\n@@ -1 +1 @@\n+ b"));
        assert_eq!(pairs.len(), 3);
        // hunk 行触发 flush：已积累的 deleted 行配对在 band 之前，不被挤到后面
        assert!(pairs[0].left.is_some() && pairs[0].right.is_none());
        assert!(pairs[1].band.is_some() && pairs[1].left.is_none());
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
        assert!(matches!(&unified[0], DisplayRow::FullWidth { text } if text.starts_with("@@")));
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

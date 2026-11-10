//! Patch/File → 浮层行模型（Task 6 实装）。

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

//! 搜索引擎核心模块
//!
//! 包含配置解析、文件遍历、文本匹配、引擎编排、行类型识别五个子模块。
//! 对外暴露 `SearchConfig`、`SearchEngine`、`SearchMatch`、`LineKind` 四个核心类型。

pub mod config;
pub mod context;
pub mod engine;
pub mod line_kind;
pub mod matcher;
pub mod walker;

pub use config::SearchConfig;
pub use engine::SearchEngine;
pub use line_kind::LineKind;
pub use matcher::SearchMatch;

//! rust-search:高性能全局文本搜索引擎 Rust 核心
//!
//! 本 crate 编译为 cdylib 供 IntelliJ 插件通过 JNI 调用,实现 Find in Path 功能。
//! 内部分三层:
//! - `search`:搜索引擎核心(配置、遍历、匹配、引擎编排)
//! - `jni`:JNI 适配层(入口函数、类型转换、结果构建)
//! - `util`:工具层(路径处理、平台抽象)
//!
//! 关键设计:文件级并行 + crossbeam-channel 流式输出 + AtomicBool 取消机制,
//! 严格遵循 JNI 引用管理规范避免内存泄漏。

pub mod error;
pub mod jni;
pub mod search;
pub mod util;

pub use error::{SearchError, SearchResult};
pub use search::{SearchConfig, SearchEngine, SearchMatch};

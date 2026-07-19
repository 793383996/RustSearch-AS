//! 统一错误类型定义
//!
//! 所有模块的错误统一转换为 `SearchError`,通过 thiserror 派生 Display 与 Error trait,
//! JNI 层捕获后转换为 Java 异常抛出。

use thiserror::Error;

/// 搜索错误类型,覆盖配置、IO、正则、JNI、取消等场景
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("无效的搜索模式: {0}")]
    InvalidPattern(String),

    #[error("根目录不存在或不可访问: {0}")]
    InvalidRoot(String),

    #[error("正则表达式编译失败: {0}")]
    RegexCompile(String),

    #[error("JNI 交互错误: {0}")]
    Jni(String),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("搜索已取消")]
    Cancelled,

    #[error("内部错误: {0}")]
    Internal(String),
}

/// 统一 Result 别名,便于全模块使用
pub type SearchResult<T> = std::result::Result<T, SearchError>;

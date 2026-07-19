//! JNI 适配层
//!
//! 负责 JVM 与 Rust 之间的类型转换、JNI 入口函数声明、结果对象构建。
//! 严格遵循 JNI 引用管理规范:所有 Local Reference 用 `auto_local` 包裹自动释放,
//! 禁止跨 JNI 调用边界持有 JVM 对象引用。
//!
//! 所有 JNI 入口函数用 `catch_unwind` 包裹,防止 panic 跨边界触发未定义行为。

pub mod bridge;
pub mod convert;
pub mod result;

package com.example.rustsearch

/**
 * 搜索异常
 *
 * 对应 Rust 侧 `convert.rs` 中 `throw_java_exception` 抛出的异常类型。
 * 全限定名 `com.example.rustsearch.SearchException` 必须与 Rust 侧一致,
 * 否则 JNI 抛出的异常无法被 Kotlin 正确捕获。
 *
 * Rust 侧通过 `env.throw_new("com/example/rustsearch/SearchException", &msg)` 抛出,
 * 消息内容来自 [SearchError] 的 Display 实现。
 *
 * @param message 错误描述,来自 Rust 侧 SearchError.to_string()
 */
class SearchException(message: String) : RuntimeException(message)

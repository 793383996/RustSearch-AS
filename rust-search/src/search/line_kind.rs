//! 行类型识别(注释/import/package/code)
//!
//! v1.2.0 新增:支持按行类型过滤搜索结果。
//!
//! 简单前缀匹配,识别行首(去空格后)的特征字符。
//! MVP 实现,不追求 100% 准确,覆盖主流语言即可:
//! - 注释://、#、/*、*、<!--、--
//! - package:package 关键字(Java/Kotlin/Go)
//! - import:import、#include、include、require、using、from
//! - 其他:Code
//!
//! 识别后由 matcher.rs 的 MatchSink::matched 根据 SearchConfig 的
//! skip_comments/skip_imports/skip_packages 标志决定是否跳过该行。

/// 行类型枚举
///
/// 序数值与 Kotlin 侧 `com.example.rustsearch.RustSearchEngine.LineKind` 严格对齐:
/// - 0 = Code
/// - 1 = Comment
/// - 2 = Import
/// - 3 = Package
///
/// JNI 传递时用 `as jint` 转为序数,Kotlin 侧用 `LineKind.fromOrdinal(v)` 转换。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// 普通代码行
    Code = 0,
    /// 注释行(//、#、/*、*、<!--、--)
    Comment = 1,
    /// import 行(import、#include、include、require、using、from)
    Import = 2,
    /// package 行(Java/Kotlin/Go 的 package 声明)
    Package = 3,
}

impl Default for LineKind {
    fn default() -> Self {
        LineKind::Code
    }
}

/// 识别行类型:去行首空格后按前缀匹配
///
/// # 参数
/// - `line`:匹配行的完整内容(可能含前导空格)
///
/// # 返回
/// 行类型枚举(`Code` / `Comment` / `Import` / `Package`)
///
/// # 识别规则(按优先级)
/// 1. package:行首以 `package ` 开头(注意尾随空格,避免匹配 `packageInfo` 等)
/// 2. import:行首以 `import `、`#include`、`include `、`require `、`using `、`from ` 开头
///    (优先于注释,因 `#include` 以 `#` 开头会被注释规则误判)
/// 3. 注释:行首(去空格)以 `//`、`#`、`/*`、`*`、`<!--`、`--` 开头
/// 4. 其他:Code
///
/// # 限制
/// - 块注释中间行(以 `*` 开头)会被识别为 Comment,但普通乘法表达式 `* a * b` 也会被误判
/// - SQL/Lua 的 `--` 注释与 Haskell 的 `--` 负号有冲突,Haskell 代码可能误判
/// - 字符串内的 `#` 不会被识别(简单前缀匹配不看上下文)
pub fn classify_line(line: &str) -> LineKind {
    let trimmed = line.trim_start();

    // 1. package 行(Java/Kotlin/Go)
    // 注意:必须带尾随空格,避免匹配 packageInfo、packageName 等标识符
    if trimmed.starts_with("package ") {
        return LineKind::Package;
    }

    // 2. import 行(多语言)
    // 优先于注释判断,因 `#include` 以 `#` 开头会被注释规则误判
    if trimmed.starts_with("import ")
        || trimmed.starts_with("#include")
        || trimmed.starts_with("include ")
        || trimmed.starts_with("require ")
        || trimmed.starts_with("using ")
        || trimmed.starts_with("from ")
    {
        return LineKind::Import;
    }

    // 3. 注释(单行)://、#、/*、*、<!--、--
    if trimmed.starts_with("//")
        || trimmed.starts_with('#')
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
        || trimmed.starts_with("<!--")
        || trimmed.starts_with("--")
    {
        return LineKind::Comment;
    }

    LineKind::Code
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_code() {
        assert_eq!(classify_line("fun main() {}"), LineKind::Code);
        assert_eq!(classify_line("    val x = 1"), LineKind::Code);
        assert_eq!(classify_line("public class Foo"), LineKind::Code);
        assert_eq!(classify_line(""), LineKind::Code);
    }

    #[test]
    fn test_classify_comment() {
        assert_eq!(classify_line("// 单行注释"), LineKind::Comment);
        assert_eq!(classify_line("    // 缩进注释"), LineKind::Comment);
        assert_eq!(classify_line("# Shell/Python 注释"), LineKind::Comment);
        assert_eq!(classify_line("/* 块注释开始"), LineKind::Comment);
        assert_eq!(classify_line(" * 块注释中间行"), LineKind::Comment);
        assert_eq!(classify_line("<!-- HTML 注释"), LineKind::Comment);
        assert_eq!(classify_line("-- SQL/Lua 注释"), LineKind::Comment);
    }

    #[test]
    fn test_classify_package() {
        assert_eq!(classify_line("package com.example.foo"), LineKind::Package);
        assert_eq!(classify_line("    package com.example.bar"), LineKind::Package);
        // 不匹配:无尾随空格(避免误判 packageInfo 等)
        assert_eq!(classify_line("packageInfo"), LineKind::Code);
    }

    #[test]
    fn test_classify_import() {
        assert_eq!(classify_line("import java.util.List"), LineKind::Import);
        assert_eq!(classify_line("    import kotlin.coroutines.*"), LineKind::Import);
        assert_eq!(classify_line("#include <stdio.h>"), LineKind::Import);
        assert_eq!(classify_line("#include \"header.h\""), LineKind::Import);
        assert_eq!(classify_line("include <iostream>"), LineKind::Import);
        assert_eq!(classify_line("require 'logger'"), LineKind::Import);
        assert_eq!(classify_line("using namespace std;"), LineKind::Import);
        assert_eq!(classify_line("from typing import List"), LineKind::Import);
    }

    #[test]
    fn test_line_kind_ordinal() {
        assert_eq!(LineKind::Code as u8, 0);
        assert_eq!(LineKind::Comment as u8, 1);
        assert_eq!(LineKind::Import as u8, 2);
        assert_eq!(LineKind::Package as u8, 3);
    }
}

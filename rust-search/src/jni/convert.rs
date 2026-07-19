//! JVM ↔ Rust 类型转换
//!
//! 提供 JNI 类型(JString/JObjectArray)与 Rust 原生类型(String/Vec)之间的转换。
//! 所有转换函数内部使用 auto_local 管理中间引用,防止内存泄漏。

use std::path::PathBuf;

use jni::objects::{JObjectArray, JString};
use jni::sys::{jboolean, jint};
use jni::JNIEnv;

use crate::error::SearchError;

/// JString → Rust String
pub fn jstring_to_rust(env: &mut JNIEnv, jstr: &JString) -> Result<String, SearchError> {
    let java_str = env
        .get_string(jstr)
        .map_err(|e| SearchError::Jni(format!("获取字符串失败: {e}")))?;
    // JavaStr 通过 Deref 到 JNIStr,JNIStr 提供 to_str()
    let s = java_str
        .to_str()
        .map_err(|e| SearchError::Jni(format!("字符串 UTF-8 转换失败: {e}")))?;
    Ok(s.to_owned())
}

/// Rust String → JString
pub fn rust_to_jstring<'local>(
    env: &mut JNIEnv<'local>,
    s: &str,
) -> Result<JString<'local>, SearchError> {
    env.new_string(s)
        .map_err(|e| SearchError::Jni(format!("创建字符串失败: {e}")))
}

/// JObjectArray(字符串数组) → Vec<String>
/// 本地引用在 JNI 调用结束时由 JVM 统一释放,数组元素数量有限无需 auto_local
pub fn jstring_array_to_vec(
    env: &mut JNIEnv,
    arr: &JObjectArray,
) -> Result<Vec<String>, SearchError> {
    let len = env
        .get_array_length(arr)
        .map_err(|e| SearchError::Jni(format!("获取数组长度失败: {e}")))?;

    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len {
        let elem = env
            .get_object_array_element(arr, i)
            .map_err(|e| SearchError::Jni(format!("获取数组元素失败: {e}")))?;

        let jstr: JString = elem.into();
        let s = jstring_to_rust(env, &jstr)?;
        result.push(s);
    }
    Ok(result)
}

/// jboolean → bool
#[inline]
pub fn jboolean_to_bool(val: jboolean) -> bool {
    val != 0
}

/// jint → usize
#[inline]
pub fn jint_to_usize(val: jint) -> usize {
    if val < 0 {
        0
    } else {
        val as usize
    }
}

/// 将搜索参数从 JNI 类型转换为 SearchConfig
pub fn build_config_from_jni(
    env: &mut JNIEnv,
    roots_arr: &JObjectArray,
    pattern: &JString,
    is_regex: jboolean,
    case_sensitive: jboolean,
    whole_words: jboolean,
    include_globs_arr: &JObjectArray,
    exclude_globs_arr: &JObjectArray,
    context_lines: jint,
) -> Result<crate::SearchConfig, SearchError> {
    let roots_strs = jstring_array_to_vec(env, roots_arr)?;
    let pattern_str = jstring_to_rust(env, pattern)?;
    let include_globs = jstring_array_to_vec(env, include_globs_arr)?;
    let exclude_globs = jstring_array_to_vec(env, exclude_globs_arr)?;

    let roots: Vec<PathBuf> = roots_strs.into_iter().map(PathBuf::from).collect();

    let mut config = crate::SearchConfig::new(roots, pattern_str);
    config.is_regex = jboolean_to_bool(is_regex);
    config.case_sensitive = jboolean_to_bool(case_sensitive);
    config.whole_words = jboolean_to_bool(whole_words);
    config.include_globs = include_globs;
    config.exclude_globs = exclude_globs;
    config.context_lines = jint_to_usize(context_lines);

    config.validate()?;
    Ok(config)
}

/// 抛出 Java 异常
pub fn throw_java_exception(env: &mut JNIEnv, error: &SearchError) {
    let msg = error.to_string();
    // 使用全限定类名抛出异常
    let _ = env.throw_new("com/example/rustsearch/SearchException", &msg);
}

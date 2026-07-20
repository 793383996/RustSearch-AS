//! Java 结果对象构建
//!
//! 将 Rust 的 SearchMatch 转换为 Java 的 SearchResult 对象数组。
//! H2:循环内用 with_local_frame_returning_local 包裹单条结果构建,
//! frame 退出时自动释放中间 local ref,防止大批量结果(>=60 条)导致
//! JVM local reference table overflow(默认上限 512)。

use jni::errors::Error as JniError;
use jni::objects::{JObject, JObjectArray, JValue};
use jni::sys::jsize;
use jni::JNIEnv;

use crate::error::SearchError;
use crate::search::SearchMatch;
use crate::jni::convert::rust_to_jstring;
use crate::search::line_kind::LineKind;

/// SearchResult 类的全限定名(内部类用 $ 分隔)
const RESULT_CLASS: &str = "com/example/rustsearch/RustSearchEngine$SearchResult";

/// H2:SearchError 从 jni::errors::Error 转换,满足 with_local_frame_returning_local 的
/// `E: From<Error>` 约束,让闭包内 jni 调用的 `?` 自动转换为 SearchError
impl From<JniError> for SearchError {
    fn from(e: JniError) -> Self {
        SearchError::Jni(e.to_string())
    }
}

/// 构建 SearchResult[] 数组返回给 JVM
pub fn build_search_result_array<'local>(
    env: &mut JNIEnv<'local>,
    matches: &[SearchMatch],
) -> Result<JObjectArray<'local>, SearchError> {
    let class = env
        .find_class(RESULT_CLASS)
        .map_err(|e| SearchError::Jni(format!("找不到 SearchResult 类: {e}")))?;

    let len: jsize = matches.len() as jsize;
    let array = env
        .new_object_array(len, &class, JObject::null())
        .map_err(|e| SearchError::Jni(format!("创建结果数组失败: {e}")))?;

    for (i, m) in matches.iter().enumerate() {
        // H2:用 with_local_frame_returning_local 包裹单条结果构建,
        // frame 退出时自动释放所有中间 local ref(JString/JObjectArray),
        // 只保留返回的 JObject(被 PopLocalFrame 提升为 frame 外 local ref)。
        // 单条结果最多 9 个 local ref,frame capacity=16 足够;
        // 整个 batch 期间同时存活的 local ref 永远 <= 16 + array 自身,
        // 远低于 JVM 默认上限 512。
        let obj = env
            .with_local_frame_returning_local(16, |env| build_single_result_in_frame(env, m))
            .map_err(|e: SearchError| SearchError::Jni(format!("构建 SearchResult 对象失败: {e}")))?;
        env.set_object_array_element(&array, i as jsize, &obj)
            .map_err(|e| SearchError::Jni(format!("设置数组元素失败: {e}")))?;
    }

    Ok(array)
}

/// 在 with_local_frame_returning_local 内构建单条 SearchResult 对象
///
/// frame 退出时所有中间 local ref 自动释放,无需手动 auto_local。
/// 返回 SearchError,jni::errors::Error 通过 From 自动转换。
///
/// v1.2.0:SearchResult 构造函数增加 `lineKind: Int` 参数(放最后,向后兼容),
/// 序数值与 Kotlin `LineKind` 枚举对齐(0=Code, 1=Comment, 2=Import, 3=Package)。
fn build_single_result_in_frame<'a>(
    env: &mut JNIEnv<'a>,
    m: &SearchMatch,
) -> Result<JObject<'a>, SearchError> {
    let class = env.find_class(RESULT_CLASS)?;
    let path_str = m.file_path.to_string_lossy().into_owned();
    let jpath = rust_to_jstring(env, &path_str)?;
    let jmatched = rust_to_jstring(env, &m.matched_text)?;
    let jbefore = build_string_array(env, &m.context_before)?;
    let jafter = build_string_array(env, &m.context_after)?;

    // v1.2.0:行类型序数(Int 传递,Kotlin 侧 LineKind.fromOrdinal 转换)
    let line_kind_ordinal: i32 = match m.line_kind {
        LineKind::Code => 0,
        LineKind::Comment => 1,
        LineKind::Import => 2,
        LineKind::Package => 3,
    };

    // SearchResult 构造函数签名(v1.2.0 新增 lineKind: Int 参数):
    // (String path, int line, int column, String matched, String[] before, String[] after, int lineKind)
    let obj = env.new_object(
        &class,
        "(Ljava/lang/String;IILjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;I)V",
        &[
            JValue::Object(&jpath),
            JValue::Int(m.line_number as i32),
            JValue::Int(m.column as i32),
            JValue::Object(&jmatched),
            JValue::Object(&jbefore),
            JValue::Object(&jafter),
            JValue::Int(line_kind_ordinal),
        ],
    )?;
    Ok(obj)
}

/// 构建 Java String[] 数组
fn build_string_array<'a>(
    env: &mut JNIEnv<'a>,
    strs: &[String],
) -> Result<JObjectArray<'a>, SearchError> {
    let class = env.find_class("java/lang/String")?;
    let len: jsize = strs.len() as jsize;
    let array = env.new_object_array(len, &class, JObject::null())?;

    for (i, s) in strs.iter().enumerate() {
        let jstr = rust_to_jstring(env, s)?;
        env.set_object_array_element(&array, i as jsize, &jstr)?;
    }

    Ok(array)
}

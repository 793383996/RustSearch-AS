//! JNI 入口函数
//!
//! 对应 Kotlin 侧的 `external` 声明,所有入口函数用 `catch_unwind` 包裹防止 panic 跨边界。
//!
//! 提供两套接口:
//! - **旧同步接口** `search(...)`:`#[deprecated]`,一次性返回全部结果,无法中途取消
//! - **新异步流式接口** `startSearch`/`pollResults`/`isSearchComplete`/`cancel`/`releaseSearch`:
//!   启动后台搜索返回 searchId,通过轮询获取批量结果,支持中途取消
//!
//! 取消机制通过全局注册表实现:
//! - `CANCEL_REGISTRY`:旧同步 search 的取消信号
//! - `SEARCH_REGISTRY`:新异步 startSearch 的会话信息(含 engine + receiver + 完成标志)

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossbeam_channel::Receiver;
use jni::objects::{JClass, JObject, JObjectArray, JString};
use jni::sys::{jboolean, jint, jlong, jobjectArray};
use jni::JNIEnv;
use once_cell::sync::Lazy;

use crate::error::{SearchError, SearchResult};
use crate::jni::{convert, result};
use crate::search::engine::SearchEngine;
use crate::search::SearchMatch;

/// 旧同步 search 的取消信号注册表:search_id -> cancel_flag
static CANCEL_REGISTRY: Lazy<Mutex<HashMap<u64, Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 新异步 startSearch 的会话注册表:search_id -> Arc<SearchSession>
/// 采用 Arc 共享:poll 时短暂持锁 clone Arc,立即释放锁,在锁外独占消费 receiver,
/// 实现"消费者独占堆"模式 —— 消费者(pollResults)拿到 Arc 后完全独占 receiver 操作,
/// 不再与 registry 交互(依赖反转),彻底消除锁内慢操作。
static SEARCH_REGISTRY: Lazy<Mutex<HashMap<u64, SharedSession>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 搜索 ID 自增生成器(新旧接口共用)
static SEARCH_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// 异步搜索会话,持有 engine 与结果 receiver
struct SearchSession {
    /// 搜索引擎,持有 cancel_flag,用于触发取消
    engine: SearchEngine,
    /// 流式结果接收端,从后台搜索线程获取匹配结果
    receiver: Receiver<SearchResult<SearchMatch>>,
    /// 搜索是否完成(后台线程结束或 receiver 关闭)
    is_complete: Arc<AtomicBool>,
}

/// 会话共享指针类型别名:registry 存储 Arc<SearchSession>,允许锁外消费
type SharedSession = Arc<SearchSession>;

/// 生成唯一搜索 ID
fn generate_search_id() -> u64 {
    SEARCH_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ============================================================================
// 旧同步接口(已废弃,保留向后兼容)
// ============================================================================

/// JNI 入口:执行文本搜索(同步,已废弃)
///
/// 对应 Kotlin:
/// ```kotlin
/// @JvmStatic
/// @Deprecated("使用 startSearch + pollResults 替代")
/// private external fun search(...): Array<SearchResult>
/// ```
#[deprecated(note = "使用 startSearch + pollResults 异步流式接口替代")]
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_search<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    roots: JObjectArray<'local>,
    pattern: JString<'local>,
    is_regex: jboolean,
    case_sensitive: jboolean,
    whole_words: jboolean,
    include_globs: JObjectArray<'local>,
    exclude_globs: JObjectArray<'local>,
    context_lines: jint,
) -> jobjectArray {
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_search(
            &mut env,
            &roots,
            &pattern,
            is_regex,
            case_sensitive,
            whole_words,
            &include_globs,
            &exclude_globs,
            context_lines,
        )
    }));

    match result {
        Ok(Ok(array)) => array.into_raw(),
        Ok(Err(e)) => {
            convert::throw_java_exception(&mut env, &e);
            std::ptr::null_mut()
        }
        Err(_) => {
            convert::throw_java_exception(
                &mut env,
                &SearchError::Internal("Rust 内部 panic".into()),
            );
            std::ptr::null_mut()
        }
    }
}

/// 旧同步搜索主流程:参数转换 → 创建引擎 → 执行搜索 → 构建结果
fn run_search<'local>(
    env: &mut JNIEnv<'local>,
    roots: &JObjectArray,
    pattern: &JString,
    is_regex: jboolean,
    case_sensitive: jboolean,
    whole_words: jboolean,
    include_globs: &JObjectArray,
    exclude_globs: &JObjectArray,
    context_lines: jint,
) -> Result<JObjectArray<'local>, SearchError> {
    let config = convert::build_config_from_jni(
        env,
        roots,
        pattern,
        is_regex,
        case_sensitive,
        whole_words,
        include_globs,
        exclude_globs,
        context_lines,
    )?;

    let engine = SearchEngine::new(config);
    let cancel_flag = engine.cancel_handle();

    let search_id = generate_search_id();
    {
        let mut registry = CANCEL_REGISTRY
            .lock()
            .map_err(|e| SearchError::Internal(format!("取消注册表锁失败: {e}")))?;
        registry.insert(search_id, Arc::clone(&cancel_flag));
    }

    let search_result = engine.search();

    {
        let mut registry = CANCEL_REGISTRY
            .lock()
            .map_err(|e| SearchError::Internal(format!("取消注册表锁失败: {e}")))?;
        registry.remove(&search_id);
    }

    let matches = search_result?;
    result::build_search_result_array(env, &matches)
}

// ============================================================================
// 新异步流式接口
// ============================================================================

/// JNI 入口:启动异步搜索,立即返回 searchId
///
/// 对应 Kotlin:
/// ```kotlin
/// @JvmStatic
/// external fun startSearch(...): Long
/// ```
///
/// 返回 search_id > 0 表示成功,0 表示失败(异常已抛出)
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_startSearch<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    roots: JObjectArray<'local>,
    pattern: JString<'local>,
    is_regex: jboolean,
    case_sensitive: jboolean,
    whole_words: jboolean,
    include_globs: JObjectArray<'local>,
    exclude_globs: JObjectArray<'local>,
    context_lines: jint,
) -> jlong {
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_start_search(
            &mut env,
            &roots,
            &pattern,
            is_regex,
            case_sensitive,
            whole_words,
            &include_globs,
            &exclude_globs,
            context_lines,
        )
    }));

    match result {
        Ok(Ok(id)) => id as jlong,
        Ok(Err(e)) => {
            convert::throw_java_exception(&mut env, &e);
            0
        }
        Err(_) => {
            convert::throw_java_exception(
                &mut env,
                &SearchError::Internal("Rust 内部 panic".into()),
            );
            0
        }
    }
}

/// JNI 入口:轮询获取一批搜索结果
///
/// 对应 Kotlin:
/// ```kotlin
/// @JvmStatic
/// external fun pollResults(searchId: Long, timeoutMs: Int): Array<SearchResult>
/// ```
///
/// 阻塞等待 timeoutMs 或拿到一批结果后返回。
/// 返回空数组表示暂无结果或搜索已完成(需配合 isSearchComplete 判断)。
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_pollResults<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    search_id: jlong,
    timeout_ms: jint,
) -> jobjectArray {
    let result = catch_unwind(AssertUnwindSafe(|| {
        run_poll_results(&mut env, search_id as u64, timeout_ms)
    }));

    match result {
        Ok(Ok(array)) => array.into_raw(),
        Ok(Err(e)) => {
            convert::throw_java_exception(&mut env, &e);
            // 异常时返回空数组,避免 JVM 侧 NPE
            empty_object_array(&mut env).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut())
        }
        Err(_) => {
            convert::throw_java_exception(
                &mut env,
                &SearchError::Internal("Rust 内部 panic".into()),
            );
            empty_object_array(&mut env).map(|a| a.into_raw()).unwrap_or(std::ptr::null_mut())
        }
    }
}

/// JNI 入口:检查搜索是否完成
///
/// 对应 Kotlin:
/// ```kotlin
/// @JvmStatic
/// external fun isSearchComplete(searchId: Long): Boolean
/// ```
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_isSearchComplete(
    _env: JNIEnv,
    _class: JClass,
    search_id: jlong,
) -> jboolean {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let id = search_id as u64;
        let registry = SEARCH_REGISTRY.lock().map_err(|e| {
            SearchError::Internal(format!("搜索注册表锁失败: {e}"))
        })?;
        Ok::<bool, SearchError>(
            registry
                .get(&id)
                .map(|s| s.is_complete.load(Ordering::Relaxed))
                .unwrap_or(true), // 不存在的 session 视为已完成
        )
    }));

    match result {
        Ok(Ok(complete)) => complete as jboolean,
        _ => 1, // 出错时返回已完成,避免 JVM 侧无限轮询
    }
}

/// JNI 入口:取消指定 ID 的搜索
///
/// 对应 Kotlin:
/// ```kotlin
/// @JvmStatic
/// external fun cancel(searchId: Long)
/// ```
///
/// 同时查询旧 CANCEL_REGISTRY 与新 SEARCH_REGISTRY,兼容两种接口。
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_cancel(
    _env: JNIEnv,
    _class: JClass,
    search_id: jlong,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let id = search_id as u64;

        // 1. 查旧 CANCEL_REGISTRY(兼容旧同步 search)
        if let Ok(registry) = CANCEL_REGISTRY.lock() {
            if let Some(flag) = registry.get(&id) {
                flag.store(true, Ordering::Relaxed);
                return;
            }
        }

        // 2. 查新 SEARCH_REGISTRY
        if let Ok(registry) = SEARCH_REGISTRY.lock() {
            if let Some(session) = registry.get(&id) {
                session.engine.cancel();
            }
        }
    }));
}

/// JNI 入口:释放搜索会话资源
///
/// 对应 Kotlin:
/// ```kotlin
/// @JvmStatic
/// external fun releaseSearch(searchId: Long)
/// ```
///
/// 必须在搜索结束后调用,清理 SEARCH_REGISTRY 中的会话,
/// 释放 engine 与 receiver 资源,防止内存泄漏。
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_releaseSearch(
    _env: JNIEnv,
    _class: JClass,
    search_id: jlong,
) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let id = search_id as u64;
        if let Ok(mut registry) = SEARCH_REGISTRY.lock() {
            registry.remove(&id);
        }
    }));
}

// ============================================================================
// 异步接口内部实现
// ============================================================================

/// 启动异步搜索主流程:参数转换 → 创建引擎 → 启动流式搜索 → 注册会话 → 返回 searchId
fn run_start_search<'local>(
    env: &mut JNIEnv<'local>,
    roots: &JObjectArray,
    pattern: &JString,
    is_regex: jboolean,
    case_sensitive: jboolean,
    whole_words: jboolean,
    include_globs: &JObjectArray,
    exclude_globs: &JObjectArray,
    context_lines: jint,
) -> Result<u64, SearchError> {
    let config = convert::build_config_from_jni(
        env,
        roots,
        pattern,
        is_regex,
        case_sensitive,
        whole_words,
        include_globs,
        exclude_globs,
        context_lines,
    )?;

    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream()?;

    let search_id = generate_search_id();
    // 包装为 Arc,允许 pollResults 锁外 clone 后独占消费
    let session: SharedSession = Arc::new(SearchSession {
        engine,
        receiver,
        is_complete: Arc::new(AtomicBool::new(false)),
    });

    {
        let mut registry = SEARCH_REGISTRY.lock().map_err(|e| {
            SearchError::Internal(format!("搜索注册表锁失败: {e}"))
        })?;
        registry.insert(search_id, session);
    }

    Ok(search_id)
}

/// 轮询获取结果:锁外独占消费 receiver(消费者独占堆模式)
///
/// 关键设计:短暂持锁 clone Arc<SearchSession>,立即释放锁,
/// 在锁外完全独占消费 receiver,锁持有时间从 200ms 降到纳秒级。
/// 采用"排空"策略:先 try_recv 循环拿走所有已就绪结果,
/// 若无结果则 recv_timeout 等待一个,再继续 try_recv 排空。
/// 标记完成状态前再次 try_recv 排空,确保结果不丢失(P2-2)。
fn run_poll_results<'local>(
    env: &mut JNIEnv<'local>,
    search_id: u64,
    timeout_ms: jint,
) -> Result<JObjectArray<'local>, SearchError> {
    let timeout = Duration::from_millis(timeout_ms.max(0) as u64);

    // 关键改造:短暂持锁 clone Arc,立即释放锁
    // 这是"消费者独占堆"的体现:拿到 Arc 后完全独占 receiver 操作,不再与 registry 交互
    let session: SharedSession = {
        let registry = SEARCH_REGISTRY.lock().map_err(|e| {
            SearchError::Internal(format!("搜索注册表锁失败: {e}"))
        })?;
        match registry.get(&search_id) {
            Some(s) => Arc::clone(s),
            None => {
                return Err(SearchError::Internal(format!("搜索会话 {search_id} 不存在")));
            }
        }
    }; // 锁在此处释放,后续 recv_timeout 完全无锁

    // 锁外独占消费 receiver
    let mut batch: Vec<SearchMatch> = Vec::new();
    let mut should_mark_complete = false;
    let mut pending_error: Option<SearchError> = None;

    // 排空当前已就绪的结果
    while let Ok(item) = session.receiver.try_recv() {
        match item {
            Ok(m) => batch.push(m),
            Err(SearchError::Cancelled) => {
                should_mark_complete = true;
                break;
            }
            Err(e) => {
                should_mark_complete = true;
                pending_error = Some(e);
                break;
            }
        }
    }

    // 如果没有就绪结果,等待一个
    if batch.is_empty() && !should_mark_complete {
        match session.receiver.recv_timeout(timeout) {
            Ok(Ok(m)) => batch.push(m),
            Ok(Err(SearchError::Cancelled)) => {
                should_mark_complete = true;
            }
            Ok(Err(e)) => {
                should_mark_complete = true;
                pending_error = Some(e);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // 超时,返回空数组
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                // channel 关闭,搜索完成
                should_mark_complete = true;
            }
        }

        // 拿到一个后,继续排空其他就绪结果
        if !batch.is_empty() {
            while let Ok(item) = session.receiver.try_recv() {
                match item {
                    Ok(m) => batch.push(m),
                    Err(SearchError::Cancelled) => {
                        should_mark_complete = true;
                        break;
                    }
                    Err(e) => {
                        should_mark_complete = true;
                        pending_error = Some(e);
                        break;
                    }
                }
            }
        }
    }

    // P2-2:标记完成前再排空一次,确保最后一批结果不丢失
    if should_mark_complete && pending_error.is_none() {
        while let Ok(Ok(m)) = session.receiver.try_recv() {
            batch.push(m);
        }
    }

    // 标记完成状态(通过 Arc 直接操作,无需再次持锁)
    if should_mark_complete {
        session.is_complete.store(true, Ordering::Release);
    }

    if let Some(e) = pending_error {
        return Err(e);
    }

    if batch.is_empty() {
        empty_object_array(env)
    } else {
        result::build_search_result_array(env, &batch)
    }
}

/// 构建空 Object 数组(用于无结果或错误场景)
fn empty_object_array<'local>(
    env: &mut JNIEnv<'local>,
) -> Result<JObjectArray<'local>, SearchError> {
    let class = env
        .find_class("java/lang/Object")
        .map_err(|e| SearchError::Jni(format!("找不到 Object 类: {e}")))?;
    env.new_object_array(0, &class, JObject::null())
        .map_err(|e| SearchError::Jni(format!("创建空数组失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;
    use tempfile::TempDir;

    #[test]
    fn test_generate_search_id_unique() {
        let id1 = generate_search_id();
        let id2 = generate_search_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_cancel_registry() {
        let flag = Arc::new(AtomicBool::new(false));
        let search_id = 999_999_999;

        {
            let mut registry = CANCEL_REGISTRY.lock().unwrap();
            registry.insert(search_id, Arc::clone(&flag));
        }

        {
            let registry = CANCEL_REGISTRY.lock().unwrap();
            if let Some(f) = registry.get(&search_id) {
                f.store(true, Ordering::Relaxed);
            }
        }

        assert!(flag.load(Ordering::Relaxed));

        {
            let mut registry = CANCEL_REGISTRY.lock().unwrap();
            registry.remove(&search_id);
        }
    }

    /// 创建测试用临时项目
    fn create_test_project() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("a.kt"), "fun main() {\n    println(\"hello\")\n}\n").unwrap();
        fs::write(root.join("b.java"), "class B {\n    void hello() {}\n}\n").unwrap();
        fs::write(root.join("c.txt"), "hello world\nfoo bar\n").unwrap();

        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("d.kt"), "val x = \"hello\"").unwrap();

        dir
    }

    #[test]
    fn test_search_session_start_and_complete() {
        use crate::search::config::SearchConfig;

        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);
        let receiver = engine.search_stream().unwrap();

        let search_id = generate_search_id();
        let session: SharedSession = Arc::new(SearchSession {
            engine,
            receiver,
            is_complete: Arc::new(AtomicBool::new(false)),
        });

        {
            let mut registry = SEARCH_REGISTRY.lock().unwrap();
            registry.insert(search_id, session);
        }

        // 锁外独占消费 receiver(验证消费者独占堆模式)
        let session_clone = {
            let registry = SEARCH_REGISTRY.lock().unwrap();
            Arc::clone(registry.get(&search_id).unwrap())
        };

        let mut count = 0;
        loop {
            match session_clone.receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(_)) => count += 1,
                Ok(Err(_)) => break,
                Err(_) => break,
            }
        }

        assert!(count >= 3, "应至少匹配 3 个 hello");

        // 标记完成(通过 Arc 直接操作,无需持锁)
        session_clone.is_complete.store(true, Ordering::Release);

        // 验证完成状态
        let registry = SEARCH_REGISTRY.lock().unwrap();
        assert!(registry.get(&search_id).unwrap().is_complete.load(Ordering::Relaxed));

        // 清理
        drop(registry);
        SEARCH_REGISTRY.lock().unwrap().remove(&search_id);
    }

    #[test]
    fn test_search_session_cancel() {
        use crate::search::config::SearchConfig;

        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);
        let cancel_flag = engine.cancel_handle();
        let receiver = engine.search_stream().unwrap();

        let search_id = generate_search_id();
        let session: SharedSession = Arc::new(SearchSession {
            engine,
            receiver,
            is_complete: Arc::new(AtomicBool::new(false)),
        });

        {
            let mut registry = SEARCH_REGISTRY.lock().unwrap();
            registry.insert(search_id, session);
        }

        // 验证初始状态:cancel_flag 为 false
        assert!(!cancel_flag.load(Ordering::Relaxed));

        // 触发取消(模拟 JNI cancel 调用,通过 Arc 操作)
        let session_clone = {
            let registry = SEARCH_REGISTRY.lock().unwrap();
            Arc::clone(registry.get(&search_id).unwrap())
        };
        session_clone.engine.cancel();

        // 验证 cancel_flag 已设置为 true,后台搜索线程会在下一个检查点停止
        assert!(cancel_flag.load(Ordering::Relaxed));

        // 清理
        SEARCH_REGISTRY.lock().unwrap().remove(&search_id);
    }

    #[test]
    fn test_release_search() {
        use crate::search::config::SearchConfig;

        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);
        let receiver = engine.search_stream().unwrap();

        let search_id = generate_search_id();
        let session: SharedSession = Arc::new(SearchSession {
            engine,
            receiver,
            is_complete: Arc::new(AtomicBool::new(false)),
        });

        {
            let mut registry = SEARCH_REGISTRY.lock().unwrap();
            registry.insert(search_id, session);
        }

        // 释放
        SEARCH_REGISTRY.lock().unwrap().remove(&search_id);

        // 验证已移除
        let registry = SEARCH_REGISTRY.lock().unwrap();
        assert!(registry.get(&search_id).is_none());
    }

    #[test]
    fn test_is_search_complete_nonexistent() {
        // 不存在的 session 应返回 true(已完成)
        let complete = {
            let registry = SEARCH_REGISTRY.lock().unwrap();
            registry
                .get(&999_999_998)
                .map(|s| s.is_complete.load(Ordering::Relaxed))
                .unwrap_or(true)
        };
        assert!(complete);
    }
}

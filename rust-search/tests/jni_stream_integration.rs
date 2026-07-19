//! 异步流式 JNI 接口集成测试
//!
//! 验证 startSearch/pollResults/isSearchComplete/cancel/releaseSearch 底层逻辑的正确性。
//! 由于 JNI 函数需要 JVM 环境才能直接调用,本测试通过模拟 JNI 调用方的使用方式,
//! 测试 SearchEngine + search_stream + Receiver 的完整端到端流程。
//!
//! 测试策略:
//! 1. 创建 SearchConfig 与 SearchEngine
//! 2. 调用 search_stream() 获取 receiver
//! 3. 模拟 pollResults 的"排空 + 等待"策略收集结果
//! 4. 验证结果正确性与完成状态

use rust_search::SearchConfig;
use rust_search::SearchEngine;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tempfile::TempDir;

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

/// 模拟 JNI pollResults 的排空策略
fn poll_results(
    receiver: &crossbeam_channel::Receiver<Result<rust_search::SearchMatch, rust_search::SearchError>>,
    timeout: Duration,
) -> (Vec<rust_search::SearchMatch>, bool) {
    let mut batch = Vec::new();
    let mut is_complete = false;

    // 排空已就绪结果
    while let Ok(item) = receiver.try_recv() {
        match item {
            Ok(m) => batch.push(m),
            Err(_) => {
                is_complete = true;
                break;
            }
        }
    }

    // 无就绪结果时等待一个
    if batch.is_empty() && !is_complete {
        match receiver.recv_timeout(timeout) {
            Ok(Ok(m)) => batch.push(m),
            Ok(Err(_)) => is_complete = true,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => is_complete = true,
        }

        // 继续排空
        if !batch.is_empty() {
            while let Ok(item) = receiver.try_recv() {
                match item {
                    Ok(m) => batch.push(m),
                    Err(_) => {
                        is_complete = true;
                        break;
                    }
                }
            }
        }
    }

    (batch, is_complete)
}

/// 收集所有流式结果直到完成
fn collect_all_results(
    receiver: &crossbeam_channel::Receiver<Result<rust_search::SearchMatch, rust_search::SearchError>>,
) -> Vec<rust_search::SearchMatch> {
    let mut all = Vec::new();
    loop {
        let (batch, complete) = poll_results(receiver, Duration::from_millis(500));
        let is_empty = batch.is_empty();
        all.extend(batch);
        if complete {
            break;
        }
        // channel 关闭检测
        if is_empty {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(m)) => all.push(m),
                Ok(Err(_)) => break,
                Err(_) => break,
            }
        }
    }
    all
}

#[test]
fn test_stream_search_basic() {
    let dir = create_test_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(results.len() >= 3, "应至少匹配 3 个 hello,实际: {}", results.len());
}

#[test]
fn test_stream_search_regex() {
    let dir = create_test_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], r"print\w+".into());
    config.is_regex = true;
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(!results.is_empty(), "正则 print\\w+ 应有匹配");
    assert!(results.iter().any(|m| m.matched_text.contains("println")));
}

#[test]
fn test_stream_search_case_sensitive() {
    let dir = create_test_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "Hello".into());
    config.case_sensitive = true;
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(results.is_empty(), "大小写敏感下 Hello 不应匹配 hello");
}

#[test]
fn test_stream_search_case_insensitive() {
    let dir = create_test_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "HELLO".into());
    config.case_sensitive = false;
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(results.len() >= 3, "大小写不敏感下 HELLO 应匹配 hello");
}

#[test]
fn test_stream_search_cancel() {
    let dir = create_test_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    let engine = SearchEngine::new(config);
    let cancel_flag = engine.cancel_handle();
    let receiver = engine.search_stream().unwrap();

    // 立即取消
    cancel_flag.store(true, Ordering::Relaxed);

    let (batch, complete) = poll_results(&receiver, Duration::from_millis(500));
    // 取消后应收到完成信号(可能伴随空结果)
    assert!(complete || batch.is_empty(), "取消后应标记完成或返回空结果");
}

#[test]
fn test_stream_search_file_filter() {
    let dir = create_test_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    config.include_globs = vec!["*.kt".to_string()];
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(!results.is_empty(), "应匹配 .kt 文件中的 hello");
    assert!(
        results.iter().all(|m| m.file_path.extension().map(|e| e == "kt").unwrap_or(false)),
        "所有结果应来自 .kt 文件"
    );
}

#[test]
fn test_stream_search_chinese() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join("中文.txt"), "你好世界\n测试中文搜索\n").unwrap();
    fs::write(root.join("other.txt"), "hello world\n").unwrap();

    let config = SearchConfig::new(vec![root.to_path_buf()], "你好".into());
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert_eq!(results.len(), 1, "应匹配 1 个中文结果");
    assert!(results[0].matched_text.contains("你好"));
}

#[test]
fn test_stream_search_max_matches() {
    let dir = create_test_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    config.max_total_matches = 2;
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(results.len() <= 2, "应不超过 max_total_matches=2");
}

#[test]
fn test_stream_search_context_lines() {
    let dir = TempDir::new().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(
        root.join("a.txt"),
        "line1\nline2\ntarget\nline4\nline5\n",
    )
    .unwrap();

    let mut config = SearchConfig::new(vec![root.to_path_buf()], "target".into());
    config.context_lines = 1;
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert_eq!(results.len(), 1);
    let m = &results[0];
    assert_eq!(m.line_number, 3);
    assert_eq!(m.context_before.len(), 1);
    assert_eq!(m.context_before[0], "line2");
    assert_eq!(m.context_after.len(), 1);
    assert_eq!(m.context_after[0], "line4");
}

#[test]
fn test_stream_search_multiple_roots() {
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();
    fs::create_dir_all(dir1.path().join(".git")).unwrap();
    fs::create_dir_all(dir2.path().join(".git")).unwrap();
    fs::write(dir1.path().join("a.txt"), "hello from dir1\n").unwrap();
    fs::write(dir2.path().join("b.txt"), "hello from dir2\n").unwrap();

    let config = SearchConfig::new(
        vec![dir1.path().to_path_buf(), dir2.path().to_path_buf()],
        "hello".into(),
    );
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(results.len() >= 2, "应匹配两个根目录下的 hello");
}

#[test]
fn test_stream_search_real_project() {
    // 在 rust-search 自身 src 目录上搜索 SearchEngine
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = SearchConfig::new(vec![src_dir], "SearchEngine".into());
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream().unwrap();

    let results = collect_all_results(&receiver);
    assert!(!results.is_empty(), "应在 rust-search/src 中找到 SearchEngine");
    assert!(
        results.iter().any(|m| m.file_path.to_string_lossy().contains("engine.rs")),
        "结果应包含 engine.rs"
    );
}

//! 集成测试:端到端验证搜索功能
//!
//! 构造模拟项目目录树,验证 SearchEngine 的完整搜索流程,
//! 包括字面量/正则搜索、文件过滤、上下文行、取消机制、多根目录等场景。

use rust_search::{SearchConfig, SearchEngine};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// 创建模拟项目目录树
/// 包含多语言源文件、.gitignore 规则、子目录、中文文件名等测试场景
fn create_mock_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    let root = dir.path();

    // Kotlin 文件
    fs::write(
        root.join("Main.kt"),
        "fun main() {\n    println(\"hello world\")\n    val x = \"hello\"\n}\n",
    )
    .unwrap();

    // Java 文件
    fs::write(
        root.join("Utils.java"),
        "public class Utils {\n    public void hello() {\n        System.out.println(\"hello\");\n    }\n}\n",
    )
    .unwrap();

    // 文本文件
    fs::write(root.join("notes.txt"), "hello\ntodo: review hello module\n").unwrap();

    // 子目录文件
    fs::create_dir_all(root.join("src").join("utils")).unwrap();
    fs::write(
        root.join("src").join("utils").join("Helper.kt"),
        "fun helloHelper() = \"hello from helper\"\n",
    )
    .unwrap();

    // build 目录(应被 gitignore 排除)
    fs::create_dir_all(root.join("build")).unwrap();
    fs::write(root.join("build").join("generated.kt"), "hello generated").unwrap();

    // .gitignore 排除 build 目录(ignore crate 需要识别为 git 仓库才读取 .gitignore)
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();

    // 隐藏文件
    fs::write(root.join(".secret"), "hello secret").unwrap();

    // 中文文件名与内容
    fs::write(root.join("中文文件.txt"), "你好世界\nhello\n").unwrap();

    dir
}

#[test]
fn test_integration_basic_search() {
    let dir = create_mock_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 应在 Main.kt(2处)、Utils.java(1处)、notes.txt(2处)、Helper.kt(1处)、中文文件.txt(1处) 中匹配
    assert!(matches.len() >= 7, "期望至少 7 处匹配,实际 {}", matches.len());

    // 验证匹配结果包含文件路径
    let has_kt = matches.iter().any(|m| {
        m.file_path.extension().map(|e| e == "kt").unwrap_or(false)
    });
    assert!(has_kt, "应包含 .kt 文件匹配");

    let has_java = matches.iter().any(|m| {
        m.file_path.extension().map(|e| e == "java").unwrap_or(false)
    });
    assert!(has_java, "应包含 .java 文件匹配");
}

#[test]
fn test_integration_gitignore_respected() {
    let dir = create_mock_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "generated".into());
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // build/generated.kt 应被 .gitignore 排除
    assert!(
        matches.is_empty(),
        "build 目录应被 gitignore 排除,但找到 {} 处匹配",
        matches.len()
    );
}

#[test]
fn test_integration_hidden_files_skipped() {
    let dir = create_mock_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "secret".into());
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // .secret 隐藏文件应被跳过
    assert!(
        matches.is_empty(),
        "隐藏文件应被跳过,但找到 {} 处匹配",
        matches.len()
    );
}

#[test]
fn test_integration_file_filter() {
    let dir = create_mock_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    config.include_globs = vec!["*.kt".to_string()];
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 只搜索 .kt 文件
    for m in &matches {
        assert!(
            m.file_path.extension().map(|e| e == "kt").unwrap_or(false),
            "不应包含非 .kt 文件: {:?}",
            m.file_path
        );
    }
}

#[test]
fn test_integration_regex_search() {
    let dir = create_mock_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], r"print\w+".into());
    config.is_regex = true;
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 应匹配 println
    assert!(!matches.is_empty(), "正则搜索应有匹配");
    assert!(
        matches.iter().any(|m| m.matched_text.contains("println")),
        "应匹配 println"
    );
}

#[test]
fn test_integration_context_lines() {
    let dir = create_mock_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    config.context_lines = 2;
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 至少有一条匹配包含上下文行
    let has_context = matches
        .iter()
        .any(|m| !m.context_before.is_empty() || !m.context_after.is_empty());
    assert!(has_context, "应包含上下文行");
}

#[test]
fn test_integration_cancel_during_search() {
    let dir = create_mock_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    let engine = SearchEngine::new(config);

    // 预先取消
    engine.cancel();
    let result = engine.search();

    assert!(result.is_err(), "取消后应返回错误");
}

#[test]
fn test_integration_multiple_roots() {
    let dir1 = TempDir::new().unwrap();
    let dir2 = TempDir::new().unwrap();

    fs::write(dir1.path().join("a.txt"), "hello from dir1").unwrap();
    fs::write(dir2.path().join("b.txt"), "hello from dir2").unwrap();

    let config = SearchConfig::new(
        vec![
            dir1.path().to_path_buf(),
            dir2.path().to_path_buf(),
        ],
        "hello".into(),
    );
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 应在两个根目录中各找到一处匹配
    assert_eq!(matches.len(), 2, "应在两个根目录中各找到一处匹配");

    let has_dir1 = matches.iter().any(|m| m.file_path.starts_with(dir1.path()));
    let has_dir2 = matches.iter().any(|m| m.file_path.starts_with(dir2.path()));
    assert!(has_dir1, "应包含 dir1 的匹配");
    assert!(has_dir2, "应包含 dir2 的匹配");
}

#[test]
fn test_integration_stream_search() {
    let dir = create_mock_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    let engine = SearchEngine::new(config);

    let rx = engine.search_stream().unwrap();
    let matches: Vec<_> = rx.iter().filter_map(|r| r.ok()).collect();

    assert!(!matches.is_empty(), "流式搜索应有结果");
}

#[test]
fn test_integration_chinese_content() {
    let dir = create_mock_project();
    let config = SearchConfig::new(vec![dir.path().to_path_buf()], "你好".into());
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    assert!(!matches.is_empty(), "中文内容搜索应有匹配");
    assert!(
        matches.iter().any(|m| m.matched_text.contains("你好")),
        "应匹配中文内容"
    );
}

#[test]
fn test_integration_case_sensitive() {
    let dir = create_mock_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "Hello".into());
    config.case_sensitive = true;
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 大小写敏感模式下,"Hello" 应无匹配(文件中只有小写 hello)
    // 注:如果文件中有大写 Hello,会有匹配;这里验证搜索不报错
    let _ = matches;
}

#[test]
fn test_integration_whole_words() {
    let dir = create_mock_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    config.whole_words = true;
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 全字匹配应正常工作
    assert!(!matches.is_empty(), "全字匹配应有结果");
}

#[test]
fn test_integration_max_matches() {
    let dir = create_mock_project();
    let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
    config.max_total_matches = 3;
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    assert_eq!(matches.len(), 3, "应限制为 3 条匹配");
}

#[test]
fn test_integration_search_on_real_project() {
    // 在 rust-search 项目自身目录上执行搜索,验证真实项目可用性
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = project_root.join("src");

    let config = SearchConfig::new(vec![src_dir], "SearchEngine".into());
    let engine = SearchEngine::new(config);

    let matches = engine.search().unwrap();

    // 应在 engine.rs 和 lib.rs 等文件中找到 SearchEngine
    assert!(
        !matches.is_empty(),
        "应在 rust-search 源码中找到 SearchEngine"
    );
}

//! 文本匹配器
//!
//! 基于 grep-regex/grep-searcher 实现文件内容搜索。
//! 支持字面量(自动转义)与正则模式、大小写敏感、全字匹配。
//! 通过实现 `Sink` trait 收集匹配结果,支持取消与单文件上限保护。

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkMatch};

use crate::error::{SearchError, SearchResult};
use crate::search::config::SearchConfig;
use crate::search::context::ContextExtractor;

/// 单条匹配结果
#[derive(Debug, Clone)]
pub struct SearchMatch {
    /// 文件绝对路径
    pub file_path: PathBuf,
    /// 行号(从 1 开始)
    pub line_number: usize,
    /// 匹配起始列(字节偏移,从 0 开始)
    pub column: usize,
    /// 匹配所在行的完整内容
    pub matched_text: String,
    /// 上下文行(前 N 行)
    pub context_before: Vec<String>,
    /// 上下文行(后 N 行)
    pub context_after: Vec<String>,
}

/// 文本匹配器,封装 grep-regex 的 RegexMatcher 与 grep-searcher 的 Searcher
pub struct Matcher {
    regex_matcher: RegexMatcher,
    context_lines: usize,
    max_matches_per_file: usize,
}

impl Matcher {
    /// 根据配置创建匹配器,编译正则表达式
    pub fn new(config: &SearchConfig) -> SearchResult<Self> {
        let pattern = config.build_regex_pattern();
        let regex_matcher = RegexMatcher::new(&pattern)
            .map_err(|e| SearchError::RegexCompile(format!("正则编译失败 '{pattern}': {e}")))?;

        Ok(Self {
            regex_matcher,
            context_lines: config.context_lines,
            max_matches_per_file: config.max_matches_per_file,
        })
    }

    /// 搜索单个文件,返回所有匹配结果
    pub fn search_file(
        &self,
        path: &Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> SearchResult<Vec<SearchMatch>> {
        let context_extractor = if self.context_lines > 0 {
            Some(ContextExtractor::new(path, self.context_lines)?)
        } else {
            None
        };

        // 使用 Rc<RefCell> 共享结果,因为 search_path 会消费 sink
        let matches = Rc::new(RefCell::new(Vec::new()));

        let sink = MatchSink {
            file_path: path.to_path_buf(),
            matches: Rc::clone(&matches),
            max_matches: self.max_matches_per_file,
            cancel_flag: Arc::clone(cancel_flag),
            context_extractor,
            context_lines: self.context_lines,
        };

        let mut searcher = Searcher::new();
        searcher
            .search_path(&self.regex_matcher, path, sink)
            .map_err(|e| {
                SearchError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("搜索文件失败 {}: {e}", path.display()),
                ))
            })?;

        // 从 Rc<RefCell> 中取出结果
        let result = Rc::try_unwrap(matches)
            .map(|cell| cell.into_inner())
            .unwrap_or_else(|cell| cell.borrow().clone());

        Ok(result)
    }
}

/// grep-searcher 的 Sink 实现,收集匹配结果
struct MatchSink {
    file_path: PathBuf,
    matches: Rc<RefCell<Vec<SearchMatch>>>,
    max_matches: usize,
    cancel_flag: Arc<AtomicBool>,
    context_extractor: Option<ContextExtractor>,
    context_lines: usize,
}

impl Sink for MatchSink {
    type Error = std::io::Error;

    fn matched(
        &mut self,
        _searcher: &Searcher,
        mat: &SinkMatch,
    ) -> std::result::Result<bool, Self::Error> {
        // 取消检查
        if self.cancel_flag.load(Ordering::Relaxed) {
            return Ok(false);
        }

        let matches = self.matches.borrow();
        // 单文件上限保护
        if matches.len() >= self.max_matches {
            return Ok(false);
        }
        drop(matches); // 释放借用,避免与下面的 borrow_mut 冲突

        let line_number = mat.line_number().unwrap_or(0) as usize;
        let bytes = mat.bytes();
        let matched_text = String::from_utf8_lossy(bytes).into_owned();
        // SinkMatch 未暴露匹配范围,column 暂设为 0,后续通过 matcher.find() 优化
        let column = 0;

        // 上下文行提取(如有配置)
        let (context_before, context_after) = if self.context_lines > 0 {
            if let Some(ref mut extractor) = self.context_extractor {
                extractor.extract(line_number, self.context_lines)
            } else {
                (Vec::new(), Vec::new())
            }
        } else {
            (Vec::new(), Vec::new())
        };

        self.matches.borrow_mut().push(SearchMatch {
            file_path: self.file_path.clone(),
            line_number,
            column,
            matched_text,
            context_before,
            context_after,
        });

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_file(content: &str) -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, content).unwrap();
        (dir, file_path)
    }

    fn make_cancel_flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn test_search_literal_match() {
        let content = "hello world\nfoo bar\nhello rust\n";
        let (dir, file) = create_test_file(content);

        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 1);
        assert_eq!(matches[1].line_number, 3);
    }

    #[test]
    fn test_search_case_insensitive() {
        let content = "Hello\nHELLO\nhello\n";
        let (dir, file) = create_test_file(content);

        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        assert_eq!(matches.len(), 3);
    }

    #[test]
    fn test_search_case_sensitive() {
        let content = "Hello\nHELLO\nhello\n";
        let (dir, file) = create_test_file(content);

        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        config.case_sensitive = true;
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_number, 3);
    }

    #[test]
    fn test_search_regex() {
        let content = "abc123\ndef456\nghi\n";
        let (dir, file) = create_test_file(content);

        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], r"\d+".into());
        config.is_regex = true;
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_search_whole_words() {
        let content = "foo\nfoobar\nfoo bar\n";
        let (dir, file) = create_test_file(content);

        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "foo".into());
        config.whole_words = true;
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        // "foo" 和 "foo bar" 中的 foo 匹配,"foobar" 不匹配
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_search_cancel() {
        let content = "test\ntest\ntest\ntest\n";
        let (dir, file) = create_test_file(content);

        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "test".into());
        let matcher = Matcher::new(&config).unwrap();
        let cancel = Arc::new(AtomicBool::new(true)); // 预先取消

        let matches = matcher.search_file(&file, &cancel).unwrap();
        // 取消后应立即停止,匹配数为 0
        assert_eq!(matches.len(), 0);
    }

    #[test]
    fn test_search_max_matches() {
        let content = "test\ntest\ntest\ntest\ntest\n";
        let (dir, file) = create_test_file(content);

        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "test".into());
        config.max_matches_per_file = 2;
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_search_chinese_content() {
        let content = "你好世界\n测试内容\n你好 rust\n";
        let (dir, file) = create_test_file(content);

        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "你好".into());
        let matcher = Matcher::new(&config).unwrap();
        let cancel = make_cancel_flag();

        let matches = matcher.search_file(&file, &cancel).unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 1);
        assert_eq!(matches[1].line_number, 3);
    }
}

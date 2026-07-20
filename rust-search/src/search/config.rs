//! 搜索配置定义与解析
//!
//! `SearchConfig` 封装一次搜索请求的全部参数,由 JNI 层从 JVM 参数转换而来。
//! 包含根目录、匹配模式、过滤规则、上下文行数、上限保护等字段。

use std::path::PathBuf;

use crate::error::{SearchError, SearchResult};

/// M6:配置上限常量,防止恶意或误用配置导致内存爆炸
const MAX_CONTEXT_LINES: usize = 50;
const MAX_MATCHES_PER_FILE: usize = 100_000;
const MAX_TOTAL_MATCHES: usize = 1_000_000;

/// 搜索配置,描述一次 Find in Path 请求的全部参数
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// 搜索根目录列表(支持多根目录,如多模块项目)
    pub roots: Vec<PathBuf>,

    /// 搜索模式(字面量或正则表达式)
    pub pattern: String,

    /// 是否为正则模式;false 时按字面量匹配(自动转义特殊字符)
    pub is_regex: bool,

    /// 大小写敏感
    pub case_sensitive: bool,

    /// 全字匹配(在模式两侧加 \b 边界)
    pub whole_words: bool,

    /// 包含文件 glob(如 ["*.kt", "*.java"]);为空表示不过滤
    pub include_globs: Vec<String>,

    /// 排除文件 glob(如 ["*/build/*", "*/.gradle/*"])
    pub exclude_globs: Vec<String>,

    /// 上下文行数(前/后各 N 行);0 表示不提取上下文
    pub context_lines: usize,

    /// 单文件最大匹配数,防止超大文件耗尽内存
    pub max_matches_per_file: usize,

    /// 全局最大匹配数,背压控制
    pub max_total_matches: usize,

    /// 是否搜索隐藏文件(默认 false)
    pub search_hidden: bool,

    /// v1.2.0:跳过注释行(//、#、/*、*、<!--、--),不返回匹配结果
    pub skip_comments: bool,

    /// v1.2.0:跳过 import 行(import、#include、include、require、using、from),不返回匹配结果
    pub skip_imports: bool,

    /// v1.2.0:跳过 package 行(Java/Kotlin/Go 的 package 声明),不返回匹配结果
    pub skip_packages: bool,
}

impl SearchConfig {
    /// 创建默认配置,上限保护采用保守默认值
    pub fn new(roots: Vec<PathBuf>, pattern: String) -> Self {
        Self {
            roots,
            pattern,
            is_regex: false,
            case_sensitive: false,
            whole_words: false,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            context_lines: 0,
            max_matches_per_file: 10_000,
            max_total_matches: 100_000,
            search_hidden: false,
            skip_comments: false,
            skip_imports: false,
            skip_packages: false,
        }
    }

    /// 校验配置合法性:根目录存在、模式非空、正则可编译
    pub fn validate(&self) -> SearchResult<()> {
        if self.pattern.is_empty() {
            return Err(SearchError::InvalidPattern("搜索模式为空".into()));
        }
        if self.roots.is_empty() {
            return Err(SearchError::InvalidRoot("根目录列表为空".into()));
        }
        for root in &self.roots {
            if !root.exists() {
                return Err(SearchError::InvalidRoot(format!(
                    "根目录不存在: {}",
                    root.display()
                )));
            }
        }
        // P2-4:正则模式预编译校验,提前暴露错误(避免用户等待 walker.files() 完成后才看到错误)
        if self.is_regex {
            let _ = regex::Regex::new(&self.pattern)
                .map_err(|e| SearchError::RegexCompile(format!("正则表达式无效: {e}")))?;
        }
        // M6:范围校验,防止恶意或误用配置导致内存爆炸
        if self.context_lines > MAX_CONTEXT_LINES {
            return Err(SearchError::InvalidPattern(format!(
                "context_lines 超过上限 {} (实际 {})",
                MAX_CONTEXT_LINES, self.context_lines
            )));
        }
        if self.max_matches_per_file > MAX_MATCHES_PER_FILE {
            return Err(SearchError::InvalidPattern(format!(
                "max_matches_per_file 超过上限 {} (实际 {})",
                MAX_MATCHES_PER_FILE, self.max_matches_per_file
            )));
        }
        if self.max_total_matches > MAX_TOTAL_MATCHES {
            return Err(SearchError::InvalidPattern(format!(
                "max_total_matches 超过上限 {} (实际 {})",
                MAX_TOTAL_MATCHES, self.max_total_matches
            )));
        }
        Ok(())
    }

    /// 构建最终的正则模式字符串(含大小写标志与全字边界)
    pub fn build_regex_pattern(&self) -> String {
        let core = if self.is_regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };

        let with_words = if self.whole_words {
            format!(r"\b{core}\b")
        } else {
            core
        };

        if self.case_sensitive {
            with_words
        } else {
            format!("(?i){with_words}")
        }
    }
}

/// 构建配置的便捷构造器
#[derive(Default)]
pub struct SearchConfigBuilder {
    roots: Vec<PathBuf>,
    pattern: String,
    is_regex: bool,
    case_sensitive: bool,
    whole_words: bool,
    include_globs: Vec<String>,
    exclude_globs: Vec<String>,
    context_lines: usize,
    max_matches_per_file: Option<usize>,
    max_total_matches: Option<usize>,
    search_hidden: bool,
    skip_comments: bool,
    skip_imports: bool,
    skip_packages: bool,
}

impl SearchConfigBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.roots = roots;
        self
    }

    pub fn pattern(mut self, pattern: impl Into<String>) -> Self {
        self.pattern = pattern.into();
        self
    }

    pub fn regex(mut self, is_regex: bool) -> Self {
        self.is_regex = is_regex;
        self
    }

    pub fn case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    pub fn whole_words(mut self, whole_words: bool) -> Self {
        self.whole_words = whole_words;
        self
    }

    pub fn include_globs(mut self, globs: Vec<String>) -> Self {
        self.include_globs = globs;
        self
    }

    pub fn exclude_globs(mut self, globs: Vec<String>) -> Self {
        self.exclude_globs = globs;
        self
    }

    pub fn context_lines(mut self, lines: usize) -> Self {
        self.context_lines = lines;
        self
    }

    pub fn max_matches_per_file(mut self, max: usize) -> Self {
        self.max_matches_per_file = Some(max);
        self
    }

    pub fn max_total_matches(mut self, max: usize) -> Self {
        self.max_total_matches = Some(max);
        self
    }

    pub fn search_hidden(mut self, search: bool) -> Self {
        self.search_hidden = search;
        self
    }

    pub fn skip_comments(mut self, skip: bool) -> Self {
        self.skip_comments = skip;
        self
    }

    pub fn skip_imports(mut self, skip: bool) -> Self {
        self.skip_imports = skip;
        self
    }

    pub fn skip_packages(mut self, skip: bool) -> Self {
        self.skip_packages = skip;
        self
    }

    pub fn build(self) -> SearchResult<SearchConfig> {
        let max_per_file = self.max_matches_per_file.unwrap_or(10_000);
        let max_total = self.max_total_matches.unwrap_or(100_000);

        let config = SearchConfig {
            roots: self.roots,
            pattern: self.pattern,
            is_regex: self.is_regex,
            case_sensitive: self.case_sensitive,
            whole_words: self.whole_words,
            include_globs: self.include_globs,
            exclude_globs: self.exclude_globs,
            context_lines: self.context_lines,
            max_matches_per_file: max_per_file,
            max_total_matches: max_total,
            search_hidden: self.search_hidden,
            skip_comments: self.skip_comments,
            skip_imports: self.skip_imports,
            skip_packages: self.skip_packages,
        };
        config.validate()?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_regex_pattern_literal() {
        let config = SearchConfig::new(vec![], "hello.world".into());
        // 字面量模式下,点号应被转义
        let pattern = config.build_regex_pattern();
        assert!(pattern.contains(r"\."));
        assert!(pattern.starts_with("(?i)"));
    }

    #[test]
    fn test_build_regex_pattern_regex() {
        let mut config = SearchConfig::new(vec![], r"\d+".into());
        config.is_regex = true;
        config.case_sensitive = true;
        let pattern = config.build_regex_pattern();
        assert_eq!(pattern, r"\d+");
    }

    #[test]
    fn test_build_regex_pattern_whole_words() {
        let mut config = SearchConfig::new(vec![], "foo".into());
        config.whole_words = true;
        let pattern = config.build_regex_pattern();
        assert!(pattern.contains(r"\bfoo\b"));
    }

    #[test]
    fn test_validate_empty_pattern() {
        let config = SearchConfig::new(vec![PathBuf::from("/")], "".into());
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_nonexistent_root() {
        let config = SearchConfig::new(
            vec![PathBuf::from("/nonexistent_path_12345")],
            "test".into(),
        );
        assert!(config.validate().is_err());
    }
}

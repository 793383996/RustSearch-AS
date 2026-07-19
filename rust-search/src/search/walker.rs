//! 文件遍历器
//!
//! 封装 ignore crate 的并行遍历能力,尊重 .gitignore/.ignore 等规则,
//! 同时支持 include_globs/exclude_globs 自定义过滤。
//! 文件级并行由 engine 层的 par_bridge 完成,walker 提供串行迭代器。

use std::path::PathBuf;

use ignore::overrides::OverrideBuilder;
use ignore::WalkBuilder;

use crate::error::{SearchError, SearchResult};
use crate::search::config::SearchConfig;

/// 文件遍历器,封装 ignore crate 的遍历能力
pub struct Walker {
    config: SearchConfig,
}

impl Walker {
    pub fn new(config: SearchConfig) -> Self {
        Self { config }
    }

    /// 构建文件迭代器,返回所有应被搜索的文件路径
    /// 串行迭代,由调用方决定如何并行化
    pub fn files(&self) -> SearchResult<Vec<PathBuf>> {
        let mut all_files = Vec::new();

        for root in &self.config.roots {
            let files = self.walk_root(root)?;
            all_files.extend(files);
        }

        Ok(all_files)
    }

    /// 构建并返回 ignore::Walk 迭代器(已应用 include/exclude globs)
    ///
    /// H3:暴露迭代器让 engine 层用 par_bridge 实现"边遍历边搜索",
    /// 避免全量 collect 到 Vec 后才开始搜索的首屏延迟。
    /// walker 产出一个文件,par_bridge 立即喂入并行管道开始搜索,
    /// 首屏延迟从"walker 全量遍历耗时"降到"首个文件产出耗时"(毫秒级)。
    ///
    /// 多根目录场景:用 WalkBuilder::add 依次添加所有根目录,
    /// 单个 ignore::Walk 迭代器按顺序遍历所有根目录;
    /// 跨根目录无并行(单根目录内 par_bridge 已并行),后续可用 WalkParallel 优化。
    pub fn walk(self) -> ignore::Walk {
        let config = self.config;
        let mut roots_iter = config.roots.into_iter();
        let first_root = roots_iter.next().unwrap_or_else(|| PathBuf::from("."));
        let mut builder = WalkBuilder::new(&first_root);
        // H3:额外根目录通过 WalkBuilder::add 添加,单个迭代器遍历所有根
        for extra in roots_iter {
            builder.add(extra);
        }
        builder
            .hidden(!config.search_hidden)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true)
            .parents(true)
            .ignore(true);

        // H3:walk(self) 已消费 self,无法调用 &self 的 build_overrides;
        // 内联构建 overrides,逻辑与 build_overrides 一致
        if !config.include_globs.is_empty() || !config.exclude_globs.is_empty() {
            let mut ob = ignore::overrides::OverrideBuilder::new(&first_root);
            for glob in &config.include_globs {
                let _ = ob.add(glob);
            }
            for glob in &config.exclude_globs {
                let _ = ob.add(&format!("!{glob}"));
            }
            if let Ok(overrides) = ob.build() {
                builder.overrides(overrides);
            }
        }

        builder.build()
    }

    /// 遍历单个根目录
    fn walk_root(&self, root: &std::path::Path) -> SearchResult<Vec<PathBuf>> {
        let mut builder = WalkBuilder::new(root);
        builder
            .hidden(!self.config.search_hidden)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true)
            .parents(true)
            .ignore(true);

        // 应用 include/exclude glob 覆盖规则
        if !self.config.include_globs.is_empty() || !self.config.exclude_globs.is_empty() {
            let overrides = self.build_overrides(root)?;
            builder.overrides(overrides);
        }

        let walker = builder.build();
        let mut files = Vec::new();

        for entry in walker {
            let entry = entry.map_err(|e| {
                SearchError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("遍历失败: {e}"),
                ))
            })?;

            if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                files.push(entry.into_path());
            }
        }

        Ok(files)
    }

    /// 构建 OverrideBuilder 处理 include/exclude globs
    /// include_globs 直接添加;exclude_globs 加 ! 前缀
    fn build_overrides(
        &self,
        root: &std::path::Path,
    ) -> SearchResult<ignore::overrides::Override> {
        let mut builder = OverrideBuilder::new(root);

        for glob in &self.config.include_globs {
            builder
                .add(glob)
                .map_err(|e| SearchError::InvalidPattern(format!("无效的 include glob '{glob}': {e}")))?;
        }

        for glob in &self.config.exclude_globs {
            let negated = format!("!{glob}");
            builder
                .add(&negated)
                .map_err(|e| SearchError::InvalidPattern(format!("无效的 exclude glob '{glob}': {e}")))?;
        }

        builder
            .build()
            .map_err(|e| SearchError::InvalidPattern(format!("构建 glob 覆盖规则失败: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_tree() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("a.kt"), "fun main()").unwrap();
        fs::write(root.join("b.java"), "class B {}").unwrap();
        fs::write(root.join("c.txt"), "hello").unwrap();

        fs::create_dir_all(root.join("build")).unwrap();
        fs::write(root.join("build").join("out.kt"), "compiled").unwrap();

        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden").join("secret.kt"), "secret").unwrap();

        dir
    }

    #[test]
    fn test_walk_all_files() {
        let dir = create_test_tree();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "test".into());
        let walker = Walker::new(config);
        let files = walker.files().unwrap();

        let names: Vec<_> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
            .collect();

        assert!(names.contains(&"a.kt".to_string()));
        assert!(names.contains(&"b.java".to_string()));
        assert!(names.contains(&"c.txt".to_string()));
        // 隐藏目录应被跳过
        assert!(!names.contains(&"secret.kt".to_string()));
    }

    #[test]
    fn test_walk_with_include_glob() {
        let dir = create_test_tree();
        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "test".into());
        config.include_globs = vec!["*.kt".to_string()];
        let walker = Walker::new(config);
        let files = walker.files().unwrap();

        // include_globs 匹配 a.kt 和 build/out.kt(build 是普通目录,非隐藏)
        assert!(files.iter().all(|f| {
            f.extension().map(|e| e == "kt").unwrap_or(false)
        }));
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_walk_with_exclude_glob() {
        let dir = create_test_tree();
        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "test".into());
        config.exclude_globs = vec!["build/*".to_string()];
        let walker = Walker::new(config);
        let files = walker.files().unwrap();

        let names: Vec<_> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(!names.contains(&"out.kt".to_string()));
    }

    #[test]
    fn test_walk_nonexistent_root() {
        let config = SearchConfig::new(
            vec![PathBuf::from("/nonexistent_path_12345")],
            "test".into(),
        );
        let walker = Walker::new(config);
        // files() 不直接报错根目录不存在,但 walk_root 会返回空或错误
        let result = walker.files();
        // 根目录不存在时,ignore 迭代器返回错误
        assert!(result.is_err() || result.unwrap().is_empty());
    }
}

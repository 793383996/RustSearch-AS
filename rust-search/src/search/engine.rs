//! 搜索引擎主流程
//!
//! 编排文件遍历(Walker)与文本匹配(Matcher),使用 rayon 实现文件级并行。
//! MVP 阶段提供同步 API `search()`,一次性返回全部结果;
//! 流式 API `search_stream()` 供 Beta 阶段使用,通过 crossbeam-channel 输出,支持背压与取消。

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, Sender, SendTimeoutError};
use rayon::prelude::*;

use crate::error::{SearchError, SearchResult};
use crate::search::config::SearchConfig;
use crate::search::matcher::{Matcher, SearchMatch};
use crate::search::walker::Walker;

/// 搜索引擎,负责编排遍历与匹配
pub struct SearchEngine {
    config: SearchConfig,
    cancel_flag: Arc<AtomicBool>,
}

impl SearchEngine {
    /// 创建搜索引擎实例
    pub fn new(config: SearchConfig) -> Self {
        Self {
            config,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 获取取消信号句柄,供外部触发取消
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    /// 触发取消,正在进行的搜索会在下一个检查点停止
    pub fn cancel(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }

    /// 同步执行搜索,返回全部匹配结果(MVP 阶段主入口)
    /// 文件级并行,文件内顺序匹配,结果顺序不保证(并行执行)
    pub fn search(&self) -> SearchResult<Vec<SearchMatch>> {
        self.config.validate()?;

        let walker = Walker::new(self.config.clone());
        let files = walker.files()?;

        if self.cancel_flag.load(Ordering::Relaxed) {
            return Err(SearchError::Cancelled);
        }

        let matcher = Matcher::new(&self.config)?;
        let max_total = self.config.max_total_matches;
        let cancel_flag = Arc::clone(&self.cancel_flag);

        // 文件级并行:每个文件独立搜索,结果合并
        // P2-D:IO/JNI 错误降级为空(单文件失败不中断整体),配置错误传播让用户感知
        let results: Vec<Vec<SearchMatch>> = files
            .par_iter()
            .map(|file| {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Ok(Vec::new());
                }

                match matcher.search_file(file, &cancel_flag) {
                    Ok(matches) => Ok(matches),
                    // IO 错误(文件权限/不存在/编码)降级为空,不中断整体搜索
                    Err(SearchError::Io(_)) | Err(SearchError::Jni(_)) => Ok(Vec::new()),
                    // 配置错误(InvalidPattern/RegexCompile 等)向上传播
                    Err(e) => Err(e),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        // 合并并应用全局上限
        let mut all_matches: Vec<SearchMatch> = results.into_iter().flatten().collect();
        if all_matches.len() > max_total {
            all_matches.truncate(max_total);
        }

        // P2-3:取消时始终返回 Cancelled,由 UI 层决定是否展示已收到的部分结果
        if self.cancel_flag.load(Ordering::Relaxed) {
            return Err(SearchError::Cancelled);
        }

        Ok(all_matches)
    }

    /// 流式执行搜索,通过 channel 返回结果(Beta 阶段使用)
    /// 背压:channel 缓冲 256 条,满时生产者阻塞
    pub fn search_stream(&self) -> SearchResult<Receiver<SearchResult<SearchMatch>>> {
        self.config.validate()?;

        let (tx, rx) = bounded::<SearchResult<SearchMatch>>(256);

        let config = self.config.clone();
        let cancel_flag = Arc::clone(&self.cancel_flag);

        // 启动后台搜索线程
        let handle = std::thread::Builder::new()
            .name("rust-search-worker".into())
            .spawn(move || {
                let result = Self::run_stream_search(&config, &cancel_flag, &tx);
                if let Err(e) = result {
                    // 发送最终错误,忽略发送失败(接收方可能已关闭)
                    let _ = tx.send(Err(e));
                }
                // tx drop 后 rx 迭代自然结束
            })
            .map_err(|e| {
                SearchError::Internal(format!("搜索线程启动失败: {e}"))
            })?;

        // 保留 handle 避免编译警告,实际不需要 join(线程会自然结束)
        let _ = handle;

        Ok(rx)
    }

    /// 流式搜索内部主循环(并行版)
    ///
    /// H3:用 par_bridge 替代 par_iter,实现"边遍历边搜索"。
    /// walker 产出一个文件,par_bridge 立即喂入 rayon 并行管道开始搜索,
    /// 首屏延迟从"walker 全量遍历耗时"降到"首个文件产出耗时"(毫秒级),
    /// 同时避免 Vec<PathBuf> 缓存全部文件路径的内存开销。
    ///
    /// 文件级并行:多个生产者线程并行处理文件,channel 天然支持多生产者无锁投递。
    /// 使用 AtomicUsize 跨线程统计已发送数量;send_timeout 防止消费者异常时线程永久阻塞。
    /// 取消错误不向上传播(已发送的结果仍然有效,用户能看到部分结果)。
    fn run_stream_search(
        config: &SearchConfig,
        cancel_flag: &Arc<AtomicBool>,
        tx: &Sender<SearchResult<SearchMatch>>,
    ) -> SearchResult<()> {
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(SearchError::Cancelled);
        }

        let matcher = Matcher::new(config)?;
        let max_total = config.max_total_matches;
        // 跨线程安全的已发送计数
        let sent_count = AtomicUsize::new(0usize);

        // H3:用 par_bridge 替代 par_iter,实现"边遍历边搜索"
        // walker 产出一个文件,par_bridge 立即喂入并行管道开始搜索
        // 首屏延迟从"walker 全量遍历耗时"降到"首个文件产出耗时"(毫秒级)
        let walker = Walker::new(config.clone()).walk();
        let result = walker
            .par_bridge()
            .try_for_each(|entry| {
                // 取消检查(每个文件开头)
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(SearchError::Cancelled);
                }

                // 全局上限检查
                if sent_count.load(Ordering::Relaxed) >= max_total {
                    return Err(SearchError::Cancelled);
                }

                // H3:par_bridge 闭包参数为 Result<DirEntry, Error>
                // 遍历错误(权限/符号链接)降级为跳过,不中断整体搜索
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => return Ok(()),
                };
                // 过滤非文件条目(目录、符号链接等)
                if entry.file_type().map(|t| !t.is_file()).unwrap_or(true) {
                    return Ok(());
                }
                let file = entry.path();

                let matches = matcher.search_file(file, cancel_flag)?;
                for m in matches {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err(SearchError::Cancelled);
                    }

                    // P2-A:先原子递增再检查上限,消除 load+fetch_add 之间的竞态窗口。
                    // fetch_add 返回旧值,若旧值 >= max_total 说明已超上限,本线程不发送。
                    // 注:多线程并发时可能有 (并行度-1) 个额外结果通过,属可接受的近似上限。
                    let prev_count = sent_count.fetch_add(1, Ordering::Relaxed);
                    if prev_count >= max_total {
                        // 回退递增,返回取消(已发送的结果仍然有效)
                        sent_count.fetch_sub(1, Ordering::Relaxed);
                        return Err(SearchError::Cancelled);
                    }

                    // P2-B + P1-4:send_timeout 超时后重试同一个 m,避免误终止搜索。
                    // 消费者可能只是处理慢(如 UI 渲染大结果集),并非 stalled。
                    loop {
                        if cancel_flag.load(Ordering::Relaxed) {
                            return Err(SearchError::Cancelled);
                        }
                        match tx.send_timeout(Ok(m.clone()), Duration::from_millis(500)) {
                            Ok(()) => break,
                            Err(SendTimeoutError::Timeout(_)) => {
                                // 超时:检查取消标志,未取消则重试(continue loop)
                                continue;
                            }
                            Err(SendTimeoutError::Disconnected(_)) => {
                                return Err(SearchError::Cancelled);
                            }
                        }
                    }
                }
                Ok(())
            });

        // 取消错误不向上传播(已发送的结果仍然有效)
        match result {
            Ok(()) => Ok(()),
            Err(SearchError::Cancelled) => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_project() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        fs::write(root.join("a.kt"), "fun main() {\n    println(\"hello\")\n}\n").unwrap();
        fs::write(root.join("b.java"), "class B {\n    void hello() {}\n}\n").unwrap();
        fs::write(root.join("c.txt"), "hello world\nfoo bar\n").unwrap();

        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("d.kt"), "val x = \"hello\"").unwrap();

        dir
    }

    #[test]
    fn test_search_basic() {
        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);

        let matches = engine.search().unwrap();
        assert!(matches.len() >= 3); // a.kt, b.java, c.txt, sub/d.kt
    }

    #[test]
    fn test_search_with_file_filter() {
        let dir = create_test_project();
        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        config.include_globs = vec!["*.kt".to_string()];
        let engine = SearchEngine::new(config);

        let matches = engine.search().unwrap();
        // 只搜索 .kt 文件:a.kt 和 sub/d.kt
        assert!(matches.iter().all(|m| {
            m.file_path.extension().map(|e| e == "kt").unwrap_or(false)
        }));
    }

    #[test]
    fn test_search_cancel() {
        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);

        // 预先取消
        engine.cancel();
        let result = engine.search();
        assert!(matches!(result, Err(SearchError::Cancelled)));
    }

    #[test]
    fn test_search_stream() {
        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);

        let rx = engine.search_stream().unwrap();
        let matches: Vec<_> = rx
            .iter()
            .filter_map(|r| r.ok())
            .collect();

        assert!(matches.len() >= 3);
    }

    #[test]
    fn test_search_stream_cancel() {
        let dir = create_test_project();
        let config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        let engine = SearchEngine::new(config);

        engine.cancel();
        let rx = engine.search_stream().unwrap();

        // 应该收到 Cancelled 错误或空结果
        let results: Vec<_> = rx.iter().collect();
        // 第一个结果应是错误
        assert!(results.iter().any(|r| r.is_err()));
    }

    #[test]
    fn test_search_max_total_matches() {
        let dir = create_test_project();
        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], "hello".into());
        config.max_total_matches = 2;
        let engine = SearchEngine::new(config);

        let matches = engine.search().unwrap();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_search_regex_pattern() {
        let dir = create_test_project();
        let mut config = SearchConfig::new(vec![dir.path().to_path_buf()], r"print\w+".into());
        config.is_regex = true;
        let engine = SearchEngine::new(config);

        let matches = engine.search().unwrap();
        assert!(!matches.is_empty());
        // 应匹配 a.kt 中的 println
        assert!(matches.iter().any(|m| m.matched_text.contains("println")));
    }
}

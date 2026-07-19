//! 上下文行提取器
//!
//! 在匹配行周围提取前 N 行与后 N 行作为上下文。
//! M1:采用 mmap + 行偏移索引,避免 fs::read 全量拷贝 + Vec<String> 内存翻倍。
//! mmap 是 lazy 按页加载,实际 I/O 量按需;行偏移索引只存字节位置(usize),
//! 不复制行内容,内存占用从 O(文件大小) 降到 O(行数 × 8 字节)。
//!
//! 大文件保护:超过 MAX_CONTEXT_FILE_SIZE(10MB) 不创建 mmap,
//! 返回空提取器,匹配结果仍正常返回,仅缺少上下文。
//! 文件变动风险:mmap 期间文件被截断会触发 SIGBUS,
//! 通过 metadata 大小校验 + 调用方降级策略降低风险(matcher.rs 已有 try-catch)。

use std::fs::{self, File};
use std::path::Path;

use memmap2::Mmap;

use crate::error::SearchResult;

/// 大文件阈值:超过此大小不提取上下文行(避免 mmap 占用虚拟地址空间)
/// 10MB 足以覆盖绝大多数源代码文件;超大文件(如 minified JS、大 JSON、日志)跳过上下文
const MAX_CONTEXT_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 上下文行提取器
///
/// M1:改用 mmap + 行偏移索引,避免 fs::read 全量拷贝 + Vec<String> 内存翻倍。
/// mmap 区域按需分页加载,行偏移索引仅存字节位置,不复制行内容。
pub struct ContextExtractor {
    /// mmap 映射区域;大文件或打开失败时为 None
    mmap: Option<Mmap>,
    /// 每行起始字节偏移(0-based);mmap 为 None 时为空
    line_offsets: Vec<usize>,
}

impl ContextExtractor {
    /// 创建提取器:打开文件 + mmap + 计算行偏移
    ///
    /// 使用 `from_utf8_lossy` 容忍非 UTF-8 编码文件(二进制、Latin-1、GBK 等),
    /// 与 matcher.rs 的处理方式保持一致,避免因编码问题中断搜索。
    ///
    /// **大文件保护**:超过 `MAX_CONTEXT_FILE_SIZE` 的文件返回空提取器,
    /// 不提取上下文行,避免内存爆炸。匹配结果仍正常返回,仅缺少上下文。
    ///
    /// M1:任何失败(metadata/open/mmap)都降级为空提取器,不返回 Err,
    /// 避免单文件上下文提取失败中断整体搜索(matcher.rs 已有 try-catch 兜底)。
    pub fn new(path: &Path, _window_size: usize) -> SearchResult<Self> {
        // M1:metadata 失败(文件被删除/权限不足)降级为 size=0
        let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if file_size > MAX_CONTEXT_FILE_SIZE {
            return Ok(Self { mmap: None, line_offsets: Vec::new() });
        }

        // M1:用 mmap 替代 fs::read
        // File::open 失败(文件被删除/权限)降级为空提取器
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Ok(Self { mmap: None, line_offsets: Vec::new() }),
        };

        // mmap 创建失败降级为空提取器(不中断搜索)
        // unsafe:文件被截断时触发 SIGBUS,由 metadata 校验 + 调用方降级控制
        let mmap = unsafe { Mmap::map(&file) }.ok();
        let mmap = match mmap {
            Some(m) => m,
            None => return Ok(Self { mmap: None, line_offsets: Vec::new() }),
        };

        // 计算行偏移索引(不复制行内容)
        let line_offsets = compute_line_offsets(&mmap[..]);

        Ok(Self {
            mmap: Some(mmap),
            line_offsets,
        })
    }

    /// 提取指定行号(从 1 开始)的上下文
    /// 返回 (前 N 行, 后 N 行)
    pub fn extract(&mut self, line_number: usize, n: usize) -> (Vec<String>, Vec<String>) {
        let mmap = match &self.mmap {
            Some(m) => m,
            None => return (Vec::new(), Vec::new()),
        };
        if self.line_offsets.is_empty() || line_number == 0 {
            return (Vec::new(), Vec::new());
        }

        let idx = line_number - 1; // 转 0-based
        if idx >= self.line_offsets.len() {
            return (Vec::new(), Vec::new());
        }

        let bytes = &mmap[..];

        // 提取前 N 行
        let before_start = idx.saturating_sub(n);
        let mut context_before = Vec::with_capacity(n);
        for i in before_start..idx {
            let line = read_line(bytes, &self.line_offsets, i);
            context_before.push(line);
        }

        // 提取后 N 行
        let after_end = std::cmp::min(idx + 1 + n, self.line_offsets.len());
        let mut context_after = Vec::with_capacity(n);
        for i in (idx + 1)..after_end {
            let line = read_line(bytes, &self.line_offsets, i);
            context_after.push(line);
        }

        (context_before, context_after)
    }
}

/// 从 mmap 字节切片读取第 `i` 行(0-based),trim 结尾换行符,容忍非 UTF-8
fn read_line(bytes: &[u8], line_offsets: &[usize], i: usize) -> String {
    let start = line_offsets[i];
    let end = if i + 1 < line_offsets.len() {
        line_offsets[i + 1]
    } else {
        bytes.len()
    };
    String::from_utf8_lossy(&bytes[start..end])
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string()
}

/// 计算每行起始字节偏移
fn compute_line_offsets(bytes: &[u8]) -> Vec<usize> {
    let mut offsets = vec![0]; // 第一行从 0 开始
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' && i + 1 < bytes.len() {
            offsets.push(i + 1);
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_extract_context() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\nline4\nline5\n").unwrap();

        let mut extractor = ContextExtractor::new(&file, 2).unwrap();
        let (before, after) = extractor.extract(3, 2);

        assert_eq!(before, vec!["line1".to_string(), "line2".to_string()]);
        assert_eq!(after, vec!["line4".to_string(), "line5".to_string()]);
    }

    #[test]
    fn test_extract_at_start() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let mut extractor = ContextExtractor::new(&file, 2).unwrap();
        let (before, after) = extractor.extract(1, 2);

        assert!(before.is_empty());
        assert_eq!(after, vec!["line2".to_string(), "line3".to_string()]);
    }

    #[test]
    fn test_extract_at_end() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "line1\nline2\nline3\n").unwrap();

        let mut extractor = ContextExtractor::new(&file, 2).unwrap();
        let (before, after) = extractor.extract(3, 2);

        assert_eq!(before, vec!["line1".to_string(), "line2".to_string()]);
        assert!(after.is_empty());
    }

    /// 非 UTF-8 文件(含非法字节)不应导致读取失败,
    /// 应通过 from_utf8_lossy 容错处理。
    #[test]
    fn test_non_utf8_file_does_not_error() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("binary.bin");
        // 0xFF 0xFE 是非 UTF-8 合法字节序列
        fs::write(&file, b"line1\n\xFF\xFEinvalid\nline3\n").unwrap();

        let mut extractor = ContextExtractor::new(&file, 2).unwrap();
        let (before, after) = extractor.extract(2, 1);

        assert_eq!(before, vec!["line1".to_string()]);
        assert_eq!(after, vec!["line3".to_string()]);
    }

    /// M1:文件被删除时降级为空提取器,不返回 Err
    #[test]
    fn test_file_deleted_does_not_error() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("deleted.txt");
        // 不创建文件,直接尝试打开
        let extractor = ContextExtractor::new(&file, 2).unwrap();
        assert!(extractor.mmap.is_none());
        assert!(extractor.line_offsets.is_empty());
    }

    /// M1:大文件(>10MB)降级为空提取器
    #[test]
    fn test_large_file_skipped() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("large.txt");
        // 写入 11MB 数据
        let large_content = "x".repeat(11 * 1024 * 1024);
        fs::write(&file, large_content).unwrap();

        let extractor = ContextExtractor::new(&file, 2).unwrap();
        assert!(extractor.mmap.is_none());
    }
}

//! 路径处理工具
//!
//! 处理 UTF-8 路径转换、中文路径兼容、路径规范化等。

use std::path::{Path, PathBuf};

/// 将路径转换为 UTF-8 字符串,丢失非 UTF-8 字节时用 lossy 转换
pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// 规范化路径:合并 `.`、`..`,解析符号链接
pub fn normalize_path(path: &Path) -> PathBuf {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical
}

/// 判断路径是否为隐藏文件(以 . 开头)
pub fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.starts_with('.'))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_hidden() {
        assert!(is_hidden(Path::new("/tmp/.hidden")));
        assert!(is_hidden(Path::new("/tmp/.gitignore")));
        assert!(!is_hidden(Path::new("/tmp/normal.txt")));
        assert!(!is_hidden(Path::new("/tmp/dir")));
    }

    #[test]
    fn test_path_to_string() {
        assert_eq!(path_to_string(Path::new("/tmp/test")), "/tmp/test");
    }
}

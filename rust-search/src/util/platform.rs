//! 平台差异抽象
//!
//! 处理不同操作系统间的路径分隔符、动态库扩展名等差异。

use std::path::PathBuf;

/// 返回当前平台的动态库文件扩展名
pub fn dynamic_lib_extension() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "dylib"
    }
    #[cfg(target_os = "linux")]
    {
        "so"
    }
    #[cfg(target_os = "windows")]
    {
        "dll"
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        "so"
    }
}

/// 返回当前平台预期的动态库文件名(不含 lib 前缀的名称)
pub fn dynamic_lib_filename(base_name: &str) -> String {
    let ext = dynamic_lib_extension();
    #[cfg(not(target_os = "windows"))]
    {
        format!("lib{base_name}.{ext}")
    }
    #[cfg(target_os = "windows")]
    {
        format!("{base_name}.{ext}")
    }
}

/// 返回当前平台的主目录路径
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_lib_extension() {
        let ext = dynamic_lib_extension();
        #[cfg(target_os = "macos")]
        assert_eq!(ext, "dylib");
        #[cfg(target_os = "linux")]
        assert_eq!(ext, "so");
        #[cfg(target_os = "windows")]
        assert_eq!(ext, "dll");
    }

    #[test]
    fn test_dynamic_lib_filename() {
        let name = dynamic_lib_filename("rust_search");
        #[cfg(target_os = "macos")]
        assert_eq!(name, "librust_search.dylib");
        #[cfg(target_os = "linux")]
        assert_eq!(name, "librust_search.so");
        #[cfg(target_os = "windows")]
        assert_eq!(name, "rust_search.dll");
    }
}

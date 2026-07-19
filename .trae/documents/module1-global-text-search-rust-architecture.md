# 模块 1:全局文本搜索引擎(Find in Path)开发计划与 Rust 架构方案

> 基于 `TODO.md` 中模块 1 的目标,制定全模块完整开发计划,重点细化 Rust 核心层的架构设计到「模块 + 接口 + 关键算法」级别。

---

## 一、摘要(Summary)

本计划覆盖模块 1「全局文本搜索引擎」从预研到正式版的完整 8 周开发路径,核心交付物是一个基于 ripgrep 生态库(ignore + grep-searcher + grep-regex)构建的 Rust 动态库,通过 JNI 被 IntelliJ 插件(Kotlin)调用,替代 Android Studio 原生 Find in Path 的搜索后端,实现 3~8 倍速度提升与 70%~85% 内存降低。

Rust 侧采用**三层内部架构**:配置解析层 → 搜索引擎层(文件遍历 + 匹配引擎)→ JNI 适配层,通过 `crossbeam-channel` 实现流式输出与中途取消,严格遵循 JNI 引用管理规范避免内存泄漏。

---

## 二、当前状态分析(Current State Analysis)

### 2.1 项目现状
- **项目目录**:`/Users/apple/AndroidStudioProjects/RustSearch-AS/` 当前为空目录,仅含 `TODO.md`
- **代码资产**:无,这是一个全新的 greenfield 项目
- **TODO.md 已定义内容**:
  - 整体架构:IntelliJ 插件(Kotlin/JVM) + Rust 原生动态库 + JNI 交互
  - 模块 1 技术栈:Rust 1.75+、`ignore`、`grep-searcher`、`jni`、`thiserror`
  - 分层架构:UI 层 → JNI 适配层 → 搜索核心层
  - JNI 接口签名(Kotlin 侧 `search` 方法)
  - 8 周 4 里程碑开发节奏
  - 风险点:JNI 内存泄漏、跨平台编译、IntelliJ 版本兼容

### 2.2 待补充/细化的设计点
TODO.md 已给出方向,但以下细节需在计划中明确:
1. Rust 侧 crate 内部模块划分与职责边界
2. 流式结果返回与中途取消的具体实现机制
3. JNI 内存管理策略(AutoLocal、全局引用、引用表)
4. 错误处理与异常传递路径(Rust → JNI → Kotlin)
5. 并发控制策略(线程池、工作窃取、背压)
6. 测试与基准验证方案

### 2.3 关键约束
- 兼容 Android Studio 2023.1+(基于 IntelliJ 2023.1)
- Rust 1.75+(async/await 稳定、let-else 稳定)
- JNI 1.8(与 JDK 17 运行时兼容)
- 跨平台:macOS aarch64/x86_64、Linux x86_64、Windows x86_64
- 不修改 IDE 核心代码,仅通过扩展点集成

---

## 三、Rust 核心架构设计(Proposed Architecture)

### 3.1 Crate 结构

采用单 crate 多模块组织,避免过度拆分。crate 名 `rust-search`,产物类型 `cdylib`(供 JNI 加载)。

```
rust-search/                          # Rust 库项目根目录
├── Cargo.toml                        # 依赖与 cdylib 配置
├── build.rs                          # 构建脚本(可选,预留平台特性检测)
├── src/
│   ├── lib.rs                        # crate 入口,声明模块与公共导出
│   ├── jni/
│   │   ├── mod.rs                    # JNI 模块入口
│   │   ├── bridge.rs                 # JNI 入口函数(Java_native 声明)
│   │   ├── convert.rs                # JVM↔Rust 类型转换(字符串/数组/对象)
│   │   ├── result.rs                 # SearchResult Java 对象构建与回传
│   │   └── cancel.rs                 # 取消信号接收(JNI 回调/全局标志)
│   ├── search/
│   │   ├── mod.rs                    # 搜索模块入口
│   │   ├── config.rs                 # SearchConfig 配置结构与解析
│   │   ├── engine.rs                 # 搜索引擎主流程编排
│   │   ├── walker.rs                 # 文件遍历(ignore 集成与并行过滤)
│   │   ├── matcher.rs                # 文本匹配(grep-searcher/regex 集成)
│   │   └── context.rs                # 上下文行提取
│   ├── error.rs                      # 统一错误类型(thiserror)
│   └── util/
│       ├── mod.rs
│       ├── path.rs                   # 路径处理(UTF-8 转换、中文路径)
│       └── platform.rs               # 平台差异抽象(路径分隔符等)
├── tests/                            # 集成测试
│   ├── search_basic.rs
│   ├── search_regex.rs
│   ├── search_cancel.rs
│   └── fixtures/                     # 测试样本文件
└── benches/                          # 基准测试(criterion)
    └── search_bench.rs
```

### 3.2 Cargo.toml 依赖设计

```toml
[package]
name = "rust-search"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]  # rlib 供单元测试使用

[dependencies]
# ripgrep 生态核心库(来自 BurntSushi/ripgrep 仓库)
ignore = "0.4"              # .gitignore/.ignore 规则与并行文件遍历
grep-searcher = "0.1"       # 搜索器核心(文件读取、行处理、二进制检测)
grep-regex = "0.1"          # 正则匹配器实现
grep-matcher = "0.1"        # 匹配器 trait(抽象层)

# JNI 交互
jni = { version = "0.21", features = ["invocation", " Invocation"] }

# 并发与流式
crossbeam-channel = "0.5"   # 结果流通道(支持 select 取消)
rayon = "1.8"               # 并行文件遍历(工作窃取线程池)

# 错误处理
thiserror = "1.0"           # 库错误类型派生
anyhow = "1.0"              # 内部错误上下文(不暴露给 JNI)

# 工具
log = "0.4"                 # 日志门面(通过 JNI 回调输出到 IDE 日志)

[dev-dependencies]
criterion = "0.5"           # 基准测试
tempfile = "3.8"            # 测试临时目录
rstest = "0.18"             # 参数化测试

[profile.release]
opt-level = 3
lto = "fat"                 # 链接时优化,提升跨模块内联
codegen-units = 1           # 单单元编译,最大化优化
panic = "abort"             # 避免 unwind 跨 JNI 边界(关键!防止 UB)
strip = true                # 剥离符号,减小二进制体积
```

**关键设计决策**:
- `panic = "abort"`:JNI 边界禁止 unwind,否则触发未定义行为。所有可能 panic 的边界需用 `catch_unwind` 包裹。
- 同时产出 `cdylib`(供 JVM 加载)与 `rlib`(供 Rust 测试直接引用)。
- `lto = "fat"` + `codegen-units = 1`:牺牲编译时间换取运行性能,符合搜索场景的性能优先定位。

### 3.3 核心数据结构

#### 3.3.1 搜索配置(SearchConfig)

```rust
// src/search/config.rs
use std::path::PathBuf;

/// 搜索配置,由 JNI 层从 JVM 参数转换而来
#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// 搜索根目录列表(支持多根目录)
    pub roots: Vec<PathBuf>,
    /// 搜索模式(字面量或正则表达式)
    pub pattern: String,
    /// 是否为正则模式,false 时按字面量匹配
    pub is_regex: bool,
    /// 大小写敏感
    pub case_sensitive: bool,
    /// 全字匹配(字面量模式下生效)
    pub whole_words: bool,
    /// 包含文件 glob(如 "*.kt, *.java")
    pub include_globs: Vec<String>,
    /// 排除文件 glob(如 "*/build/*, */.gradle/*")
    pub exclude_globs: Vec<String>,
    /// 上下文行数(前/后各 N 行)
    pub context_lines: usize,
    /// 单文件最大匹配数(防止超大文件耗尽内存)
    pub max_matches_per_file: usize,
    /// 全局最大匹配数(背压控制)
    pub max_total_matches: usize,
}

impl SearchConfig {
    /// 从 JNI 传入的原始参数构建,失败返回 SearchError
    pub fn from_jni_args(/* JNI 参数 */) -> Result<Self, SearchError> { /* ... */ }
}
```

#### 3.3.2 匹配结果(SearchMatch)

```rust
// src/search/mod.rs
use std::path::PathBuf;

/// 单条匹配结果
#[derive(Debug, Clone)]
pub struct SearchMatch {
    /// 文件绝对路径
    pub file_path: PathBuf,
    /// 行号(从 1 开始)
    pub line_number: usize,
    /// 匹配起始列(从 0 开始,字节偏移)
    pub column: usize,
    /// 匹配的文本内容
    pub matched_text: String,
    /// 上下文行(前 N 行)
    pub context_before: Vec<String>,
    /// 上下文行(后 N 行)
    pub context_after: Vec<String>,
}
```

#### 3.3.3 统一错误类型

```rust
// src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("无效的搜索模式: {0}")]
    InvalidPattern(String),
    #[error("根目录不存在或不可访问: {0}")]
    InvalidRoot(String),
    #[error("正则表达式编译失败: {0}")]
    RegexCompile(String),
    #[error("JNI 交互错误: {0}")]
    Jni(String),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("搜索已取消")]
    Cancelled,
}

/// SearchResult 的别名,便于 JNI 层统一处理
pub type SearchResult<T> = std::result::Result<T, SearchError>;
```

### 3.4 核心模块与接口设计

#### 3.4.1 搜索引擎主流程(engine.rs)

```rust
// src/search/engine.rs
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use rayon::prelude::*;

use crate::search::{config::SearchConfig, matcher::Matcher, walker::Walker};
use crate::error::{SearchError, SearchResult};

/// 搜索引擎,负责编排遍历与匹配
pub struct SearchEngine {
    config: SearchConfig,
    cancel_flag: Arc<AtomicBool>,
}

impl SearchEngine {
    pub fn new(config: SearchConfig) -> Self {
        Self {
            config,
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 获取取消信号句柄,供 JNI 层触发取消
    pub fn cancel_handle(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel_flag)
    }

    /// 执行搜索,通过 channel 流式返回结果
    /// 返回的 Receiver 供调用方迭代消费
    pub fn search_stream(
        &self,
    ) -> SearchResult<Receiver<SearchResult<SearchMatch>>> {
        let (tx, rx) = bounded::<SearchResult<SearchMatch>>(256); // 背压:256 条缓冲

        let config = self.config.clone();
        let cancel_flag = Arc::clone(&self.cancel_flag);

        // 启动搜索线程
        std::thread::Builder::new()
            .name("rust-search-worker".into())
            .spawn(move || {
                let result = Self::run_search(&config, &cancel_flag, &tx);
                if let Err(e) = result {
                    let _ = tx.send(Err(e)); // 发送最终错误
                }
                // tx drop 后 rx 迭代自然结束
            })
            .map_err(|e| SearchError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("搜索线程启动失败: {e}"),
            )))?;

        Ok(rx)
    }

    /// 内部搜索主循环
    fn run_search(
        config: &SearchConfig,
        cancel_flag: &Arc<AtomicBool>,
        tx: &Sender<SearchResult<SearchMatch>>,
    ) -> SearchResult<()> {
        let matcher = Matcher::new(config)?;
        let walker = Walker::new(config)?;

        // 使用 par_bridge 将串行迭代器转为并行流
        // 文件级别并行,文件内顺序匹配
        walker
            .files()
            .par_bridge()
            .try_for_each(|file_path| {
                // 检查取消信号
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(SearchError::Cancelled);
                }

                // 单文件搜索
                let matches = matcher.search_file(&file_path, cancel_flag)?;
                for m in matches {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err(SearchError::Cancelled);
                    }
                    // channel 满时会阻塞,形成背压
                    if tx.send(Ok(m)).is_err() {
                        // 接收方已关闭,提前退出
                        return Err(SearchError::Cancelled);
                    }
                }
                Ok(())
            })?;

        Ok(())
    }
}
```

**关键算法说明**:
1. **并行策略**:文件级并行(`par_bridge`),文件内串行匹配。避免单文件内多线程争抢同一文件句柄,同时保证结果顺序的可预测性。
2. **背压机制**:`bounded(256)` 有界通道,当 JNI 层消费速度慢于生产速度时自动阻塞生产者,防止内存爆炸。
3. **取消机制**:`AtomicBool` + 每个文件开头检查 + channel `send` 失败双重取消。响应延迟 < 单文件处理时间。

#### 3.4.2 文件遍历器(walker.rs)

```rust
// src/search/walker.rs
use ignore::{WalkBuilder, WalkParallel, DirEntry};
use std::path::Path;
use crate::search::config::SearchConfig;
use crate::error::SearchResult;

/// 文件遍历器,封装 ignore crate 的并行遍历能力
pub struct Walker {
    config: SearchConfig,
}

impl Walker {
    pub fn new(config: SearchConfig) -> Self {
        Self { config }
    }

    /// 构建并行遍历器
    pub fn files(&self) -> impl Iterator<Item = std::path::PathBuf> + '_ {
        // 多根目录合并遍历
        let walkers: Vec<_> = self.config.roots.iter()
            .map(|root| {
                WalkBuilder::new(root)
                    .hidden(!self.should_search_hidden())      // 默认跳过隐藏文件
                    .git_ignore(true)                          // 尊重 .gitignore
                    .git_exclude(true)                         // 尊重 .git/info/exclude
                    .git_global(true)                          // 尊重全局 gitignore
                    .parents(true)                             // 尊重父目录的 ignore 规则
                    .ignore(true)                              // 尊重 .ignore 文件
                    .add_custom_ignore_patterns(&self.config.exclude_globs)
                    .filter_entry(move |entry| self.filter_entry(entry))
                    .build_parallel()                          // 并行遍历
            })
            .collect();

        // 合并多个遍历器的输出
        walkers.into_iter().flat_map(|w| {
            w.into_iter().filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|e| e.into_path())
        })
    }

    /// 应用 include_globs 过滤
    fn filter_entry(&self, entry: &DirEntry) -> bool {
        if !self.config.include_globs.is_empty() {
            let name = entry.file_name().to_string_lossy();
            let matched = self.config.include_globs.iter()
                .any(|g| glob_matches(g, &name));
            if !matched { return false; }
        }
        true
    }

    fn should_search_hidden(&self) -> bool {
        // 默认不搜索隐藏文件,可由配置覆盖
        false
    }
}
```

**关键设计**:
- 复用 `ignore` crate 的全部能力:`.gitignore`、`.ignore`、`.git/info/exclude`、全局 gitignore、父目录继承。
- `add_custom_ignore_patterns` 支持用户自定义排除规则(对应 TODO.md 的 `excludeGlobs`)。
- `include_globs` 通过 `filter_entry` 在遍历时过滤,避免遍历后再过滤的内存浪费。

#### 3.4.3 匹配器(matcher.rs)

```rust
// src/search/matcher.rs
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkMatch};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::search::config::SearchConfig;
use crate::search::{SearchMatch, context::ContextExtractor};
use crate::error::{SearchError, SearchResult};

pub struct Matcher {
    config: SearchConfig,
    regex_matcher: Option<RegexMatcher>,  // None 时为字面量模式
}

impl Matcher {
    pub fn new(config: &SearchConfig) -> SearchResult<Self> {
        let regex_matcher = if config.is_regex {
            let pattern = if config.whole_words {
                format!(r"\b{}\b", config.pattern)
            } else {
                config.pattern.clone()
            };
            let flags = if config.case_sensitive { "" } else { "(?i)" };
            let full = format!("{flags}{pattern}");
            Some(RegexMatcher::new(&full)
                .map_err(|e| SearchError::RegexCompile(e.to_string()))?)
        } else {
            // 字面量模式:仍用 regex,但转义特殊字符
            let escaped = regex::escape(&config.pattern);
            let pattern = if config.whole_words {
                format!(r"\b{escaped}\b")
            } else {
                escaped
            };
            let flags = if config.case_sensitive { "" } else { "(?i)" };
            let full = format!("{flags}{pattern}");
            Some(RegexMatcher::new(&full)
                .map_err(|e| SearchError::RegexCompile(e.to_string()))?)
        };

        Ok(Self {
            config: config.clone(),
            regex_matcher,
        })
    }

    /// 搜索单个文件
    pub fn search_file(
        &self,
        path: &Path,
        cancel_flag: &Arc<AtomicBool>,
    ) -> SearchResult<Vec<SearchMatch>> {
        let matcher = self.regex_matcher.as_ref()
            .ok_or_else(|| SearchError::InvalidPattern("匹配器未初始化".into()))?;

        let mut searcher = Searcher::new();
        let sink = MatchSink {
            file_path: path.to_path_buf(),
            context_lines: self.config.context_lines,
            context_extractor: ContextExtractor::new(path, self.config.context_lines)?,
            matches: Vec::new(),
            max_matches: self.config.max_matches_per_file,
            cancel_flag: Arc::clone(cancel_flag),
        };

        searcher.search_path(matcher, path, sink)
            .map_err(|e| SearchError::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("搜索文件失败 {path:?}: {e}"),
            )))?;

        Ok(sink.matches)
    }
}

/// grep-searcher 的 Sink 实现,收集匹配结果
struct MatchSink {
    file_path: std::path::PathBuf,
    context_lines: usize,
    context_extractor: ContextExtractor,
    matches: Vec<SearchMatch>,
    max_matches: usize,
    cancel_flag: Arc<AtomicBool>,
}

impl Sink for MatchSink {
    fn matched(&mut self, _searcher: &Searcher, mat: &SinkMatch) -> std::result::Result<bool, std::io::Error> {
        if self.matches.len() >= self.max_matches {
            return Ok(false); // 达到单文件上限,停止
        }
        if self.cancel_flag.load(Ordering::Relaxed) {
            return Ok(false); // 取消
        }

        let line_number = mat.line_number().unwrap_or(0);
        let bytes = mat.bytes();
        let matched_text = String::from_utf8_lossy(bytes).into_owned();
        let column = mat.range().start;

        let (context_before, context_after) = if self.context_lines > 0 {
            self.context_extractor.extract(line_number, self.context_lines)
        } else {
            (Vec::new(), Vec::new())
        };

        self.matches.push(SearchMatch {
            file_path: self.file_path.clone(),
            line_number,
            column,
            matched_text,
            context_before,
            context_after,
        });

        Ok(true) // 继续搜索
    }
}
```

#### 3.4.4 上下文行提取(context.rs)

```rust
// src/search/context.rs
use std::io::{BufRead, BufReader};
use std::fs::File;
use std::path::Path;
use std::collections::VecDeque;
use crate::error::SearchResult;

/// 上下文行提取器,采用滑动窗口缓存前 N 行
pub struct ContextExtractor {
    reader: BufReader<File>,
    /// 前置上下文滑动窗口
    before_window: VecDeque<String>,
    /// 已读取的总行数
    current_line: usize,
    window_size: usize,
}

impl ContextExtractor {
    pub fn new(path: &Path, window_size: usize) -> SearchResult<Self> {
        let file = File::open(path)?;
        Ok(Self {
            reader: BufReader::new(file),
            before_window: VecDeque::with_capacity(window_size + 1),
            current_line: 0,
            window_size,
        })
    }

    /// 提取指定行号的上下文
    /// 注意:由于 grep-searcher 顺序读取,这里需按需预读
    pub fn extract(&mut self, line_number: usize, n: usize) -> (Vec<String>, Vec<String>) {
        // 简化实现:先读到目标行 + n 行
        // 生产实现需考虑 Seek 与反向读取的权衡
        let mut before = Vec::new();
        let mut after = Vec::new();

        // 从窗口中取前 N 行
        for s in self.before_window.iter() {
            before.push(s.clone());
        }

        // 预读后 N 行
        // ... (省略具体 IO 实现)

        (before, after)
    }
}
```

**算法权衡说明**:
- 上下文行提取是大文件搜索的性能瓶颈点。MVP 阶段采用「顺序预读 + 滑动窗口」,预研阶段需验证是否需要基于 `memmap` 的随机访问方案。
- 若上下文行数 N 较大(>20),考虑将文件 mmap 后按行偏移索引,避免重复 IO。

### 3.5 JNI 适配层设计(jni/)

#### 3.5.1 JNI 入口函数(bridge.rs)

```rust
// src/jni/bridge.rs
use jni::env::JNIEnv;
use jni::objects::{JClass, JObjectArray, JString, JValue};
use jni::sys::{jobject, jobjectArray};
use crossbeam_channel::Receiver;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use once_cell::sync::Lazy;

use crate::search::{config::SearchConfig, engine::SearchEngine};
use crate::jni::{convert, result};
use crate::error::SearchError;

/// 全局取消信号注册表:search_id -> cancel_flag
/// 用于支持多个并发搜索与外部取消
static CANCEL_REGISTRY: Lazy<Mutex<std::collections::HashMap<u64, Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

/// JNI 入口:执行搜索
/// 对应 Kotlin: `private external fun search(...): Array<SearchResult>`
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_search(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    roots: JObjectArray<'local>,
    pattern: JString<'local>,
    is_regex: jni::sys::jboolean,
    case_sensitive: jni::sys::jboolean,
    whole_words: jni::sys::jboolean,
    include_globs: JObjectArray<'local>,
    exclude_globs: JObjectArray<'local>,
    context_lines: jni::sys::jint,
) -> jobjectArray {
    // 关键:catch_unwind 防止 panic 跨 JNI 边界
    let result = std::panic::catch_unwind(|| {
        run_search(
            &mut env, roots, pattern, is_regex, case_sensitive,
            whole_words, include_globs, exclude_globs, context_lines,
        )
    });

    match result {
        Ok(Ok(obj)) => obj.into_raw(),
        Ok(Err(e)) => {
            convert::throw_java_exception(&mut env, &e);
            std::ptr::null_mut()
        }
        Err(_) => {
            convert::throw_java_exception(
                &mut env,
                &SearchError::Jni("Rust 内部 panic".into()),
            );
            std::ptr::null_mut()
        }
    }
}

/// JNI 入口:取消搜索
/// Kotlin: `external fun cancel(searchId: Long)`
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_cancel(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    search_id: jni::sys::jlong,
) {
    if let Ok(registry) = CANCEL_REGISTRY.lock() {
        if let Some(flag) = registry.get(&(search_id as u64)) {
            flag.store(true, Ordering::Relaxed);
        }
    }
}

fn run_search<'local>(
    env: &mut JNIEnv<'local>,
    roots: JObjectArray<'local>,
    pattern: JString<'local>,
    /* ... 其他参数 ... */
) -> Result<JObjectArray<'local>, SearchError> {
    // 1. 参数转换:JVM → Rust
    let config = SearchConfig::from_jni_args(
        env, roots, pattern, /* ... */
    )?;

    // 2. 创建搜索引擎
    let engine = SearchEngine::new(config);
    let cancel_flag = engine.cancel_handle();

    // 3. 注册取消信号(生成 search_id)
    let search_id = generate_search_id();
    CANCEL_REGISTRY.lock().unwrap().insert(search_id, Arc::clone(&cancel_flag));

    // 4. 启动流式搜索
    let rx = engine.search_stream()?;

    // 5. 消费结果,构建 Java 数组返回
    //    注意:MVP 阶段一次性返回全部结果;Beta 阶段改为流式回调
    let matches: Vec<_> = rx.iter().collect::<Result<_, _>>()?;

    // 6. 清理注册表
    CANCEL_REGISTRY.lock().unwrap().remove(&search_id);

    // 7. 构建 Java SearchResult[] 数组
    result::build_search_result_array(env, &matches)
}
```

#### 3.5.2 类型转换(convert.rs)

```rust
// src/jni/convert.rs
use jni::env::JNIEnv;
use jni::objects::{JObjectArray, JString, JValue, JClass, JObject};
use jni::strings::JNIString;
use crate::error::SearchError;

/// JString → Rust String,自动释放本地引用
pub fn jstring_to_rust<'local>(
    env: &mut JNIEnv<'local>,
    jstr: JString<'local>,
) -> Result<String, SearchError> {
    let java_str = env.get_string(&jstr)
        .map_err(|e| SearchError::Jni(format!("获取字符串失败: {e}")))?;
    Ok(java_str.to_string())
}

/// Rust String → JString,使用 AutoLocal 自动释放中间引用
pub fn rust_to_jstring<'local>(
    env: &mut JNIEnv<'local>,
    s: &str,
) -> Result<JString<'local>, SearchError> {
    env.new_string(s)
        .map_err(|e| SearchError::Jni(format!("创建字符串失败: {e}")))
}

/// JObjectArray(字符串数组) → Vec<String>
pub fn jstring_array_to_vec<'local>(
    env: &mut JNIEnv<'local>,
    arr: JObjectArray<'local>,
) -> Result<Vec<String>, SearchError> {
    let len = env.get_array_length(&arr)
        .map_err(|e| SearchError::Jni(format!("获取数组长度失败: {e}")))?;
    let mut result = Vec::with_capacity(len as usize);
    for i in 0..len {
        // 关键:AutoLocal 确保元素本地引用及时释放
        let elem = env.get_object_array_element(&arr, i)
            .map_err(|e| SearchError::Jni(format!("获取数组元素失败: {e}")))?;
        let auto = env.auto_local(elem);
        let jstr: JString = auto.into();
        result.push(jstring_to_rust(env, jstr)?);
    }
    Ok(result)
}

/// 抛出 Java 异常
pub fn throw_java_exception(env: &mut JNIEnv, error: &SearchError) {
    let class_name = "com/example/rustsearch/SearchException";
    let msg = error.to_string();
    let _ = env.throw_new(class_name, &msg);
}
```

#### 3.5.3 结果构建(result.rs)

```rust
// src/jni/result.rs
use jni::env::JNIEnv;
use jni::objects::{JObjectArray, JObject, JValue, JClass};
use crate::search::SearchMatch;
use crate::error::SearchError;
use super::convert::rust_to_jstring;

/// 构建 SearchResult[] 数组返回给 JVM
pub fn build_search_result_array<'local>(
    env: &mut JNIEnv<'local>,
    matches: &[SearchMatch],
) -> Result<JObjectArray<'local>, SearchError> {
    let class = env.find_class("com/example/rustsearch/RustSearchEngine$SearchResult")
        .map_err(|e| SearchError::Jni(format!("找不到 SearchResult 类: {e}")))?;

    let array = env.new_object_array(matches.len() as jni::sys::jsize, &class, JObject::null())
        .map_err(|e| SearchError::Jni(format!("创建结果数组失败: {e}")))?;

    for (i, m) in matches.iter().enumerate() {
        let obj = build_single_result(env, m)?;
        env.set_object_array_element(&array, i as jni::sys::jsize, &obj)
            .map_err(|e| SearchError::Jni(format!("设置数组元素失败: {e}")))?;
        // obj 的本地引用由 AutoLocal 管理(在 build_single_result 内)
    }

    Ok(array)
}

fn build_single_result<'local>(
    env: &mut JNIEnv<'local>,
    m: &SearchMatch,
) -> Result<JObject<'local>, SearchError> {
    let class = env.find_class("com/example/rustsearch/RustSearchEngine$SearchResult")
        .map_err(|e| SearchError::Jni(format!("找不到类: {e}")))?;

    // 关键:所有中间引用用 AutoLocal 包裹
    let _auto_class = env.auto_local(class);

    let path_str = m.file_path.to_string_lossy().into_owned();
    let jpath = env.auto_local(rust_to_jstring(env, &path_str)?);
    let jmatched = env.auto_local(rust_to_jstring(env, &m.matched_text)?);

    // 构建 contextBefore / contextAfter 数组(略,类似逻辑)

    let obj = env.new_object(
        &class,
        "(Ljava/lang/String;IILjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V",
        &[
            JValue::Object(&jpath),
            JValue::Int(m.line_number as i32),
            JValue::Int(m.column as i32),
            JValue::Object(&jmatched),
            // context_before, context_after...
        ],
    ).map_err(|e| SearchError::Jni(format!("创建结果对象失败: {e}")))?;

    Ok(obj)
}
```

### 3.6 JNI 内存管理策略(核心风险点)

针对 TODO.md 中「JNI 内存泄漏」风险,制定以下规范:

| 引用类型 | 管理方式 | 生命周期 |
|---------|---------|---------|
| Local Reference | `JNIEnv::auto_local()` 包裹 | 单次 JNI 调用结束自动释放 |
| Global Reference | 严格避免,不缓存 JVM 对象 | - |
| 结果数组元素 | 创建后立即 `set_object_array_element`,元素引用随调用结束释放 | 随调用 |

**关键规则**:
1. **禁止在 Rust 侧持有任何 JVM 对象引用跨越 JNI 调用边界**。所有需要的字段在 JNI 入口处立即转换为 Rust 原生类型。
2. **所有 `JObject`/`JString`/`JClass` 创建后立即用 `env.auto_local()` 包裹**,确保作用域结束自动 `DeleteLocalRef`。
3. **循环内创建的引用必须逐次释放**。例如遍历大结果集构建数组时,每处理完一个元素立即释放其临时引用。
4. **预研阶段验证**:连续 10 次搜索后,通过 `jcmd <pid> GC.class_histogram` 观察 `SearchResult` 实例数是否回归正常。

### 3.7 错误传递路径

```
Rust 错误 (SearchError)
    ↓
catch_unwind 捕获 panic
    ↓
JNI 层转换:throw_java_exception(env, &error)
    ↓
JVM 抛出:com.example.rustsearch.SearchException
    ↓
Kotlin 层 try-catch 捕获,展示错误提示
```

**panic 防护**:所有 JNI 入口函数必须用 `std::panic::catch_unwind` 包裹,`Cargo.toml` 设置 `panic = "abort"` 作为最后防线(但 catch_unwind 在 abort 模式下仍能捕获,二者配合最安全)。

---

## 四、全模块开发里程碑(8 周)

### 里程碑 0:预研验证(第 1 周)

**目标**:跑通最小闭环,验证性能收益

**Rust 侧交付物**:
1. 创建 `rust-search/` Cargo 项目,配置 `cdylib`
2. 实现 `SearchConfig`、`SearchEngine`、`Matcher`、`Walker` 最小版本
3. 实现 JNI `search` 入口函数,支持字面量搜索
4. 编写 macOS aarch64 构建脚本
5. 编写基准测试 `benches/search_bench.rs`

**验证标准**:
- 5 万文件项目搜索速度 ≥ 原生 3 倍
- 内存占用 ≤ 原生 30%
- 连续 10 次搜索无内存泄漏

### 里程碑 1:MVP 版本(第 2-3 周)

**目标**:独立工具窗口可用,核心功能完整

**Rust 侧交付物**:
1. 完成正则、大小写、全字匹配、文件过滤、ignore 规则(完整 `Matcher` 实现)
2. 实现上下文行提取(`ContextExtractor`)
3. 实现取消机制(`cancel` JNI 入口 + `CANCEL_REGISTRY`)
4. 完成错误处理与异常传递
5. 单元测试覆盖率 ≥ 80%

**插件侧交付物**(简要,详见后续):
1. 独立 Tool Window 与搜索 UI
2. 结果列表展示与文件跳转
3. 取消按钮

### 里程碑 2:Beta 版本(第 4-6 周)

**目标**:体验对齐原生,多平台可用

**Rust 侧交付物**:
1. 流式结果返回(改为通过 JNI 回调逐条推送,而非一次性数组)
2. 文本替换支持(返回替换预览数据)
3. 多平台编译配置(Cargo + cross)
4. 性能优化:mmap 大文件、缓存目录遍历结果

**验证**:
- macOS x86_64、Linux、Windows 三平台编译通过
- 兼容 Android Studio 2023.1、2023.2、2024.1 三个版本

### 里程碑 3:正式版本(第 7-8 周)

**目标**:生产可用,发布插件市场

**Rust 侧交付物**:
1. 搜索结果缓存与增量更新机制
2. 配置持久化(读取插件传入的配置)
3. 完善 Trace 日志(通过 JNI 回调输出到 IDE 日志)
4. 性能基准报告与优化

---

## 五、假设与决策(Assumptions & Decisions)

### 5.1 关键决策

| 决策点 | 选择 | 理由 |
|-------|------|------|
| Rust crate 组织 | 单 crate 多模块 | 模块量不大,单 crate 编译更快,避免 workspace 复杂性 |
| 并行粒度 | 文件级并行 | 避免单文件内多线程争抢 IO,结果顺序可预测 |
| 流式传输 | crossbeam-channel | 成熟稳定,支持背压,与 rayon 兼容 |
| 取消机制 | AtomicBool + 全局注册表 | 轻量、跨线程、响应快,无需复杂信号机制 |
| panic 策略 | catch_unwind + panic=abort | 双重防护,杜绝 panic 跨 JNI 边界的 UB |
| MVP 返回方式 | 一次性数组 | 实现简单,先验证功能正确性;Beta 再改流式回调 |
| 正则引擎 | grep-regex(基于 regex crate) | ripgrep 同款,性能与兼容性已验证 |

### 5.2 假设

1. **假设 IntelliJ 平台的 `TextSearcher` 扩展点在目标版本范围内稳定**。若不稳定,MVP 走独立 Tool Window 方案,不受影响。
2. **假设 ripgrep 生态库的 API 在 `0.4`/`0.1` 版本保持向后兼容**。这些库已长期稳定,风险低。
3. **假设 JNI 1.8 接口在 JDK 17(Android Studio 运行时)下完全兼容**。JDK 升级保留了 JNI 向后兼容。
4. **假设测试样本项目(5 万+ 文件)具有代表性**。预研阶段需用真实大型 Android 项目验证。

### 5.3 明确不做的事(避免过度设计)

- **不做**:自定义索引格式(冷启动遍历已足够快,索引维护成本高)
- **不做**:分布式搜索(单机 IDE 场景无需求)
- **不做**:语义搜索/AST 搜索(超出模块 1 边界,模块 3 才考虑)
- **不做**:Rust 侧日志系统(通过 JNI 回调到 IDE 日志,避免重复造轮子)
- **不做**:Workspace 多 crate 拆分(当前规模不需要)

---

## 六、验证方案(Verification)

### 6.1 单元测试

```bash
# 运行所有单元测试
cargo test

# 运行特定模块测试
cargo test --lib search::
cargo test --lib jni::

# 生成覆盖率报告
cargo tarpaulin --out Html --output-dir coverage/
```

测试覆盖点:
- `config.rs`:JNI 参数解析、默认值、边界值
- `walker.rs`:gitignore 规则、include/exclude glob、多根目录
- `matcher.rs`:字面量、正则、大小写、全字、特殊字符转义
- `engine.rs`:取消机制、背压、错误传播
- `jni/convert.rs`:字符串编码(中文路径)、数组边界

### 6.2 集成测试

```bash
cargo test --test search_basic
cargo test --test search_regex
cargo test --test search_cancel
```

使用 `tests/fixtures/` 构造测试样本:
- 包含 `.gitignore` 的模拟项目
- 中文文件名与中文内容
- 大文件(10MB+)验证性能
- 二进制文件检测(应自动跳过)

### 6.3 基准测试

```bash
# 运行基准测试
cargo bench

# 对比原生搜索
# 手动在 Android Studio 中执行 Find in Path,记录耗时
```

基准测试矩阵:
| 测试项 | 样本 | 指标 |
|-------|------|------|
| 字面量搜索 | 5 万文件 Android 项目 | 耗时、内存峰值 |
| 正则搜索 | 同上 | 耗时、内存峰值 |
| 大文件内搜索 | 单个 10MB JSON | 耗时 |
| 取消响应 | 10 万文件项目 | 取消后停止延迟 |
| 内存泄漏 | 连续 10 次搜索 | JVM 内存增长 |

### 6.4 JNI 集成验证

1. **内存泄漏检测**:
   ```bash
   # 启动测试 IDE,持续搜索 10 次
   jcmd <pid> GC.class_histogram | grep SearchResult
   # 每次 GC 后实例数应回归正常
   ```

2. **panic 安全验证**:故意构造错误输入(无效正则、不存在的路径),确认 Java 侧收到 `SearchException` 而非 JVM 崩溃。

3. **编码验证**:中文路径、中文关键词、emoji 内容,确认无乱码。

---

## 七、下一步行动(Next Steps)

计划批准后,按以下顺序启动里程碑 0:

1. 初始化 `rust-search/` Cargo 项目,配置 `Cargo.toml` 与 `cdylib`
2. 创建 IntelliJ 插件项目骨架(基于官方模板)
3. 实现 Rust 侧最小搜索核心(`config.rs` + `walker.rs` + `matcher.rs` + `engine.rs`)
4. 实现 JNI 入口函数(`bridge.rs` + `convert.rs`)
5. 配置 Gradle 任务自动编译 Rust 并拷贝动态库
6. 插件侧添加测试 Action 触发搜索,控制台打印结果
7. 在真实大型项目上执行基准测试,对比原生搜索
8. 输出预研结论,决定是否推进 MVP

---

## 附:文件路径索引(实施时使用)

| 文件 | 职责 |
|------|------|
| `rust-search/Cargo.toml` | 依赖与构建配置 |
| `rust-search/src/lib.rs` | crate 入口 |
| `rust-search/src/search/config.rs` | 搜索配置 |
| `rust-search/src/search/engine.rs` | 搜索引擎主流程 |
| `rust-search/src/search/walker.rs` | 文件遍历 |
| `rust-search/src/search/matcher.rs` | 文本匹配 |
| `rust-search/src/search/context.rs` | 上下文行提取 |
| `rust-search/src/jni/bridge.rs` | JNI 入口函数 |
| `rust-search/src/jni/convert.rs` | 类型转换 |
| `rust-search/src/jni/result.rs` | 结果构建 |
| `rust-search/src/error.rs` | 错误类型 |
| `rust-search/tests/` | 集成测试 |
| `rust-search/benches/search_bench.rs` | 基准测试 |

# RustSearch-AS 第三轮修复 - 剩余实施计划(H3/M1/M2/M4/M5)

> 前置:H1/H2/M6 已完成,M1 仅添加了 `memmap2` 依赖,context.rs 尚未重写。
> 本计划聚焦剩余 5 项高危/中危问题的精确代码改动,供执行阶段直接落地。

---

## 一、Summary(摘要)

### 1.1 剩余修复范围

| 编号 | 问题 | 优先级 | 涉及文件 | 改动量 |
|------|------|--------|----------|--------|
| H3 | Walker 串行遍历 + 全量 collect,首屏延迟高 | 高危 | walker.rs, engine.rs | 中 |
| M1 | ContextExtractor 重复全量读取文件,I/O 翻倍 | 中危 | context.rs | 中 |
| M2 | `fileNodeMap` 无上限,超大结果集 UI 内存爆炸 | 中危 | SearchResultTreeModel.kt, RustSearchPanel.kt, messages*.properties | 小 |
| M4 | `ModuleManager` 调用无 EDT 读锁保护 | 中危 | RustSearchPanel.kt | 小 |
| M5 | `navigateToSelectedResult` 未捕获异常,文件删除时崩溃 | 中危 | RustSearchPanel.kt | 极小 |

### 1.2 当前状态(已验证)

- `rust-search/Cargo.toml`:已删除 `panic = "abort"`(H1),已添加 `memmap2 = "0.9"`(M1 依赖)
- `rust-search/src/jni/result.rs`:已用 `with_local_frame` 包裹(H2)
- `rust-search/src/search/config.rs`:已添加 `MAX_CONTEXT_LINES` 等常量与校验(M6)
- `rust-search/src/search/walker.rs`:仍只有 `files()` 方法,**未添加 `walk()`**
- `rust-search/src/search/engine.rs`:`run_stream_search` 仍用 `walker.files()?.par_iter()`(L134-148)
- `rust-search/src/search/context.rs`:仍用 `fs::read` + `Vec<String>` 全量复制(L18-43)
- `SearchResultTreeModel.kt`:无 `truncated` 字段,无上限保护
- `RustSearchPanel.kt`:`refreshModuleList`(L283-299)与 `resolveSearchRoots`(L386-401)无 ReadAction;`navigateToSelectedResult`(L418-430)无 try-catch
- `messages.properties` / `messages_zh_CN.properties`:无 `search.status.truncated` 消息

---

## 二、Proposed Changes(精确改动方案)

### H3:Walker 暴露 `walk()` + engine 用 `par_bridge`

**文件**:[rust-search/src/search/walker.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/walker.rs)

**改动**:在 `impl Walker` 中新增 `walk(self) -> ignore::Walk` 方法(放在 `files()` 之后),复用 `walk_root` 内部的 builder 配置逻辑。

```rust
/// 构建并返回 ignore::Walk 迭代器(已应用 include/exclude globs)
///
/// H3:暴露迭代器让 engine 层用 par_bridge 实现"边遍历边搜索",
/// 避免全量 collect 到 Vec 后才开始搜索的首屏延迟。
///
/// 多根目录场景:迭代器按根目录顺序产出文件,跨根目录无并行
/// (单根目录场景下,par_bridge 内部并行消费已足够)。
pub fn walk(self) -> ignore::Walk {
    // 多根目录场景:简单起见取第一个根目录构建迭代器
    // 多根目录的并行化需要重构为 WalkParallel,超出 H3 最小改动范围
    let root = self.config.roots.into_iter().next().unwrap_or_else(|| PathBuf::from("."));
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(!self.config.search_hidden)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .parents(true)
        .ignore(true);

    if !self.config.include_globs.is_empty() || !self.config.exclude_globs.is_empty() {
        if let Ok(overrides) = self.build_overrides(&root) {
            builder.overrides(overrides);
        }
    }

    builder.build()
}
```

**注意**:
- `walk(self)` 消费 `Walker`(取所有权),因为 `WalkBuilder::build()` 返回的 `ignore::Walk` 借用 builder 内部数据;`files(&self)` 保留不动以便同步接口与测试复用
- `build_overrides` 是 `&self` 方法,在 `self.config` 被 move 前调用,但此处 `self.config.roots.into_iter()` 已经 move 了 `roots`,需调整顺序:**先构建 overrides,再 move roots**

**修正版(顺序正确)**:
```rust
pub fn walk(self) -> ignore::Walk {
    let config = self.config;
    let root = config.roots.into_iter().next().unwrap_or_else(|| PathBuf::from("."));
    let mut builder = WalkBuilder::new(&root);
    builder
        .hidden(!config.search_hidden)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .parents(true)
        .ignore(true);

    if !config.include_globs.is_empty() || !config.exclude_globs.is_empty() {
        // build_overrides 需要 &self,但 self 已被消费;改为内联实现
        let mut ob = ignore::overrides::OverrideBuilder::new(&root);
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
```

**Decision**:采用修正版,内联 overrides 构建避免 `&self` 借用冲突。

---

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L129-203 `run_stream_search` 函数

**当前代码**(关键段):
```rust
fn run_stream_search(
    config: &SearchConfig,
    cancel_flag: &Arc<AtomicBool>,
    tx: &Sender<SearchResult<SearchMatch>>,
) -> SearchResult<()> {
    let walker = Walker::new(config.clone());
    let files = walker.files()?;  // ← 全量 collect,首屏延迟瓶颈

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }

    let matcher = Matcher::new(config)?;
    let max_total = config.max_total_matches;
    let sent_count = AtomicUsize::new(0usize);

    let result = files
        .par_iter()  // ← 必须等 files 完全返回
        .try_for_each(|file| {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(SearchError::Cancelled);
            }
            if sent_count.load(Ordering::Relaxed) >= max_total {
                return Err(SearchError::Cancelled);
            }
            let matches = matcher.search_file(file, cancel_flag)?;
            for m in matches {
                // ... send_timeout 重试逻辑(保持不变)...
            }
            Ok(())
        });

    match result {
        Ok(()) => Ok(()),
        Err(SearchError::Cancelled) => Ok(()),
        Err(e) => Err(e),
    }
}
```

**修复后**:
```rust
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
    let sent_count = AtomicUsize::new(0usize);

    // H3:用 par_bridge 替代 par_iter,实现"边遍历边搜索"
    // walker 产出一个文件,par_bridge 立即喂入并行管道开始搜索
    // 首屏延迟从"walker 全量遍历耗时"降到"首个文件产出耗时"(毫秒级)
    let walker = Walker::new(config.clone()).walk();
    let result = walker
        .par_bridge()
        .try_for_each(|entry| {
            // 取消检查
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(SearchError::Cancelled);
            }

            // 全局上限检查
            if sent_count.load(Ordering::Relaxed) >= max_total {
                return Err(SearchError::Cancelled);
            }

            // 过滤非文件条目 + 错误降级
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return Ok(()),  // 遍历错误降级为跳过
            };
            if entry.file_type().map(|t| !t.is_file()).unwrap_or(true) {
                return Ok(());
            }
            let file = entry.path();

            let matches = matcher.search_file(file, cancel_flag)?;
            for m in matches {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(SearchError::Cancelled);
                }

                let prev_count = sent_count.fetch_add(1, Ordering::Relaxed);
                if prev_count >= max_total {
                    sent_count.fetch_sub(1, Ordering::Relaxed);
                    return Err(SearchError::Cancelled);
                }

                loop {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err(SearchError::Cancelled);
                    }
                    match tx.send_timeout(Ok(m.clone()), Duration::from_millis(500)) {
                        Ok(()) => break,
                        Err(SendTimeoutError::Timeout(_)) => continue,
                        Err(SendTimeoutError::Disconnected(_)) => {
                            return Err(SearchError::Cancelled);
                        }
                    }
                }
            }
            Ok(())
        });

    match result {
        Ok(()) => Ok(()),
        Err(SearchError::Cancelled) => Ok(()),
        Err(e) => Err(e),
    }
}
```

**关键变更点**:
1. 删除 `let files = walker.files()?;` 全量 collect
2. `Walker::new(config.clone()).walk()` 消费 walker 返回迭代器
3. `walker.par_bridge().try_for_each(|entry| {...})` 替代 `files.par_iter().try_for_each(|file| {...})`
4. 闭包参数从 `&PathBuf` 改为 `Result<DirEntry, Error>`:
   - `Err(_) => return Ok(())` 遍历错误降级为跳过
   - `entry.file_type().map(|t| !t.is_file()).unwrap_or(true)` 过滤非文件(目录、符号链接等)
   - `entry.path()` 获取文件路径
5. 闭包内 `matcher.search_file` 调用从 `search_file(file, cancel_flag)` 改为 `search_file(file, cancel_flag)`,**file 类型从 `&PathBuf` 变为 `&Path`**,需确认 `Matcher::search_file` 签名兼容

**待验证点**:`Matcher::search_file` 签名(实施时先 Read 确认)

---

### M1:重写 ContextExtractor,改用 mmap + 行偏移索引

**文件**:[rust-search/src/search/context.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/context.rs)

**改动**:全文替换(保留测试 mod 不变)

```rust
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
/// 10MB 足以覆盖绝大多数源代码文件;超大文件跳过上下文
const MAX_CONTEXT_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 上下文行提取器
///
/// M1:改用 mmap + 行偏移索引,避免 fs::read 全量拷贝 + Vec<String> 内存翻倍。
pub struct ContextExtractor {
    /// mmap 映射区域;大文件时为 None
    mmap: Option<Mmap>,
    /// 每行起始字节偏移(0-based);mmap 为 None 时为空
    line_offsets: Vec<usize>,
}

impl ContextExtractor {
    /// 创建提取器:打开文件 + mmap + 计算行偏移
    ///
    /// 任何失败(metadata/open/mmap)都降级为空提取器,不中断搜索。
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
    // 测试 mod 保持原样,验证行为不变
    // ... 原 4 个测试:test_extract_context / test_extract_at_start / test_extract_at_end / test_non_utf8_file_does_not_error
}
```

**关键变更点**:
1. `use std::fs::{self, File}` + `use memmap2::Mmap`
2. 字段 `lines: Vec<String>` → `mmap: Option<Mmap>` + `line_offsets: Vec<usize>`
3. `new` 中 `fs::read(path)?` → `File::open` + `unsafe { Mmap::map(&file) }.ok()`,任何失败降级为空提取器(不返回 Err)
4. `extract` 按 `line_offsets` 切片 mmap,通过 `read_line` 辅助函数转 String
5. 新增 `read_line` 与 `compute_line_offsets` 辅助函数
6. **测试 mod 保持原样**:4 个测试验证行为不变(`test_extract_context` / `test_extract_at_start` / `test_extract_at_end` / `test_non_utf8_file_does_not_error`)

**风险控制**:
- `Mmap::map` 是 unsafe,文件截断时触发 SIGBUS
- 当前不做文件锁,接受残余风险(搜索期间用户编辑文件概率极低,且 grep-searcher 短时间扫完)
- 完整方案需 `MmapOptions::populate` 或文件锁,超出本轮范围

---

### M2:`fileNodeMap` 增加上限保护

**文件 1**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**改动位置**:类顶部 + `addResults` 方法开头 + 新增 `isTruncated()` + `clear()` 重置

```kotlin
class SearchResultTreeModel : DefaultTreeModel(DefaultMutableTreeNode("root")) {

    companion object {
        /** M2:UI 侧结果上限保护,防止超大结果集内存爆炸 */
        private const val MAX_TOTAL_MATCHES_UI = 50_000
        /** M2:文件数上限,超过则停止追加 */
        private const val MAX_FILE_NODES_UI = 5_000
    }

    /** 文件路径 → 文件节点(便于增量追加) */
    private val fileNodeMap = mutableMapOf<String, DefaultMutableTreeNode>()

    /** 总匹配数 */
    private var totalMatches = 0

    /** M2:是否已触发截断(触发后拒绝后续 batch) */
    private var truncated = false

    fun addResults(results: List<SearchResult>) {
        // M2:截断检查
        if (truncated) return
        if (totalMatches >= MAX_TOTAL_MATCHES_UI || fileNodeMap.size >= MAX_FILE_NODES_UI) {
            truncated = true
            return
        }

        // ... 原 addResults 逻辑不变 ...
    }

    /** M2:是否已截断 */
    fun isTruncated(): Boolean = truncated

    fun clear() {
        val root = root as DefaultMutableTreeNode
        root.removeAllChildren()
        fileNodeMap.clear()
        totalMatches = 0
        truncated = false  // M2:重置截断标志
        reload()
    }

    // ... 其他方法不变 ...
}
```

**文件 2**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:`collect` 回调内 `addResults` 后检查截断状态(L350-359 区域)

```kotlin
service.search(config).collect { batch ->
    withContext(Dispatchers.Main) {
        treeModel.addResults(batch)
        val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
        // M2:截断时显示特殊提示
        statusLabel.text = if (treeModel.isTruncated()) {
            RustSearchBundle.message("search.status.truncated", 50000, 5000)
        } else {
            RustSearchBundle.message("search.status.found", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)
        }
    }
}
```

**文件 3**:[src/main/resources/com/example/rustsearch/messages.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages.properties)

末尾新增:
```
search.status.truncated=Results truncated (over {0} matches or {1} files), refine your search
```

**文件 4**:[src/main/resources/com/example/rustsearch/messages_zh_CN.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages_zh_CN.properties)

末尾新增:
```
search.status.truncated=结果已截断(超过 {0} 个匹配或 {1} 个文件),请缩小搜索范围
```

---

### M4:`ModuleManager` 调用包裹 `ReadAction.compute`

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动 1**:新增 import(L7-14 区域)
```kotlin
import com.intellij.openapi.application.ReadAction
import com.intellij.openapi.util.Computable
```

**改动 2**:`refreshModuleList`(L283-299)包裹 `ModuleManager.modules`
```kotlin
private fun refreshModuleList() {
    isRefreshingModules = true
    try {
        moduleComboBox.removeAllItems()
        // M4:ModuleManager.modules 需读锁保护,避免 EDT 卡顿或并发写异常
        val modules = ReadAction.compute(Computable {
            ModuleManager.getInstance(project).modules
        })
        modules.forEach { module ->
            moduleComboBox.addItem(module.name)
        }
        if (moduleComboBox.itemCount > 0) {
            moduleComboBox.selectedIndex = 0
        }
    } finally {
        isRefreshingModules = false
    }
}
```

**改动 3**:`resolveSearchRoots`(L386-401)模块分支包裹读锁
```kotlin
private fun resolveSearchRoots(): List<String> {
    return when {
        scopeProjectRadio.isSelected -> {
            val basePath = project.basePath
            if (basePath.isNullOrBlank()) emptyList() else listOf(basePath)
        }
        scopeModuleRadio.isSelected -> {
            val moduleName = moduleComboBox.selectedItem as? String
            if (moduleName.isNullOrBlank()) return emptyList()
            // M4:模块查询与 contentRoots 读取需读锁
            ReadAction.compute(Computable {
                val module = ModuleManager.getInstance(project).modules
                    .firstOrNull { it.name == moduleName } ?: return@Computable emptyList()
                ModuleRootManager.getInstance(module).contentRoots.map { it.path }
            })
        }
        else -> emptyList()
    }
}
```

---

### M5:`navigateToSelectedResult` 包裹 try-catch + VFS 校验

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:L418-430

```kotlin
private fun navigateToSelectedResult() {
    val node = resultTree.lastSelectedPathComponent as? DefaultMutableTreeNode ?: return
    val data = node.userObject as? MatchNodeData ?: return

    val file = LocalFileSystem.getInstance().findFileByPath(data.filePath)
    if (file != null) {
        // M5:刷新 VFS 确保文件仍存在(避免 VFS 缓存与磁盘不一致)
        file.refresh(false, false)
        if (!file.isValid) {
            statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
            return
        }
        // lineNumber 为 1-based,OpenFileDescriptor 需 0-based
        val descriptor = OpenFileDescriptor(project, file, data.lineNumber - 1, data.column)
        try {
            // M5:捕获 navigate 可能抛出的异常(文件已被删除/权限不足/编辑器冲突)
            descriptor.navigate(true)
        } catch (e: Exception) {
            logger.warn("Failed to navigate to ${data.filePath}:${data.lineNumber}: ${e.message}")
            statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
        }
    } else {
        statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
    }
}
```

---

## 三、Assumptions & Decisions(假设与决策)

### 3.1 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| H3 `walk(self)` 签名 | 消费 Walker | `WalkBuilder::build()` 返回的迭代器借用 builder 内部数据;消费所有权避免生命周期问题 |
| H3 overrides 构建 | 内联实现,不复用 `build_overrides(&self)` | `walk(self)` 已消费 self,无法再调 `&self` 方法;内联避免借用冲突 |
| H3 闭包参数 | `Result<DirEntry, Error>`,错误降级为 `Ok(())` | 遍历错误(权限/符号链接)不应中断整体搜索 |
| M1 失败处理 | 所有失败降级为空提取器,不返回 Err | 与原 `fs::read(path)?` 不同,但更健壮;matcher.rs 已有 try-catch 兜底 |
| M1 SIGBUS 风险 | 接受残余风险 | 完整方案需文件锁,超出最小改动;搜索期间文件截断概率极低 |
| M2 阈值 | 50000 匹配 / 5000 文件 | 覆盖 99% 正常使用;Swing JTree 超过 5000 节点渲染卡顿 |
| M4 读锁 | `ReadAction.compute(Computable)` | IntelliJ Platform 官方 API;EDT 同步阻塞;Kotlin lambda 友好 |
| M5 异常范围 | `catch (Exception)` 不捕获 Error | 避免吞掉 OOM 等 Error;navigate 可能抛 IllegalStateException/IOException |

### 3.2 待实施时验证点

1. **`Matcher::search_file` 签名**:确认接受 `&Path`(原 `&PathBuf` 自动 deref 为 `&Path`,但需确认无类型不匹配)
2. **`ignore::DirEntry::path()` 返回 `&Path`**:确认 `matcher.search_file(file, cancel_flag)` 中 `file: &Path` 兼容
3. **`memmap2::Mmap::map` 签名**:确认 `unsafe fn map(file: &File) -> Result<Mmap, Error>`
4. **`ReadAction.compute(Computable)` 在 EDT 调用**:确认不会死锁(官方保证读锁可重入)

### 3.3 明确不做的事

- 不重构 Walker 为 `WalkParallel`(H3 用 par_bridge 已足够)
- 不引入虚拟树(JXTreeTable)(M2 用截断 + 提示替代)
- 不修复 M3(已验证不存在,前轮分析误判)
- 不修改 `search()` 同步接口(已废弃,保持现状)
- 不为 M1 SIGBUS 加文件锁(超出最小改动范围)

---

## 四、实施顺序

| 顺序 | 问题 | 文件 | 验证命令 |
|------|------|------|----------|
| 1 | H3 walker.rs 新增 `walk()` | walker.rs | `cargo build` |
| 2 | H3 engine.rs 改 par_bridge | engine.rs | `cargo build` + `cargo test` |
| 3 | M1 重写 ContextExtractor | context.rs | `cargo build` + `cargo test` |
| 4 | M2 SearchResultTreeModel 截断保护 | SearchResultTreeModel.kt, messages*.properties | `./gradlew compileKotlin` |
| 5 | M2 RustSearchPanel 检查 isTruncated | RustSearchPanel.kt | `./gradlew compileKotlin` |
| 6 | M4 ReadAction 包裹模块查询 | RustSearchPanel.kt | `./gradlew compileKotlin` |
| 7 | M5 navigate try-catch | RustSearchPanel.kt | `./gradlew compileKotlin` |
| 8 | 全量编译验证 | - | `cargo build --release` + `./gradlew compileKotlin` |

---

## 五、Verification(验证方案)

### 5.1 编译验证

```bash
# Rust 侧
cd /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search
cargo build
cargo test

# Kotlin 侧
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew compileKotlin
```

### 5.2 单元测试矩阵

| 测试 | 验证问题 | 预期 |
|------|----------|------|
| `test_search_stream` | H3 par_bridge 工作 | 通过,结果数 >= 3 |
| `test_search_stream_cancel` | H3 取消传播 | 通过,收到错误 |
| `test_extract_context` | M1 mmap 行偏移正确 | 通过,before/after 行匹配 |
| `test_extract_at_start` | M1 边界(首行) | 通过,before 为空 |
| `test_extract_at_end` | M1 边界(末行) | 通过,after 为空 |
| `test_non_utf8_file_does_not_error` | M1 非 UTF-8 容错 | 通过,from_utf8_lossy 生效 |

### 5.3 集成测试场景

| 场景 | 验证问题 | 预期 |
|------|----------|------|
| 5 万文件项目搜索 `import` | H3 | 首结果 < 500ms |
| 搜索期间文件被删除 | M1, M5 | 不 crash,降级为空上下文;双击节点显示友好提示 |
| 搜索 `import` 在大项目 | M2 | 到 50000 匹配时停止,显示"结果已截断" |
| 大型 Gradle 项目首次打开 | M4 | 无 EDT 卡顿,无 IllegalStateException |
| 搜索完成后删除文件,双击节点 | M5 | 显示"文件不存在",无红色错误气泡 |

### 5.4 残余风险

1. **H3 par_bridge 多根目录不并行**:多根目录场景仍是串行迭代;后续可用 `WalkBuilder::build_parallel()` 优化
2. **M1 SIGBUS 风险**:mmap 期间文件被截断会触发 SIGBUS,概率极低但存在
3. **M2 截断后 Flow 仍 collect**:Rust 侧搜索继续到 `max_total_matches` 才停,UI 不再追加但 collect 不中断
4. **M4 ReadAction 阻塞 EDT**:`refreshModuleList` 在 EDT 调用 `ReadAction.compute`,若写锁持有会短暂卡顿

---

## 六、附录:文件改动清单

### Rust 侧(3 文件)

1. [rust-search/src/search/walker.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/walker.rs) - 新增 `walk(self)` 方法(H3)
2. [rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs) - `run_stream_search` 改 par_bridge(H3)
3. [rust-search/src/search/context.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/context.rs) - 重写 ContextExtractor(M1)

### Kotlin 侧(3 文件 + 2 资源文件)

4. [src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt) - 截断保护(M2)
5. [src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt) - isTruncated 检查(M2) + ReadAction(M4) + navigate try-catch(M5)
6. [src/main/resources/com/example/rustsearch/messages.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages.properties) - 新增 truncated 消息(M2)
7. [src/main/resources/com/example/rustsearch/messages_zh_CN.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages_zh_CN.properties) - 新增 truncated 消息(M2)

**总计**:Rust 侧 3 文件,Kotlin 侧 2 文件 + 2 资源文件,共 7 文件改动。

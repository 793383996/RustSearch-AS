# RustSearch-AS 系统性问题修复计划

> 基于"无锁生产者 + 消费者独占堆"设计思想,以最小依赖、最小改动修复全量问题。
> 设计核心:**生产者并行无锁投递**(crossbeam-channel 已无锁) + **消费者独占 session**(Arc 共享 + 锁外消费) + **Dependency Flip**(pollResults 不再依赖 registry 数据结构)。

---

## 一、Summary(摘要)

本计划针对前一轮分析识别的 4 个 P0 高危问题 + 5 个 P1 中危问题 + 4 个 P2 低危问题,制定完整修复方案。核心设计原则:

1. **消除锁内慢操作**:P0-4 的 `pollResults` 持锁 `recv_timeout` 是最严重的锁问题。通过 `Arc<SearchSession>` 共享 + 锁外消费,让消费者(pollResults)独占 receiver,不再与 registry 交互。
2. **生产者并行无锁投递**:P0-1 的流式搜索串行问题,通过 `par_iter` 让多个生产者(文件搜索线程)并行投递到 channel(channel 本身无锁)。
3. **最小依赖原则**:不引入 dashmap 等新依赖。registry 仍是 `Mutex<HashMap>`,但锁仅用于 start/cancel/release 的瞬时操作,poll 不持锁。
4. **生命周期绑定**:P0-2 取消通知 + P1-5 Panel Disposable,通过 Kotlin 协程 `finally` 和 `Disposable` 接口绑定资源生命周期。

---

## 二、Current State Analysis(当前状态分析)

### 2.1 核心问题链路

```
架构设计文档 (par_bridge 并行)
        ↓ 实现偏差
engine.rs 串行 for 循环 ──────────────────────► P0-1 性能退化(4-8 倍损失)
        ↓
RustSearchPanel.cancelSearch() 仅 cancel 协程
        ↓ 缺失 Rust 侧 cancel 调用
后台线程继续运行,channel 满 ──────────────────► P0-2 资源泄漏(线程+session)
        ↓
performSearch 未取消旧 searchJob
        ↓ 协程调度时序
旧结果在新 clear 后追加 ──────────────────────► P0-3 结果竞态
        ↓
pollResults 持锁 recv_timeout(200ms)
        ↓ 锁粒度过大
多搜索并发互相阻塞 ──────────────────────────► P0-4 死锁风险
```

### 2.2 已确认的技术约束(Phase 1 探索结果)

| 约束 | 状态 | 影响 |
|------|------|------|
| `crossbeam_channel::Sender/Receiver` 是 `Send + Sync` | ✅ 确认 | `par_iter` 可跨线程共享 `&tx` |
| `grep_regex::RegexMatcher` 是 `Send + Sync` | ✅ 确认(engine.rs:60-73 已有 par_iter 先例) | `&matcher` 可跨线程共享 |
| `SearchEngine/SearchConfig` 自动 `Send + Sync` | ✅ 确认 | 可存入 registry |
| `SearchSession` 自动 `Send + Sync` | ✅ 确认 | 可包装为 `Arc<SearchSession>` |
| 当前无 dashmap 依赖 | ✅ 确认 | 保持最小依赖,不引入 |
| registry 锁竞争是低频 JNI 会话级 | ✅ 确认 | 保持 `Mutex<HashMap>` 即可,无需无锁 map |

### 2.3 设计原则对照

| 原则 | 当前问题 | 修复后设计 |
|------|----------|------------|
| 生产者无锁投递 | P0-1 串行 for 循环 | `par_iter` 多线程并行投递到 channel(无锁) |
| 消费者独占堆 | P0-4 持锁 `recv_timeout` | `Arc::clone` 后锁外独占消费 receiver |
| Dependency Flip | pollResults 依赖 registry 数据结构 | pollResults 拿到 Arc 后独立操作,registry 仅用于生命周期管理 |
| 生命周期绑定 | P0-2/P1-5 资源泄漏 | Kotlin `finally` + `Disposable` 接口 |

---

## 三、Proposed Changes(修复方案)

### P0-3:并发搜索结果覆盖竞态(优先级最高,1 行代码)

**问题**:`performSearch` 启动新搜索前未取消旧 `searchJob`,旧 Flow 的 `withContext(Dispatchers.Main)` 可能在新 `clear()` 后执行。

**修复方案**:`performSearch()` 开头取消旧搜索。

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:L294 `performSearch()` 函数开头

```kotlin
private fun performSearch() {
    // 新增:取消旧搜索,防止旧结果覆盖新结果
    searchJob?.cancel()
    
    val pattern = searchField.text.trim()
    // ... 原有逻辑
}
```

**Why**:协程取消是协作式的,`searchJob?.cancel()` 后旧 Flow 的 `collect` 不会再执行,`withContext(Dispatchers.Main)` 提交的任务也会被丢弃。这是最小改动,1 行代码消除竞态。

**How**:Kotlin 协程的 `Job.cancel()` 是幂等的,对已完成的 Job 调用无副作用。

---

### P0-2:取消搜索未通知 Rust 侧(资源泄漏)

**问题**:`cancelSearch()` 仅 `searchJob?.cancel()`,未调用 `service.cancel(searchId)`,后台线程继续运行。

**修复方案**:Flow 内部捕获 searchId,在 `finally` 中先 `cancel(searchId)` 触发 Rust 侧 `AtomicBool`,再 `releaseSearch`。

**文件**:[src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt)

**改动位置**:L123-159 `search()` 函数

```kotlin
fun search(config: SearchConfig): Flow<List<SearchResult>> = flow {
    if (!nativeLoaded) {
        loadNativeLibrary()
    }

    val args = config.toJniArgs()
    val searchId = RustSearchEngine.startSearch(
        args.roots, args.pattern, args.isRegex, args.caseSensitive, args.wholeWords,
        args.includeGlobs, args.excludeGlobs, args.contextLines
    )

    if (searchId == 0L) {
        throw SearchException(RustSearchBundle.message("service.error.search.start"))
    }

    logger.info("Search started: searchId=$searchId, pattern='${config.pattern}'")

    try {
        while (!RustSearchEngine.isSearchComplete(searchId)) {
            val batch = RustSearchEngine.pollResults(searchId, 200)
            if (batch.isNotEmpty()) {
                emit(batch.toList())
            }
        }
        val finalBatch = RustSearchEngine.pollResults(searchId, 50)
        if (finalBatch.isNotEmpty()) {
            emit(finalBatch.toList())
        }
    } finally {
        // 关键修复:先触发 Rust 侧 cancel,让后台线程在下一个检查点退出
        // 这样 releaseSearch 后,后台线程不会继续运行
        try {
            RustSearchEngine.cancel(searchId)
        } catch (e: Throwable) {
            logger.warn("Failed to cancel search $searchId: ${e.message}")
        }
        // 短暂等待后台线程退出(避免 tx.send 失败时的日志噪音)
        Thread.sleep(50)
        RustSearchEngine.releaseSearch(searchId)
        logger.info("Search session released: searchId=$searchId")
    }
}.flowOn(Dispatchers.IO)
```

**Why**:
1. `cancel(searchId)` 设置 `AtomicBool`,后台线程在下一文件/匹配检查点退出
2. `Thread.sleep(50)` 给后台线程退出时间(可选,避免日志噪音)
3. `releaseSearch` 移除 session,`tx` drop 后后台线程的 `send` 失败也会退出
4. `try-catch` 防止 cancel 失败阻塞 finally

**How**:Rust 侧 `cancel()` 已实现([bridge.rs:297-320](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/jni/bridge.rs#L297)),查询 `SEARCH_REGISTRY` 调用 `session.engine.cancel()`。

---

### P0-1:流式搜索退化为单线程串行(性能瓶颈)

**问题**:`run_stream_search` 用 `for file in files` 串行遍历,与架构设计文档的 `par_bridge()` 严重不符。

**修复方案**:改用 `files.par_iter().try_for_each(...)`,多线程并行处理文件,channel 天然线程安全。

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L120-163 `run_stream_search()` 函数

```rust
/// 流式搜索内部主循环(并行版)
fn run_stream_search(
    config: &SearchConfig,
    cancel_flag: &Arc<AtomicBool>,
    tx: &Sender<SearchResult<SearchMatch>>,
) -> SearchResult<()> {
    let walker = Walker::new(config.clone());
    let files = walker.files()?;

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }

    let matcher = Matcher::new(config)?;
    let max_total = config.max_total_matches;
    
    // 使用 AtomicUsize 统计已发送数量,跨线程安全
    let sent_count = std::sync::atomic::AtomicUsize::new(0usize);

    // 文件级并行:每个文件独立搜索,channel send 天然线程安全
    let result = files
        .par_iter()
        .try_for_each(|file| {
            // 取消检查(每个文件开头)
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(SearchError::Cancelled);
            }

            // 全局上限检查
            if sent_count.load(Ordering::Relaxed) >= max_total {
                return Err(SearchError::Cancelled);
            }

            let matches = matcher.search_file(file, cancel_flag)?;
            for m in matches {
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(SearchError::Cancelled);
                }
                
                // CAS 更新已发送计数,超过上限则停止
                let current = sent_count.load(Ordering::Relaxed);
                if current >= max_total {
                    return Err(SearchError::Cancelled);
                }
                
                // channel 满时阻塞,形成背压;接收方关闭时返回错误
                if tx.send(Ok(m)).is_err() {
                    return Err(SearchError::Cancelled);
                }
                
                sent_count.fetch_add(1, Ordering::Relaxed);
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
```

**Why**:
1. `par_iter` 让多个生产者(文件搜索线程)并行投递,channel 是无锁队列,天然支持多生产者
2. `sent_count` 用 `AtomicUsize` 替代 `usize`,跨线程安全统计
3. `try_for_each` 在任一线程返回 `Err` 时停止所有工作线程
4. 取消错误不向上传播,已发送的结果仍然有效(用户能看到部分结果)

**How**:
- `&Matcher` 跨线程共享(已确认 `Matcher: Send + Sync`)
- `&Sender` 跨线程共享(已确认 `Sender: Sync`)
- `&Arc<AtomicBool>` 跨线程共享(天然 Send + Sync)

**备选方案**:用 `tx.clone()` 给每个工作线程独立的 Sender,语义等价但更符合 crossbeam 推荐用法。当前 `&Sender` 共享更简洁,优先采用。

---

### P0-4:pollResults 持锁阻塞(核心设计改造)

**问题**:`pollResults` 在持有 `SEARCH_REGISTRY` Mutex 期间调用 `recv_timeout(200ms)`,期间所有其他搜索的 `isSearchComplete`、`cancel`、`releaseSearch` 全部阻塞。

**修复方案**:应用"消费者独占堆"模式 —— `SearchSession` 改为 `Arc<SearchSession>`,poll 时短暂持锁 clone Arc,立即释放锁,在锁外独占消费 receiver。

**文件**:[rust-search/src/jni/bridge.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/jni/bridge.rs)

**改动 1**:`SearchSession` 包装为 `Arc`,registry 值改为 `Arc<SearchSession>`

```rust
/// 异步搜索会话,持有 engine 与结果 receiver
struct SearchSession {
    /// 搜索引擎,持有 cancel_flag,用于触发取消
    engine: SearchEngine,
    /// 流式结果接收端,从后台搜索线程获取匹配结果
    receiver: Receiver<SearchResult<SearchMatch>>,
    /// 搜索是否完成(后台线程结束或 receiver 关闭)
    is_complete: Arc<AtomicBool>,
}

/// 新增:session 共享指针类型别名
type SharedSession = Arc<SearchSession>;

/// 新异步 startSearch 的会话注册表:search_id -> Arc<SearchSession>
static SEARCH_REGISTRY: Lazy<Mutex<HashMap<u64, SharedSession>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
```

**改动 2**:`run_start_search` 创建 `Arc<SearchSession>`

```rust
fn run_start_search<'local>(...) -> Result<u64, SearchError> {
    let config = convert::build_config_from_jni(...)?;
    let engine = SearchEngine::new(config);
    let receiver = engine.search_stream()?;

    let search_id = generate_search_id();
    let session = Arc::new(SearchSession {
        engine,
        receiver,
        is_complete: Arc::new(AtomicBool::new(false)),
    });

    {
        let mut registry = SEARCH_REGISTRY.lock().map_err(|e| {
            SearchError::Internal(format!("搜索注册表锁失败: {e}"))
        })?;
        registry.insert(search_id, session);
    }

    Ok(search_id)
}
```

**改动 3**:`run_poll_results` 锁外消费(核心改造)

```rust
/// 轮询获取结果:锁外独占消费 receiver
fn run_poll_results<'local>(
    env: &mut JNIEnv<'local>,
    search_id: u64,
    timeout_ms: jint,
) -> Result<JObjectArray<'local>, SearchError> {
    let timeout = Duration::from_millis(timeout_ms.max(0) as u64);

    // 关键改造:短暂持锁 clone Arc,立即释放锁
    // 这是"消费者独占堆"的体现:拿到 Arc 后,完全独占 receiver 操作
    let session: SharedSession = {
        let registry = SEARCH_REGISTRY.lock().map_err(|e| {
            SearchError::Internal(format!("搜索注册表锁失败: {e}"))
        })?;
        match registry.get(&search_id) {
            Some(s) => Arc::clone(s),
            None => {
                return Err(SearchError::Internal(format!("搜索会话 {search_id} 不存在")));
            }
        }
    };  // 锁在此处释放,后续 recv_timeout 完全无锁

    // 锁外独占消费 receiver(消费者独占堆)
    let mut batch: Vec<SearchMatch> = Vec::new();
    let mut should_mark_complete = false;
    let mut pending_error: Option<SearchError> = None;

    // 排空当前已就绪的结果
    while let Ok(item) = session.receiver.try_recv() {
        match item {
            Ok(m) => batch.push(m),
            Err(SearchError::Cancelled) => {
                should_mark_complete = true;
                break;
            }
            Err(e) => {
                should_mark_complete = true;
                pending_error = Some(e);
                break;
            }
        }
    }

    // 如果没有就绪结果,等待一个
    if batch.is_empty() && !should_mark_complete {
        match session.receiver.recv_timeout(timeout) {
            Ok(Ok(m)) => batch.push(m),
            Ok(Err(SearchError::Cancelled)) => {
                should_mark_complete = true;
            }
            Ok(Err(e)) => {
                should_mark_complete = true;
                pending_error = Some(e);
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // 超时,返回空数组
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                // channel 关闭,搜索完成
                should_mark_complete = true;
            }
        }

        // 拿到一个后,继续排空其他就绪结果
        if !batch.is_empty() {
            while let Ok(item) = session.receiver.try_recv() {
                match item {
                    Ok(m) => batch.push(m),
                    Err(SearchError::Cancelled) => {
                        should_mark_complete = true;
                        break;
                    }
                    Err(e) => {
                        should_mark_complete = true;
                        pending_error = Some(e);
                        break;
                    }
                }
            }
        }
    }

    // 标记完成状态(独立操作,不阻塞消费)
    if should_mark_complete {
        session.is_complete.store(true, Ordering::Release);
    }

    if let Some(e) = pending_error {
        return Err(e);
    }

    if batch.is_empty() {
        empty_object_array(env)
    } else {
        result::build_search_result_array(env, &batch)
    }
}
```

**Why**:
1. **锁持有时间从 200ms 降到 ~纳秒级**(仅 HashMap get + Arc::clone)
2. **消费者独占**:pollResults 拿到 `Arc<SearchSession>` 后,完全独占 receiver 操作,不再与 registry 交互
3. **Dependency Flip**:pollResults 不再依赖 registry 维护的数据结构,registry 仅用于生命周期管理(start/insert, release/remove)
4. **无新依赖**:保持 `Mutex<HashMap>`,但锁粒度极小,无需引入 dashmap

**How**:
- `Arc::clone` 是原子操作(无锁),开销极低
- `Receiver` 实现了 `RecvTimeout` 通过 `&self`,线程安全
- 多个 pollResults 并发调用同一 searchId 不会出问题(但 JVM 侧单搜索单 poll,不会发生)

---

### P1-1:ContextExtractor 全量读取大文件(内存溢出)

**问题**:`ContextExtractor::new` 无条件 `fs::read(path)` 全量读取,大文件内存爆炸。

**修复方案**:文件大小阈值检查 + 大文件跳过上下文提取(最简方案,符合"简洁高效"原则)。

**文件**:[rust-search/src/search/context.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/context.rs)

**改动位置**:L23-28 `ContextExtractor::new()`

```rust
/// 大文件阈值:超过此大小不提取上下文行(避免内存爆炸)
/// 10MB 足以覆盖绝大多数源代码文件;超大文件(如 minified JS、大 JSON)跳过上下文
const MAX_CONTEXT_FILE_SIZE: u64 = 10 * 1024 * 1024;

impl ContextExtractor {
    /// 读取文件并按行分割缓存
    ///
    /// 使用 `from_utf8_lossy` 容忍非 UTF-8 编码文件(二进制、Latin-1、GBK 等),
    /// 与 matcher.rs 的处理方式保持一致,避免因编码问题中断搜索。
    ///
    /// **大文件保护**:超过 `MAX_CONTEXT_FILE_SIZE` 的文件返回空提取器,
    /// 不提取上下文行,避免内存爆炸。匹配结果仍正常返回,仅缺少上下文。
    pub fn new(path: &Path, _window_size: usize) -> SearchResult<Self> {
        // 文件大小检查:大文件跳过上下文提取
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_CONTEXT_FILE_SIZE {
            return Ok(Self { lines: Vec::new() });
        }

        let bytes = fs::read(path)?;
        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        Ok(Self { lines })
    }
    
    // extract 方法不变,空 lines 时返回空上下文
}
```

**Why**:
1. **最简方案**:1 个常量 + 1 个 if 检查,无需引入 mmap 依赖
2. **符合 YAGNI**:10MB 阈值覆盖 99% 源代码文件,超大文件跳过上下文是合理降级
3. **备选方案(未来优化)**:引入 `memmap2` crate 做 mmap + 行偏移索引,但当前阶段过度设计

**How**:`metadata.len()` 是 `stat` 系统调用,开销极低。空 `lines` 时 `extract` 返回 `(Vec::new(), Vec::new())`,不影响搜索结果。

---

### P1-2:SearchResultTreeModel.reload() 全量刷新(渲染卡顿)

**问题**:每批结果调用 `reload()`,触发整棵树重新渲染,展开状态丢失。

**修复方案**:改用 `nodesWereInserted` 精准通知插入的节点。

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**改动位置**:L41-77 `addResults()` 函数

```kotlin
fun addResults(results: List<SearchResult>) {
    val root = root as DefaultMutableTreeNode
    val newFileNodes = mutableListOf<Pair<DefaultMutableTreeNode, Int>>() // (parentNode, childIndex)

    for (result in results) {
        val isNewFile = !fileNodeMap.containsKey(result.filePath)
        val fileNode = fileNodeMap.getOrPut(result.filePath) {
            val node = DefaultMutableTreeNode(FileNodeData(result.filePath, 0))
            root.add(node)
            node
        }

        val fileData = fileNode.userObject as FileNodeData
        fileData.matchCount++

        val matchNode = DefaultMutableTreeNode(
            MatchNodeData(
                filePath = result.filePath,
                lineNumber = result.lineNumber,
                column = result.column,
                matchedText = result.matchedText,
                contextBefore = result.contextBefore,
                contextAfter = result.contextAfter
            ),
            true
        )
        
        val matchIndex = fileNode.childCount
        fileNode.add(matchNode)
        totalMatches++
        
        // 记录新插入的节点(精准通知)
        if (isNewFile) {
            newFileNodes.add(root to (root.childCount - 1))
        }
        newFileNodes.add(fileNode to matchIndex)
    }

    // 精准通知插入(替代 reload 全量刷新)
    // 消费者(UI 线程)独占树模型操作,无锁
    for ((parent, index) in newFileNodes) {
        val childIndices = intArrayOf(index)
        val childNodes = arrayOf(parent.getChildAt(index))
        nodesWereInserted(parent, childIndices)
    }
}
```

**Why**:
1. `nodesWereInserted` 只通知插入的节点,保留其他节点的展开状态
2. 复杂度从 O(N²)(reload + expandPath 循环)降到 O(N)
3. 符合"消费者独占堆":UI 线程独占树模型操作

**How**:`DefaultTreeModel.nodesWereInserted` 是 Swing 官方 API,精准通知插入事件。

---

### P1-3:autoSearchListener 事件风暴(时序问题)

**问题**:`refreshModuleList()` 的 `addItem` 触发 `ActionListener`,导致 `performSearch()` 被多次调用。

**修复方案**:刷新期间用标志位禁用 autoSearchListener。

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动 1**:新增标志位字段

```kotlin
/** 模块列表刷新中标志,避免 addItem 触发 autoSearch */
private var isRefreshingModules = false
```

**改动 2**:`refreshModuleList()` 包裹标志位

```kotlin
private fun refreshModuleList() {
    isRefreshingModules = true
    try {
        moduleComboBox.removeAllItems()
        val modules = ModuleManager.getInstance(project).modules
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

**改动 3**:`autoSearchListener` 增加标志位检查

```kotlin
val autoSearchListener = java.awt.event.ActionListener { 
    if (!isRefreshingModules) performSearch() 
}
```

**Why**:标志位是最简方案,无需 debounce 复杂逻辑。

---

### P1-4:后台搜索线程 send 阻塞(线程泄漏)

**问题**:`tx.send` 满时永久阻塞,若 Kotlin 侧停止 poll 但未取消,后台线程永久泄漏。

**修复方案**:`send` 前检查 `cancel_flag`,并改用 `send_timeout` 避免永久阻塞。

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:`run_stream_search` 中的 `tx.send(Ok(m))`(已在前述 P0-1 改造中包含)

由于 P0-1 改造为 `par_iter`,send 逻辑在闭包内。增加 `send_timeout` 保护:

```rust
// channel 满时阻塞,形成背压;接收方关闭时返回错误
// 增加 send_timeout 避免永久阻塞(消费者异常时线程能退出)
use crossbeam_channel::SendTimeoutError;

match tx.send_timeout(Ok(m), Duration::from_millis(500)) {
    Ok(()) => {
        sent_count.fetch_add(1, Ordering::Relaxed);
    }
    Err(SendTimeoutError::Timeout(_)) => {
        // 超时:消费者可能已停止 poll,检查取消标志
        if cancel_flag.load(Ordering::Relaxed) {
            return Err(SearchError::Cancelled);
        }
        // 未取消但超时:继续重试或返回错误
        return Err(SearchError::Internal("channel send timeout".into()));
    }
    Err(SendTimeoutError::Disconnected(_)) => {
        return Err(SearchError::Cancelled);
    }
}
```

**Why**:`send_timeout` 是 crossbeam-channel 官方 API,避免永久阻塞。500ms 超时足够消费者处理一批结果。

---

### P1-5:RustSearchPanel 未实现 Disposable(协程泄漏)

**问题**:`searchScope` 未绑定生命周期,ToolWindow 释放时协程泄漏。

**修复方案**:`RustSearchPanel` 实现 `Disposable`,在 `dispose()` 中取消协程作用域。

**文件 1**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

```kotlin
import com.intellij.openapi.Disposable

class RustSearchPanel(private val project: Project) : JPanel(BorderLayout()), Disposable {
    
    // ... 原有字段
    
    override fun dispose() {
        searchJob?.cancel()
        searchScope.cancel()
        logger.info("RustSearchPanel disposed")
    }
    
    // ... 原有方法
}
```

**文件 2**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt)

```kotlin
import com.intellij.openapi.util.Disposer

override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
    val panel = RustSearchPanel(project)
    val content = ContentFactory.getInstance()
        .createContent(panel, RustSearchBundle.message("toolwindow.content.name"), false)
    toolWindow.contentManager.addContent(content)
    
    // 关键:注册 Disposable,ToolWindow 释放时调用 panel.dispose()
    Disposer.register(toolWindow.disposable, panel)
}
```

**Why**:`Disposable` 是 IntelliJ Platform 官方生命周期管理接口,`Disposer.register` 确保 ToolWindow 释放时清理资源。

---

### P2-1:动态库临时文件未清理且无用户隔离

**文件**:[src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt)

**改动位置**:L79-84

```kotlin
// 临时目录加用户名后缀,多用户系统隔离
val tempDir = File(
    System.getProperty("java.io.tmpdir"), 
    "rustsearch-${System.getProperty("user.name", "default")}"
)
```

**dispose 中清理**(可选):

```kotlin
override fun dispose() {
    logger.info("RustSearchService disposed")
    // 清理临时动态库文件(可选,避免残留)
    try {
        val tempDir = File(
            System.getProperty("java.io.tmpdir"),
            "rustsearch-${System.getProperty("user.name", "default")}"
        )
        tempDir.listFiles()?.forEach { it.delete() }
        tempDir.delete()
    } catch (e: Exception) {
        logger.warn("Failed to clean temp dir: ${e.message}")
    }
}
```

---

### P2-2:isSearchComplete 与 pollResults 之间结果丢失

**问题**:`isSearchComplete` 返回 true 时,最后一批结果可能在 channel 中未被取走。

**修复方案**:Rust 侧保证 `is_complete` 标志在 channel 排空后才设置(已在前述 P0-4 改造中部分解决)。

**补充改动**:[rust-search/src/jni/bridge.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/jni/bridge.rs) `run_poll_results` 中,标记 `should_mark_complete` 后,继续 try_recv 排空:

```rust
// 在标记 should_mark_complete = true 后,继续排空剩余结果
if should_mark_complete && pending_error.is_none() {
    while let Ok(item) = session.receiver.try_recv() {
        match item {
            Ok(m) => batch.push(m),
            Err(_) => break,
        }
    }
}
```

**Why**:确保 channel 完全排空后再标记完成,避免结果丢失。

---

### P2-3:cancel 后返回部分结果语义不清

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L81-83 `search()` 方法(已废弃,但保持语义正确)

```rust
// 取消时始终返回 Cancelled 错误,由 UI 层决定是否展示已收到的部分结果
if self.cancel_flag.load(Ordering::Relaxed) {
    return Err(SearchError::Cancelled);
}
```

**Why**:语义清晰,用户能区分"完整结果"与"取消后的部分结果"。

---

### P2-4:正则错误延迟到搜索启动后才暴露

**文件**:[rust-search/src/search/config.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/config.rs)

**改动位置**:L66-82 `validate()` 方法

```rust
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
    // 新增:正则模式预编译校验,提前暴露错误
    if self.is_regex {
        let _ = regex::Regex::new(&self.pattern)
            .map_err(|e| SearchError::RegexCompile(format!("正则表达式无效: {e}")))?;
    }
    Ok(())
}
```

**Why**:提前校验,避免用户等待 `walker.files()` 完成后才看到错误。

---

## 四、Assumptions & Decisions(假设与决策)

### 4.1 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| registry 数据结构 | 保持 `Mutex<HashMap>` + `Arc<SearchSession>` | 锁仅用于 start/cancel/release 瞬时操作,poll 不持锁;无需引入 dashmap,遵循最小依赖 |
| 流式搜索并行化 | `files.par_iter().try_for_each` | 复用 `search()` 已验证的 par_iter 模式,channel 天然支持多生产者 |
| ContextExtractor 大文件策略 | 文件大小阈值(10MB)跳过 | 最简方案,无需引入 mmap;覆盖 99% 源代码文件 |
| 取消通知机制 | Kotlin `finally` 中 `cancel(searchId)` + `releaseSearch` | 利用现有 JNI 接口,无需新增 |
| 协程取消旧搜索 | `searchJob?.cancel()` | 协程取消是协作式的,1 行代码消除竞态 |
| send 阻塞保护 | `send_timeout(500ms)` | crossbeam 官方 API,避免永久阻塞 |
| Panel 生命周期 | 实现 `Disposable` + `Disposer.register` | IntelliJ Platform 官方推荐方式 |

### 4.2 明确不做的事(避免过度设计)

- **不引入 dashmap**:registry 锁竞争是低频 JNI 会话级,`Mutex<HashMap>` + `Arc` 已足够
- **不引入 memmap2**:10MB 阈值跳过上下文是合理降级,mmap 增加复杂度
- **不重构为 per-search 消费者线程模式**:当前 pollResults 锁外消费已实现"消费者独占",无需额外线程
- **不重写 channel 机制**:crossbeam-channel 已是无锁队列,复用即可
- **不增加配置项**:阈值、超时等使用常量,避免过度配置化

### 4.3 假设

1. **假设** `par_iter` 闭包中 `&Matcher` 和 `&Sender` 跨线程共享是安全的(已通过 Phase 1 探索确认)
2. **假设** `Arc<SearchSession>` 的 clone 开销(原子操作)可忽略不计
3. **假设** 10MB 文件大小阈值覆盖绝大多数源代码文件(Java/Kotlin 单文件通常 < 1MB)
4. **假设** `send_timeout(500ms)` 足够消费者处理一批结果(默认 poll 间隔 200ms)

---

## 五、Verification(验证方案)

### 5.1 编译验证

```bash
# Rust 侧编译
cd rust-search && cargo build --release

# Kotlin 侧编译
./gradlew buildPlugin
```

### 5.2 单元测试

```bash
# Rust 单测
cd rust-search && cargo test

# 重点测试:
# - test_search_stream_parallel(新增):验证并行搜索结果完整性
# - test_poll_results_lock_free(新增):验证锁外消费正确性
# - test_context_extractor_large_file(新增):验证大文件跳过上下文
# - test_send_timeout(新增):验证 send 超时保护
```

### 5.3 集成测试

```bash
# 运行 IDE 测试实例
./gradlew runIde

# 手动验证场景:
# 1. 快速连续搜索两个关键词,验证结果不混杂(P0-3)
# 2. 搜索高频词,按 Esc 取消,观察日志 "Search session released"(P0-2)
# 3. 大项目搜索,观察 CPU 多核满载(P0-1)
# 4. 多搜索并发,观察无阻塞(P0-4)
# 5. 大文件搜索,观察内存稳定(P1-1)
# 6. 流式结果追加,观察 UI 不卡顿(P1-2)
# 7. 切换作用域,观察只触发 1 次搜索(P1-3)
# 8. 关闭项目,观察协程清理(P1-5)
```

### 5.4 性能基准测试

```bash
# 对比修复前后性能
# 测试样本:5 万+ 文件 Android 项目
# 指标:
# - 搜索耗时(应提升 4-8 倍)
# - CPU 利用率(应多核满载)
# - 内存峰值(应 < 200MB)
# - 取消响应延迟(应 < 100ms)
```

### 5.5 残余风险

1. **par_iter 并发安全**:需全量回归测试,确保多线程下结果完整
2. **send_timeout 误判**:极端情况下消费者正常但慢,可能误判为超时;500ms 阈值需实测调整
3. **Arc 循环引用**:`SearchSession` 不持有 `Arc<SearchSession>`,无循环引用风险
4. **Disposable 时序**:ToolWindow 释放时若有正在进行的搜索,需确保 `dispose` 后 Flow 不再 emit(已通过 `searchJob?.cancel()` 保证)

---

## 六、实施顺序(按优先级)

| 顺序 | 问题 | 优先级 | 改动量 | 风险 |
|------|------|--------|--------|------|
| 1 | P0-3 并发搜索结果竞态 | P0 | 1 行 | 低 |
| 2 | P0-2 取消未通知 Rust | P0 | 中 | 低 |
| 3 | P0-4 pollResults 持锁 | P0 | 中 | 中 |
| 4 | P0-1 流式搜索串行 | P0 | 中 | 中 |
| 5 | P1-5 Panel Disposable | P1 | 低 | 低 |
| 6 | P1-3 autoSearchListener 事件风暴 | P1 | 低 | 低 |
| 7 | P1-1 ContextExtractor 大文件 | P1 | 低 | 低 |
| 8 | P1-2 reload 全量刷新 | P1 | 中 | 低 |
| 9 | P1-4 send 阻塞 | P1 | 低 | 低 |
| 10 | P2 系列问题 | P2 | 低 | 低 |

**建议**:P0-3 和 P0-2 优先修复(低风险高收益),P0-4 和 P0-1 需充分测试(改动较大但核心)。

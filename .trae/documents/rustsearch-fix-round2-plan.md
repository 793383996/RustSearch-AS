# RustSearch-AS 第二轮精准修复计划

> 核心原则:最少代码改动、精准修复根因、不扩大改动范围、不引入新依赖。
> 所有修复均基于前一轮分析报告的根因定位,直接修改问题代码点。

---

## 一、Summary(摘要)

针对前一轮分析识别的 7 个问题(2 P1 + 5 P2),制定最小改动修复方案。核心思路:

1. **P1-A/P1-B(渲染卡顿回归)**:P1-2 修复引入的 `nodeChanged` 全量遍历和 `expandPath` 冗余循环,通过"只通知受影响节点"和"移除冗余代码"修复
2. **P2-A(max_total 竞态)**:`load + fetch_add` 非原子,改为 `fetch_add` 后检查上限,超出则不发送(允许少量超出 1 个/线程,可接受)
3. **P2-B(send_timeout 误判)**:超时后不返回错误,改为检查 `cancel_flag` 后重试 send
4. **P2-C(metadata 失败)**:`?` 传播改为 `unwrap_or(0)` 降级
5. **P2-D(错误吞没)**:`Err(_)` 改为区分 IO 错误与其他错误
6. **P2-E(dispose 时序)**:searchId 递增不复用,残余风险极低,记录为已知限制不修复

---

## 二、Current State Analysis(当前状态分析)

### 2.1 问题根因定位

| 问题 | 根因文件:行 | 根因描述 |
|------|------------|----------|
| P1-A | SearchResultTreeModel.kt:88-93 | `for (fileNode in fileNodeMap.values)` 遍历所有文件节点调用 `nodeChanged`,应只通知本批受影响节点 |
| P1-B | RustSearchPanel.kt:355-364 | `expandPath` 循环注释过时(说 reload 会折叠),实际 P1-2 已改 `nodesWereInserted`,展开状态自动保留 |
| P2-A | engine.rs:163-170 | `sent_count.load()` 检查与 `fetch_add` 非原子,多线程并发窗口内同时通过检查 |
| P2-B | engine.rs:172-179 | `send_timeout` 超时后返回 `Internal` 错误,导致 `try_for_each` 停止所有线程 |
| P2-C | context.rs:33 | `fs::metadata(path)?` 直接传播错误,文件变动时导致整个文件搜索失败 |
| P2-D | engine.rs:69-72 | `Err(_) => Vec::new()` 无差别吞掉所有错误,包括配置错误 |
| P2-E | RustSearchPanel.kt:446-450 | `dispose()` 不等待 Flow finally 完成,但 searchId 递增不复用,实际风险极低 |

### 2.2 修复策略

- **删除优先于新增**:P1-B 是删除冗余代码,P1-A 是缩小遍历范围
- **降级优先于中断**:P2-C 用 `unwrap_or` 降级而非 `?` 中断
- **重试优先于失败**:P2-B 超时后重试而非直接报错
- **区分优先于吞没**:P2-D 区分 IO 错误与配置错误

---

## 三、Proposed Changes(修复方案)

### P1-A:nodeChanged 全量遍历改为只通知受影响节点

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**改动位置**:L82-93 `addResults()` 方法末尾的 `nodeChanged` 循环

**当前代码**:
```kotlin
// 更新文件节点显示文本(匹配数变化)
for (fileNode in fileNodeMap.values) {
    val data = fileNode.userObject as FileNodeData
    fileNode.userObject = data // matchCount 已更新,触发重绘
    nodeChanged(fileNode)
}
```

**修复后**:
```kotlin
// 只通知本批受影响文件节点(避免 O(N) 全量遍历,N=总文件数)
// collectedFileNodes 在上面的 for 循环中收集(见下文完整代码)
for (fileNode in affectedFileNodes) {
    nodeChanged(fileNode)
}
```

**完整改动**(在 `for (result in results)` 循环中收集受影响节点):

```kotlin
fun addResults(results: List<SearchResult>) {
    val root = root as DefaultMutableTreeNode
    val insertedNodes = mutableListOf<Pair<DefaultMutableTreeNode, Int>>()
    // P1-A:收集本批受影响的文件节点(去重),只对这些节点调用 nodeChanged
    val affectedFileNodes = LinkedHashSet<DefaultMutableTreeNode>()

    for (result in results) {
        val isNewFile = !fileNodeMap.containsKey(result.filePath)
        val fileNode = fileNodeMap.getOrPut(result.filePath) {
            val node = DefaultMutableTreeNode(FileNodeData(result.filePath, 0))
            root.add(node)
            node
        }

        if (isNewFile) {
            insertedNodes.add(root to (root.childCount - 1))
        }

        val fileData = fileNode.userObject as FileNodeData
        fileData.matchCount++
        affectedFileNodes.add(fileNode)  // P1-A:收集受影响节点

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
        insertedNodes.add(fileNode to matchIndex)
        totalMatches++
    }

    for ((parent, index) in insertedNodes) {
        val childIndices = intArrayOf(index)
        nodesWereInserted(parent, childIndices)
    }

    // P1-A:只通知受影响文件节点(替代原 fileNodeMap.values 全量遍历)
    for (fileNode in affectedFileNodes) {
        nodeChanged(fileNode)
    }
}
```

**Why**:原代码遍历所有文件节点(可能 1000+),实际只有本批结果涉及的文件节点的 `matchCount` 变化。用 `LinkedHashSet` 去重收集受影响节点,复杂度从 O(总文件数) 降到 O(本批文件数)。

---

### P1-B:移除 performSearch 中冗余的 expandPath 循环

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:L352-367 `performSearch()` 中 `collect` 回调内的 `expandPath` 循环

**当前代码**:
```kotlin
service.search(config).collect { batch ->
    // 在 UI 线程更新树
    withContext(Dispatchers.Main) {
        treeModel.addResults(batch)
        // 通过 TreePath 展开所有文件节点。
        // treeModel.addResults 内部调用 reload() 会折叠全部节点,
        // 且流式追加后 expandRow 依赖的行映射对新节点不可靠;
        // 改用 expandPath 直接按节点路径展开,确保滚动到下方时新节点也已展开。
        val root = treeModel.root as DefaultMutableTreeNode
        for (i in 0 until root.childCount) {
            val fileNode = root.getChildAt(i) as DefaultMutableTreeNode
            val path = TreePath(treeModel.getPathToRoot(fileNode))
            resultTree.expandPath(path)
        }
        val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
        statusLabel.text = RustSearchBundle.message("search.status.found", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)
    }
}
```

**修复后**:
```kotlin
service.search(config).collect { batch ->
    // 在 UI 线程更新树
    withContext(Dispatchers.Main) {
        treeModel.addResults(batch)
        // P1-B:移除冗余 expandPath 循环。
        // P1-2 修复后 addResults 改用 nodesWereInserted(不再 reload),
        // 节点展开状态自动保留,无需手动展开所有文件节点。
        val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
        statusLabel.text = RustSearchBundle.message("search.status.found", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)
    }
}
```

**Why**:P1-2 修复后 `addResults` 不再调用 `reload()`(改用 `nodesWereInserted`),Swing 的 `DefaultTreeModel` 会保留现有节点的展开状态。原 `expandPath` 循环是基于过时注释("reload 会折叠全部节点")的冗余操作,删除后复杂度从 O(N) 降为 O(1)。

**注意**:删除后需移除未使用的 import `TreePath`(若文件中其他位置未使用)。检查:`TreePath` 仅在 L362 使用,删除后需移除 L36 的 import。

---

### P2-A:max_total_matches TOCTOU 竞态(接受近似上限)

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L163-170 `run_stream_search` 中 `sent_count` 检查与递增

**当前代码**:
```rust
if sent_count.load(Ordering::Relaxed) >= max_total {
    return Err(SearchError::Cancelled);
}

// P1-4:send_timeout 避免消费者异常时线程永久阻塞(500ms 背压超时)
match tx.send_timeout(Ok(m), Duration::from_millis(500)) {
    Ok(()) => {
        sent_count.fetch_add(1, Ordering::Relaxed);
    }
    // ...
}
```

**修复后**(先 fetch_add 再检查,超出则不发送):
```rust
// P2-A:先原子递增再检查上限,消除 load+fetch_add 之间的竞态窗口。
// 采用 fetch_add 后比较旧值:若已达到上限,直接返回(本线程不发送)。
// 注:多线程并发时可能有 (并行度-1) 个额外结果通过,属可接受的近似上限。
let prev_count = sent_count.fetch_add(1, Ordering::Relaxed);
if prev_count >= max_total {
    return Err(SearchError::Cancelled);
}

// P1-4:send_timeout 避免消费者异常时线程永久阻塞(500ms 背压超时)
match tx.send_timeout(Ok(m), Duration::from_millis(500)) {
    Ok(()) => {
        // sent_count 已在上面递增
    }
    Err(SendTimeoutError::Timeout(_)) => {
        // P2-B:超时后回退计数并重试(见 P2-B 修复)
        sent_count.fetch_sub(1, Ordering::Relaxed);
        // ... P2-B 重试逻辑
    }
    Err(SendTimeoutError::Disconnected(_)) => {
        sent_count.fetch_sub(1, Ordering::Relaxed);
        return Err(SearchError::Cancelled);
    }
}
```

**Why**:`fetch_add` 返回旧值,若旧值 >= max_total 说明已超上限,本线程不发送。虽然并发时可能有少量超出(多线程同时 fetch_add 都 < max_total),但结果数严格 <= max_total + 并行度 - 1,属可接受的近似上限。比原 TOCTOU 竞态更严格。

**简化方案**(更少代码改动,接受少量超出):
保持原 `load` 检查,但在 `fetch_add` 后再检查一次,超出则不发送(结果可能少 1 个,但不会多)。**推荐采用 fetch_add 优先方案**。

---

### P2-B:send_timeout 超时改为重试而非报错

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L172-184 `send_timeout` 的 `Timeout` 分支

**当前代码**:
```rust
Err(SendTimeoutError::Timeout(_)) => {
    // 超时:消费者可能已停止 poll,检查取消标志
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }
    return Err(SearchError::Internal(
        "channel send timeout (consumer stalled)".into(),
    ));
}
```

**修复后**(超时后检查取消标志,未取消则重试):
```rust
Err(SendTimeoutError::Timeout(_)) => {
    // P2-B:超时可能是消费者处理慢(如 UI 渲染大结果集),而非 stalled。
    // 检查取消标志:已取消则退出,未取消则重试 send(避免误终止搜索)。
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }
    // 未取消:继续循环重试 send(下方 continue 到 for m in matches 的下一次迭代)
    // 注意:此处不 return,让外层 for 循环继续处理下一个 m(本 m 已丢失,可接受)
    continue;
}
```

**备选方案**(更严谨,重试同一个 m):
```rust
// P2-B:超时后重试同一个 m,而非放弃
loop {
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }
    match tx.send_timeout(Ok(m.clone()), Duration::from_millis(500)) {
        Ok(()) => break,
        Err(SendTimeoutError::Timeout(_)) => continue,  // 重试
        Err(SendTimeoutError::Disconnected(_)) => {
            return Err(SearchError::Cancelled);
        }
    }
}
```

**推荐采用备选方案**(重试同一个 m,不丢失结果)。需要 `SearchMatch: Clone`(已确认 `#[derive(Clone)]`,见 matcher.rs:21)。

**Why**:原代码超时即返回 `Internal` 错误,导致 `try_for_each` 停止所有工作线程,整个搜索提前终止。实际上消费者可能只是处理慢(如 UI 渲染 1000+ 结果),并非异常。重试机制避免误终止。

---

### P2-C:metadata 失败降级为空提取器

**文件**:[rust-search/src/search/context.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/context.rs)

**改动位置**:L33 `fs::metadata(path)?`

**当前代码**:
```rust
pub fn new(path: &Path, _window_size: usize) -> SearchResult<Self> {
    // P1-1:文件大小检查,大文件跳过上下文提取(避免内存爆炸)
    let metadata = fs::metadata(path)?;
    if metadata.len() > MAX_CONTEXT_FILE_SIZE {
        return Ok(Self { lines: Vec::new() });
    }
    // ...
}
```

**修复后**:
```rust
pub fn new(path: &Path, _window_size: usize) -> SearchResult<Self> {
    // P1-1:文件大小检查,大文件跳过上下文提取(避免内存爆炸)
    // P2-C:metadata 失败(文件被删除/权限不足)时降级为空提取器,不中断搜索
    let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    if file_size > MAX_CONTEXT_FILE_SIZE {
        return Ok(Self { lines: Vec::new() });
    }
    // ...
}
```

**Why**:`metadata` 失败时(文件被删除、权限不足),`unwrap_or(0)` 返回 0,不会触发大文件跳过,继续尝试 `fs::read`。若 `fs::read` 也失败,才向上传播错误(此时确属 IO 异常)。避免文件变动场景下的结果丢失。

---

### P2-D:engine.rs search() 区分 IO 错误与配置错误

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L69-72 `search()` 方法中 `par_iter` 的 `Err(_)` 分支

**当前代码**:
```rust
match matcher.search_file(file, &cancel_flag) {
    Ok(matches) => matches,
    Err(_) => Vec::new(), // 单文件失败不中断整体,记为空
}
```

**修复后**:
```rust
match matcher.search_file(file, &cancel_flag) {
    Ok(matches) => matches,
    // P2-D:IO 错误(文件权限/不存在/编码)降级为空,不中断整体搜索;
    // 配置错误(InvalidPattern/RegexCompile)向上传播,让用户看到错误。
    Err(SearchError::Io(_)) | Err(SearchError::Jni(_)) => Vec::new(),
    Err(e) => return Err(e),
}
```

**Why**:原 `Err(_)` 吞掉所有错误,包括 `InvalidPattern`、`RegexCompile` 等配置错误,用户以为"没匹配"而非"搜索出错"。区分后,IO 错误降级(单文件失败不影响整体),配置错误传播(让用户知道配置有误)。

**注意**:`par_iter` 的闭包返回 `Result` 时,`Err` 会传播到 `try_for_each` 的返回值。但原代码用 `.map()` 返回 `Vec`,需改为 `.map(|file| -> Result<Vec<SearchMatch>, SearchError> {...})` + `try_for_each`。实际上当前 `search()` 用的是 `.par_iter().map().collect()`,不会传播错误。需改为:

```rust
// 文件级并行:每个文件独立搜索,结果合并
let results: Vec<Vec<SearchMatch>> = files
    .par_iter()
    .map(|file| {
        if cancel_flag.load(Ordering::Relaxed) {
            return Ok(Vec::new());
        }

        match matcher.search_file(file, &cancel_flag) {
            Ok(matches) => Ok(matches),
            Err(SearchError::Io(_)) | Err(SearchError::Jni(_)) => Ok(Vec::new()),
            Err(e) => Err(e),
        }
    })
    .collect::<Result<Vec<_>, _>>()?;

let mut all_matches: Vec<SearchMatch> = results.into_iter().flatten().collect();
```

**Why**:`.collect::<Result<Vec<_>, _>>()` 在任一文件返回 `Err` 时提前终止并传播错误。IO/JNI 错误降级为 `Ok(Vec::new())`,配置错误传播为 `Err`。

---

### P2-E:dispose 时序(记录为已知限制,不修复)

**分析**:`dispose()` 调用 `searchScope.cancel()` 后,Flow 的 `finally` 块在 IO 线程异步执行。但:
1. `searchId` 由 `AtomicU64::fetch_add` 生成,全局递增不复用
2. 旧搜索的 `releaseSearch(searchId)` 只会移除自己的 session,不会误删新搜索的 session
3. 实际风险极低,修复需阻塞 EDT 等待 finally 完成,代价过高

**决策**:记录为已知限制,不修复。在 `dispose()` 中添加注释说明。

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动**(仅添加注释):
```kotlin
override fun dispose() {
    searchJob?.cancel()
    searchScope.cancel()
    // P2-E:已知限制 — Flow finally(cancel+releaseSearch)在 IO 线程异步执行,
    // 可能在 dispose 返回后仍在运行。由于 searchId 全局递增不复用,
    // 旧搜索的 releaseSearch 不会误删新搜索的 session,实际风险极低。
    logger.info("RustSearchPanel disposed")
}
```

---

## 四、Assumptions & Decisions(假设与决策)

### 4.1 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| P1-A nodeChanged 范围 | `LinkedHashSet` 收集受影响节点 | 去重 + 保序,复杂度从 O(总文件数) 降到 O(本批文件数) |
| P1-B expandPath 处理 | 直接删除循环 | P1-2 已改用 `nodesWereInserted`,展开状态自动保留,循环冗余 |
| P2-A max_total 竞态 | `fetch_add` 优先 + 接受近似上限 | 严格消除 TOCTOU 窗口,少量超出(≤并行度-1)可接受 |
| P2-B send_timeout 误判 | 重试同一个 m(需 Clone) | `SearchMatch: Clone` 已确认,重试不丢失结果 |
| P2-C metadata 失败 | `unwrap_or(0)` 降级 | 最小改动,1 行代码 |
| P2-D 错误吞没 | 区分 IO/JNI 与配置错误 | 配置错误传播让用户感知,IO 错误降级不中断 |
| P2-E dispose 时序 | 不修复,记录已知限制 | searchId 不复用,实际风险极低,修复代价过高 |

### 4.2 不做的事

- 不重构 `addResults` 的整体结构,只优化 `nodeChanged` 范围
- 不引入 debounce/throttle 处理 UI 渲染
- 不修改 `send_timeout` 的超时时长(500ms 保持)
- 不修改 `dispose()` 的实现逻辑(仅添加注释)

---

## 五、Verification(验证方案)

### 5.1 编译验证

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search && cargo build --release
cd /Users/apple/AndroidStudioProjects/RustSearch-AS && ./gradlew compileKotlin
```

### 5.2 测试验证

```bash
# Rust 测试(验证无回归)
cd /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search && cargo test
```

### 5.3 重点验证场景

| 场景 | 验证问题 | 预期结果 |
|------|----------|----------|
| 搜索匹配 500+ 文件的关键词 | P1-A | 每批结果 nodeChanged 调用次数 = 本批文件数,非总文件数 |
| 流式结果追加 | P1-B | 文件节点自动展开,无 expandPath 循环 |
| max_total=10,高频词搜索 | P2-A | 结果数 ≤ 10 + 并行度 - 1 |
| 大结果集搜索,UI 慢消费 | P2-B | 搜索正常完成,不提前终止 |
| 搜索期间文件被删除 | P2-C | 其他文件结果正常返回 |
| 无效 glob 搜索 | P2-D | 返回错误提示,非空结果 |
| 快速开关 ToolWindow | P2-E | 无 session 误删(已知限制) |

### 5.4 残余风险

1. **P2-A 近似上限**:实际结果数可能略超 max_total(≤并行度-1),文档需说明"近似上限"语义
2. **P2-B 重试开销**:极端慢消费者场景下,重试会占用工作线程,但比误终止更可接受
3. **P2-E 已知限制**:dispose 后 Flow finally 异步执行,实际无影响但需记录

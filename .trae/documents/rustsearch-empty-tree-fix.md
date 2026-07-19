# RustSearch-AS 搜索结果树空白修复计划

> 现象:搜索完成后状态栏显示"301 匹配",但结果树空白无内容。
> 根因:旧搜索协程的 EDT 任务在新搜索 `clear()` 之后执行,导致 `totalMatches` 被污染到 301,但 JTree 内部状态已被 `reload()` 重置,`nodesWereInserted` 事件被忽略,tree 视图空白。
> 策略:**令牌机制 + invokeLater 治本**(用户已确认)

---

## 一、Summary(摘要)

### 1.1 问题表现

- 状态栏显示"搜索完成: 301 匹配(X 个文件),耗时 Ys"
- 结果树区域完全空白,无文件节点,无匹配节点
- 无错误日志,无异常堆栈
- 重现条件:快速连续搜索(如输入 'K' → 'a'),间隔 < 200ms

### 1.2 根因(已通过代码+日志分析定位)

**根因 A(主因,高置信)**:旧协程 EDT 任务滞后执行,污染新 tree 状态

时序链路:
```
t0: 用户输入 'K' → performSearch() 启动 coroutine_K
t1: coroutine_K emit batch1 → withContext(Main) 排队 EDT 任务 task1
t2: EDT 执行 task1 → addResults(batch1), totalMatches=50

t3: 用户输入 'a' → performSearch()
t4: searchJob?.cancel()  (仅取消协程,不撤销已排队的 EDT 任务)
t5: treeModel.clear()    (totalMatches=0, reload() 重置 JTree 状态)
t6: 启动 coroutine_Ka

t7: EDT 处理 coroutine_K 的 batch2 → addResults(batch2)
    ⚠️ 在 clear() 之后执行!
    → totalMatches=251, fileNodeMap 有数据
    → nodesWereInserted 通知 JTree
    → 但 JTree 内部 TreeState 已被 reload() 重置,事件被忽略

t8: coroutine_Ka 完成(0 结果)
t9: EDT 显示 "complete: 251 matches"  (50 + 251 = 301)
```

**根因 B(次因,中置信)**:`Dispatchers.Main` 在 IntelliJ 中的 modality 调度风险

- IC-2023.1 的 `Dispatchers.Main` 通过 `ModalityState.defaultModalityState()` 调度
- 若 IDE 处于任何模态上下文,任务可能延迟
- `DefaultTreeModel` 非线程安全,若 modality 异常导致非 EDT 执行,Swing 状态可能损坏

### 1.3 修复方案(用户已确认:令牌+invokeLater)

| 修复点 | 位置 | 作用 |
|--------|------|------|
| 令牌机制 | `RustSearchPanel.performSearch` | 滞后 EDT 任务自检后丢弃,不污染新 tree |
| invokeLater | `RustSearchPanel` collect 块 3 处 | 显式调度到 EDT,绕过 Dispatchers.Main 的 modality 风险 |
| 诊断日志 | `SearchResultTreeModel.addResults/clear` | 验证修复后无滞后,便于回归 |

---

## 二、Current State Analysis(当前状态分析)

### 2.1 关键代码现状

**`RustSearchPanel.kt:315-390` performSearch**:
- L317: `searchJob?.cancel()` 仅设置取消标志,不撤销已排队的 EDT 任务
- L322/L351: `treeModel.clear()` 同步执行 `reload()`
- L361/L377/L383: 三处 `withContext(Dispatchers.Main) { treeModel.addResults(...) }`
- collect 块内无 searchId 校验,batch 可能属于已取消的旧搜索

**`SearchResultTreeModel.kt:61-120` addResults**:
- 无线程断言,可能在非 EDT 被调用
- 无日志,totalMatches 递增过程不可观测
- `nodesWereInserted` 通知后若 JTree 状态已重置,事件被忽略

**`SearchResultTreeModel.kt:127-134` clear**:
- 调用 `reload()` 触发 `treeStructureChanged`,JTree 重置内部 TreeState
- 后续 `nodesWereInserted` 事件可能被忽略

**`RustSearchService.kt:126-170` search Flow**:
- `flowOn(Dispatchers.IO)` emit 在 IO 线程
- 无 batch 大小日志,无法从日志侧验证 emit 次数

### 2.2 日志证据

`/Users/apple/AndroidStudioProjects/RustSearch-AS/build/idea-sandbox/IC-2023.1/log/idea.log`:

```
17:58:35,426  Search started: searchId=1, pattern='K'
17:58:35,570  Search session released: searchId=1   (144ms,含 50ms sleep)
17:58:44,359  Search started: searchId=2, pattern='Ka'
17:58:44,423  Search session released: searchId=2   (64ms,含 50ms sleep)
```

- 无错误日志,无 Exception 堆栈
- Rust 侧无任何日志输出(无 tracing/log 依赖)
- SearchResultTreeModel 无日志,addResults/clear 时序不可观测

---

## 三、Proposed Changes(精确改动方案)

### 3.1 改动 1:`RustSearchPanel.kt` 新增令牌字段

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**位置**:类字段区域(L74-80 附近,`searchJob` 之后)

**新增字段**:
```kotlin
/** 当前搜索令牌,用于丢弃滞后 EDT 任务(根因 A 修复) */
private var activeSearchToken: Long = 0L
```

**理由**:
- 用 `Long` 而非 `AtomicLong`:该字段仅在 EDT 读写(performSearch 在 EDT 调用,invokeLater 回调在 EDT)
- 初始值 0L,首次搜索 token=1
- `System.nanoTime()` 不适合(返回值可能重复,用自增更稳定)

---

### 3.2 改动 2:`performSearch` 生成新令牌

**位置**:`performSearch` 方法开头(L315-322)

**修复后**:
```kotlin
private fun performSearch() {
    // 取消旧搜索,防止旧 Flow 的 withContext(Main) 在新 clear() 后追加结果导致竞态
    searchJob?.cancel()

    // 根因 A 修复:生成新令牌,滞后的旧 EDT 任务通过令牌校验自检丢弃
    activeSearchToken++

    val pattern = searchField.text.trim()
    if (pattern.isEmpty()) {
        treeModel.clear()
        statusLabel.text = RustSearchBundle.message("search.status.empty.input")
        return
    }

    val roots = resolveSearchRoots()
    if (roots.isEmpty()) {
        statusLabel.text = RustSearchBundle.message("search.status.no.roots")
        return
    }

    // ... includeGlobs / config 构建不变 ...
```

**关键点**:
- `activeSearchToken++` 必须在 `treeModel.clear()` 之前执行,确保新令牌生效后才清空 tree
- 令牌在 EDT 上递增(performSearch 由 EDT 触发),无需同步原语

---

### 3.3 改动 3:collect 块改用 invokeLater + 令牌校验

**位置**:`performSearch` 的 collect 块(L348-389)

**修复后**:
```kotlin
// 启动协程收集 Flow
searchJob = searchScope.launch {
    // 捕获当前搜索令牌,用于 EDT 任务自检
    val currentToken = activeSearchToken
    try {
        service.search(config).collect { batch ->
            // 根因 A+B 修复:用 invokeLater 显式调度到 EDT,绕过 Dispatchers.Main 的 modality 风险
            // 用令牌校验丢弃滞后的旧搜索 EDT 任务,避免污染新 tree
            ApplicationManager.getApplication().invokeLater {
                if (currentToken != activeSearchToken) {
                    logger.debug("Discarding stale batch: token=$currentToken != current=$activeSearchToken, batchSize=${batch.size}")
                    return@invokeLater
                }
                treeModel.addResults(batch)
                val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
                // M2:截断时显示特殊提示,引导用户缩小搜索范围
                statusLabel.text = if (treeModel.isTruncated()) {
                    RustSearchBundle.message("search.status.truncated", 50000, 5000)
                } else {
                    RustSearchBundle.message("search.status.found", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)
                }
            }
        }

        // 搜索完成
        ApplicationManager.getApplication().invokeLater {
            if (currentToken != activeSearchToken) return@invokeLater
            val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
            statusLabel.text = RustSearchBundle.message("search.status.complete", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)
        }
    } catch (e: Exception) {
        logger.error("搜索出错: pattern='$pattern', roots=$roots", e)
        ApplicationManager.getApplication().invokeLater {
            if (currentToken != activeSearchToken) return@invokeLater
            statusLabel.text = RustSearchBundle.message("search.status.error", e.message ?: "")
        }
    } finally {
        // 搜索结束,无需切换按钮状态(已移除搜索/取消按钮)
    }
}
```

**关键变更点**:
1. `withContext(Dispatchers.Main)` → `ApplicationManager.getApplication().invokeLater`
2. 每个 invokeLater 回调开头校验 `currentToken != activeSearchToken`,不等则 return
3. `currentToken` 在协程启动时捕获,作为闭包变量(不可变)
4. `activeSearchToken` 在 EDT 上读写,无并发问题
5. 新增 `logger.debug` 记录丢弃的滞后 batch(便于回归验证)

**import 调整**:
- 已存在:`import com.intellij.openapi.application.ApplicationManager`(L8)
- 无需新增 import

---

### 3.4 改动 4:`SearchResultTreeModel` 增加诊断日志

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**改动 1**:新增 import(L1-14 区域)
```kotlin
import com.intellij.openapi.diagnostic.Logger
import javax.swing.SwingUtilities
```

**改动 2**:类顶部新增 logger 字段(L29 附近)
```kotlin
class SearchResultTreeModel : DefaultTreeModel(DefaultMutableTreeNode("root")) {

    companion object {
        /** M2:UI 侧总匹配数上限,超过则停止追加(防止 Swing 树内存爆炸) */
        private const val MAX_TOTAL_MATCHES_UI = 50_000
        /** M2:文件节点数上限,超过则停止追加(Swing JTree 超过 5000 节点渲染卡顿) */
        private const val MAX_FILE_NODES_UI = 5_000
        private val LOGGER = Logger.getInstance(SearchResultTreeModel::class.java)
    }
    // ...
```

**改动 3**:`addResults` 入口加诊断日志(L61-66 区域)
```kotlin
fun addResults(results: List<SearchResult>) {
    // 诊断日志:验证线程与令牌时序(修复后应全部 EDT + 无滞后)
    LOGGER.info(
        "addResults: batch=${results.size}, totalBefore=$totalMatches, " +
        "filesBefore=${fileNodeMap.size}, isEDT=${SwingUtilities.isEventDispatchThread()}, " +
        "thread=${Thread.currentThread().name}"
    )

    // M2:截断检查 — 已截断后拒绝后续 batch
    if (truncated) return
    // ...
```

**改动 4**:`clear` 入口加诊断日志(L127-134 区域)
```kotlin
fun clear() {
    LOGGER.info(
        "clear: totalBefore=$totalMatches, filesBefore=${fileNodeMap.size}, " +
        "isEDT=${SwingUtilities.isEventDispatchThread()}, thread=${Thread.currentThread().name}"
    )
    val root = root as DefaultMutableTreeNode
    root.removeAllChildren()
    fileNodeMap.clear()
    totalMatches = 0
    truncated = false
    reload()
}
```

**理由**:
- 验证修复后所有 addResults/clear 都在 EDT 执行
- 验证无滞后 batch(totalMatches 不会被旧 batch 污染)
- 日志级别 INFO,便于在 idea.log 中直接查看
- 不影响生产性能(单次 addResults 日志开销 < 0.1ms)

---

### 3.5 改动 5:`RustSearchService.search` 增加 emit 诊断日志

**文件**:[src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt)

**位置**:search Flow 的 emit 处(L126-170)

**修复后**(在 emit 前后加日志):
```kotlin
try {
    // 轮询获取结果直到搜索完成
    while (!RustSearchEngine.isSearchComplete(searchId)) {
        val batch = RustSearchEngine.pollResults(searchId, 200)
        if (batch.isNotEmpty()) {
            logger.info("Emit batch: searchId=$searchId, batchSize=${batch.size}, sample=${batch.firstOrNull()?.filePath}")
            emit(batch.toList())
        }
    }

    // 最后再 poll 一次,确保拿到剩余结果
    val finalBatch = RustSearchEngine.pollResults(searchId, 50)
    if (finalBatch.isNotEmpty()) {
        logger.info("Emit final batch: searchId=$searchId, batchSize=${finalBatch.size}")
        emit(finalBatch.toList())
    }
} finally {
    // ...
```

**理由**:
- 验证 emit 次数与 batch 大小
- 验证 301 来源(单次 batch 还是多次累计)
- sample filePath 用于确认 batch 内容非空

---

## 四、Assumptions & Decisions(假设与决策)

### 4.1 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| 令牌类型 | `Long`(自增) | EDT 单线程读写,无需 AtomicLong;Long 范围足够(2^63 次搜索) |
| 令牌校验位置 | 每个 invokeLater 回调开头 | 覆盖 batch 处理、完成提示、错误提示三种场景 |
| 调度方式 | `invokeLater` | 显式调度到 EDT,绕过 Dispatchers.Main 的 modality 风险 |
| 日志级别 | INFO | 便于在 idea.log 直接查看,不依赖 DEBUG 开关 |
| 日志位置 | addResults/clear/emit 三处 | 覆盖 tree 状态变更的所有关键节点 |
| 是否撤销已排队的 EDT 任务 | 不撤销(无法撤销) | Swing EDT 队列不支持撤销;用令牌校验让任务自检后 return |

### 4.2 不做的事

- **不改 Rust 侧**:Rust 链路已验证正确(matcher.rs/bridge.rs/result.rs 无问题)
- **不改 SearchResultTreeModel 的 nodesWereInserted 机制**:P1-2 的精准通知是性能优化,根因不在通知方式
- **不引入 AtomicLong**:activeSearchToken 仅在 EDT 读写,无并发问题
- **不加单元测试**:Swing EDT 竞态难以在单元测试中复现,用诊断日志+手动验证更可靠
- **不改 flowOn(Dispatchers.IO)**:Flow emit 在 IO 线程是正确的,问题在消费侧

### 4.3 假设

1. **假设** `ApplicationManager.getApplication().invokeLater` 默认使用 `ModalityState.nonModal()`,在非模态上下文立即执行
2. **假设** `activeSearchToken` 仅在 EDT 读写(performSearch 由 ActionListener 触发在 EDT,invokeLater 回调在 EDT)
3. **假设** 用户不会在 1ms 内连续触发两次搜索(令牌递增足够快)
4. **假设** 修复后诊断日志会显示所有 addResults/clear 都在 EDT 执行(若非 EDT,说明根因 B 成立,需进一步排查)

---

## 五、Verification(验证方案)

### 5.1 编译验证

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew compileKotlin
```

### 5.2 手动验证场景

| 场景 | 操作 | 预期 |
|------|------|------|
| 单次搜索 | 输入 'K' 回车 | 树正常显示结果,状态栏显示正确匹配数 |
| 快速连续搜索 | 输入 'K' → 立即输入 'a' | 树显示 'Ka' 的结果(非 'K' 残留),状态栏显示 'Ka' 匹配数 |
| 极速连续搜索 | 输入 'K' → 'a' → 't' (每个 < 100ms) | 树显示 'Kat' 的结果,无残留 |
| 取消后搜索 | 搜索 'K' → Esc → 搜索 'Kat' | 树显示 'Kat' 的结果 |
| 大结果集 | 搜索高频词(如 'val') | 树正常显示,到 50000 截断 |

### 5.3 日志验证

复现"快速连续搜索"后,检查 `build/idea-sandbox/IC-2023.1/log/idea.log`:

**预期日志(修复后)**:
```
INFO - RustSearchService - Search started: searchId=1, pattern='K'
INFO - RustSearchService - Emit batch: searchId=1, batchSize=50, sample=/path/to/file.kt
INFO - SearchResultTreeModel - addResults: batch=50, totalBefore=0, filesBefore=0, isEDT=true, thread=AWT-EventQueue-0
INFO - RustSearchService - Search session released: searchId=1
INFO - RustSearchService - Search started: searchId=2, pattern='Ka'
INFO - SearchResultTreeModel - clear: totalBefore=50, filesBefore=10, isEDT=true, thread=AWT-EventQueue-0
INFO - RustSearchService - Emit batch: searchId=2, batchSize=5, sample=/path/to/other.kt
INFO - SearchResultTreeModel - addResults: batch=5, totalBefore=0, filesBefore=0, isEDT=true, thread=AWT-EventQueue-0
INFO - RustSearchService - Search session released: searchId=2
```

**异常日志(若修复无效)**:
- `isEDT=false` → 根因 B 成立,需进一步排查 Dispatchers.Main
- `Discarding stale batch` 日志大量出现 → 令牌机制生效,但旧搜索 emit 过多
- `addResults` 在 `clear` 之前 → 令牌机制未生效(检查令牌校验逻辑)

### 5.4 残余风险

1. **令牌校验在 invokeLater 回调内**:若 invokeLater 回调延迟超过下一次 performSearch,旧 batch 仍可能在新 clear() 之前执行(但令牌已校验,不会污染)
2. **invokeLater 默认 modality**:若 IDE 处于模态对话框(如进度条),invokeLater 仍可能延迟;但令牌校验可兜底
3. **诊断日志开销**:每次 addResults/clear 多 1 条 INFO 日志,生产环境可接受(< 0.1ms/次)

---

## 六、实施顺序

| 顺序 | 改动 | 文件 | 验证 |
|------|------|------|------|
| 1 | 新增 activeSearchToken 字段 | RustSearchPanel.kt | 编译通过 |
| 2 | performSearch 生成新令牌 | RustSearchPanel.kt | 编译通过 |
| 3 | collect 块改 invokeLater + 令牌校验 | RustSearchPanel.kt | 编译通过 |
| 4 | addResults/clear 加诊断日志 | SearchResultTreeModel.kt | 编译通过 |
| 5 | emit 加诊断日志 | RustSearchService.kt | 编译通过 |
| 6 | 全量编译验证 | - | `./gradlew compileKotlin` 通过 |
| 7 | runIde 手动验证 | - | 快速连续搜索无残留 |

---

## 七、附录:文件改动清单

### Kotlin 侧(3 文件)

1. [src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)
   - 新增 `activeSearchToken` 字段
   - `performSearch` 生成新令牌
   - collect 块 3 处 `withContext(Dispatchers.Main)` → `invokeLater` + 令牌校验

2. [src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)
   - 新增 LOGGER 字段
   - `addResults` 入口加诊断日志
   - `clear` 入口加诊断日志

3. [src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt)
   - `emit` 前加诊断日志(2 处:循环内 + finalBatch)

**总计**:3 文件改动,无 Rust 侧改动,无资源文件改动。

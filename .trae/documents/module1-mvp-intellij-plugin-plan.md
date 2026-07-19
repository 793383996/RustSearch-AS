# 模块 1 MVP：IntelliJ 插件层搭建与 JNI 链路打通

> 阶段：里程碑 1（MVP 版本）
> 范围：修正 Rust JNI 接口 → 搭建 IntelliJ 插件工程 → 实现 Tool Window UI → 端到端验证
> 目标：跑通「IDE UI 触发搜索 → Rust 核心执行 → 流式返回结果展示」完整链路，支持中途取消

---

## 一、当前状态分析

### 1.1 已完成（Rust 核心层）

| 模块 | 文件 | 状态 |
|------|------|------|
| 配置层 | `rust-search/src/search/config.rs` | ✅ 完成（12 字段 + Builder + validate + 正则构建） |
| 搜索引擎 | `rust-search/src/search/engine.rs` | ✅ 完成（同步 `search()` + 流式 `search_stream()`） |
| 文件遍历 | `rust-search/src/search/walker.rs` | ✅ 完成（ignore 集成 + include/exclude globs） |
| 文本匹配 | `rust-search/src/search/matcher.rs` | ✅ 完成（字面量/正则/大小写/全字） |
| 上下文行 | `rust-search/src/search/context.rs` | ✅ MVP 完成（整文件读入） |
| JNI 入口 | `rust-search/src/jni/bridge.rs` | ⚠️ 有设计缺陷（见下） |
| 类型转换 | `rust-search/src/jni/convert.rs` | ✅ 完成 |
| 结果构建 | `rust-search/src/jni/result.rs` | ✅ 完成 |
| 错误处理 | `rust-search/src/error.rs` | ✅ 完成（7 变体） |
| 测试 | 35 单元 + 13 集成 | ✅ 通过 |
| 构建产物 | `target/release/librust_search.dylib` | ✅ 已生成（1.9MB） |

### 1.2 关键设计缺陷：JNI 接口无法支持取消

**问题**：`bridge.rs` 中 `search(...)` 返回 `Array<SearchResult>`（同步阻塞），不返回 `searchId`；但 `cancel(searchId: Long)` 需要 `searchId`。Kotlin 侧**无法拿到 searchId 来取消搜索**。

**影响**：违反 TODO.md 里程碑 1 明确要求的「支持搜索中途取消」。大项目搜索可能耗时 30 秒+，无法取消会导致 UI 卡死风险。

**根因**：当前 `bridge.rs` 的 `run_search` 内部生成了 `search_id` 并注册到 `CANCEL_REGISTRY`，但该 ID 未暴露给 JVM 调用方。

### 1.3 未完成（IntelliJ 插件层）

| 项目 | 状态 |
|------|------|
| Gradle 构建脚本（`build.gradle.kts` / `settings.gradle.kts`） | ❌ 不存在 |
| `plugin.xml` 扩展点声明 | ❌ 不存在 |
| Kotlin 源码目录（`src/main/kotlin/`） | ❌ 不存在 |
| `RustSearchEngine.kt`（JNI 调用入口） | ❌ 不存在 |
| `SearchResult.kt` / `SearchException.kt` | ❌ 不存在 |
| 搜索 Tool Window UI | ❌ 不存在 |
| Gradle 自动编译 Rust 任务 | ❌ 不存在 |
| 基准测试（`benches/`） | ❌ 不存在 |

---

## 二、 Proposed Changes

### Part A：修正 Rust JNI 接口为异步流式模式

#### A1. 修改 `rust-search/src/jni/bridge.rs`

**目标**：替换同步 `search(...)` 为异步流式接口，使 `cancel(searchId)` 可用。

**新增 JNI 入口函数**（替换原 `search`）：

```rust
// 1. 启动异步搜索，立即返回 searchId
//    后台线程执行 search_stream()，结果通过 channel 缓冲
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_startSearch(
    env: JNIEnv, _class: JClass,
    roots: JObjectArray, pattern: JString,
    is_regex: jboolean, case_sensitive: jboolean, whole_words: jboolean,
    include_globs: JObjectArray, exclude_globs: JObjectArray,
    context_lines: jint,
) -> jlong  // 返回 searchId，0 表示失败

// 2. 轮询获取一批结果（阻塞等待 timeoutMs 或拿到结果）
//    返回 SearchResult[]，空数组 + isComplete=true 表示搜索结束
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_pollResults(
    env: JNIEnv, _class: JClass,
    search_id: jlong, timeout_ms: jint,
) -> jobjectArray  // 返回 SearchResult[]

// 3. 检查搜索是否完成
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_isSearchComplete(
    env: JNIEnv, _class: JClass, search_id: jlong,
) -> jboolean

// 4. 取消搜索（保持不变）
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_cancel(
    env: JNIEnv, _class: JClass, search_id: jlong)

// 5. 释放搜索资源（必须调用，清理 channel + 注册表）
#[no_mangle]
pub extern "system" fn Java_com_example_rustsearch_RustSearchEngine_releaseSearch(
    env: JNIEnv, _class: JClass, search_id: jlong)
```

**新增全局状态**：`SEARCH_REGISTRY` 存储 `(SearchEngine, Receiver, is_complete)` 三元组

```rust
struct SearchSession {
    engine: SearchEngine,
    receiver: Receiver<SearchResult<SearchMatch>>,
    is_complete: AtomicBool,
}
static SEARCH_REGISTRY: Lazy<Mutex<HashMap<u64, SearchSession>>> = ...;
```

**实现要点**：
- `startSearch`：构建 config → 创建 engine → 调用 `search_stream()` 拿到 receiver → 存入 registry → 返回 searchId
- `pollResults`：从 registry 取 receiver → `recv_timeout(Duration::from_millis(timeout_ms))` → 收到 Ok 转 SearchResult[] → 收到 Err 标记完成并返回空数组
- `isSearchComplete`：读 registry 中 session 的 `is_complete` 标志
- `releaseSearch`：从 registry 移除 session（drop engine + receiver，释放资源）
- 所有入口保留 `catch_unwind` 包裹

**保留旧 `search` 函数**：暂不删除，标记 `#[deprecated]`，避免破坏现有测试。新增 `startSearch` 等函数后，旧函数可在里程碑 2 移除。

#### A2. 修改 `rust-search/src/jni/result.rs`

**新增** `build_search_result_batch`：从 `Vec<SearchMatch>` 批量构建 `SearchResult[]`（复用现有 `build_single_result` 逻辑）。

#### A3. 新增 `rust-search/tests/jni_stream_integration.rs`

**目标**：验证异步流式 JNI 接口的端到端正确性（通过 `JNIEnv::attach` 在 Rust 测试中模拟 JVM 调用，或仅测试 `SearchSession` 逻辑层）。

**测试用例**：
1. `startSearch` 返回非零 searchId
2. `pollResults` 能拿到匹配结果
3. `isSearchComplete` 在搜索完成后返回 true
4. `cancel` 后 `pollResults` 返回空且 `isSearchComplete` 为 true
5. `releaseSearch` 后 registry 中无残留

---

### Part B：搭建 IntelliJ 插件工程

#### B1. 创建 `settings.gradle.kts`

```kotlin
rootProject.name = "RustSearch-AS"
```

#### B2. 创建 `build.gradle.kts`

**关键配置**：
- `intellij-platform-gradle-plugin`（`org.jetbrains.intellij`）版本 1.17.0
- `kotlin("jvm")` 版本 1.9.22
- IntelliJ Platform：`android-studio` 2023.1（IC-231）
- Java/JVM target：17
- 插件描述：name=`RustSearch`，version=`0.1.0`

**依赖**：
- `org.jetbrains.kotlin:kotlin-stdlib-jdk8`
- `org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3`（搜索后台调度）
- `com.squareup.moshi:moshi:1.15.0`（可选，配置序列化）

**自定义任务**：
- `buildRust`：执行 `cargo build --release`（cwd=`rust-search/`）
- `copyNativeLib`：将 `rust-search/target/release/librust_search.dylib`（或 `.so`/`.dll`）拷贝到 `src/main/resources/native/`
- `patchPluginXml` 依赖 `buildRust` + `copyNativeLib`

#### B3. 创建 `gradle.properties`

```properties
pluginGroup=com.example.rustsearch
pluginName=RustSearch
pluginVersion=0.1.0
pluginSinceBuild=231
pluginUntilBuild=241.*
platformType=IC
platformVersion=2023.1
platformPlugins=
javaVersion=17
```

#### B4. 创建 `src/main/resources/META-INF/plugin.xml`

**扩展点声明**：
- `<application-configurable>`：设置面板入口
- `<tool-window>`：注册搜索工具窗口（id=`RustSearch`，anchor=`left`，icon=搜索图标）
- `<action>`：注册 `RustSearchAction`（快捷键 `Ctrl+Shift+Alt+F` / Mac `Cmd+Shift+Alt+F`）
- `<applicationService>`：注册 `RustSearchService`（管理 native 库加载与搜索会话）

#### B5. 创建 `src/main/resources/native/` 目录

存放编译后的动态库（由 `copyNativeLib` 任务自动填充）。运行时通过 `RustSearchService` 提取到临时目录并 `System.load`。

---

### Part C：实现 Kotlin 侧 JNI 调用代码

#### C1. `src/main/kotlin/com/example/rustsearch/native/RustSearchEngine.kt`

**职责**：JNI native 声明 + 动态库加载

```kotlin
package com.example.rustsearch.native

object RustSearchEngine {
    init { RustSearchService.loadNativeLibrary() }

    @JvmStatic external fun startSearch(
        roots: Array<String>, pattern: String,
        isRegex: Boolean, caseSensitive: Boolean, wholeWords: Boolean,
        includeGlobs: Array<String>, excludeGlobs: Array<String>,
        contextLines: Int
    ): Long

    @JvmStatic external fun pollResults(searchId: Long, timeoutMs: Int): Array<SearchResult>
    @JvmStatic external fun isSearchComplete(searchId: Long): Boolean
    @JvmStatic external fun cancel(searchId: Long)
    @JvmStatic external fun releaseSearch(searchId: Long)
}
```

> 注意：包名 `com.example.rustsearch.native` 会导致 `native` 关键字冲突，实际使用 `com.example.rustsearch.jni` 或 `com.example.rustsearch.core`。Rust 侧 JNI 函数名需对应调整（如 `Java_com_example_rustsearch_core_RustSearchEngine_startSearch`）。

#### C2. `src/main/kotlin/com/example/rustsearch/model/SearchResult.kt`

```kotlin
package com.example.rustsearch.model

data class SearchResult(
    val filePath: String,
    val lineNumber: Int,
    val column: Int,
    val matchedText: String,
    val contextBefore: Array<String>,
    val contextAfter: Array<String>
) {
    // data class 中 Array 需手动实现 equals/hashCode
    override fun equals(other: Any?): Boolean { ... }
    override fun hashCode(): Int { ... }
}
```

**构造函数签名必须与 Rust `result.rs` 中 `build_single_result` 一致**：`(Ljava/lang/String;IILjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V`

#### C3. `src/main/kotlin/com/example/rustsearch/model/SearchException.kt`

```kotlin
package com.example.rustsearch.model

class SearchException(message: String) : RuntimeException(message)
```

对应 Rust `convert.rs` 中 `throw_java_exception` 抛出的 `com/example/rustsearch/model/SearchException`。

#### C4. `src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt`

**职责**：
- 动态库加载（从插件资源提取到临时目录，`System.load`）
- 搜索会话管理（封装 `startSearch` + `pollResults` + `releaseSearch` 生命周期）
- 提供 `Flow<List<SearchMatch>>` 流式 API 供 UI 层消费

```kotlin
@Service
class RustSearchService : Disposable {
    fun search(config: SearchConfig): Flow<List<SearchMatch>> = flow {
        val searchId = RustSearchEngine.startSearch(...)
        try {
            while (!RustSearchEngine.isSearchComplete(searchId)) {
                val batch = RustSearchEngine.pollResults(searchId, 100)
                if (batch.isNotEmpty()) emit(batch.toList())
            }
        } finally {
            RustSearchEngine.releaseSearch(searchId)
        }
    }
}
```

#### C5. 包名调整决策

由于 Rust 侧 `bridge.rs` 当前函数名前缀为 `Java_com_example_rustsearch_RustSearchEngine_*`，Kotlin 侧 `RustSearchEngine` 必须放在 `com.example.rustsearch` 包下（不能加子包）。否则需同步修改 Rust 侧函数名并重新编译。

**决策**：Kotlin 侧 `RustSearchEngine.kt` 直接放 `com.example.rustsearch` 包（根包），其余类按功能分子包。避免修改 Rust 侧已编译的 JNI 函数名。

---

### Part D：实现搜索 Tool Window UI

#### D1. `src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt`

**职责**：实现 `ToolWindowFactory`，创建搜索面板

```kotlin
class RustSearchToolWindowFactory : ToolWindowFactory {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val panel = RustSearchPanel(project)
        val content = ContentFactory.getInstance().createContent(panel, "Search", false)
        toolWindow.contentManager.addContent(content)
    }
}
```

#### D2. `src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt`

**UI 布局**（基于 `JBPanel` + `BorderLayout`）：

```
┌─────────────────────────────────────────────┐
│ [搜索模式输入框] [正则?] [大小写?] [全字?] │  ← 顶部搜索栏
│ [包含通配符] [排除通配符] [搜索按钮] [取消] │
├─────────────────────────────────────────────┤
│ 找到 N 个匹配，耗时 T 秒                    │  ← 状态栏
├─────────────────────────────────────────────┤
│ ▼ 文件路径1.kt (3 matches)                 │
│   12:  val x = "matched"                   │  ← 结果树（JBTree）
│   45:  fun matched() {}                    │
│ ▼ 文件路径2.java (1 match)                 │
│   8:   String s = "matched";               │
└─────────────────────────────────────────────┘
```

**关键组件**：
- `SearchTextField`：搜索模式输入
- `JBCheckBox`：正则/大小写/全字开关
- `JBTextField`：include/exclude globs
- `JBButton`：「搜索」/「取消」
- `Tree`：结果树，按文件分组
- `StatusBar`：匹配数与耗时

**交互逻辑**：
- 点击「搜索」：启动 Coroutine 调用 `RustSearchService.search()`，收集 Flow 更新树
- 点击「取消」：调用 `RustSearchEngine.cancel(searchId)`
- 双击树节点：通过 `FileEditorManager.openFile` 跳转到对应文件行

#### D3. `src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt`

**职责**：树模型，按文件分组展示匹配结果

- 根节点：隐藏
- 一级子节点：文件路径 + 匹配数
- 二级子节点：`行号:列 | 匹配内容` + 上下文预览

#### D4. `src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt`

**职责**：注册 Action，快捷键触发后打开 Tool Window

```kotlin
class RustSearchAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        ToolWindowManager.getInstance(project)
            .getToolWindow("RustSearch")?.show()
    }
}
```

---

### Part E：自动化构建与原生库打包

#### E1. `build.gradle.kts` 中的 Rust 编译任务

```kotlin
val buildRust by tasks.registering(Exec::class) {
    workingDir = file("rust-search")
    commandLine("cargo", "build", "--release")
    // 仅在 rust-search/src 变更时重建
    inputs.dir(file("rust-search/src"))
    inputs.file(file("rust-search/Cargo.toml"))
    outputs.dir(file("rust-search/target/release"))
}

val copyNativeLib by tasks.registering(Copy::class) {
    dependsOn(buildRust)
    from("rust-search/target/release") {
        include("librust_search.dylib", "librust_search.so", "rust_search.dll")
    }
    into("src/main/resources/native")
}

tasks.prepareTestingSandbox { dependsOn(copyNativeLib) }
tasks.buildPlugin { dependsOn(copyNativeLib) }
```

#### E2. 动态库运行时加载策略

`RustSearchService.loadNativeLibrary()`：
1. 从插件 classpath 读取 `/native/librust_search.dylib`（按 OS 选择扩展名）
2. 拷贝到 `${System.getProperty("java.io.tmpdir")}/rustsearch/` 临时目录
3. `System.load(临时路径)` 加载

**原因**：IntelliJ 插件 jar 内的 .so/.dylib 不能直接 `System.load`，必须先释放到文件系统。

---

## 三、文件清单与执行顺序

### 阶段 1：修正 Rust JNI 接口（Part A）

| 序号 | 操作 | 文件路径 |
|------|------|----------|
| 1 | 修改 | `rust-search/src/jni/bridge.rs`（新增 5 个异步 JNI 函数 + SEARCH_REGISTRY） |
| 2 | 修改 | `rust-search/src/jni/result.rs`（新增 build_search_result_batch） |
| 3 | 修改 | `rust-search/src/jni/mod.rs`（导出新函数） |
| 4 | 新增 | `rust-search/tests/jni_stream_integration.rs` |
| 5 | 验证 | `cd rust-search && cargo build --release && cargo test` |

### 阶段 2：搭建插件工程骨架（Part B + E）

| 序号 | 操作 | 文件路径 |
|------|------|----------|
| 6 | 新增 | `settings.gradle.kts` |
| 7 | 新增 | `build.gradle.kts`（含 Rust 编译任务） |
| 8 | 新增 | `gradle.properties` |
| 9 | 新增 | `src/main/resources/META-INF/plugin.xml` |
| 10 | 新增 | `src/main/resources/native/.gitkeep`（占位） |
| 11 | 验证 | `./gradlew buildPlugin`（确认插件可打包） |

### 阶段 3：实现 JNI 调用层（Part C）

| 序号 | 操作 | 文件路径 |
|------|------|----------|
| 12 | 新增 | `src/main/kotlin/com/example/rustsearch/RustSearchEngine.kt` |
| 13 | 新增 | `src/main/kotlin/com/example/rustsearch/model/SearchResult.kt` |
| 14 | 新增 | `src/main/kotlin/com/example/rustsearch/model/SearchException.kt` |
| 15 | 新增 | `src/main/kotlin/com/example/rustsearch/model/SearchConfig.kt`（Kotlin 侧配置） |
| 16 | 新增 | `src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt` |
| 17 | 验证 | `./gradlew runIde`（启动 IDE 实例，确认 native 库加载成功无异常） |

### 阶段 4：实现 Tool Window UI（Part D）

| 序号 | 操作 | 文件路径 |
|------|------|----------|
| 18 | 新增 | `src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt` |
| 19 | 新增 | `src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt` |
| 20 | 新增 | `src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt` |
| 21 | 新增 | `src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt` |
| 22 | 验证 | `./gradlew runIde`，打开 Tool Window，执行搜索 |

### 阶段 5：端到端验证

| 序号 | 验证项 | 方法 |
|------|--------|------|
| 23 | 基础搜索 | 在 runIde 实例中搜索当前项目 "SearchEngine"，确认结果正确 |
| 24 | 中途取消 | 大项目搜索启动后立即点「取消」，确认 UI 响应且无崩溃 |
| 25 | 中文搜索 | 搜索中文关键词，确认编码正确 |
| 26 | 正则搜索 | 搜索 `print\w+`，确认正则生效 |
| 27 | 文件跳转 | 双击结果树节点，确认跳转到对应文件行 |
| 28 | 内存泄漏 | 连续执行 10 次搜索，监控 JVM 内存无持续上涨 |

---

## 四、Assumptions & Decisions

### 4.1 关键决策

| 决策 | 选项 | 理由 |
|------|------|------|
| MVP 集成路径 | 方案 B：独立 Tool Window | TODO.md 推荐，开发周期短，不依赖 IDE 内部 API |
| JNI 接口模式 | 异步流式（startSearch + pollResults） | 修正当前 cancel 不可用缺陷；engine.rs 已有 search_stream() 能力 |
| 目标平台 | Android Studio 2023.1+（IC-231） | 与 TODO.md 一致，覆盖主流用户 |
| Rust 编译集成 | Gradle Exec 任务自动 cargo build | 开发体验好，避免手动编译 |
| Kotlin 包名 | `com.example.rustsearch`（根包放 RustSearchEngine） | 避免修改 Rust 侧已编译的 JNI 函数名前缀 |
| JVM 版本 | 17 | IntelliJ 2023.1 要求 |
| Kotlin 版本 | 1.9.22 | 与 IntelliJ Platform 2023.1 兼容 |

### 4.2 假设

1. **Rust 工具链已安装**：`cargo` 命令在 PATH 中可用（aarch64-apple-darwin）
2. **Android Studio 2023.1+ 已安装**：Gradle 插件需要本地 IDE 用于 runIde 任务
3. **当前 Rust 核心测试全部通过**：基于探索报告（35 单元 + 13 集成通过）
4. **column=0 限制可接受**：MVP 阶段不要求精确列号，里程碑 2 再通过 `matcher.find()` 优化
5. **macOS aarch64 优先**：当前仅构建 macOS Apple Silicon 动态库，Linux/Windows 在里程碑 2 补齐

### 4.3 风险与规避

| 风险 | 影响 | 规避 |
|------|------|------|
| JNI 函数名与 Kotlin 包名不匹配 | native 库加载后调用失败 | 严格对齐：`Java_com_example_rustsearch_RustSearchEngine_*` |
| 动态库无法从 jar 内直接加载 | `UnsatisfiedLinkError` | 运行时释放到 tmpdir 再 System.load |
| searchId 泄漏（未调用 releaseSearch） | Rust 侧 registry 内存泄漏 | Kotlin 侧用 `try/finally` + Coroutine `use` 模式保证释放 |
| pollResults 阻塞 JVM 线程 | UI 卡顿 | timeoutMs 上限 500ms，Kotlin 侧在 IO Dispatcher 调用 |
| IntelliJ Platform API 版本差异 | 编译错误 | 仅使用 2023.1 公开稳定 API |

---

## 五、Verification Steps

### 5.1 Rust 侧验证（阶段 1 完成后）

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search
cargo build --release 2>&1 | tail -5
cargo test 2>&1 | tail -20
cargo test --test jni_stream_integration 2>&1 | tail -10
```

**预期**：
- release 构建产出 `target/release/librust_search.dylib`
- 全部单元测试通过
- 新增 jni_stream_integration 测试通过

### 5.2 插件工程验证（阶段 2-3 完成后）

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew buildPlugin
ls build/distributions/
```

**预期**：
- 产出 `build/distributions/RustSearch-0.1.0.zip`
- zip 内含 `lib/RustSearch-0.1.0.jar` + `native/librust_search.dylib`

### 5.3 端到端功能验证（阶段 4 完成后）

```bash
./gradlew runIde
```

在启动的 IDE 实例中：
1. 打开任意项目
2. `Cmd+Shift+A` → 搜索 "RustSearch" → 打开 Tool Window
3. 输入 "SearchEngine" → 点击搜索 → 确认结果树展示匹配
4. 大项目搜索启动后点「取消」→ 确认 UI 响应
5. 双击结果节点 → 确认跳转到对应文件行

### 5.4 性能验证（可选，里程碑 0 遗留）

```bash
# 在 rust-search 下新增 benches/search_bench.rs（criterion）
cd rust-search
cargo bench --bench search_bench
```

**预期指标**（5 万文件项目）：
- 字面量搜索 ≤ 2 秒
- 正则搜索 ≤ 5 秒
- 内存占用 ≤ 100MB

---

## 六、里程碑 1 完成标准

- [ ] Rust 异步流式 JNI 接口实现完成，所有测试通过
- [ ] IntelliJ 插件工程可 `./gradlew buildPlugin` 打包
- [ ] runIde 实例中 Tool Window 可打开
- [ ] 搜索功能可用，结果正确展示
- [ ] 中途取消功能可用
- [ ] 双击结果可跳转到文件
- [ ] 连续 10 次搜索无内存泄漏
- [ ] macOS Apple Silicon 平台验证通过

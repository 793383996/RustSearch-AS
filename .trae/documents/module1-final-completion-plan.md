# 模块 1 最终收尾计划:修复编译类路径 + 完成构建 + 端到端验证

> 阶段：里程碑 1（MVP 版本）最终收尾
> 范围：诊断 Kotlin 编译错误根因 → 修复 IntelliJ Platform 2.0 编译类路径 → `buildPlugin` 产出 zip → `runIde` 端到端验证
> 目标：达成里程碑 1 完成标准，跑通「IDE UI 触发搜索 → Rust 核心执行 → 流式返回结果展示 → 双击跳转」完整闭环
> 前置文档（不重复其内容）：
> - `module1-global-text-search-rust-architecture.md`（Rust 架构方案，已落地）
> - `module1-mvp-intellij-plugin-plan.md`（插件开发计划，已落地）
> - `module1-build-and-verification-plan.md`（前期构建计划，阶段 A/B 已完成，阶段 B2 进行中）

---

## 一、摘要(Summary)

本计划聚焦模块 1 当前唯一阻塞点：`buildPlugin` 构建失败于 Kotlin 编译错误。代码层已 95% 就绪，Rust 核心 62 个测试通过，动态库已编译完成。本计划通过 **诊断 → 修复 → 验证** 三步走打通里程碑 1 完成标准。

**核心诊断假设**：当前 Kotlin 编译错误（`Unresolved reference 'Tree'`、`Unresolved reference 'loadNativeLibrary'`）表明 IntelliJ Platform 类与项目内 Kotlin 类未进入 `compileClasspath`。根因可能是 IntelliJ Platform Gradle Plugin 2.0 的 `local()` 依赖声明需要额外配置（如 `pluginLibrary()` 或 `intellijIdeaCommunity()` 显式声明），或本地 Android Studio（AI-261）的 platform jar 未被正确解压到依赖图。

---

## 二、当前状态分析(Current State Analysis)

### 2.1 已完成（无需再改动）

| 层级 | 模块 | 文件 | 状态 |
|------|------|------|------|
| Rust 核心 | 异步流式 JNI 接口 | `rust-search/src/jni/bridge.rs` | ✅ 5 个 JNI 函数 + SearchSession + SEARCH_REGISTRY |
| Rust 核心 | 流式集成测试 | `rust-search/tests/jni_stream_integration.rs` | ✅ 11 个测试全通过 |
| Rust 核心 | release 构建产物 | `rust-search/target/release/librust_search.dylib` | ✅ 2,009,536 字节 |
| Rust 核心 | 单元/集成测试 | 62 个测试 | ✅ 全通过 |
| Kotlin JNI | JNI 入口 | `src/main/kotlin/.../RustSearchEngine.kt` | ✅ 5 external 函数 + SearchResult 内部类 |
| Kotlin JNI | 配置/异常 | `SearchConfig.kt` / `SearchException.kt` | ✅ |
| Kotlin 服务 | 服务层 | `service/RustSearchService.kt` | ✅ Flow + 动态库加载 |
| Tool Window | 工厂 + Action | `RustSearchToolWindowFactory.kt` / `RustSearchAction.kt` | ✅ |
| Tool Window | 搜索面板 | `RustSearchPanel.kt` | ✅ 搜索栏 + 结果树 + 状态栏 |
| Tool Window | 树模型 + 渲染器 | `SearchResultTreeModel.kt` | ✅ 含 `SearchResultTreeCellRenderer` |
| 配置 | plugin.xml | `src/main/resources/META-INF/plugin.xml` | ✅ toolWindow + applicationService + action |
| 配置 | Gradle Wrapper 8.11.1 | `gradlew*` / `gradle/wrapper/*` | ✅ |
| 构建产物 | 动态库已拷贝 | `build/resources/main/native/librust_search.dylib` | ✅（上次构建部分成功） |

### 2.2 当前阻塞点（本计划要解决）

**最近一次构建（`/tmp/buildplugin-17.log`）的 Kotlin 编译错误**：

```
e: RustSearchEngine.kt:22:27 Unresolved reference 'loadNativeLibrary'.
e: RustSearchPanel.kt:14:29 Unresolved reference 'Tree'.
e: RustSearchPanel.kt:114:30 Unresolved reference 'Tree'.
e: RustSearchPanel.kt:114:46 Cannot infer type for type parameter 'T'.
e: RustSearchPanel.kt:115:9 Unresolved reference 'isRootVisible'.
e: RustSearchPanel.kt:116:9 Unresolved reference 'showsRootHandles'.
e: RustSearchPanel.kt:117:9 Unresolved reference 'selectionModel'.
e: RustSearchPanel.kt:118:9 Unresolved reference 'setCellRenderer'.
e: RustSearchPanel.kt:199:20 Unresolved reference 'addMouseListener'.
e: RustSearchPanel.kt:304:31 Unresolved reference 'lastSelectedPathComponent'.
```

**错误分类**：

| 类别 | 受影响引用 | 根因推断 |
|------|-----------|----------|
| IntelliJ Platform 类未解析 | `com.intellij.ui.tree.Tree`、`com.intellij.icons.AllIcons`、`com.intellij.ui.JBColor`、`com.intellij.util.ui.UIUtil`、`com.intellij.openapi.*` | platform jar 未在 `compileClasspath` |
| 项目内类未解析 | `RustSearchService.loadNativeLibrary()` | Kotlin 编译单元间依赖断裂（通常因前一错误级联触发） |
| Swing 方法未解析 | `isRootVisible`、`showsRootHandles`、`selectionModel`、`setCellRenderer`、`addMouseListener`、`lastSelectedPathComponent` | `Tree` 类型未解析导致其继承的 `JTree` 方法也不可用 |

### 2.3 已完成的构建配置迁移

| # | 配置项 | 原值 | 现值 | 状态 |
|---|--------|------|------|------|
| 1 | IntelliJ Platform Gradle Plugin | `org.jetbrains.intellij` 1.17.3 | `org.jetbrains.intellij.platform` 2.0.1 | ✅ |
| 2 | Kotlin 版本 | 1.9.22 | 2.2.20 | ✅ |
| 3 | `kotlinOptions` DSL | `kotlinOptions { jvmTarget = "17" }` | `compilerOptions { jvmTarget.set(JvmTarget.JVM_21) }` | ✅ |
| 4 | Java sourceCompatibility | 17 | 21 | ✅ |
| 5 | 本地 IDE 依赖语法 | `intellij { localPath.set(...) }` | `dependencies { intellijPlatform { local("/Applications/Android Studio.app") } }` | ✅ |
| 6 | 本地 Ivy 仓库 | 无 | `repositories { intellijPlatform { localPlatformArtifacts() } }` | ✅ |
| 7 | RepositoriesMode | `PREFER_SETTINGS` | `PREFER_PROJECT` | ✅ |
| 8 | SSL 配置 | `KeychainStore`（破坏 cacerts） | 已移除，使用阿里云镜像 | ✅ |
| 9 | `processResources` 隐式依赖 | 未声明 | `dependsOn(copyNativeLib)` | ✅ |

### 2.4 关键约束

- **本地 `gradle` 命令未安装**：必须使用缓存二进制 `~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle`
- **JDK 21**：`JAVA_HOME=/Applications/Android Studio.app/Contents/jbr/Contents/Home`（AS 261 自带 JBR 21）
- **本地 Android Studio 版本**：AI-261.23567.138.2611.15646644（2026.1，对应 IntelliJ 261）
- **plugin.xml 兼容范围**：`since-build="231" until-build="261.*"`（与本地 AS 261 兼容）
- **Rust 动态库已就绪**：`rust-search/target/release/librust_search.dylib`（arm64）
- **架构一致性**：macOS Apple Silicon（aarch64），dylib 与 JVM 架构一致

---

## 三、Proposed Changes

### Part A：诊断编译类路径（只读，确认根因）

**目标**：在修改任何配置前，先用只读命令确认 IntelliJ Platform jar 是否真的缺失于 `compileClasspath`，避免盲目修改。

#### A1. 检查 `compileClasspath` 是否包含 platform jar

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
GRADLE=~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle
"$GRADLE" dependencies --configuration compileClasspath --no-daemon 2>&1 | tee /tmp/deps-compile.log | grep -E "intellij|idea|platform|localIde" | head -30
```

**预期判定**：

| 输出 | 判定 | 下一步 |
|------|------|--------|
| 含 `localIde:AI:261...` 或 `intellij-platform-...` | 类路径配置正确，错误另有根因 | 转 A2 |
| 无任何 IntelliJ 相关依赖 | 确认 `local()` 未生效 | 转 Part B 方案 1 |
| 报 `Could not find localIde:AI:261...` | `localPlatformArtifacts()` 仓库未生效 | 转 Part B 方案 2 |

#### A2. 检查 Kotlin 编译任务的类路径输入

```bash
"$GRADLE" compileKotlin --info --no-daemon 2>&1 | tee /tmp/compile-info.log | grep -E "classpath|Classpath|platform|idea" | head -40
```

**关注点**：编译任务的 `classpath` 输入中是否包含 `app.jar`、`platform-impl.jar`、`util.jar` 等 IntelliJ 平台核心 jar。

#### A3. 检查 IntelliJ Platform 2.0 的依赖配置文档

通过 `WebSearch` 查询 `IntelliJ Platform Gradle Plugin 2.0 local dependency compileClasspath`，确认 `local()` 语法是否需要额外配置（如 `pluginLibrary()`、`intellijIdeaCommunity()`、`creationType` 等）。

**关键文档**：
- https://plugins.jetbrains.com/docs/intellij/tools-intellij-platform-gradle-plugin-dependencies-extension.html
- https://github.com/JetBrains/intellij-platform-gradle-plugin

---

### Part B：修复编译类路径（核心修复）

**目标**：让 IntelliJ Platform jar 进入 `compileClasspath`，使 `com.intellij.ui.tree.Tree` 等类可解析。

#### 方案优先级（按代价由小到大，依次尝试）

---

#### B-方案 1：补充 `intellijPlatform { pluginLibrary() }` 依赖声明

**依据**：IntelliJ Platform Gradle Plugin 2.0 的 `dependencies { intellijPlatform { ... } }` 块可能需要显式声明 `pluginLibrary()` 来触发平台 jar 的依赖注入。`local()` 仅注册本地 IDE 路径，但不自动添加为编译依赖。

**修改文件**：`/Users/apple/AndroidStudioProjects/RustSearch-AS/build.gradle.kts`

**修改位置**：`dependencies { intellijPlatform { ... } }` 块（第 33-35 行）

**修改前**：
```kotlin
intellijPlatform {
    local("/Applications/Android Studio.app")
}
```

**修改后**：
```kotlin
intellijPlatform {
    local("/Applications/Android Studio.app")
    // 显式声明插件运行所需的 IntelliJ Platform 库依赖
    // local() 仅注册 IDE 路径,pluginLibrary() 触发平台 jar 注入到 compileClasspath
    pluginLibrary()
}
```

**验证**：
```bash
"$GRADLE" dependencies --configuration compileClasspath --no-daemon 2>&1 | grep -E "intellij|idea|localIde" | head -10
"$GRADLE" compileKotlin --no-daemon 2>&1 | tee /tmp/compile-2.log | tail -20
```

**判定**：若 `compileKotlin` 不再报 `Unresolved reference 'Tree'`，方案 1 成功，跳到 Part C。否则转方案 2。

---

#### B-方案 2：切换到远程 `intellijIdeaCommunity("2023.1")` 依赖

**依据**：若 `local()` + `pluginLibrary()` 仍无法解析本地 AI-261 的平台类，可能是 AI-261（2026.1 预览版）的 jar 结构与 IntelliJ Platform 2.0 插件不兼容。切换到远程下载的稳定版 IC-2023.1（与 `plugin.xml` 的 `since-build=231` 严格对齐），通过阿里云镜像规避 SSL 问题。

**修改文件**：`/Users/apple/AndroidStudioProjects/RustSearch-AS/build.gradle.kts`

**修改位置**：
1. `repositories { intellijPlatform { ... } }` 块（第 18-23 行）—— 添加 `releases()` 已有，确认 `marketplace()` 可选
2. `dependencies { intellijPlatform { ... } }` 块（第 33-35 行）

**修改前**：
```kotlin
intellijPlatform {
    local("/Applications/Android Studio.app")
}
```

**修改后**：
```kotlin
intellijPlatform {
    // 切换到远程 IC-2023.1,与 plugin.xml since-build=231 严格对齐
    // 阿里云镜像已在 settings.gradle.kts 配置,规避 SSL 问题
    intellijIdeaCommunity("2023.1")
    // 本地 AS 仍可用于 runIde（通过 local() 在 intellijPlatform {} 扩展块中声明）
}
```

**同时修改 `gradle.properties`**：
```properties
# 与远程依赖对齐
platformType=IC
platformVersion=2023.1
```

（注：`gradle.properties` 当前已是此值，无需改动。）

**验证**：
```bash
# 先验证依赖可下载（阿里云镜像走 HTTPS，默认 cacerts 可访问）
"$GRADLE" dependencies --configuration compileClasspath --no-daemon 2>&1 | tee /tmp/deps-3.log | grep -E "ideaIC|intellij" | head -10
"$GRADLE" compileKotlin --no-daemon 2>&1 | tee /tmp/compile-3.log | tail -20
```

**判定**：若 `compileKotlin` 成功（`BUILD SUCCESSFUL`），方案 2 成功，跳到 Part C。若 SSL 下载失败，转方案 3。

---

#### B-方案 3：使用本地 IntelliJ 2023.1 Community 备用 IDE

**依据**：若远程下载因 SSL 持续失败，且本地 AI-261 不兼容，可下载 IntelliJ IDEA Community 2023.1 独立安装包到本地，用 `local()` 指向它。

**前提**：手动下载并安装 IntelliJ IDEA Community 2023.1 到 `/Applications/IntelliJ IDEA Community.app`（非 Android Studio）。

**修改文件**：`build.gradle.kts`

**修改后**：
```kotlin
intellijPlatform {
    local("/Applications/IntelliJ IDEA Community.app")
    pluginLibrary()
}
```

**判定**：此方案为最终兜底，仅在方案 1、2 均失败时启用。

---

#### B-备选：若 `loadNativeLibrary` 错误单独存在

若 Part B 修复后 `Tree` 等错误消失但 `RustSearchEngine.kt:22 Unresolved reference 'loadNativeLibrary'` 仍存在，说明是项目内 Kotlin 类间依赖问题。

**根因**：`RustSearchEngine` 是 `object`，其 `init { RustSearchService.loadNativeLibrary() }` 中 `RustSearchService` 来自 `service` 包。检查 import 是否正确。

**修改文件**：`/Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/RustSearchEngine.kt`

**当前第 3 行**：`import com.example.rustsearch.service.RustSearchService`

**验证**：`RustSearchService.kt` 第 65 行 `fun loadNativeLibrary()` 是 `@Synchronized` 公开方法，应可访问。若仍报错，检查 `RustSearchService` 是否被 `applicationService` 注册导致 Kotlin 编译器无法解析（不太可能，因为 plugin.xml 不影响编译）。

**最可能情况**：此错误是 `Tree` 错误的级联效应，Part B 修复后会自动消失。

---

### Part C：验证 `buildPlugin` 产出 zip

**目标**：确认 `build/distributions/RustSearch-0.1.0.zip` 成功生成且包含必要文件。

#### C1. 执行完整构建

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
GRADLE=~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle
"$GRADLE" buildPlugin --no-daemon 2>&1 | tee /tmp/buildplugin-final.log | tail -30
```

**预期输出**：`BUILD SUCCESSFUL`

#### C2. 验证 zip 产出

```bash
ls -la /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/
# 预期: RustSearch-0.1.0.zip 存在,大小约 2~3 MB

unzip -l /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip | grep -E "(lib/|native/)"
# 预期输出:
# lib/RustSearch-0.1.0.jar
# native/librust_search.dylib
```

#### C3. 解压验证内容

```bash
cd /tmp && rm -rf rustsearch-verify && mkdir rustsearch-verify && cd rustsearch-verify
unzip -q /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip
find . -type f | sort
```

**预期文件清单**：
- `RustSearch/lib/RustSearch-0.1.0.jar`
- `RustSearch/native/librust_search.dylib`

#### C4. 验证动态库架构

```bash
file /tmp/rustsearch-verify/RustSearch/native/librust_search.dylib
# 预期: Mach-O 64-bit dynamically linked shared library arm64
```

#### C5. 验证 jar 内 Kotlin 类

```bash
unzip -l /tmp/rustsearch-verify/RustSearch/lib/RustSearch-0.1.0.jar | grep "\.class"
# 预期包含:
# com/example/rustsearch/RustSearchEngine.class
# com/example/rustsearch/RustSearchEngine$SearchResult.class
# com/example/rustsearch/service/RustSearchService.class
# com/example/rustsearch/ui/RustSearchPanel.class
# com/example/rustsearch/ui/SearchResultTreeModel.class
# com/example/rustsearch/ui/SearchResultTreeModel$SearchResultTreeCellRenderer.class
# com/example/rustsearch/ui/RustSearchToolWindowFactory.class
# com/example/rustsearch/action/RustSearchAction.class
```

---

### Part D：`runIde` 端到端验证

**目标**：在 IDE 沙箱实例中验证完整搜索链路，达成里程碑 1 完成标准。

#### D1. 启动 runIde 实例

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
GRADLE=~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle
"$GRADLE" runIde --no-daemon 2>&1 | tee /tmp/runide-final.log | tail -20
```

**预期**：启动一个新的 IntelliJ/Android Studio 实例（sandbox），左侧出现 RustSearch 工具窗口图标。

**启动失败排查表**：

| 问题 | 处理 |
|------|------|
| `UnsatisfiedLinkError: ...librust_search.dylib: dlopen(...)` | `file` 命令确认 dylib 架构；`xattr -d com.apple.quarantine` 移除隔离属性 |
| `UnsatisfiedLinkError: can't find class RustSearchEngine` | 检查 `RustSearchEngine` 是否在 `com.example.rustsearch` 根包 |
| Tool Window 不显示 | `View → Tool Windows → RustSearch` 手动激活 |
| IDE 启动 OOM | `gradle.properties` 调到 `-Xmx4g` |
| `Plugin was compiled with an incompatible version` | 检查 `plugin.xml` 的 `since-build`/`until-build` 与运行 IDE 版本匹配 |

#### D2. 功能验证清单（11 项）

由用户在启动的 IDE 中手动验证：

| # | 验证项 | 操作步骤 | 预期结果 | 通过? |
|---|--------|----------|----------|-------|
| 1 | Tool Window 可打开 | 点击左侧 RustSearch 图标，或 `Cmd+Shift+Alt+F` | 工具窗口展开，显示搜索面板 | ☐ |
| 2 | native 库加载成功 | `Help → Show Log in Finder` 查看 `idea.log` | 输出 "Rust 动态库加载成功: /var/folders/.../librust_search.dylib" | ☐ |
| 3 | 基础字面量搜索 | 输入 `SearchEngine` → 点搜索 | 结果树展示匹配文件与行 | ☐ |
| 4 | 流式结果展示 | 大项目搜索（如 Android Studio 源码目录） | 结果逐步出现，非一次性返回 | ☐ |
| 5 | 正则搜索 | 勾选「正则」→ 输入 `print\w+` → 搜索 | 正则匹配生效 | ☐ |
| 6 | 大小写敏感 | 勾选「大小写」→ 搜索 `hello` | 仅匹配 `hello`，不匹配 `Hello` | ☐ |
| 7 | 中文搜索 | 输入中文关键词（如「搜索」） | 编码正确，结果正常 | ☐ |
| 8 | 文件过滤 | 在「包含」输入 `*.kt` → 搜索 | 仅搜索 .kt 文件 | ☐ |
| 9 | 中途取消 | 大项目搜索启动后立即点「取消」 | UI 响应，搜索停止，状态栏显示「已取消」 | ☐ |
| 10 | 双击跳转 | 双击结果树匹配节点 | 打开对应文件 | ☐ |
| 11 | 状态栏信息 | 搜索完成后查看状态栏 | 显示「N 个匹配(M 个文件)，耗时 T s」 | ☐ |

#### D3. 稳定性验证（3 项）

| # | 验证项 | 操作步骤 | 预期结果 | 通过? |
|---|--------|----------|----------|-------|
| 12 | 内存泄漏检查 | 连续执行 10 次相同搜索，Activity Monitor 监控 IDE 进程内存 | JVM 内存无持续上涨（允许波动 ±50MB） | ☐ |
| 13 | 会话释放验证 | 搜索完成后查看 `idea.log` | 输出 "搜索会话已释放: searchId=..." | ☐ |
| 14 | 异常恢复 | 输入非法正则 `(` → 搜索 | 状态栏显示错误，不崩溃，UI 可继续使用 | ☐ |

---

## 四、执行顺序与文件清单

### 阶段 A：诊断（只读）

| 序号 | 操作 | 命令 | 说明 |
|------|------|------|------|
| 1 | 执行 | `"$GRADLE" dependencies --configuration compileClasspath` | 检查 platform jar 是否在类路径 |
| 2 | 执行 | `"$GRADLE" compileKotlin --info` | 检查编译任务类路径输入 |
| 3 | 调研 | `WebSearch` IntelliJ Platform Gradle Plugin 2.0 文档 | 确认 `local()` 正确语法 |

### 阶段 B：修复（核心改动）

| 序号 | 操作 | 文件 | 说明 |
|------|------|------|------|
| 4 | 修改 | `build.gradle.kts`（方案 1：加 `pluginLibrary()`） | 首选方案 |
| 5 | 验证 | `"$GRADLE" compileKotlin` | 确认编译错误消失 |
| 6 | (备选) 修改 | `build.gradle.kts`（方案 2：切到 `intellijIdeaCommunity("2023.1")`） | 方案 1 失败时启用 |
| 7 | (备选) 验证 | `"$GRADLE" compileKotlin` | 确认编译错误消失 |

### 阶段 C：构建验证

| 序号 | 操作 | 命令 | 说明 |
|------|------|------|------|
| 8 | 执行 | `"$GRADLE" buildPlugin` | 完整构建 |
| 9 | 验证 | `ls build/distributions/RustSearch-0.1.0.zip` | 确认 zip 存在 |
| 10 | 验证 | `unzip -l build/distributions/RustSearch-0.1.0.zip` | 确认含 jar + dylib |
| 11 | 验证 | `file /tmp/rustsearch-verify/.../librust_search.dylib` | 确认 arm64 |
| 12 | 验证 | `unzip -l .../RustSearch-0.1.0.jar \| grep "\.class"` | 确认 Kotlin 类已编译 |

### 阶段 D：端到端验证

| 序号 | 操作 | 命令/动作 | 说明 |
|------|------|-----------|------|
| 13 | 执行 | `"$GRADLE" runIde` | 启动 IDE 沙箱 |
| 14 | 手动 | 按 D2 清单逐项验证 | 11 项功能验证 |
| 15 | 手动 | 按 D3 清单验证稳定性 | 3 项稳定性验证 |

---

## 五、Assumptions & Decisions

### 5.1 关键决策

| 决策 | 选项 | 理由 |
|------|------|------|
| 修复策略优先级 | 方案 1（`pluginLibrary()`）> 方案 2（远程 IC-231）> 方案 3（本地 IC-231） | 代价由小到大，先试最低成本 |
| 不回退 IntelliJ 插件版本 | 保持 2.0.1 | 1.17.3 不支持 AS 261，回退无意义 |
| 不回退 Kotlin 版本 | 保持 2.2.20 | AS 261 的 Kotlin metadata 要求 2.2.x 编译器 |
| 不重复 Rust 架构设计 | 引用现有 `module1-global-text-search-rust-architecture.md` | 架构方案已落地，Rust 代码已完成 95% |
| 行号定位增强 | MVP 不做 | 当前 `navigateToSelectedResult()` 仅打开文件，行号导航作为里程碑 2 优先级 |
| 渲染器基类 | `DefaultTreeCellRenderer` | 已完成，MVP 简单文本足够 |
| 文件图标 | `AllIcons.FileTypes.Any_type` | 通用占位，里程碑 2 再按扩展名精确映射 |
| 匹配文本高亮 | 不做 | MVP 保持简单，高亮需 SpeedSearchUtil，留待里程碑 2 |

### 5.2 假设

1. **Android Studio 2026.1（AI-261）已安装**：`/Applications/Android Studio.app` 存在且包含 `Contents/Resources/build.txt`、`Contents/lib/`、`Contents/plugins/`、`Contents/jbr/`
2. **JBR 21 可用**：`/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java -version` 返回 21
3. **Rust 工具链已安装**：`cargo` 在 PATH 中，`rust-search/target/release/librust_search.dylib` 已存在
4. **Gradle 8.11.1 缓存二进制可用**：`~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle` 存在且可执行
5. **阿里云镜像可访问**：`https://maven.aliyun.com/repository/public` 与 `/central` 走 HTTPS，Java 默认 cacerts 可验证（已验证）
6. **Rust 核心测试全通过**：62 个测试已验证
7. **代码层 95% 就绪**：仅剩 `buildPlugin` 构建与端到端验证未完成

### 5.3 风险与规避

| 风险 | 影响 | 规避 |
|------|------|------|
| 方案 1（`pluginLibrary()`）无效 | 需切换到方案 2 | 方案 2 已备好，直接切换 |
| 方案 2 远程下载 SSL 失败 | 无法获取 IC-231 | 阿里云镜像已配置；最终兜底方案 3（本地 IC-231） |
| AI-261 与 plugin.xml `until-build=261.*` 不匹配 | 插件不加载 | 确认 AS 版本号在 231~261.* 范围内（AI-261 满足） |
| runIde 启动 OOM | IDE 沙箱内存不足 | `gradle.properties` 可调到 `-Xmx4g` |
| native 库加载失败（架构不匹配） | `UnsatisfiedLinkError` | `file` 命令确认 dylib 为 arm64 |
| 动态库未拷贝到 sandbox | 运行时找不到库 | 确认 `prepareSandbox` 依赖 `copyNativeLib`（已配置） |
| macOS Gatekeeper 拦截 dylib | 加载失败 | `xattr -d com.apple.quarantine /path/to/librust_search.dylib` |
| `Tree` 类在 AS 261 中 API 变化 | 编译通过但运行异常 | MVP 用 `com.intellij.ui.tree.Tree` 是稳定 API，风险低 |

---

## 六、Verification Steps

### 6.1 阶段 A 完成后：诊断结论

- `compileClasspath` 中是否包含 IntelliJ platform jar（是/否）
- 编译任务的 `classpath` 输入中是否包含 `app.jar`、`platform-impl.jar` 等
- IntelliJ Platform Gradle Plugin 2.0 文档确认的 `local()` 正确语法

### 6.2 阶段 B 完成后：Kotlin 编译通过

```bash
"$GRADLE" compileKotlin --no-daemon 2>&1 | tail -5
# 预期: BUILD SUCCESSFUL
```

无 `Unresolved reference` 错误。

### 6.3 阶段 C 完成后：插件打包成功

```bash
ls /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip
# 预期: 文件存在

unzip -l /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip
# 预期: 含 lib/RustSearch-0.1.0.jar + native/librust_search.dylib

file /tmp/rustsearch-verify/RustSearch/native/librust_search.dylib
# 预期: Mach-O 64-bit ... arm64

unzip -l /tmp/rustsearch-verify/RustSearch/lib/RustSearch-0.1.0.jar | grep "\.class"
# 预期: 8+ 个 .class 文件,包括 SearchResultTreeCellRenderer
```

### 6.4 阶段 D 完成后：端到端功能

按 D2（11 项）+ D3（3 项）清单逐项手动验证，全部 ☐ 变 ☑ 即达成里程碑 1。

---

## 七、里程碑 1 完成标准对照

| 标准 | 状态 | 验证方式 |
|------|------|----------|
| Rust 异步流式 JNI 接口实现完成，所有测试通过 | ✅ 已完成 | 62 个测试通过 |
| IntelliJ 插件工程可 `./gradlew buildPlugin` 打包 | ⏳ 待验证 | 阶段 B + C |
| 插件 zip 内含 Rust 动态库与 Kotlin 类 | ⏳ 待验证 | 阶段 C |
| runIde 实例中 Tool Window 可打开 | ⏳ 待验证 | D2-1 |
| native 库加载成功 | ⏳ 待验证 | D2-2 |
| 搜索功能可用，结果正确展示 | ⏳ 待验证 | D2-3~8 |
| 中途取消功能可用 | ⏳ 待验证 | D2-9 |
| 双击结果可跳转到文件 | ⏳ 待验证 | D2-10 |
| 连续 10 次搜索无内存泄漏 | ⏳ 待验证 | D3-12 |
| macOS Apple Silicon 平台验证通过 | ⏳ 待验证 | 全流程在 aarch64 上运行 |

---

## 八、回滚与备选方案

### 8.1 若所有方案均失败

**备选方案 A**：使用 Android Studio IDE 内置 Gradle
1. 用 Android Studio 打开 `/Users/apple/AndroidStudioProjects/RustSearch-AS`
2. 等待 IDE sync 完成（自动使用 Wrapper）
3. 在 IDE Gradle 面板执行 `buildPlugin` 任务
4. 在 IDE Run Configuration 中执行 `runIde`

**备选方案 B**：使用 IntelliJ IDEA Community 2023.1 打开项目
- 下载并安装独立的 IntelliJ IDEA Community 2023.1
- 用它打开项目，IDE 自带的 Gradle 同步可能更稳定
- `since-build=231` 与 IDE 版本严格对齐

### 8.2 若动态库加载失败

**排查步骤**：
1. `file librust_search.dylib` 确认架构为 arm64
2. `otool -L librust_search.dylib` 确认依赖链
3. `xattr -l librust_search.dylib` 检查 quarantine 属性
4. 手动 `System.load("/absolute/path/librust_search.dylib")` 在 Kotlin 中测试

---

## 九、与现有计划文档的关系

| 现有文档 | 关系 | 说明 |
|----------|------|------|
| `module1-global-text-search-rust-architecture.md` | 引用，不重复 | Rust 架构方案已落地，代码已完成 |
| `module1-mvp-intellij-plugin-plan.md` | 引用，不重复 | 插件开发计划已落地 |
| `module1-mvp-completion-plan.md` | 引用，不重复 | 前期收尾计划已完成阶段 A/B |
| `module1-build-and-verification-plan.md` | 继承与细化 | 本计划是其阶段 B2 的具体化，聚焦编译类路径修复 |

本计划不重复上述文档的架构设计与开发计划内容，仅聚焦当前唯一阻塞点（编译类路径修复）与剩余验证路径。

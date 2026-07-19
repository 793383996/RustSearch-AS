# 模块 1 MVP 构建收尾与端到端验证计划

> 阶段：里程碑 1（MVP 版本）收尾
> 范围：buildPlugin 构建阻塞解决 → Kotlin 编译验证 → 动态库打包 → runIde 端到端功能验证
> 目标：达成里程碑 1 完成标准,跑通「IDE UI 触发搜索 → Rust 核心执行 → 流式返回结果展示 → 双击跳转」完整闭环
> 前置文档：`module1-global-text-search-rust-architecture.md`（架构方案）、`module1-mvp-intellij-plugin-plan.md`（插件开发计划）、`module1-mvp-completion-plan.md`（前期收尾计划,已完成阶段 A/B）

---

## 一、当前状态分析

### 1.1 已完成（代码层 95% 就绪）

| 层级 | 模块 | 文件 | 状态 |
|------|------|------|------|
| Rust 核心 | 异步流式 JNI 接口 | `rust-search/src/jni/bridge.rs` | ✅ 5 个 JNI 函数 + SearchSession + SEARCH_REGISTRY |
| Rust 核心 | 流式集成测试 | `rust-search/tests/jni_stream_integration.rs` | ✅ 11 个测试全通过 |
| Rust 核心 | release 构建产物 | `rust-search/target/release/librust_search.dylib` | ✅ 已生成(2,009,536 字节) |
| 插件骨架 | Gradle 构建脚本 | `build.gradle.kts` / `gradle.properties` | ✅ buildRust + copyNativeLib 任务已配置 |
| 插件骨架 | Gradle Wrapper | `gradlew` / `gradlew.bat` / `gradle/wrapper/*` | ✅ 8.11.1 版本已生成 |
| 插件骨架 | 扩展点声明 | `src/main/resources/META-INF/plugin.xml` | ✅ toolWindow + applicationService + action |
| Kotlin JNI | JNI 入口 | `src/main/kotlin/.../RustSearchEngine.kt` | ✅ 5 external 函数 + SearchResult 内部类 |
| Kotlin JNI | 配置/异常 | `SearchConfig.kt` / `SearchException.kt` | ✅ |
| Kotlin JNI | 服务层 | `service/RustSearchService.kt` | ✅ Flow<List<SearchResult>> + 动态库加载 |
| Tool Window | 工厂 + Action | `RustSearchToolWindowFactory.kt` / `RustSearchAction.kt` | ✅ DumbAware |
| Tool Window | 搜索面板 | `RustSearchPanel.kt` | ✅ 搜索栏 + 结果树 + 状态栏 + 协程收集 Flow |
| Tool Window | 树模型 + 渲染器 | `SearchResultTreeModel.kt` | ✅ 含 `SearchResultTreeCellRenderer`（前期已完成阶段 A） |

### 1.2 当前阻塞点

| # | 阻塞项 | 表现 | 根因 |
|---|--------|------|------|
| 1 | `buildPlugin` 未成功执行 | `build/` 目录下仅有 `reports/problems/problems-report.html`,无 `distributions/` 产物 | 上次执行失败于 `localPath` 路径解析,已修复为 `/Applications/Android Studio.app/Contents`,但尚未重新运行验证 |
| 2 | 动态库未拷贝到插件资源 | `src/main/resources/native/` 仅有 `.gitkeep`,`copyNativeLib` 任务未触发 | `buildPlugin` 未成功,`prepareSandbox` 依赖链未走通 |
| 3 | Kotlin 编译未验证 | 无法确认 `SearchResultTreeCellRenderer` 与其他类是否存在 API 引用错误 | `buildPlugin` 未跑通 |
| 4 | 端到端功能未验证 | 里程碑 1 完成标准未达成 | runIde 实例未启动 |
| 5 | SSL 证书问题（次要） | Gradle Wrapper 自动下载、Maven 依赖下载可能失败 | ICUBE 代理环境 + Java cacerts 与 macOS Keychain 不同步 |

### 1.3 关键约束

- **本地 `gradle` 命令未安装**：`which gradle` 返回 not found,必须使用缓存中的 Gradle 8.11.1 二进制绕过 Wrapper 下载：
  `~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle`
- **Rust 动态库已就绪**：`rust-search/target/release/librust_search.dylib` 存在,`copyNativeLib` 任务会自动拷贝
- **IntelliJ SDK 来源**：`localPath.set("/Applications/Android Studio.app/Contents")` 指向本地 Android Studio 2023.1,规避 SSL 证书下载问题
- **JNI 函数名绑定**：`Java_com_example_rustsearch_RustSearchEngine_*`,Kotlin 侧 `RustSearchEngine` 必须在 `com.example.rustsearch` 根包（已满足）
- **JDK 17**：使用 Android Studio 自带 JBR：`/Applications/Android Studio.app/Contents/jbr/Contents/Home`
- **架构一致性**：macOS Apple Silicon（aarch64）,dylib 与 JVM 架构一致

---

## 二、Proposed Changes

### Part A：构建环境最终验证

**目标**：确认所有构建前置条件就绪,排除低级错误

#### A1. 验证 Rust 动态库就绪

```bash
ls -la /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/target/release/librust_search.dylib
# 预期: 文件存在,大小约 2MB
```

#### A2. 验证 Android Studio 路径与 build.txt

```bash
ls "/Applications/Android Studio.app/Contents/Resources/build.txt"
ls "/Applications/Android Studio.app/Contents/lib/"
ls "/Applications/Android Studio.app/Contents/plugins/"
# 预期: build.txt 存在, lib/ 包含 IntelliJ 平台 jar, plugins/ 包含内置插件
```

#### A3. 验证 Gradle Wrapper 与缓存二进制

```bash
ls /Users/apple/AndroidStudioProjects/RustSearch-AS/gradlew
ls /Users/apple/AndroidStudioProjects/RustSearch-AS/gradle/wrapper/gradle-wrapper.jar
ls ~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle
# 预期: 三个文件均存在
```

#### A4. 验证 JAVA_HOME

```bash
ls "/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java"
"/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java" -version
# 预期: JBR 17 (aarch64)
```

---

### Part B：执行 buildPlugin 构建并修复编译错误

**目标**：让 `./gradlew buildPlugin` 成功产出 `build/distributions/RustSearch-0.1.0.zip`

#### B1. 首次构建尝试（使用缓存 Gradle 二进制绕过 Wrapper 下载）

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
GRADLE=~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle
"$GRADLE" buildPlugin --no-daemon 2>&1 | tee /tmp/buildplugin-1.log | tail -50
```

**预期三种结果之一**：

| 结果 | 表现 | 下一步 |
|------|------|--------|
| ✅ 成功 | `BUILD SUCCESSFUL`,产出 `build/distributions/RustSearch-0.1.0.zip` | 跳到 Part C 验证产物 |
| ⚠️ Kotlin 编译错误 | `e: file:///...: error: unresolved reference` 等 | 进入 B2 修复流程 |
| ❌ SSL/网络错误 | `Could not resolve com.jetbrains.intellij.idea:ideaIC:...` 或 `PKIX path building failed` | 进入 B3 SSL 处理流程 |

#### B2. Kotlin 编译错误修复策略

**优先级排序**：

1. **Unresolved reference（IntelliJ API）**：
   - 检查 `build.gradle.kts` 的 `intellij { localPath.set(...) }` 是否正确解析平台 jar
   - 确认使用的 API 在 Android Studio 2023.1（IC-231）中存在
   - 参考：[IntelliJ Platform SDK 2023.1 API 文档](https://plugins.jetbrains.com/docs/intellij/api-notable-changes-2023.html)

2. **`SearchResultTreeCellRenderer` 相关错误**：
   - 检查 `SearchResultTreeModel.kt` 文件末尾的类定义是否完整
   - 确认 import：`com.intellij.icons.AllIcons`、`com.intellij.ui.JBColor`、`com.intellij.util.ui.UIUtil`、`java.awt.Component`、`javax.swing.JTree`、`javax.swing.tree.DefaultMutableTreeNode`、`javax.swing.tree.DefaultTreeCellRenderer`
   - 验证 `DefaultTreeCellRenderer` 的 `getTreeCellRendererComponent` 签名匹配

3. **`RustSearchPanel.kt` 相关错误**：
   - 第 116 行 `cellRenderer = SearchResultTreeCellRenderer()` 引用是否解析
   - 第 302 行 `resultTree.lastSelectedPathComponent as? DefaultMutableTreeNode` 是否需要 import `javax.swing.tree.DefaultMutableTreeNode`（当前文件未 import）

4. **kotlinx-coroutines 版本冲突**：
   - 若 IntelliJ Platform 自带 coroutines 与 `1.7.3` 冲突,改为 `implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3")` 且不加 `api`
   - 极端情况降级到 IntelliJ 内置版本

5. **JVM target 不匹配**：
   - 确认 `kotlinOptions.jvmTarget = "17"` 与 `javaVersion=17` 一致
   - 确认 JAVA_HOME 指向 JDK 17

**每个错误修复后重新执行**：

```bash
"$GRADLE" buildPlugin --no-daemon 2>&1 | tee /tmp/buildplugin-2.log | tail -30
```

#### B3. SSL 证书问题处理（仅 B1 出现 SSL 错误时执行）

**当前已生效的缓解措施**：
- `gradle.properties` 已添加 `systemProp.javax.net.ssl.trustStoreType=KeychainStore`
- `localPath` 已改为本地 AS 引用,IntelliJ SDK 不走网络

**仍可能失败的环节**：
- Maven 依赖下载（`kotlinx-coroutines-core`、`kotlin-stdlib-jdk8`）走 `repo.maven.apache.org`

**递进处理方案**（按代价从小到大）：

| 序号 | 方案 | 操作 | 适用场景 |
|------|------|------|----------|
| 1 | 使用阿里云镜像 | 在 `settings.gradle.kts` 添加 `mirror` 仓库 | Maven Central HTTPS 证书问题 |
| 2 | 离线模式 | `"$GRADLE" buildPlugin --offline --no-daemon`（依赖已缓存） | 依赖已在 `~/.gradle/caches/` 中 |
| 3 | 临时禁用 SSL 验证 | `systemProp.javax.net.ssl.trustAll=true`（仅诊断用,不提交） | 诊断是否为 SSL 问题 |
| 4 | 导入 ICUBE 代理 CA | `keytool -importcert -alias icube-ca -keystore "$JAVA_HOME/lib/security/cacerts" -file <代理CA证书>` | 长期解决方案 |

**推荐先尝试方案 2（离线模式）**,因为：
- 之前已成功解析过依赖（Gradle 9.6.1 缓存可见）
- 离线模式完全规避 SSL
- 若离线失败,说明缓存不全,再尝试方案 1 或 4

#### B4. 构建成功的判定标准

```bash
ls -la /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/
# 预期: RustSearch-0.1.0.zip 存在,大小约 2~3 MB

unzip -l /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip | grep -E "(lib/|native/)"
# 预期输出:
# lib/RustSearch-0.1.0.jar
# native/librust_search.dylib
```

---

### Part C：验证插件打包内容

**目标**：确认 zip 内动态库与 jar 正确打包

#### C1. 解压检查

```bash
cd /tmp && rm -rf rustsearch-verify && mkdir rustsearch-verify && cd rustsearch-verify
unzip -q /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip
find . -type f | sort
```

**预期文件清单**：
- `RustSearch/lib/RustSearch-0.1.0.jar`
- `RustSearch/native/librust_search.dylib`
- `RustSearch/META-INF/plugin.xml`（在 jar 内,无需单独检查）

#### C2. 验证动态库架构

```bash
file /tmp/rustsearch-verify/RustSearch/native/librust_search.dylib
# 预期: Mach-O 64-bit dynamically linked shared library arm64
```

#### C3. 验证 jar 内 Kotlin 类

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

### Part D：runIde 端到端功能验证

**目标**：在 IDE 沙箱实例中验证完整搜索链路,达成里程碑 1 完成标准

#### D1. 启动 runIde 实例

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
GRADLE=~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle
"$GRADLE" runIde --no-daemon 2>&1 | tee /tmp/runide-1.log | tail -20
```

**预期**：启动一个新的 Android Studio / IntelliJ 实例（sandbox）,左侧出现 RustSearch 工具窗口图标（`AllIcons.Actions.Search`）

**启动失败排查**：

| 问题 | 处理 |
|------|------|
| `UnsatisfiedLinkError: /var/folders/.../librust_search.dylib: dlopen(...) image not found` | 检查 dylib 架构（`file` 命令）,确认与 JVM 一致 |
| `UnsatisfiedLinkError: can't find class RustSearchEngine` | 检查 `RustSearchEngine` 是否在 `com.example.rustsearch` 根包 |
| Tool Window 不显示 | 检查 `plugin.xml` 的 `toolWindow` 扩展点,从 `View → Tool Windows → RustSearch` 手动激活 |
| IDE 启动 OOM | 调大 `gradle.properties` 的 `org.gradle.jvmargs=-Xmx4g` |

#### D2. 功能验证清单（11 项）

| # | 验证项 | 操作步骤 | 预期结果 | 通过? |
|---|--------|----------|----------|-------|
| 1 | Tool Window 可打开 | 点击左侧 RustSearch 图标,或 `Cmd+Shift+Alt+F` | 工具窗口展开,显示搜索面板 | ☐ |
| 2 | native 库加载成功 | 查看 `idea.log`（`Help → Show Log in Finder`）或控制台 | 输出 "Rust 动态库加载成功: /var/folders/.../librust_search.dylib" | ☐ |
| 3 | 基础字面量搜索 | 输入 `SearchEngine` → 点搜索 | 结果树展示匹配文件与行 | ☐ |
| 4 | 流式结果展示 | 大项目搜索（如 Android Studio 源码目录） | 结果逐步出现,非一次性返回 | ☐ |
| 5 | 正则搜索 | 勾选「正则」→ 输入 `print\w+` → 搜索 | 正则匹配生效 | ☐ |
| 6 | 大小写敏感 | 勾选「大小写」→ 搜索 `hello` | 仅匹配 `hello`,不匹配 `Hello` | ☐ |
| 7 | 中文搜索 | 输入中文关键词（如「搜索」） | 编码正确,结果正常 | ☐ |
| 8 | 文件过滤 | 在「包含」输入 `*.kt` → 搜索 | 仅搜索 .kt 文件 | ☐ |
| 9 | 中途取消 | 大项目搜索启动后立即点「取消」 | UI 响应,搜索停止,状态栏显示「已取消」 | ☐ |
| 10 | 双击跳转 | 双击结果树匹配节点 | 打开对应文件 | ☐ |
| 11 | 状态栏信息 | 搜索完成后查看状态栏 | 显示「N 个匹配(M 个文件),耗时 T s」 | ☐ |

#### D3. 稳定性验证（3 项）

| # | 验证项 | 操作步骤 | 预期结果 | 通过? |
|---|--------|----------|----------|-------|
| 12 | 内存泄漏检查 | 连续执行 10 次相同搜索,Activity Monitor 监控 IDE 进程内存 | JVM 内存无持续上涨（允许波动 ±50MB） | ☐ |
| 13 | 会话释放验证 | 搜索完成后查看 `idea.log` | 输出 "搜索会话已释放: searchId=..." | ☐ |
| 14 | 异常恢复 | 输入非法正则 `(` → 搜索 | 状态栏显示错误,不崩溃,UI 可继续使用 | ☐ |

#### D4. 行号定位增强（MVP 可选,非阻塞）

**当前状态**：`RustSearchPanel.navigateToSelectedResult()` 第 307 行仅 `openFile`,未定位到具体行（标注了 TODO）

**增强方案**（若 D2-10 通过且时间充裕则实施）：

```kotlin
// 在 RustSearchPanel.kt 第 307 行后追加:
FileEditorManager.getInstance(project).openFile(file, true).also { editors ->
    val editor = FileEditorManager.getInstance(project).selectedTextEditor
    editor?.let {
        val offset = it.document.getLineStartOffset(data.lineNumber - 1)
        it.caretModel.moveToOffset(offset)
        it.scrollingModel.scrollToCaret(com.intellij.openapi.editor.ScrollType.CENTER)
    }
}
```

**决策**：MVP 阶段可选。若 D2-10 双击跳转已通过,此项作为体验增强,可纳入里程碑 2 优先级。

---

## 三、执行顺序与文件清单

### 阶段 A：构建环境最终验证（只读）

| 序号 | 操作 | 命令/文件 | 说明 |
|------|------|-----------|------|
| 1 | 验证 | `ls rust-search/target/release/librust_search.dylib` | 确认 Rust 动态库就绪 |
| 2 | 验证 | `ls "/Applications/Android Studio.app/Contents/Resources/build.txt"` | 确认 AS 路径可被 IntelliJ 插件识别 |
| 3 | 验证 | `ls ~/.gradle/wrapper/dists/gradle-8.11.1-all/.../bin/gradle` | 确认缓存 Gradle 二进制可用 |
| 4 | 验证 | `"/Applications/Android Studio.app/Contents/jbr/Contents/Home/bin/java" -version` | 确认 JBR 17 可用 |

### 阶段 B：buildPlugin 构建与修复

| 序号 | 操作 | 命令/文件 | 说明 |
|------|------|-----------|------|
| 5 | 执行 | `"$GRADLE" buildPlugin --no-daemon` | 首次构建尝试 |
| 6 | 修复 | 视编译错误而定 | 逐个修复 Kotlin 编译问题（B2 策略） |
| 7 | 修复（如需） | `build.gradle.kts` / `settings.gradle.kts` | 处理 SSL 问题（B3 策略,优先离线模式） |
| 8 | 验证 | `ls build/distributions/RustSearch-0.1.0.zip` | 确认产出存在 |
| 9 | 验证 | `unzip -l build/distributions/RustSearch-0.1.0.zip` | 确认含 jar + dylib |

### 阶段 C：打包内容验证

| 序号 | 操作 | 命令 | 说明 |
|------|------|------|------|
| 10 | 执行 | `unzip -q build/distributions/RustSearch-0.1.0.zip -d /tmp/rustsearch-verify` | 解压到临时目录 |
| 11 | 验证 | `find /tmp/rustsearch-verify -type f` | 确认文件清单完整 |
| 12 | 验证 | `file /tmp/rustsearch-verify/RustSearch/native/librust_search.dylib` | 确认 arm64 架构 |
| 13 | 验证 | `unzip -l /tmp/rustsearch-verify/RustSearch/lib/RustSearch-0.1.0.jar \| grep "\.class"` | 确认 Kotlin 类已编译 |

### 阶段 D：端到端功能验证

| 序号 | 操作 | 命令/动作 | 说明 |
|------|------|-----------|------|
| 14 | 执行 | `"$GRADLE" runIde --no-daemon` | 启动 IDE 沙箱实例 |
| 15 | 手动 | 按 D2 清单逐项验证 | 11 项功能验证 |
| 16 | 手动 | 按 D3 清单验证稳定性 | 3 项稳定性验证 |
| 17 | (可选) | 编辑 `RustSearchPanel.kt` 第 307 行后追加行号导航 | D4 行号定位增强 |

---

## 四、Assumptions & Decisions

### 4.1 关键决策

| 决策 | 选项 | 理由 |
|------|------|------|
| 构建策略 | 本地 Android Studio 引用 + 缓存 Gradle 8.11.1 二进制 | 当前已生效的缓解方案,最低成本验证 |
| SSL 问题处理优先级 | 离线模式 > 阿里云镜像 > 导入 ICUBE CA | 离线模式零成本,优先尝试；镜像次之；CA 导入为长期方案 |
| 编译错误修复策略 | 优先修复 Kotlin 引用错误,再处理版本冲突 | 引用错误通常阻塞编译,优先解决 |
| 行号定位 | MVP 可选 | 当前仅打开文件,行号导航作为增强项,不阻塞里程碑 1 达成 |
| 渲染器基类 | `DefaultTreeCellRenderer` | 已完成（阶段 A）,MVP 简单文本足够 |
| 文件图标 | `AllIcons.FileTypes.Any_type` | 通用占位,里程碑 2 再按扩展名精确映射 |
| 匹配文本高亮 | 不做 | MVP 保持简单,高亮需 SpeedSearchUtil,留待里程碑 2 |

### 4.2 假设

1. **Android Studio 2023.1+ 已安装**：`/Applications/Android Studio.app/Contents` 存在且包含 `Resources/build.txt`、`lib/`、`plugins/`、`jbr/`
2. **Rust 工具链已安装**：`cargo` 在 PATH 中,aarch64-apple-darwin 目标已安装,动态库已编译
3. **Gradle 8.11.1 缓存二进制可用**：`~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle` 存在且可执行
4. **Rust 核心测试全通过**：62 个测试（37 单元 + 14 集成 + 11 jni_stream）已验证
5. **Maven 依赖已部分缓存**：`~/.gradle/caches/modules-2/files-2.1/` 下应有 `kotlinx-coroutines-core`、`kotlin-stdlib-jdk8`,离线模式可成功
6. **代码层 95% 就绪**：仅剩 `buildPlugin` 构建与端到端验证未完成

### 4.3 风险与规避

| 风险 | 影响 | 规避 |
|------|------|------|
| Kotlin 编译错误（IntelliJ API 版本差异） | buildPlugin 失败 | 仅用 2023.1 公开 API,逐个修复编译错误（B2 策略） |
| SSL 证书问题持续影响 Maven 下载 | 依赖无法解析 | 优先离线模式,次选阿里云镜像,最终导入 ICUBE CA |
| runIde 启动 OOM | IDE 沙箱内存不足 | `gradle.properties` 已配 `-Xmx2g`,必要时调到 `-Xmx4g` |
| native 库加载失败（架构不匹配） | `UnsatisfiedLinkError` | `file` 命令确认 dylib 为 arm64,与 JVM 架构一致 |
| 动态库未拷贝到 sandbox | 运行时找不到库 | 确认 `prepareSandbox` 依赖 `copyNativeLib`,检查 `build/idea-sandbox/` 下 native 目录 |
| macOS Gatekeeper 拦截 dylib | 加载失败 | `xattr -d com.apple.quarantine /path/to/librust_search.dylib`（如需） |
| Android Studio 版本与 `sinceBuild` 不匹配 | 插件不加载 | 确认 AS 版本号在 231~241.* 范围内 |

---

## 五、Verification Steps

### 5.1 阶段 A 完成后：构建前置条件

所有 A1~A4 验证命令返回预期结果,文件均存在。

### 5.2 阶段 B 完成后：插件打包

```bash
ls /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip
# 预期: 文件存在

unzip -l /Users/apple/AndroidStudioProjects/RustSearch-AS/build/distributions/RustSearch-0.1.0.zip
# 预期: 含 lib/RustSearch-0.1.0.jar + native/librust_search.dylib
```

### 5.3 阶段 C 完成后：打包内容完整

- `/tmp/rustsearch-verify/RustSearch/native/librust_search.dylib` 为 `Mach-O 64-bit ... arm64`
- `/tmp/rustsearch-verify/RustSearch/lib/RustSearch-0.1.0.jar` 内含 8+ 个 `.class` 文件,包括 `SearchResultTreeCellRenderer`

### 5.4 阶段 D 完成后：端到端功能

按 D2（11 项）+ D3（3 项）清单逐项手动验证,全部 ☐ 变 ☑ 即达成里程碑 1。

---

## 六、里程碑 1 完成标准对照

| 标准 | 状态 | 验证方式 |
|------|------|----------|
| Rust 异步流式 JNI 接口实现完成,所有测试通过 | ✅ 已完成 | 62 个测试通过 |
| IntelliJ 插件工程可 `./gradlew buildPlugin` 打包 | ⏳ 待验证 | 阶段 B |
| 插件 zip 内含 Rust 动态库与 Kotlin 类 | ⏳ 待验证 | 阶段 C |
| runIde 实例中 Tool Window 可打开 | ⏳ 待验证 | D2-1 |
| native 库加载成功 | ⏳ 待验证 | D2-2 |
| 搜索功能可用,结果正确展示 | ⏳ 待验证 | D2-3~8 |
| 中途取消功能可用 | ⏳ 待验证 | D2-9 |
| 双击结果可跳转到文件 | ⏳ 待验证 | D2-10 |
| 连续 10 次搜索无内存泄漏 | ⏳ 待验证 | D3-12 |
| macOS Apple Silicon 平台验证通过 | ⏳ 待验证 | 全流程在 aarch64 上运行 |

---

## 七、回滚与备选方案

### 7.1 若 buildPlugin 持续失败

**备选方案 A**：使用 Android Studio IDE 内置 Gradle
1. 用 Android Studio 打开 `/Users/apple/AndroidStudioProjects/RustSearch-AS`
2. 等待 IDE sync 完成（自动使用 Wrapper）
3. 在 IDE Gradle 面板执行 `buildPlugin` 任务
4. 在 IDE Run Configuration 中执行 `runIde`

**备选方案 B**：降级 Kotlin 或 IntelliJ 插件版本
- 若 Kotlin 1.9.22 与 IntelliJ 插件 1.17.3 不兼容,降级 Kotlin 到 1.8.22
- 若 IntelliJ 插件 1.17.3 有 bug,升级到 1.17.4

### 7.2 若 SSL 问题彻底阻塞

**长期方案**：
1. 从 macOS Keychain 导出 ICUBE 代理 CA 证书
2. 导入到 JBR cacerts：
   ```bash
   keytool -importcert -alias icube-ca \
     -keystore "/Applications/Android Studio.app/Contents/jbr/Contents/Home/lib/security/cacerts" \
     -storepass changeit \
     -file /path/to/icube-ca.pem
   ```
3. 验证：`keytool -list -keystore ... -storepass changeit | grep icube`

### 7.3 若动态库加载失败

**排查步骤**：
1. `file librust_search.dylib` 确认架构
2. `otool -L librust_search.dylib` 确认依赖链
3. `xattr -l librust_search.dylib` 检查 quarantine 属性
4. 手动 `System.load("/absolute/path/librust_search.dylib")` 在 Kotlin 中测试

# 模块 1 MVP 收尾与端到端验证计划

> 阶段：里程碑 1（MVP 版本）收尾
> 范围：补全 Tool Window UI 最后一个缺失类 → 生成 Gradle Wrapper → 构建验证 → 端到端功能验证
> 目标：跑通「IDE UI 触发搜索 → Rust 核心执行 → 流式返回结果展示 → 双击跳转」完整闭环，达成里程碑 1 完成标准

---

## 一、当前状态分析

### 1.1 已完成

| 层级 | 模块 | 文件 | 状态 |
|------|------|------|------|
| Rust 核心 | 异步流式 JNI 接口 | `rust-search/src/jni/bridge.rs` | ✅ 5 个 JNI 函数 + SearchSession + SEARCH_REGISTRY |
| Rust 核心 | 流式集成测试 | `rust-search/tests/jni_stream_integration.rs` | ✅ 11 个测试全通过 |
| Rust 核心 | release 构建产物 | `rust-search/target/release/librust_search.dylib` | ✅ 已生成 |
| 插件骨架 | Gradle 构建脚本 | `build.gradle.kts` / `settings.gradle.kts` / `gradle.properties` | ✅ 含 buildRust + copyNativeLib 任务 |
| 插件骨架 | 扩展点声明 | `src/main/resources/META-INF/plugin.xml` | ✅ toolWindow + applicationService + action |
| Kotlin JNI | JNI 入口 | `src/main/kotlin/.../RustSearchEngine.kt` | ✅ 5 external 函数 + SearchResult 内部类 |
| Kotlin JNI | 配置/异常 | `SearchConfig.kt` / `SearchException.kt` | ✅ |
| Kotlin JNI | 服务层 | `service/RustSearchService.kt` | ✅ Flow<List<SearchResult>> + 动态库加载 |
| Tool Window | 工厂 + Action | `RustSearchToolWindowFactory.kt` / `RustSearchAction.kt` | ✅ DumbAware |
| Tool Window | 搜索面板 | `RustSearchPanel.kt` | ✅ 搜索栏 + 结果树 + 状态栏 + 协程收集 Flow |
| Tool Window | 树模型 | `SearchResultTreeModel.kt` | ⚠️ 模型完成,缺 `SearchResultTreeCellRenderer` |

### 1.2 缺失项（本计划要解决的）

| # | 缺失项 | 影响 | 位置 |
|---|--------|------|------|
| 1 | `SearchResultTreeCellRenderer` 类未定义 | `RustSearchPanel.kt` 第 116 行 `cellRenderer = SearchResultTreeCellRenderer()` 编译失败 | `SearchResultTreeModel.kt` 末尾追加 |
| 2 | Gradle Wrapper 缺失（`gradlew` / `gradlew.bat` / `gradle/wrapper/`） | 无法执行 `./gradlew buildPlugin` 与 `./gradlew runIde` | 项目根目录 |
| 3 | 端到端功能未验证 | 里程碑 1 完成标准未达成 | runIde 实例中手动验证 |

### 1.3 关键约束

- **本地 `gradle` 命令未安装**：`which gradle` 返回 not found,需通过 Homebrew 安装或由 Android Studio 自动生成 Wrapper
- **Rust 动态库已就绪**：`rust-search/target/release/librust_search.dylib` 已存在,`copyNativeLib` 任务会自动拷贝
- **JNI 函数名绑定**：`Java_com_example_rustsearch_RustSearchEngine_*`,Kotlin 侧 `RustSearchEngine` 必须在 `com.example.rustsearch` 根包（已满足）

---

## 二、Proposed Changes

### Part A：补全 SearchResultTreeCellRenderer

#### 文件：`src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt`

**操作**：在文件末尾（`MatchNodeData` 类之后）追加 `SearchResultTreeCellRenderer` 类

**目标**：为结果树提供自定义单元格渲染器,区分文件节点与匹配节点的视觉表现

**实现细节**：

```kotlin
/**
 * 搜索结果树单元格渲染器
 *
 * 自定义两类节点的视觉表现:
 * - 文件节点(FileNodeData): 文件图标 + 文件名 + 匹配数(灰色)
 * - 匹配节点(MatchNodeData): 行号(蓝色) + 匹配内容(高亮匹配文本)
 */
class SearchResultTreeCellRenderer : DefaultTreeCellRenderer() {

    override fun getTreeCellRendererComponent(
        tree: JTree, value: Any, sel: Boolean, expanded: Boolean,
        leaf: Boolean, row: Int, hasFocus: Boolean
    ): Component {
        val comp = super.getTreeCellRendererComponent(tree, value, sel, expanded, leaf, row, hasFocus)
        // comp 已是 JLabel(this)
        when (val data = (value as? DefaultMutableTreeNode)?.userObject) {
            is FileNodeData -> {
                icon = AllIcons.FileTypes.Any_type  // 通用文件图标
                text = data.displayName()           // "FileName.kt (3 个匹配)"
                foreground = if (sel) UIUtil.getTreeSelectionForeground(true)
                             else UIUtil.getTreeForeground()
            }
            is MatchNodeData -> {
                icon = null  // 叶子节点不显示图标
                text = "行 ${data.lineNumber}: ${data.matchedText}"
                foreground = if (sel) UIUtil.getTreeSelectionForeground(true)
                             else JBColor(0x4A6F8E, 0x9BAFC4)  // 柔和的蓝色
            }
            else -> {
                // 根节点或其他,使用默认渲染
            }
        }
        return comp
    }
}
```

**需要新增的 import**（文件已有部分 import,需补齐）：
- `com.intellij.icons.AllIcons`

**已存在的 import**（无需重复添加）：
- `com.intellij.ui.JBColor`
- `com.intellij.util.ui.UIUtil`
- `java.awt.Component`
- `javax.swing.JTree`
- `javax.swing.tree.DefaultMutableTreeNode`
- `javax.swing.tree.DefaultTreeCellRenderer`

**决策说明**：
- 使用 `DefaultTreeCellRenderer` 而非 `ColoredTreeCellRenderer`：MVP 阶段简单文本即可,避免过度设计
- 文件图标用 `AllIcons.FileTypes.Any_type`：通用占位,里程碑 2 再按文件扩展名映射精确图标
- 匹配节点用柔和蓝色：与文件节点形成视觉区分,但不抢眼
- 不做匹配文本高亮：MVP 阶段保持简单,高亮需 `SpeedSearchUtil` 且增加复杂度,留待里程碑 2

---

### Part B：生成 Gradle Wrapper

**背景**：项目根目录无 `gradlew` / `gradlew.bat` / `gradle/wrapper/`,`gradle` 命令未安装。无法执行任何 Gradle 任务。

**方案**：通过 Homebrew 安装 Gradle 后生成 Wrapper（一次性操作,安装后可卸载）

**执行步骤**：

```bash
# 1. 安装 Gradle(Homebrew,一次性)
brew install gradle

# 2. 在项目根目录生成 Wrapper(指定 8.5 版本,兼容 IntelliJ Plugin 1.17.3 + JDK 17)
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
gradle wrapper --gradle-version 8.5 --distribution-type bin

# 3. 验证 Wrapper 生成
ls -la gradlew gradlew.bat gradle/wrapper/
# 预期: gradlew, gradlew.bat, gradle/wrapper/gradle-wrapper.jar, gradle/wrapper/gradle-wrapper.properties

# 4. (可选) 验证 Wrapper 可用
./gradlew --version
# 预期: Gradle 8.5, Kotlin 1.9.22

# 5. (可选) 卸载 Gradle(Wrapper 生成后不再需要本地 gradle)
# brew uninstall gradle
```

**备选方案**（若用户不愿安装 Gradle）：
- 用 Android Studio 打开项目 → IDE 首次 sync 时自动生成 Wrapper
- 但无法在命令行验证,需依赖 IDE 内置终端

**决策**：采用 `brew install gradle` 方案,因为：
1. 命令行可控,便于自动化验证
2. 生成后可卸载,不污染系统
3. `--distribution-type bin` 体积小,下载快

**生成的文件**（应提交到版本控制）：
- `gradlew`（Unix shell 脚本）
- `gradlew.bat`（Windows 批处理脚本）
- `gradle/wrapper/gradle-wrapper.jar`（Wrapper 启动器）
- `gradle/wrapper/gradle-wrapper.properties`（指定 Gradle 版本与下载地址）

---

### Part C：构建验证

**目标**：确认插件工程可编译、可打包,动态库正确包含

#### C1. 编译 Rust 动态库（确认已就绪）

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search
cargo build --release 2>&1 | tail -3
ls -la target/release/librust_search.dylib
# 预期: librust_search.dylib 已存在(1.9MB)
```

#### C2. Gradle 编译 + 打包插件

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew buildPlugin 2>&1 | tail -30
```

**预期产出**：
- `build/distributions/RustSearch-0.1.0.zip`
- zip 内含 `lib/RustSearch-0.1.0.jar` + `native/librust_search.dylib`

**可能遇到的问题与处理**：

| 问题 | 原因 | 处理 |
|------|------|------|
| Kotlin 编译错误:Unresolved reference | 缺少 IntelliJ Platform 依赖或 API 用错 | 根据 `./gradlew build` 输出逐个修复 |
| `UnsatisfiedLinkError` 在测试阶段 | 动态库未拷贝到 sandbox | 确认 `copyNativeLib` 任务执行,检查 `build/idea-sandbox/` 下 native 目录 |
| `patchPluginXml` 版本范围警告 | sinceBuild/untilBuild 配置 | 确认 231~241.* 范围 |
| `kotlinx-coroutines` 版本冲突 | IntelliJ Platform 自带 coroutines | 改用 `implementation` 而非 `api`,或降级到 1.7.3 |

#### C3. 编译错误修复策略

若 `buildPlugin` 失败,按以下优先级排查：
1. **Kotlin 语法/引用错误**：逐个看编译器输出,修复 import 或 API 调用
2. **IntelliJ Platform API 不存在**：确认 `platformVersion=2023.1` 提供的 API,查阅 2023.1 文档
3. **资源缺失**：确认 `src/main/resources/native/librust_search.dylib` 存在（由 `copyNativeLib` 拷贝）

---

### Part D：端到端功能验证（Stage 5）

**目标**：在 runIde 实例中验证完整搜索链路,达成里程碑 1 完成标准

#### D1. 启动 runIde 实例

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew runIde 2>&1 | tail -20
```

**预期**：启动一个新的 Android Studio / IntelliJ 实例（sandbox）,左侧出现 RustSearch 工具窗口图标

#### D2. 功能验证清单

| # | 验证项 | 操作步骤 | 预期结果 | 通过? |
|---|--------|----------|----------|-------|
| 1 | Tool Window 可打开 | 点击左侧 RustSearch 图标,或 `Cmd+Shift+Alt+F` | 工具窗口展开,显示搜索面板 | ☐ |
| 2 | native 库加载成功 | 查看 `idea.log` 或控制台 | 输出 "Rust 动态库加载成功" | ☐ |
| 3 | 基础字面量搜索 | 输入 `SearchEngine` → 点搜索 | 结果树展示匹配文件与行 | ☐ |
| 4 | 流式结果展示 | 大项目搜索 | 结果逐步出现,非一次性返回 | ☐ |
| 5 | 正则搜索 | 勾选「正则」→ 输入 `print\w+` → 搜索 | 正则匹配生效 | ☐ |
| 6 | 大小写敏感 | 勾选「大小写」→ 搜索 `hello` | 仅匹配 `hello`,不匹配 `Hello` | ☐ |
| 7 | 中文搜索 | 输入中文关键词 | 编码正确,结果正常 | ☐ |
| 8 | 文件过滤 | 在「包含」输入 `*.kt` → 搜索 | 仅搜索 .kt 文件 | ☐ |
| 9 | 中途取消 | 大项目搜索启动后立即点「取消」 | UI 响应,搜索停止,状态栏显示「已取消」 | ☐ |
| 10 | 双击跳转 | 双击结果树匹配节点 | 打开对应文件 | ☐ |
| 11 | 状态栏信息 | 搜索完成后查看状态栏 | 显示「N 个匹配(M 个文件),耗时 T s」 | ☐ |

#### D3. 稳定性验证

| # | 验证项 | 操作步骤 | 预期结果 | 通过? |
|---|--------|----------|----------|-------|
| 12 | 内存泄漏检查 | 连续执行 10 次相同搜索 | JVM 内存无持续上涨(监控 Activity Monitor) | ☐ |
| 13 | 会话释放验证 | 搜索完成后查看 Rust 日志 | 输出 "搜索会话已释放: searchId=..." | ☐ |
| 14 | 异常恢复 | 输入非法正则 `(` → 搜索 | 状态栏显示错误,不崩溃 | ☐ |

#### D4. 行号定位增强（可选,非 MVP 必需）

当前 `RustSearchPanel.navigateToSelectedResult()` 只打开文件,未定位到具体行（标注了 TODO）。若时间允许,补充行号导航：

```kotlin
// 在 navigateToSelectedResult() 中,openFile 后追加:
val editor = FileEditorManager.getInstance(project).selectedTextEditor
editor?.caretModel?.moveToOffset(
    editor.document.getLineStartOffset(data.lineNumber - 1)
)
editor?.scrollingModel?.scrollToCaret(ScrollType.CENTER)
```

**决策**：MVP 阶段可选,若验证顺利且时间充裕则补充,否则留待里程碑 2。

---

## 三、文件清单与执行顺序

### 阶段 A：补全 UI 渲染器

| 序号 | 操作 | 文件路径 | 说明 |
|------|------|----------|------|
| 1 | 编辑 | `src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt` | 追加 `SearchResultTreeCellRenderer` 类 + 补 `AllIcons` import |

### 阶段 B：生成 Gradle Wrapper

| 序号 | 操作 | 命令/文件 | 说明 |
|------|------|-----------|------|
| 2 | 执行 | `brew install gradle` | 安装 Gradle(一次性) |
| 3 | 执行 | `gradle wrapper --gradle-version 8.5 --distribution-type bin` | 生成 4 个 Wrapper 文件 |
| 4 | 验证 | `./gradlew --version` | 确认 Wrapper 可用 |

### 阶段 C：构建验证

| 序号 | 操作 | 命令 | 说明 |
|------|------|------|------|
| 5 | 验证 | `cd rust-search && cargo build --release` | 确认 Rust 动态库就绪 |
| 6 | 执行 | `./gradlew buildPlugin` | 编译 + 打包插件 |
| 7 | 修复 | 视编译错误而定 | 逐个修复 Kotlin 编译问题 |
| 8 | 验证 | `ls build/distributions/RustSearch-0.1.0.zip` | 确认产出存在 |

### 阶段 D：端到端功能验证

| 序号 | 操作 | 命令/动作 | 说明 |
|------|------|-----------|------|
| 9 | 执行 | `./gradlew runIde` | 启动 IDE 沙箱实例 |
| 10 | 手动 | 按 D2 清单逐项验证 | 11 项功能验证 |
| 11 | 手动 | 按 D3 清单验证稳定性 | 3 项稳定性验证 |
| 12 | (可选) | 补充行号定位代码 | D4 行号导航增强 |

---

## 四、Assumptions & Decisions

### 4.1 关键决策

| 决策 | 选项 | 理由 |
|------|------|------|
| 渲染器基类 | `DefaultTreeCellRenderer` | MVP 简单文本足够,避免 `ColoredTreeCellRenderer` 的复杂度 |
| 文件图标 | `AllIcons.FileTypes.Any_type` | 通用占位,里程碑 2 再按扩展名精确映射 |
| 匹配文本高亮 | 不做 | MVP 保持简单,高亮需 SpeedSearchUtil,留待里程碑 2 |
| Gradle Wrapper 生成方式 | `brew install gradle` + `gradle wrapper` | 命令行可控,生成后可卸载 |
| Gradle 版本 | 8.5 | 兼容 IntelliJ Plugin 1.17.3 + JDK 17,稳定 LTS |
| Wrapper distribution-type | `bin` | 体积小(~100MB),下载快,开发足够 |
| 行号定位 | MVP 可选 | 当前仅打开文件,行号导航作为增强项 |

### 4.2 假设

1. **Homebrew 可用**：macOS 环境,`brew` 命令在 PATH 中
2. **Rust 工具链已安装**：`cargo` 在 PATH 中,aarch64-apple-darwin 目标已安装
3. **Android Studio 2023.1+ 已安装**：runIde 需要本地 IDE（或使用 IntelliJ Platform 下载）
4. **Rust 核心测试全通过**：62 个测试（37 单元 + 14 集成 + 11 jni_stream）已验证
5. **动态库已编译**：`rust-search/target/release/librust_search.dylib` 存在

### 4.3 风险与规避

| 风险 | 影响 | 规避 |
|------|------|------|
| Kotlin 编译错误（IntelliJ API 版本差异） | buildPlugin 失败 | 仅用 2023.1 公开 API,逐个修复编译错误 |
| `brew install gradle` 下载慢 | 阻塞 Wrapper 生成 | 使用国内镜像或 `--distribution-type bin` 减小体积 |
| runIde 启动 OOM | IDE 沙箱内存不足 | `gradle.properties` 已配 `-Xmx2g` |
| native 库加载失败（架构不匹配） | `UnsatisfiedLinkError` | 确认 dylib 为 aarch64,与 JVM 架构一致 |
| 动态库未拷贝到 sandbox | 运行时找不到库 | 确认 `prepareSandbox` 依赖 `copyNativeLib`,检查 `build/idea-sandbox/` |

---

## 五、Verification Steps

### 5.1 阶段 A 完成后：渲染器编译检查

```bash
# 暂无独立编译手段,留待阶段 C 的 buildPlugin 一起验证
# 仅确认文件语法:SearchResultTreeCellRenderer 类已追加,import 已补齐
```

### 5.2 阶段 B 完成后：Wrapper 可用性

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew --version
# 预期: Gradle 8.5
```

### 5.3 阶段 C 完成后：插件打包

```bash
./gradlew buildPlugin 2>&1 | tail -10
ls -la build/distributions/
# 预期: RustSearch-0.1.0.zip
unzip -l build/distributions/RustSearch-0.1.0.zip | grep -E "(lib/|native/)"
# 预期: lib/RustSearch-0.1.0.jar + native/librust_search.dylib
```

### 5.4 阶段 D 完成后：端到端功能

按 D2 + D3 清单逐项手动验证,全部 ☐ 变 ☑ 即达成里程碑 1。

---

## 六、里程碑 1 完成标准对照

| 标准 | 状态 | 验证方式 |
|------|------|----------|
| Rust 异步流式 JNI 接口实现完成,所有测试通过 | ✅ 已完成 | 62 个测试通过 |
| IntelliJ 插件工程可 `./gradlew buildPlugin` 打包 | ⏳ 待验证 | 阶段 C |
| runIde 实例中 Tool Window 可打开 | ⏳ 待验证 | D2-1 |
| 搜索功能可用,结果正确展示 | ⏳ 待验证 | D2-3~8 |
| 中途取消功能可用 | ⏳ 待验证 | D2-9 |
| 双击结果可跳转到文件 | ⏳ 待验证 | D2-10 |
| 连续 10 次搜索无内存泄漏 | ⏳ 待验证 | D3-12 |
| macOS Apple Silicon 平台验证通过 | ⏳ 待验证 | 全流程在 aarch64 上运行 |

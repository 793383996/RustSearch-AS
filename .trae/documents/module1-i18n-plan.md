# 模块 1：中英文语言自适应（i18n）实施计划

## 一、摘要

为 RustSearch 插件添加中英文语言自适应能力，使所有面向用户的 UI 文本能根据 IDE 当前语言（中文/英文）自动切换。采用 IntelliJ Platform 官方推荐的 `DynamicBundle` 方案，通过 message bundle（`.properties`）+ `%key` 占位符实现，无需重启即可跟随 IDE 语言切换。

**范围**：P0（UI 文本）+ P1（面向用户异常）必做；P2（日志）/ P3（插件市场描述）不做，理由见"决策"章节。

## 二、现状分析

### 2.1 i18n 基础设施：无
- `src/main/resources` 下无任何 `.properties` 消息包
- 源码无 `ResourceBundle`/`DynamicBundle`/`Bundle.message` 调用
- `plugin.xml` 无 `<resource-bundle>` 声明，未使用 `%key` 占位符
- `build.gradle.kts` 无 i18n 相关任务（2.x 插件自动处理，无需配置）

### 2.2 硬编码字符串分布（30+ 处）

| 文件 | 硬编码数 | 类型 |
|------|---------|------|
| `RustSearchPanel.kt` | ~24 处 | 复选框/单选按钮文本、tooltip、状态栏消息（含插值） |
| `SearchResultTreeModel.kt` | 3 处 | 树节点显示文本（含插值） |
| `RustSearchToolWindowFactory.kt` | 1 处 | ToolWindow content displayName（"Search"） |
| `plugin.xml` | 2 处 | Action 的 text/description |
| `RustSearchService.kt` | 4 处 | 面向用户的异常消息（UnsatisfiedLinkError/SearchException） |

### 2.3 关键技术约束
- IC-2023.1（`since-build=231`）支持 `DynamicBundle`（2020.1+ 引入）
- IntelliJ 约定：**默认 `messages.properties` 必须是英文**（作为 fallback），中文放 `messages_zh_CN.properties`
- `%key` 解析在 IDE 运行时自动完成，**必须先创建 bundle 再改 plugin.xml**，否则 `%key` 会原样显示
- 带插值的字符串（如 `"已找到 ${total} 个匹配"`）需在 properties 中用 `{0}/{1}` 占位符

## 三、决策与假设

### 决策
1. **P0 UI 文本 + P1 面向用户异常**：必做。这是用户直接看到的文本，是"语言自适应"的核心诉求。
2. **P2 日志消息**：不做国际化，但**顺便改为英文**便于排障（仅 4 处 `logger.info`/`error`，改动极小，符合"日志通常用英文"的行业惯例）。不创建 key。
3. **P3 插件市场描述（`<description>`/`<change-notes>`）**：不做。IntelliJ 对 CDATA HTML 的 `%key` 支持有限，且这是插件市场展示文本（非运行时 UI），保持中文不动。低风险。
4. **ToolWindow 显示名**：保持 `id="RustSearch"` 不变（侧边栏标题即 id）。不额外加 `text="%toolwindow.xxx"`。理由：`RustSearch` 是品牌名，中英文均显示 "RustSearch" 更一致；加 text 属性反而引入不必要的复杂度。
5. **Bundle 命名**：`com.example.rustsearch.messages`，放在 `src/main/resources/com/example/rustsearch/` 下（与包名对应，符合 IntelliJ 约定）。
6. **不改 build.gradle.kts**：Platform Gradle Plugin 2.x 自动将 `src/main/resources` 下的 `.properties` 打入 jar，无需配置。

### 假设
- IDE 语言由用户在 `Appearance & Behavior → System Settings → Language` 设置，插件无需主动检测，`DynamicBundle` 自动读取当前 locale。
- 英文为默认 fallback，缺失 `messages_zh_CN.properties` 时回退到英文（反之不可，故默认 bundle 必须英文完整）。

## 四、实施方案

### 步骤 1：创建 message bundle（2 个 properties 文件）

**新建目录**：`src/main/resources/com/example/rustsearch/`

**文件 1**：`messages.properties`（英文默认，必须存在且完整）

```properties
# Action
action.rustsearch.open.text=RustSearch: Global Text Search
action.rustsearch.open.description=Open RustSearch tool window

# Search panel - tooltips
search.field.tooltip=Enter search content (literal or regex)
search.regex.tooltip=Parse search content as regular expression
search.case.sensitive.tooltip=Case sensitive
search.whole.words.tooltip=Whole word matching (word boundary)
search.scope.project.tooltip=Search entire project root directory
search.scope.module.tooltip=Search selected module content roots
search.module.combo.tooltip=Select module to search
search.extension.checkbox.tooltip=Search .{0} files

# Search panel - labels
search.scope.label=Scope:
search.module.label=Module:
search.file.type.label=File type:
search.file.type.hint=(unchecked = all)

# Search panel - checkbox/radio text
search.regex.text=Regex
search.case.sensitive.text=Case
search.whole.words.text=Word
search.scope.project.text=Project
search.scope.module.text=Module

# Search panel - status messages
search.status.ready=Ready
search.status.empty.input=Please enter search content
search.status.no.roots=Cannot determine search root directory
search.status.searching=Searching... (Esc to cancel)
search.status.found=Found {0} matches ({1} files), {2}s
search.status.complete=Search complete: {0} matches ({1} files), {2}s
search.status.error=Search error: {0}
search.status.cancelled=Search cancelled: {0} matches
search.status.file.not.found=File not found: {0}

# ToolWindow content
toolwindow.content.name=Search

# Result tree
tree.file.node.display={0} ({1} matches)
tree.match.node.display=Line {0}: {1}

# Service exceptions
service.error.library.not.found=Native library resource not found: {0}, please ensure copyNativeLib task executed
service.error.copy.failed=Failed to copy native library to temp directory: {0}
service.error.search.start=Failed to start search, please check parameters or view log
service.error.unsupported.os=Unsupported operating system: {0}
```

**文件 2**：`messages_zh_CN.properties`（简体中文）

```properties
# Action
action.rustsearch.open.text=RustSearch: 全局文本搜索
action.rustsearch.open.description=打开 RustSearch 搜索工具窗口

# Search panel - tooltips
search.field.tooltip=输入搜索内容(字面量或正则)
search.regex.tooltip=将搜索内容作为正则表达式解析
search.case.sensitive.tooltip=区分大小写
search.whole.words.tooltip=全字匹配(单词边界)
search.scope.project.tooltip=搜索整个项目根目录
search.scope.module.tooltip=搜索选中模块的内容根目录
search.module.combo.tooltip=选择要搜索的模块
search.extension.checkbox.tooltip=搜索 .{0} 文件

# Search panel - labels
search.scope.label=作用域:
search.module.label=模块:
search.file.type.label=文件类型:
search.file.type.hint=(不勾选=全部)

# Search panel - checkbox/radio text
search.regex.text=正则
search.case.sensitive.text=大小写
search.whole.words.text=全字
search.scope.project.text=项目
search.scope.module.text=模块

# Search panel - status messages
search.status.ready=就绪
search.status.empty.input=请输入搜索内容
search.status.no.roots=无法确定搜索根目录
search.status.searching=搜索中...(Esc 取消)
search.status.found=已找到 {0} 个匹配({1} 个文件),{2}s
search.status.complete=搜索完成: {0} 个匹配({1} 个文件),耗时 {2}s
search.status.error=搜索出错: {0}
search.status.cancelled=搜索已取消: {0} 个匹配
search.status.file.not.found=文件不存在: {0}

# ToolWindow content
toolwindow.content.name=搜索

# Result tree
tree.file.node.display={0} ({1} 个匹配)
tree.match.node.display=行 {0}: {1}

# Service exceptions
service.error.library.not.found=找不到动态库资源: {0},请确保 copyNativeLib 任务已执行
service.error.copy.failed=拷贝动态库到临时目录失败: {0}
service.error.search.start=启动搜索失败,请检查参数或查看日志
service.error.unsupported.os=不支持的操作系统: {0}
```

### 步骤 2：创建 RustSearchBundle.kt

**新建文件**：`src/main/kotlin/com/example/rustsearch/RustSearchBundle.kt`

```kotlin
package com.example.rustsearch

import com.intellij.DynamicBundle
import org.jetbrains.annotations.PropertyKey

/**
 * RustSearch 插件消息包访问入口
 *
 * 基于 IntelliJ Platform 官方推荐的 DynamicBundle 实现,
 * 自动根据 IDE 当前语言(中文/英文)加载对应 messages_xx.properties。
 * 默认 bundle(messages.properties)为英文,作为 fallback。
 *
 * 调用示例:
 *   RustSearchBundle.message("search.status.found", matchCount, fileCount, elapsed)
 */
class RustSearchBundle private constructor() : DynamicBundle(BUNDLE) {
    companion object {
        @JvmField
        val INSTANCE = RustSearchBundle()

        private const val BUNDLE = "com.example.rustsearch.messages"

        @JvmStatic
        fun message(
            @PropertyKey(resourceBundle = BUNDLE) key: String,
            vararg params: Any
        ): String = INSTANCE.getMessage(key, *params)
    }
}
```

### 步骤 3：改造 plugin.xml（Action 文本 %key 化）

**文件**：`src/main/resources/META-INF/plugin.xml`

**改动**：第 48-49 行
```xml
<!-- 改前 -->
<action id="RustSearch.OpenSearch" class="..." text="RustSearch: 全局文本搜索" description="打开 RustSearch 搜索工具窗口">

<!-- 改后 -->
<action id="RustSearch.OpenSearch" class="..." text="%action.rustsearch.open.text" description="%action.rustsearch.open.description">
```

`<description>`/`<change-notes>` 保持中文不动（P3 决策）。

### 步骤 4：改造 RustSearchPanel.kt（24 处硬编码 → bundle 调用）

**文件**：`src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt`

**改动原则**：
- 静态文本：`JBCheckBox("正则")` → `JBCheckBox(RustSearchBundle.message("search.regex.text"))`
- tooltip：`toolTipText = "..."` → `toolTipText = RustSearchBundle.message("search.xxx.tooltip")`
- 状态消息（含插值）：`statusLabel.text = "已找到 ${total} 个匹配..."` → `statusLabel.text = RustSearchBundle.message("search.status.found", total, fileCount, elapsed)`
- 动态后缀复选框：`JBCheckBox(".$ext")` 文本本身是 `.kt` 这类技术标识符，**不国际化**（保持 `.$ext`）；仅 tooltip 国际化

**新增 import**：
```kotlin
import com.example.rustsearch.RustSearchBundle
```

**逐行映射表**（行号基于当前文件状态）：

| 原行号 | 原文本 | 改后 |
|--------|--------|------|
| 83 | `toolTipText = "输入搜索内容(字面量或正则)"` | `toolTipText = RustSearchBundle.message("search.field.tooltip")` |
| 87 | `JBCheckBox("正则")` | `JBCheckBox(RustSearchBundle.message("search.regex.text"))` |
| 88 | `toolTipText = "将搜索内容作为正则表达式解析"` | `toolTipText = RustSearchBundle.message("search.regex.tooltip")` |
| 92 | `JBCheckBox("大小写")` | `JBCheckBox(RustSearchBundle.message("search.case.sensitive.text"))` |
| 93 | `toolTipText = "区分大小写"` | `toolTipText = RustSearchBundle.message("search.case.sensitive.tooltip")` |
| 97 | `JBCheckBox("全字")` | `JBCheckBox(RustSearchBundle.message("search.whole.words.text"))` |
| 98 | `toolTipText = "全字匹配(单词边界)"` | `toolTipText = RustSearchBundle.message("search.whole.words.tooltip")` |
| 104 | `JRadioButton("项目", true)` | `JRadioButton(RustSearchBundle.message("search.scope.project.text"), true)` |
| 105 | `toolTipText = "搜索整个项目根目录"` | `toolTipText = RustSearchBundle.message("search.scope.project.tooltip")` |
| 109 | `JRadioButton("模块", false)` | `JRadioButton(RustSearchBundle.message("search.scope.module.text"), false)` |
| 110 | `toolTipText = "搜索选中模块的内容根目录"` | `toolTipText = RustSearchBundle.message("search.scope.module.tooltip")` |
| 121 | `toolTipText = "选择要搜索的模块"` | `toolTipText = RustSearchBundle.message("search.module.combo.tooltip")` |
| 135 | `toolTipText = "搜索 .$ext 文件"` | `toolTipText = RustSearchBundle.message("search.extension.checkbox.tooltip", ext)` |
| 147 | `JBLabel("就绪", ...)` | `JBLabel(RustSearchBundle.message("search.status.ready"), ...)` |
| 177 | `JBLabel("作用域:")` | `JBLabel(RustSearchBundle.message("search.scope.label"))` |
| 182 | `JBLabel("模块:")` | `JBLabel(RustSearchBundle.message("search.module.label"))` |
| 188 | `JBLabel("文件类型:")` | `JBLabel(RustSearchBundle.message("search.file.type.label"))` |
| 190 | `JBLabel("(不勾选=全部)")` | `JBLabel(RustSearchBundle.message("search.file.type.hint"))` |
| 298 | `statusLabel.text = "请输入搜索内容"` | `statusLabel.text = RustSearchBundle.message("search.status.empty.input")` |
| 305 | `statusLabel.text = "无法确定搜索根目录"` | `statusLabel.text = RustSearchBundle.message("search.status.no.roots")` |
| 327 | `statusLabel.text = "搜索中...(Esc 取消)"` | `statusLabel.text = RustSearchBundle.message("search.status.searching")` |
| 349 | `statusLabel.text = "已找到 ${...} 个匹配(${...} 个文件),${elapsed}s"` | `statusLabel.text = RustSearchBundle.message("search.status.found", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)` |
| 356 | `statusLabel.text = "搜索完成: ${...} 个匹配(${...} 个文件),耗时 ${elapsed}s"` | `statusLabel.text = RustSearchBundle.message("search.status.complete", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)` |
| 361 | `statusLabel.text = "搜索出错: ${e.message}"` | `statusLabel.text = RustSearchBundle.message("search.status.error", e.message ?: "")` |
| 399 | `statusLabel.text = "搜索已取消: ${...} 个匹配"` | `statusLabel.text = RustSearchBundle.message("search.status.cancelled", treeModel.getTotalMatches())` |
| 419 | `statusLabel.text = "文件不存在: ${data.filePath}"` | `statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)` |

### 步骤 5：改造 SearchResultTreeModel.kt（3 处）

**文件**：`src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt`

**新增 import**：
```kotlin
import com.example.rustsearch.RustSearchBundle
```

| 原行号 | 原文本 | 改后 |
|--------|--------|------|
| 113 | `return "$name ($matchCount 个匹配)"` | `return RustSearchBundle.message("tree.file.node.display", name, matchCount)` |
| 131 | `return "行 $lineNumber: $matchedText"` | `return RustSearchBundle.message("tree.match.node.display", lineNumber, matchedText)` |
| 173 | `text = "行 ${data.lineNumber}: ${data.matchedText}"` | `text = RustSearchBundle.message("tree.match.node.display", data.lineNumber, data.matchedText)` |

### 步骤 6：改造 RustSearchToolWindowFactory.kt（1 处）

**文件**：`src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt`

**新增 import**：
```kotlin
import com.example.rustsearch.RustSearchBundle
```

| 原行号 | 原文本 | 改后 |
|--------|--------|------|
| 29 | `createContent(panel, "Search", false)` | `createContent(panel, RustSearchBundle.message("toolwindow.content.name"), false)` |

### 步骤 7：改造 RustSearchService.kt（4 处面向用户异常 + 日志改英文）

**文件**：`src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt`

**新增 import**：
```kotlin
import com.example.rustsearch.RustSearchBundle
```

**面向用户异常（i18n）**：

| 原行号 | 原文本 | 改后 |
|--------|--------|------|
| 75 | `throw UnsatisfiedLinkError("找不到动态库资源: $resourcePath,请确保 copyNativeLib 任务已执行")` | `throw UnsatisfiedLinkError(RustSearchBundle.message("service.error.library.not.found", resourcePath))` |
| 88 | `throw UnsatisfiedLinkError("拷贝动态库到临时目录失败: ${e.message}")` | `throw UnsatisfiedLinkError(RustSearchBundle.message("service.error.copy.failed", e.message ?: ""))` |
| 134 | `throw SearchException("启动搜索失败,请检查参数或查看日志")` | `throw SearchException(RustSearchBundle.message("service.error.search.start"))` |
| 185 | `throw UnsatisfiedLinkError("不支持的操作系统: $osName")` | `throw UnsatisfiedLinkError(RustSearchBundle.message("service.error.unsupported.os", osName))` |

**日志消息改英文（不创建 key，直接改字符串）**：将第 71、98、100、137、156、168、195 行的中文日志改为英文。例如 `logger.info("动态库已加载: $libPath")` → `logger.info("Native library loaded: $libPath")`。这是 P2 决策的附带改动，便于排障。

## 五、验证步骤

### 5.1 编译验证
```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
~/.gradle/wrapper/dists/gradle-8.11.1-all/6gcpoccneql1b0krsle0llw37/gradle-8.11.1/bin/gradle compileKotlin --no-daemon
```
预期：BUILD SUCCESSFUL，无 `Unresolved reference: RustSearchBundle` 错误。

### 5.2 资源打包验证
```bash
unzip -l build/distributions/RustSearch-0.1.0.zip | grep "messages"
```
预期：jar 内含 `com/example/rustsearch/messages.properties` 和 `messages_zh_CN.properties`。

### 5.3 IDE 运行时验证（中文环境，默认）
启动 `runIde`，验证：
1. Tool Window 打开后，复选框显示「正则」「大小写」「全字」
2. 单选按钮显示「项目」「模块」
3. 标签显示「作用域:」「模块:」「文件类型:」「(不勾选=全部)」
4. 状态栏初始显示「就绪」
5. 执行搜索后显示「已找到 N 个匹配(M 个文件),Xs」
6. Action 菜单显示「RustSearch: 全局文本搜索」
7. 结果树文件节点显示「FileName.kt (3 个匹配)」
8. 结果树匹配节点显示「行 12: matchedText」

### 5.4 IDE 运行时验证（英文环境）
在 runIde 沙箱中切换 IDE 语言为英文（`Appearance & Behavior → System Settings → Language` → English），重启沙箱 IDE，验证：
1. 复选框显示「Regex」「Case」「Word」
2. 单选按钮显示「Project」「Module」
3. 状态栏显示「Ready」
4. 搜索后显示「Found N matches (M files), Xs」
5. Action 菜单显示「RustSearch: Global Text Search」
6. 结果树显示「FileName.kt (3 matches)」「Line 12: matchedText」

### 5.5 Fallback 验证
临时删除 `messages_zh_CN.properties` 中的某个 key（如 `search.regex.text`），在中文 IDE 下应 fallback 到英文「Regex」，不报错。

## 六、风险评估

| 风险 | 等级 | 缓解 |
|------|------|------|
| `%key` 未在 bundle 中定义，IDE 显示原始 `%xxx` | 中 | 先创建 bundle 再改 plugin.xml；编译后检查 jar 内 properties 完整性 |
| properties 文件中文乱码 | 低 | IntelliJ 默认 UTF-8 读取 properties；`.properties` 文件用 UTF-8 编码保存 |
| `DynamicBundle` 在 IC-2023.1 不可用 | 低 | DynamicBundle 自 2020.1 引入，231 版本已稳定支持 |
| 插值参数顺序错误 | 中 | 验证步骤 5.3/5.4 逐一核对带 `{0}/{1}` 的消息 |

## 七、改动文件清单

| 文件 | 操作 | 改动量 |
|------|------|--------|
| `src/main/resources/com/example/rustsearch/messages.properties` | 新建 | ~40 行 |
| `src/main/resources/com/example/rustsearch/messages_zh_CN.properties` | 新建 | ~40 行 |
| `src/main/kotlin/com/example/rustsearch/RustSearchBundle.kt` | 新建 | ~25 行 |
| `src/main/resources/META-INF/plugin.xml` | 编辑 | 2 行（Action text/description） |
| `src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt` | 编辑 | ~24 处替换 + 1 import |
| `src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt` | 编辑 | 3 处替换 + 1 import |
| `src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt` | 编辑 | 1 处替换 + 1 import |
| `src/main/kotlin/com/example/rustsearch/service/RustSearchService.kt` | 编辑 | 4 处异常 + ~7 处日志改英文 + 1 import |

**不改动**：`build.gradle.kts`、`gradle.properties`、`settings.gradle.kts`、`plugin.xml` 的 description/change-notes。

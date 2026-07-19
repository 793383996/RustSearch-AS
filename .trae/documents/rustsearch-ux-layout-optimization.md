# RustSearch 体验与排版优化计划

> 目标:让 RustSearch 体验对齐 Android Studio 内置 Find in Files。
> 用户已确认:需求 1 ToolWindow+预填自动搜、需求 2 完全对齐 Find in Files 渲染、需求 3 跳过。

---

## 一、Summary(摘要)

### 1.1 用户需求

1. **需求 1**:在编辑器选中文字后,按下 RustSearch 快捷键(`Cmd+Shift+Alt+F`),ToolWindow 自动打开,搜索框预填选中文字并立即触发搜索,显示结果
2. **需求 2**:结果树渲染对齐 Find in Files
   - 行号(左侧,灰色,固定宽度)
   - 代码行(中间,关键字黄色高亮 `STYLE_SEARCH_MATCH`)
   - 文件名(右侧,灰色,右对齐)
3. **需求 3**:跳过(已选 ToolWindow 模式)

### 1.2 当前状态

- `RustSearchAction.actionPerformed` 仅调用 `toolWindow.show()`,未读取选中文本,未预填搜索框,未自动触发搜索
- `SearchResultTreeCellRenderer` 继承 `DefaultTreeCellRenderer`,整行单色单文本,无高亮、无右对齐文件名
- `RustSearchPanel` 未暴露"预填并自动搜索"的公开方法
- Rust 侧 `matched_text` 字段已是整行文本(无需改 Rust)

### 1.3 修复策略(最小改动)

| 改动 | 文件 | 作用 |
|------|------|------|
| RustSearchAction 读选中文本 + 预填 + 自动搜 | RustSearchAction.kt | 需求 1 |
| RustSearchPanel 暴露 setInitialSearchText(text, autoTrigger) | RustSearchPanel.kt | 需求 1 |
| RustSearchToolWindowFactory 缓存 panel 引用供 Action 获取 | RustSearchToolWindowFactory.kt | 需求 1 |
| 重写 SearchResultTreeCellRenderer 继承 ColoredTreeCellRenderer | SearchResultTreeModel.kt | 需求 2 |
| SearchResultTreeModel 持有当前 pattern,传给 renderer 做高亮 | SearchResultTreeModel.kt | 需求 2 |
| FileNodeData/MatchNodeData 调整:文件节点也用左名+右路径 | SearchResultTreeModel.kt | 需求 2 |

---

## 二、Current State Analysis(当前状态分析)

### 2.1 RustSearchAction 现状

**文件**:[src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt)

```kotlin
class RustSearchAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project: Project = e.project ?: return
        val toolWindow = ToolWindowManager.getInstance(project)
            .getToolWindow("RustSearch") ?: return
        toolWindow.show()
    }
    override fun update(e: AnActionEvent) {
        e.presentation.isEnabled = e.project != null
    }
}
```

**问题**:
1. 不读取 `CommonDataKeys.EDITOR` + `selectionModel.selectedText`
2. `toolWindow.show()` 后无回调,无法预填搜索框
3. `update` 未动态修改 Action 文本(选中时显示"搜索选中文字")

### 2.2 RustSearchPanel 现状

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

- `performSearch()` 是 private,无外部入口
- `searchField.text = ...` 可直接预填,但无"预填+触发"的封装方法
- `treeModel` 是 private,Action 无法直接调用

### 2.3 RustSearchToolWindowFactory 现状

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt)

需要查看是否缓存 panel 引用。如果未缓存,需要新增缓存机制让 Action 能取到 panel。

### 2.4 SearchResultTreeCellRenderer 现状

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt#L257-L283](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

继承 `DefaultTreeCellRenderer`,只能整行单色单文本:
```kotlin
class SearchResultTreeCellRenderer : DefaultTreeCellRenderer() {
    override fun getTreeCellRendererComponent(...) {
        val comp = super.getTreeCellRendererComponent(...)
        when (val data = ...) {
            is FileNodeData -> {
                icon = AllIcons.FileTypes.Any_type
                text = data.displayName()  // 整行单文本
                foreground = ...
            }
            is MatchNodeData -> {
                text = RustSearchBundle.message("tree.match.node.display", ...)
                foreground = ...
            }
        }
    }
}
```

### 2.5 Rust 侧 matched_text 字段

**文件**:[rust-search/src/search/matcher.rs#L131-L133](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/matcher.rs)

```rust
let bytes = mat.bytes();  // 返回匹配所在行的完整字节
let matched_text = String::from_utf8_lossy(bytes).into_owned();
```

**关键发现**:`matched_text` 实际是**整行文本**(grep/ripgrep 的 `bytes()` 返回匹配所在行的全部内容),不是匹配片段。字段语义已对齐"整行文本",**无需修改 Rust 侧**。

---

## 三、Proposed Changes(精确改动方案)

### 3.1 改动 1:RustSearchToolWindowFactory 缓存 panel 引用

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt)

**改动**:
- 在 `createToolWindowContent` 中创建 panel 后,通过 `ToolWindow` 的 `putUserData` / `UserData` 机制缓存引用
- 或用伴生对象 `Map<Project, RustSearchPanel>` 缓存

**推荐方案**(用 UserData,生命周期跟随 ToolWindow):
```kotlin
class RustSearchToolWindowFactory : ToolWindowFactory {
    companion object {
        val PANEL_KEY: com.intellij.openapi.util.Key<RustSearchPanel> =
            com.intellij.openapi.util.Key.create("RustSearch.Panel")
    }

    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val panel = RustSearchPanel(project)
        val content = ContentFactory.getInstance().createContent(panel, "", false)
        toolWindow.contentManager.addContent(content)
        toolWindow.putUserData(PANEL_KEY, panel)
    }
    // ...
}
```

Action 通过 `toolWindow.getUserData(RustSearchToolWindowFactory.PANEL_KEY)` 获取 panel。

### 3.2 改动 2:RustSearchPanel 暴露预填方法

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**新增公开方法**:
```kotlin
/**
 * 预填搜索框并可选自动触发搜索
 *
 * 需求 1:供 RustSearchAction 在用户选中文本后调用,
 * 把选中文本填入搜索框并立即触发搜索,实现"选中+快捷键=自动搜索"。
 *
 * @param text 待预填的文本(选中文本);空字符串仅聚焦搜索框
 * @param autoTrigger 是否自动触发搜索(选中文字场景为 true)
 */
fun setInitialSearchText(text: String, autoTrigger: Boolean) {
    searchField.text = text
    searchField.requestFocusInWindow()
    if (autoTrigger && text.isNotBlank()) {
        performSearch()
    }
}
```

**关键点**:
- `performSearch()` 已是 private,本类内可直接调用
- `searchField.text = ...` 触发 DocumentListener 但不触发 ActionListener(回车),不会重复搜索
- `autoTrigger=true` 时显式调用 `performSearch()`

### 3.3 改动 3:RustSearchAction 读选中文本 + 预填 + 自动搜

**文件**:[src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt)

**修复后**:
```kotlin
class RustSearchAction : AnAction() {
    override fun actionPerformed(e: AnActionEvent) {
        val project: Project = e.project ?: return
        val toolWindow = ToolWindowManager.getInstance(project)
            .getToolWindow("RustSearch") ?: return

        // 需求 1:读取当前编辑器选中的文本
        val editor = e.getData(CommonDataKeys.EDITOR)
        val selectedText = editor?.selectionModel?.selectedText
            ?.takeIf { it.isNotBlank() && it.length <= 200 }

        toolWindow.show {
            // ToolWindow 显示后,获取 panel 并预填
            ApplicationManager.getApplication().invokeLater {
                val panel = toolWindow.getUserData(RustSearchToolWindowFactory.PANEL_KEY)
                if (panel != null && !selectedText.isNullOrBlank()) {
                    // 有选中文本:预填并自动搜索
                    panel.setInitialSearchText(selectedText, autoTrigger = true)
                } else if (panel != null) {
                    // 无选中文本:仅聚焦搜索框
                    panel.setInitialSearchText("", autoTrigger = false)
                }
            }
        }
    }

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabled = e.project != null
        // 需求 1:有选中文本时动态修改 Action 文本
        val editor = e.getData(CommonDataKeys.EDITOR)
        val hasSelection = editor?.selectionModel?.hasSelection() == true
        e.presentation.text = if (hasSelection) {
            RustSearchBundle.message("action.rustsearch.search.selection")
        } else {
            RustSearchBundle.message("action.rustsearch.open.text")
        }
    }
}
```

**关键点**:
- `toolWindow.show { ... }` 的回调在 ToolWindow 显示后执行
- 回调用 `invokeLater` 确保在 EDT 上操作 panel
- `selectedText.length <= 200` 限制:避免大段选中导致搜索卡顿
- `update` 中动态文本:让用户在菜单上看到"搜索选中文字"提示

### 3.4 改动 4:新增 messages 国际化 key

**文件**:
- [src/main/resources/com/example/rustsearch/messages.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages.properties)
- [src/main/resources/com/example/rustsearch/messages_zh_CN.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages_zh_CN.properties)

**新增**:
```properties
# messages.properties
action.rustsearch.search.selection=RustSearch: Search Selection

# messages_zh_CN.properties
action.rustsearch.search.selection=RustSearch: 搜索选中文字
```

### 3.5 改动 5:SearchResultTreeModel 持有 pattern

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**目的**:让 renderer 能拿到当前搜索词做关键字高亮。

**改动**:
```kotlin
class SearchResultTreeModel : DefaultTreeModel(DefaultMutableTreeNode("root")) {
    // ... 现有字段 ...

    /** 当前搜索词(用于 renderer 高亮关键字),由 RustSearchPanel 在 performSearch 时设置 */
    private var currentPattern: String = ""

    /**
     * 设置当前搜索词(供 renderer 做关键字高亮)
     *
     * @param pattern 搜索词(字面量或正则源串);空字符串表示无高亮
     */
    fun setCurrentPattern(pattern: String) {
        currentPattern = pattern
    }

    /** 获取当前搜索词(供 renderer 使用) */
    fun getCurrentPattern(): String = currentPattern
    // ...
}
```

**RustSearchPanel.performSearch 调整**:
在 `treeModel.clear()` 后调用 `treeModel.setCurrentPattern(pattern)`。

### 3.6 改动 6:重写 SearchResultTreeCellRenderer

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**重写后**:
```kotlin
/**
 * 搜索结果树单元格渲染器
 *
 * 需求 2:对齐 Find in Files 视觉布局
 * - 文件节点:文件图标 + 文件名(左) + 匹配数(右对齐,灰色)
 * - 匹配节点:行号(左,5位宽,灰色) + 代码行(中,关键字高亮) + 文件名(右对齐,灰色)
 *
 * 基于 SimpleColoredComponent 的 Fragment 体系,支持一行多色多对齐。
 */
class SearchResultTreeCellRenderer(
    private val patternProvider: () -> String = { "" }
) : ColoredTreeCellRenderer() {

    override fun customizeCellRenderer(
        tree: JTree, value: Any, selected: Boolean, expanded: Boolean,
        leaf: Boolean, row: Int, hasFocus: Boolean
    ) {
        clear()
        val node = value as? DefaultMutableTreeNode ?: return
        when (val data = node.userObject) {
            is FileNodeData -> renderFileNode(data)
            is MatchNodeData -> renderMatchNode(data, selected)
        }
    }

    private fun renderFileNode(data: FileNodeData) {
        icon = AllIcons.FileTypes.Any_type
        // 左:文件名
        val sep = data.filePath.lastIndexOf('/')
        val fileName = if (sep >= 0) data.filePath.substring(sep + 1) else data.filePath
        append(fileName, SimpleTextAttributes.REGULAR_ATTRIBUTES)
        // 右:匹配数(灰色,右对齐)
        append("  (${data.matchCount} 个匹配)", SimpleTextAttributes.GRAYED_ATTRIBUTES)
    }

    private fun renderMatchNode(data: MatchNodeData, selected: Boolean) {
        // 左:行号(5 位宽度,灰色)
        append(String.format("%5d: ", data.lineNumber), SimpleTextAttributes.GRAYED_ATTRIBUTES)

        // 中:代码行(关键字高亮)
        val pattern = patternProvider()
        if (pattern.isNotEmpty()) {
            appendWithHighlight(data.matchedText, pattern, selected)
        } else {
            append(data.matchedText, SimpleTextAttributes.REGULAR_ATTRIBUTES)
        }

        // 右:文件名(灰色,右对齐)
        append("    ")
        val sep = data.filePath.lastIndexOf('/')
        val fileName = if (sep >= 0) data.filePath.substring(sep + 1) else data.filePath
        append(fileName, SimpleTextAttributes.GRAYED_ATTRIBUTES, /* tag */ null, /* rightAligned */ true)
    }

    /** 关键字高亮:简单实现,在 matchedText 中查找 pattern 的所有出现并高亮 */
    private fun appendWithHighlight(text: String, pattern: String, selected: Boolean) {
        if (pattern.isEmpty()) {
            append(text, SimpleTextAttributes.REGULAR_ATTRIBUTES)
            return
        }
        val baseAttr = if (selected) SimpleTextAttributes.SELECTED_ATTRIBUTES
                       else SimpleTextAttributes.REGULAR_ATTRIBUTES
        val matchAttr = SimpleTextAttributes(
            baseAttr.style or SimpleTextAttributes.STYLE_SEARCH_MATCH,
            baseAttr.fgColor,
            JBColor.YELLOW,
            baseAttr.waveColor,
            baseAttr.fontType
        )

        // 简单字面量查找(忽略大小写);正则模式暂不高亮(避免误高亮)
        val lowerText = text.lowercase()
        val lowerPattern = pattern.lowercase()
        var start = 0
        while (true) {
            val idx = lowerText.indexOf(lowerPattern, start)
            if (idx < 0) {
                append(text.substring(start), baseAttr)
                break
            }
            if (idx > start) {
                append(text.substring(start, idx), baseAttr)
            }
            append(text.substring(idx, idx + pattern.length), matchAttr)
            start = idx + pattern.length
        }
    }
}
```

**关键点**:
1. 继承 `ColoredTreeCellRenderer`,获得 `append(String, SimpleTextAttributes, tag, rightAligned)` 能力
2. `patternProvider: () -> String` 让 renderer 能动态拿到当前搜索词(从 `SearchResultTreeModel.getCurrentPattern()`)
3. `STYLE_SEARCH_MATCH` 是 IntelliJ 内置的搜索匹配样式(黄色背景)
4. 右对齐文件名用 `append(fragment, attr, tag, rightAligned = true)`
5. 简单字面量高亮:正则模式暂不高亮(避免 `\b` 等元字符被当字面量误高亮),只对字面量搜索做高亮
6. `clear()` 在每次渲染前清空(SimpleColoredComponent 复用机制)

### 3.7 改动 7:SearchResultTreeModel 实例化 renderer 时传入 patternProvider

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**问题**:`SearchResultTreeCellRenderer` 现在需要 `patternProvider` 参数,但当前在 `RustSearchPanel` 中通过 `setCellRenderer(SearchResultTreeCellRenderer())` 实例化。

**改动**:
```kotlin
// RustSearchPanel.kt 中
private val treeModel = SearchResultTreeModel()

private val resultTree = JTree(treeModel).apply {
    isRootVisible = false
    showsRootHandles = true
    selectionModel.selectionMode = TreeSelectionModel.SINGLE_TREE_SELECTION
    setCellRenderer(SearchResultTreeCellRenderer(patternProvider = { treeModel.getCurrentPattern() }))
}
```

---

## 四、Assumptions & Decisions(假设与决策)

### 4.1 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| 需求 1 实现方式 | ToolWindow + 预填自动搜 | 用户确认;改动最小,不引入弹窗复杂度 |
| 需求 1 文本长度限制 | ≤ 200 字符 | 避免大段选中导致搜索卡顿 |
| 需求 1 触发时机 | `toolWindow.show { ... }` 回调 | 确保 ToolWindow 已显示再操作 panel |
| 需求 1 panel 引用传递 | `toolWindow.putUserData` | 生命周期跟随 ToolWindow,自动回收 |
| 需求 2 渲染器基类 | `ColoredTreeCellRenderer` | IntelliJ 官方多色多对齐渲染器 |
| 需求 2 关键字高亮样式 | `STYLE_SEARCH_MATCH`(黄色背景) | 对齐 Find in Files 视觉 |
| 需求 2 高亮范围 | 仅字面量搜索 | 正则高亮复杂且易误高亮,首版不做 |
| 需求 2 行号格式 | `%5d: ` | 5 位宽度,避免行号变化导致抖动 |
| 需求 2 文件名右对齐 | `append(..., rightAligned = true)` | SimpleColoredComponent 内置支持 |
| Rust 侧 matched_text | 不改 | 已是整行文本,字段语义对齐 |
| 需求 3 弹窗 | 不做 | 用户确认与需求 1 联动,选 ToolWindow 模式 |

### 4.2 不做的事

- **不引入 JBPopup 弹窗模式**:用户选 ToolWindow 模式
- **不修改 Rust 侧**:matched_text 已是整行文本
- **不做正则高亮**:正则元字符高亮复杂,首版仅支持字面量高亮
- **不替换 JTree 为 AsyncListing**:保留按文件分组树形结构,改动最小
- **不新增 Action 快捷键**:沿用现有 `Cmd+Shift+Alt+F`
- **不引入 SpeedSearch 二次过滤**:首版只做搜索词高亮,不做结果再筛

### 4.3 假设

1. **假设** `ColoredTreeCellRenderer.append(fragment, attr, tag, rightAligned)` 重载在 IC-231~261 都可用(编译期 IC-231 验证)
2. **假设** `toolWindow.show(Runnable)` 回调在 ToolWindow 完全显示后执行
3. **假设** `SimpleTextAttributes.STYLE_SEARCH_MATCH` 在 IC-231~261 都可用
4. **假设** `editor.selectionModel.selectedText` 在 Action 触发时返回当前选中文本(无 PSI 依赖)
5. **假设** 用户选中文本长度合理(<200 字符),超长时不预填

---

## 五、Verification(验证方案)

### 5.1 编译验证

```bash
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew buildPlugin
```

### 5.2 功能验证场景

| 场景 | 操作 | 预期 |
|------|------|------|
| 需求 1:选中文本+快捷键 | 在编辑器选中 `shouldForward` → 按 `Cmd+Shift+Alt+F` | ToolWindow 打开,搜索框预填 `shouldForward`,立即显示搜索结果 |
| 需求 1:无选中+快捷键 | 编辑器无选中 → 按 `Cmd+Shift+Alt+F` | ToolWindow 打开,搜索框为空并聚焦,无自动搜索 |
| 需求 1:超长选中 | 选中 > 200 字符 → 按快捷键 | ToolWindow 打开,搜索框为空(不预填超长文本) |
| 需求 1:Action 文本动态 | 选中文字 → 查看 Edit → Find 菜单 | Action 显示"RustSearch: 搜索选中文字" |
| 需求 2:文件节点渲染 | 任意搜索 | 文件节点:图标+文件名+匹配数(灰色) |
| 需求 2:匹配节点渲染 | 任意字面量搜索 | 行号(灰色5位)+ 代码行(关键字黄色高亮) + 文件名(右对齐灰色) |
| 需求 2:正则搜索渲染 | 勾选正则 → 搜索 `should\w+` | 行号+代码行(无高亮,因正则不高亮)+ 文件名 |
| 需求 2:多次搜索 | 连续搜索多个词 | 高亮正确切换,无残留 |

### 5.3 视觉验证

参考 Android Studio 内置 Find in Files(`Cmd+Shift+F`)的渲染效果对比:
- 行号位置、颜色
- 关键字高亮颜色、范围
- 文件名位置、对齐方式

### 5.4 残余风险

1. **正则搜索不高亮**:首版仅字面量高亮,正则搜索时代码行无高亮(但行号+文件名仍正确)
2. **跨平台颜色差异**:`STYLE_SEARCH_MATCH` 在不同主题(Light/Dark)下颜色可能略异,但符合 IntelliJ 主题
3. **大结果集性能**:每次渲染都做字面量查找,50000 匹配时可能有性能影响(但 JTree 按需渲染,实际可见行数有限)
4. **UserData 生命周期**:ToolWindow 销毁后 panel 引用自动失效,无内存泄漏

---

## 六、实施顺序

| 顺序 | 改动 | 文件 | 验证 |
|------|------|------|------|
| 1 | ToolWindowFactory 缓存 panel 引用(PANEL_KEY) | RustSearchToolWindowFactory.kt | 编译通过 |
| 2 | RustSearchPanel 暴露 setInitialSearchText | RustSearchPanel.kt | 编译通过 |
| 3 | RustSearchAction 读选中文本 + 预填 + 自动搜 + update 动态文本 | RustSearchAction.kt | 编译通过 |
| 4 | 新增 messages key(action.rustsearch.search.selection) | messages.properties + messages_zh_CN.properties | 编译通过 |
| 5 | SearchResultTreeModel 持有 currentPattern + setCurrentPattern/getCurrentPattern | SearchResultTreeModel.kt | 编译通过 |
| 6 | 重写 SearchResultTreeCellRenderer 继承 ColoredTreeCellRenderer | SearchResultTreeModel.kt | 编译通过 |
| 7 | RustSearchPanel 实例化 renderer 时传 patternProvider + performSearch 设置 pattern | RustSearchPanel.kt | 编译通过 |
| 8 | 全量编译 + buildPlugin | - | `./gradlew buildPlugin` 通过 |
| 9 | 安装到 AS 手动验证 | - | 需求 1+2 场景通过 |

---

## 七、附录:文件改动清单

### Kotlin 侧(4 文件)

1. [src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/action/RustSearchAction.kt)
   - 读 `CommonDataKeys.EDITOR` + `selectionModel.selectedText`
   - `toolWindow.show { ... }` 回调中预填 panel
   - `update` 动态修改 Action 文本

2. [src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)
   - 新增 `setInitialSearchText(text, autoTrigger)` 公开方法
   - `performSearch` 中调用 `treeModel.setCurrentPattern(pattern)`
   - `setCellRenderer` 传 `patternProvider`

3. [src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchToolWindowFactory.kt)
   - 新增 `PANEL_KEY` companion 字段
   - `createToolWindowContent` 中 `toolWindow.putUserData(PANEL_KEY, panel)`

4. [src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)
   - 新增 `currentPattern` 字段 + `setCurrentPattern`/`getCurrentPattern`
   - 重写 `SearchResultTreeCellRenderer` 继承 `ColoredTreeCellRenderer`

### 资源文件(2 文件)

5. [src/main/resources/com/example/rustsearch/messages.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages.properties)
   - 新增 `action.rustsearch.search.selection`

6. [src/main/resources/com/example/rustsearch/messages_zh_CN.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages_zh_CN.properties)
   - 新增 `action.rustsearch.search.selection`

**总计**:4 Kotlin 文件 + 2 资源文件,无 Rust 改动。

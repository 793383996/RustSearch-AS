package com.example.rustsearch.ui

import com.example.rustsearch.RustSearchBundle
import com.example.rustsearch.RustSearchEngine.SearchResult
import com.intellij.icons.AllIcons
import com.intellij.openapi.diagnostic.Logger
import com.intellij.ui.ColoredTreeCellRenderer
import com.intellij.ui.JBColor
import com.intellij.ui.SimpleTextAttributes
import com.intellij.util.ui.UIUtil
import java.awt.Component
import javax.swing.Icon
import javax.swing.JLabel
import javax.swing.JTree
import javax.swing.SwingUtilities
import javax.swing.tree.DefaultMutableTreeNode
import javax.swing.tree.DefaultTreeModel

/**
 * 搜索结果树模型
 *
 * 按文件分组展示匹配结果:
 * - 根节点(隐藏)
 *   - 文件节点(显示路径 + 匹配数)
 *     - 匹配节点(显示行号:列 | 匹配内容)
 *     - 匹配节点
 *   - 文件节点
 *
 * 支持增量添加结果(流式搜索每收到一批就追加),避免全量重建树。
 *
 * M2:UI 侧结果上限保护,达到 MAX_TOTAL_MATCHES_UI 或 MAX_FILE_NODES_UI 时
 * 停止追加并标记 truncated,调用方通过 isTruncated() 显示截断提示,
 * 防止超大结果集(如搜索 `import` 在 AOSP 子模块)导致 Swing 树内存爆炸。
 */
class SearchResultTreeModel : DefaultTreeModel(DefaultMutableTreeNode("root")) {

    companion object {
        /** M2:UI 侧总匹配数上限,超过则停止追加(防止 Swing 树内存爆炸) */
        private const val MAX_TOTAL_MATCHES_UI = 50_000
        /** M2:文件节点数上限,超过则停止追加(Swing JTree 超过 5000 节点渲染卡顿) */
        private const val MAX_FILE_NODES_UI = 5_000
        /** 诊断日志:验证线程与令牌时序(修复后应全部 EDT + 无滞后) */
        private val LOGGER = Logger.getInstance(SearchResultTreeModel::class.java)
    }

    /** 文件路径 → 文件节点(便于增量追加) */
    private val fileNodeMap = mutableMapOf<String, DefaultMutableTreeNode>()

    /** 总匹配数 */
    private var totalMatches = 0

    /** M2:是否已触发截断(触发后拒绝后续 batch,调用方应显示截断提示) */
    private var truncated = false

    /**
     * 需求 2:当前搜索词(用于 renderer 关键字高亮)
     *
     * 由 RustSearchPanel.performSearch 在搜索前设置,
     * SearchResultTreeCellRenderer 通过 patternProvider 读取,
     * 在代码行中高亮所有字面量匹配片段(STYLE_SEARCH_MATCH 黄色背景)。
     * 正则搜索时不高亮(避免元字符误匹配)。
     */
    private var currentPattern: String = ""

    /**
     * 需求 2:设置当前搜索词(供 renderer 做关键字高亮)
     *
     * @param pattern 搜索词(字面量或正则源串);空字符串表示无高亮
     */
    fun setCurrentPattern(pattern: String) {
        currentPattern = pattern
    }

    /** 需求 2:获取当前搜索词(供 renderer 使用) */
    fun getCurrentPattern(): String = currentPattern

    /**
     * 追加一批搜索结果
     *
     * P1-2:改用 `nodesWereInserted` 精准通知插入的节点,替代 `reload()` 全量刷新。
     * 复杂度从 O(N²)(reload + expandPath 循环)降到 O(N),保留展开状态。
     *
     * M2:达到 MAX_TOTAL_MATCHES_UI 或 MAX_FILE_NODES_UI 时停止追加,
     * 标记 truncated=true,调用方通过 isTruncated() 显示截断提示。
     *
     * @param results 一批匹配结果
     */
    fun addResults(results: List<SearchResult>) {
        // 诊断日志:验证线程与令牌时序(修复后应全部 EDT + 无滞后)
        LOGGER.info(
            "addResults: batch=${results.size}, totalBefore=$totalMatches, " +
            "filesBefore=${fileNodeMap.size}, isEDT=${SwingUtilities.isEventDispatchThread()}, " +
            "thread=${Thread.currentThread().name}"
        )

        // M2:截断检查 — 已截断后拒绝后续 batch
        if (truncated) return
        if (totalMatches >= MAX_TOTAL_MATCHES_UI || fileNodeMap.size >= MAX_FILE_NODES_UI) {
            truncated = true
            LOGGER.info("addResults: truncated triggered, total=$totalMatches, files=${fileNodeMap.size}")
            return
        }

        val root = root as DefaultMutableTreeNode
        // 记录新插入的节点(parent, childIndex),用于精准通知
        val insertedNodes = mutableListOf<Pair<DefaultMutableTreeNode, Int>>()
        // P1-A:收集本批受影响的文件节点(去重),只对这些节点调用 nodeChanged
        val affectedFileNodes = LinkedHashSet<DefaultMutableTreeNode>()

        for ((idx, result) in results.withIndex()) {
            val isNewFile = !fileNodeMap.containsKey(result.filePath)
            val fileNode = fileNodeMap.getOrPut(result.filePath) {
                val node = DefaultMutableTreeNode(FileNodeData(result.filePath, 0))
                root.add(node)
                node
            }

            // 新文件节点加入通知队列
            if (isNewFile) {
                insertedNodes.add(root to (root.childCount - 1))
            }

            val fileData = fileNode.userObject as FileNodeData
            fileData.matchCount++
            affectedFileNodes.add(fileNode) // P1-A:收集受影响节点

            val matchNode = DefaultMutableTreeNode(
                MatchNodeData(
                    filePath = result.filePath,
                    lineNumber = result.lineNumber,
                    column = result.column,
                    matchedText = result.matchedText,
                    contextBefore = result.contextBefore,
                    contextAfter = result.contextAfter
                ),
                true // allowsChildren = false,叶子节点
            )
            val matchIndex = fileNode.childCount
            fileNode.add(matchNode)
            insertedNodes.add(fileNode to matchIndex)
            totalMatches++

            // 诊断日志:首条结果内容(便于确认 filePath/matchedText 是否非空)
            if (idx == 0) {
                LOGGER.info(
                    "addResults first item: filePath='${result.filePath}', line=${result.lineNumber}, " +
                    "matched='${result.matchedText.take(80)}', " +
                    "contextBefore=${result.contextBefore.size}, contextAfter=${result.contextAfter.size}"
                )
            }
        }

        // 精准通知插入(替代 reload 全量刷新),保留其他节点展开状态
        for ((parent, index) in insertedNodes) {
            val childIndices = intArrayOf(index)
            nodesWereInserted(parent, childIndices)
        }

        // P1-A:只通知受影响文件节点(替代原 fileNodeMap.values 全量遍历)
        // 复杂度从 O(总文件数) 降到 O(本批文件数)
        for (fileNode in affectedFileNodes) {
            nodeChanged(fileNode)
        }

        // 默认展开文件节点:让用户直接看到匹配内容,无需手动点击展开。
        // isRootVisible=false 时 JTree 不会自动展开,需显式调用 expandPath。
        // model 不持有 JTree 引用,展开由 RustSearchPanel 在 addResults 后统一处理。
        // 诊断日志:addResults 后状态(用于验证 root childCount 与 fileNodeMap 一致)
        LOGGER.info(
            "addResults done: totalAfter=$totalMatches, filesAfter=${fileNodeMap.size}, " +
            "rootChildCount=${root.childCount}, insertedNodes=${insertedNodes.size}"
        )
    }

    /**
     * 获取所有文件节点的 TreePath(用于 JTree 展开)
     *
     * 调用方(RustSearchPanel)在 addResults 后调用此方法,
     * 对返回的路径列表调用 JTree.expandPath 展开所有文件节点。
     *
     * @return 文件节点 TreePath 列表(root → fileNode)
     */
    fun getFileNodePaths(): List<javax.swing.tree.TreePath> {
        val root = root as DefaultMutableTreeNode
        return (0 until root.childCount).map { i ->
            javax.swing.tree.TreePath(arrayOf(root, root.getChildAt(i)))
        }
    }

    /**
     * 清空所有结果
     *
     * M2:同时重置 truncated 标志,允许新搜索正常追加
     */
    fun clear() {
        // 诊断日志:验证线程与令牌时序(修复后应全部 EDT)
        LOGGER.info(
            "clear: totalBefore=$totalMatches, filesBefore=${fileNodeMap.size}, " +
            "isEDT=${SwingUtilities.isEventDispatchThread()}, thread=${Thread.currentThread().name}"
        )
        val root = root as DefaultMutableTreeNode
        root.removeAllChildren()
        fileNodeMap.clear()
        totalMatches = 0
        truncated = false  // M2:重置截断标志
        reload()
    }

    /**
     * 获取总匹配数
     */
    fun getTotalMatches(): Int = totalMatches

    /**
     * 获取文件数
     */
    fun getFileCount(): Int = fileNodeMap.size

    /** M2:是否已触发截断(达到上限,后续 batch 被拒绝) */
    fun isTruncated(): Boolean = truncated
}

/**
 * 文件节点数据
 */
data class FileNodeData(
    val filePath: String,
    var matchCount: Int
) {
    /**
     * 获取用于显示的文件名(不含目录路径)
     */
    fun displayName(): String {
        val sep = filePath.lastIndexOf('/')
        val name = if (sep >= 0) filePath.substring(sep + 1) else filePath
        return RustSearchBundle.message("tree.file.node.display", name, matchCount)
    }

    override fun toString(): String = displayName()
}

/**
 * 匹配节点数据
 */
data class MatchNodeData(
    val filePath: String,
    val lineNumber: Int,
    val column: Int,
    val matchedText: String,
    val contextBefore: Array<String>,
    val contextAfter: Array<String>
) {
    override fun toString(): String {
        return RustSearchBundle.message("tree.match.node.display", lineNumber, matchedText)
    }

    override fun equals(other: Any?): Boolean {
        if (this === other) return true
        if (other !is MatchNodeData) return false
        return filePath == other.filePath &&
                lineNumber == other.lineNumber &&
                column == other.column
    }

    override fun hashCode(): Int {
        var result = filePath.hashCode()
        result = 31 * result + lineNumber
        result = 31 * result + column
        return result
    }
}

/**
 * 搜索结果树单元格渲染器
 *
 * 需求 2:对齐 Android Studio Find in Files 视觉布局
 * - 文件节点(FileNodeData): 文件图标 + 文件名(左) + 匹配数(右对齐,灰色)
 * - 匹配节点(MatchNodeData): 行号(左,5位宽,灰色) + 代码行(中,关键字黄色高亮) + 文件名(右对齐,灰色)
 *
 * 基于 SimpleColoredComponent 的 Fragment 体系,支持一行多色多对齐,
 * 替代原 DefaultTreeCellRenderer 的整行单色单文本。
 *
 * @param patternProvider 返回当前搜索词的回调(从 SearchResultTreeModel.getCurrentPattern() 读取),
 *                        用于在匹配节点代码行中高亮关键字。正则模式时调用方应返回空字符串(不高亮)。
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
            is MatchNodeData -> renderMatchNode(data)
            else -> {
                // 根节点或其他,留空
            }
        }
    }

    /**
     * 渲染文件节点:文件图标 + 文件名(左) + 匹配数(真右对齐,灰色)
     *
     * 注:selected 态由 ColoredTreeCellRenderer 父类(SimpleColoredComponent)
     * 通过 JTree selectionBackground 自动渲染,文字颜色统一用 REGULAR_ATTRIBUTES。
     */
    private fun renderFileNode(data: FileNodeData) {
        icon = AllIcons.FileTypes.Any_type
        // 左:文件名
        val sep = data.filePath.lastIndexOf('/')
        val fileName = if (sep >= 0) data.filePath.substring(sep + 1) else data.filePath
        append(fileName, SimpleTextAttributes.REGULAR_ATTRIBUTES)
        // 右:匹配数(真右对齐,灰色)
        // 231 SDK: SimpleColoredComponent.append(text, attr, rightAligned:boolean)
        append("(${data.matchCount})", SimpleTextAttributes.GRAYED_ATTRIBUTES, true)
    }

    /**
     * 渲染匹配节点:行号(左,5位宽,灰色) + 代码行(中,关键字高亮)
     *
     * 需求 2(用户反馈):右侧文件名已移除,因为匹配节点已在对应文件节点下,
     * 文件名重复显示冗余。仅保留行号 + 代码行 + 关键字高亮。
     */
    private fun renderMatchNode(data: MatchNodeData) {
        // 左:行号(5 位宽度,灰色)
        append(String.format("%5d: ", data.lineNumber), SimpleTextAttributes.GRAYED_ATTRIBUTES)

        // 中:代码行(关键字高亮)
        val pattern = patternProvider()
        if (pattern.isNotEmpty()) {
            appendWithHighlight(data.matchedText, pattern)
        } else {
            append(data.matchedText, SimpleTextAttributes.REGULAR_ATTRIBUTES)
        }
    }

    /**
     * 关键字高亮:在 text 中查找 pattern 的所有出现(忽略大小写),
     * 匹配区间用 STYLE_SEARCH_MATCH(黄色背景)高亮,非匹配区间用 REGULAR_ATTRIBUTES。
     *
     * 仅做字面量查找;正则模式由调用方传空 pattern 跳过高亮。
     *
     * 注:231 SDK 的 SimpleTextAttributes 4 参数构造函数签名为
     * `(fgColor, bgColor, waveColor, style)`,无 5 参数版本,无 fontType 字段。
     */
    private fun appendWithHighlight(text: String, pattern: String) {
        val baseAttr = SimpleTextAttributes.REGULAR_ATTRIBUTES
        val matchAttr = SimpleTextAttributes(
            baseAttr.fgColor,
            JBColor.YELLOW,
            baseAttr.waveColor,
            baseAttr.style or SimpleTextAttributes.STYLE_SEARCH_MATCH
        )

        val lowerText = text.lowercase()
        val lowerPattern = pattern.lowercase()
        var start = 0
        while (true) {
            val idx = lowerText.indexOf(lowerPattern, start)
            if (idx < 0) {
                if (start < text.length) {
                    append(text.substring(start), baseAttr)
                }
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

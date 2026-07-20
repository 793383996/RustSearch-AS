package com.example.rustsearch.ui

import com.example.rustsearch.RustSearchEngine.SearchResult
import com.example.rustsearch.SearchConfig
import com.example.rustsearch.RustSearchBundle
import com.example.rustsearch.service.RustSearchService
import com.intellij.icons.AllIcons
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.application.ReadAction
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.fileEditor.OpenFileDescriptor
import com.intellij.openapi.module.Module
import com.intellij.openapi.module.ModuleManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.roots.ModuleRootManager
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.ui.components.JBCheckBox
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBTextField
import com.intellij.util.ui.JBUI
import kotlinx.coroutines.*
import java.awt.BorderLayout
import java.awt.Dimension
import java.awt.FlowLayout
import java.awt.event.MouseAdapter
import java.awt.event.MouseEvent
import javax.swing.Box
import javax.swing.BoxLayout
import javax.swing.ButtonGroup
import javax.swing.Icon
import javax.swing.JButton
import javax.swing.JComboBox
import javax.swing.JPanel
import javax.swing.JRadioButton
import javax.swing.JScrollPane
import javax.swing.JTree
import javax.swing.KeyStroke
import javax.swing.SwingConstants
import javax.swing.tree.DefaultMutableTreeNode
import javax.swing.tree.TreeSelectionModel

/**
 * RustSearch 搜索面板
 *
 * UI 布局:
 * ```
 * ┌──────────────────────────────────────────────────────────────┐
 * │ [搜索输入          ] [正则][大小写][全字]                     │ ← 搜索栏
 * │ 作用域:(●) 项目  ( ) 模块:[模块选择▼]                        │ ← 作用域(单选)
 * │ 文件类型:[.kt][.java][.xml][.gradle][.kts][.properties]...   │ ← 后缀过滤
 * ├──────────────────────────────────────────────────────────────┤
 * │ 找到 N 个匹配,耗时 T 秒                                      │ ← 状态栏
 * ├──────────────────────────────────────────────────────────────┤
 * │ ▼ File.kt (3 matches)                                       │
 * │   12:  matched content                                       │ ← 结果树(默认展开)
 * │   45:  another match                                        │
 * └──────────────────────────────────────────────────────────────┘
 * ```
 *
 * 交互逻辑:
 * - 搜索触发:回车搜索;任意筛选条件变化(正则/大小写/全字/作用域/模块/后缀)自动重新搜索
 * - 作用域:项目(整个项目) / 模块(选中模块的 contentRoots);选中模块时启用模块下拉框
 * - 文件类型:不勾选=搜索全部文件;勾选=仅搜索勾选的后缀
 * - Esc:取消正在进行的搜索
 * - 双击树节点:通过 OpenFileDescriptor 打开文件并定位到行
 * - 结果树:流式追加结果后自动展开所有文件节点
 */
class RustSearchPanel(private val project: Project) : JPanel(BorderLayout()), Disposable {

    private val logger = Logger.getInstance(RustSearchPanel::class.java)

    private val service = RustSearchService.getInstance()

    /** 搜索协程作用域,独立于 UI 线程 */
    private val searchScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /** 当前搜索任务,用于取消 */
    private var searchJob: Job? = null

    /** 当前搜索令牌,用于丢弃滞后 EDT 任务(根因 A 修复)
     *
     * performSearch 每次 ++ 生成新令牌;collect 块捕获该令牌,
     * invokeLater 回调中校验 currentToken == activeSearchToken,
     * 不等则说明该 batch 属于已被取消的旧搜索,直接丢弃。
     * 仅在 EDT 读写(performSearch 由 EDT 触发,invokeLater 回调在 EDT),无需同步原语。
     */
    private var activeSearchToken: Long = 0L

    /** 模块列表刷新中标志,避免 addItem 触发 autoSearch(P1-3) */
    private var isRefreshingModules = false

    /** 结果树模型 */
    private val treeModel = SearchResultTreeModel()

    // ==================== UI 组件 ====================

    /** 搜索模式输入框 */
    private val searchField = JBTextField().apply {
        preferredSize = Dimension(300, 30)
        toolTipText = RustSearchBundle.message("search.field.tooltip")
    }

    /** 正则模式开关(图标 toggle,对齐 Find in Path 风格) */
    private val regexButton = IconToggleButton(
        AllIcons.Actions.Regex,
        AllIcons.Actions.RegexHovered,
        AllIcons.Actions.RegexSelected,
        RustSearchBundle.message("search.regex.tooltip")
    )

    /** 大小写敏感开关(图标 toggle) */
    private val caseSensitiveButton = IconToggleButton(
        AllIcons.Actions.MatchCase,
        AllIcons.Actions.MatchCaseHovered,
        AllIcons.Actions.MatchCaseSelected,
        RustSearchBundle.message("search.case.sensitive.tooltip")
    )

    /** 全字匹配开关(图标 toggle) */
    private val wholeWordsButton = IconToggleButton(
        AllIcons.Actions.Words,
        AllIcons.Actions.WordsHovered,
        AllIcons.Actions.WordsSelected,
        RustSearchBundle.message("search.whole.words.tooltip")
    )

    // ==================== 作用域(单选:项目 / 模块) ====================

    /** 作用域单选按钮:项目(默认选中,搜索整个项目) */
    private val scopeProjectRadio = JRadioButton(RustSearchBundle.message("search.scope.project.text"), true).apply {
        toolTipText = RustSearchBundle.message("search.scope.project.tooltip")
    }

    /** 作用域单选按钮:模块(搜索选中模块的 contentRoots) */
    private val scopeModuleRadio = JRadioButton(RustSearchBundle.message("search.scope.module.text"), false).apply {
        toolTipText = RustSearchBundle.message("search.scope.module.tooltip")
    }

    /** 单选按钮组,确保项目/模块互斥 */
    private val scopeButtonGroup = ButtonGroup().apply {
        add(scopeProjectRadio)
        add(scopeModuleRadio)
    }

    /** 模块下拉框(作用域=模块时启用,否则禁用) */
    private val moduleComboBox = JComboBox<String>().apply {
        toolTipText = RustSearchBundle.message("search.module.combo.tooltip")
        preferredSize = Dimension(200, 30)
        isEnabled = false // 默认作用域=项目,禁用模块选择
    }

    // ==================== 文件后缀过滤 ====================

    /** 预定义文件后缀列表(不含点号) */
    private val fileExtensionOptions = listOf(
        "kt", "java", "xml", "gradle", "kts", "properties", "toml", "md", "txt", "json", "yml", "yaml"
    )

    /** 文件后缀复选框列表;不勾选任何项=搜索全部文件 */
    private val extensionCheckBoxes: List<JBCheckBox> = fileExtensionOptions.map { ext ->
        JBCheckBox(".$ext").apply { toolTipText = RustSearchBundle.message("search.extension.checkbox.tooltip", ext) }
    }

    /** 结果树 */
    private val resultTree = JTree(treeModel).apply {
        isRootVisible = false
        showsRootHandles = true
        selectionModel.selectionMode = TreeSelectionModel.SINGLE_TREE_SELECTION
        // 需求 2:传 patternProvider 让 renderer 读取 currentPattern 做关键字高亮
        setCellRenderer(SearchResultTreeCellRenderer(patternProvider = { treeModel.getCurrentPattern() }))
    }

    /** 状态栏 */
    private val statusLabel = JBLabel(RustSearchBundle.message("search.status.ready"), SwingConstants.LEFT).apply {
        border = JBUI.Borders.empty(2, 4)
    }

    init {
        setupUI()
        setupListeners()
    }

    /**
     * 构建 UI 布局
     */
    private fun setupUI() {
        // 顶部搜索栏
        val topPanel = JPanel().apply {
            layout = BoxLayout(this, BoxLayout.Y_AXIS)
            border = JBUI.Borders.empty(4)
        }

        // 第一行:搜索输入 + 选项(回车搜索,Esc 取消)
        val row1 = Box.createHorizontalBox().apply {
            add(searchField)
            add(Box.createHorizontalStrut(4))
            add(regexButton)
            add(Box.createHorizontalStrut(3))
            add(caseSensitiveButton)
            add(Box.createHorizontalStrut(3))
            add(wholeWordsButton)
        }

        // 第二行:作用域单选(项目/模块) + 模块下拉框
        val row2 = Box.createHorizontalBox().apply {
            add(JBLabel(RustSearchBundle.message("search.scope.label")))
            add(scopeProjectRadio)
            add(Box.createHorizontalStrut(4))
            add(scopeModuleRadio)
            add(Box.createHorizontalStrut(8))
            add(JBLabel(RustSearchBundle.message("search.module.label")))
            add(moduleComboBox)
        }

        // 第三行:文件后缀过滤(FlowLayout 自动换行)
        val row3 = JPanel(FlowLayout(FlowLayout.LEFT, 2, 0)).apply {
            add(JBLabel(RustSearchBundle.message("search.file.type.label")))
            // 提示:不勾选=搜索全部
            add(JBLabel(RustSearchBundle.message("search.file.type.hint")).apply {
                font = font.deriveFont(font.size2D - 1f)
                foreground = JBUI.CurrentTheme.ContextHelp.FOREGROUND
            })
            extensionCheckBoxes.forEach { add(it) }
        }

        topPanel.add(row1)
        topPanel.add(Box.createVerticalStrut(4))
        topPanel.add(row2)
        topPanel.add(Box.createVerticalStrut(2))
        topPanel.add(row3)

        // 中间结果树
        val treeScroll = JScrollPane(resultTree)

        // 底部状态栏
        val bottomPanel = JPanel(BorderLayout()).apply {
            add(statusLabel, BorderLayout.CENTER)
        }

        // 组装
        add(topPanel, BorderLayout.NORTH)
        add(treeScroll, BorderLayout.CENTER)
        add(bottomPanel, BorderLayout.SOUTH)

        // 初始化模块下拉框
        refreshModuleList()
    }

    /**
     * 绑定事件监听器
     *
     * - 搜索框:回车触发搜索
     * - Esc:取消正在进行的搜索
     * - 筛选条件变化(正则/大小写/全字/作用域/模块/文件后缀):自动重新搜索
     * - 结果树:双击跳转
     */
    private fun setupListeners() {
        // 搜索框回车触发搜索
        searchField.addActionListener { _ -> performSearch() }

        // Esc 键取消正在进行的搜索
        registerKeyboardAction(
            { cancelSearch() },
            KeyStroke.getKeyStroke(java.awt.event.KeyEvent.VK_ESCAPE, 0),
            WHEN_ANCESTOR_OF_FOCUSED_COMPONENT
        )

        // 筛选条件变化自动触发搜索(ActionListener 需接受 ActionEvent 参数)
        // P1-3:模块列表刷新期间(isRefreshingModules=true)跳过,避免 addItem 触发事件风暴
        val autoSearchListener = java.awt.event.ActionListener {
            if (!isRefreshingModules) performSearch()
        }
        regexButton.addActionListener(autoSearchListener)
        caseSensitiveButton.addActionListener(autoSearchListener)
        wholeWordsButton.addActionListener(autoSearchListener)
        extensionCheckBoxes.forEach { it.addActionListener(autoSearchListener) }

        // 作用域切换:启用/禁用模块下拉框并自动搜索
        scopeProjectRadio.addActionListener { _ ->
            moduleComboBox.isEnabled = false
            performSearch()
        }
        scopeModuleRadio.addActionListener { _ ->
            moduleComboBox.isEnabled = true
            // 切换到模块作用域时,若无模块数据则先刷新;有数据则直接搜索
            if (moduleComboBox.itemCount == 0) refreshModuleList()
            performSearch()
        }

        // 模块选择变化自动触发搜索
        moduleComboBox.addActionListener(autoSearchListener)

        // 结果树双击跳转
        resultTree.addMouseListener(object : MouseAdapter() {
            override fun mouseClicked(e: MouseEvent) {
                if (e.clickCount >= 2) {
                    navigateToSelectedResult()
                }
            }
        })
    }

    /**
     * 刷新模块下拉框列表
     *
     * 从 [ModuleManager] 获取当前项目所有模块名称。
     *
     * M4:`ModuleManager.modules` 调用需读锁保护(IntelliJ Platform SDK 官方要求),
     * 用 `ReadAction.compute(Computable { ... })` 包裹,避免 EDT 卡顿或并发写异常。
     */
    private fun refreshModuleList() {
        // P1-3:刷新期间设置标志位,避免 addItem/removeAllItems 触发 autoSearchListener 事件风暴
        isRefreshingModules = true
        try {
            moduleComboBox.removeAllItems()
            // M4:ModuleManager.modules 需读锁保护,避免并发写异常
            // Kotlin 重载解析匹配 ThrowableComputable 版本,显式指定 <T, E> 类型参数
            val modules = ReadAction.compute<Array<Module>, Exception> {
                ModuleManager.getInstance(project).modules
            }
            modules.forEach { module ->
                moduleComboBox.addItem(module.name)
            }
            // 默认选第一个模块(如果有)
            if (moduleComboBox.itemCount > 0) {
                moduleComboBox.selectedIndex = 0
            }
        } finally {
            isRefreshingModules = false
        }
    }

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

    /**
     * 执行搜索
     *
     * 当 pattern 为空时静默返回(自动搜索场景下用户可能尚未输入内容)。
     *
     * 根因 A 修复:每次搜索递增 activeSearchToken,collect 块捕获该令牌,
     * invokeLater 回调中校验 currentToken == activeSearchToken,
     * 不等则说明该 batch 属于已被取消的旧搜索(旧协程的 EDT 任务滞后到达),
     * 直接丢弃,避免污染新 tree 的 totalMatches 与 JTree 状态。
     *
     * 根因 B 修复:用 ApplicationManager.getApplication().invokeLater 替代
     * withContext(Dispatchers.Main),显式调度到 EDT,
     * 绕过 Dispatchers.Main 在 IntelliJ 中的 modality 调度风险。
     */
    private fun performSearch() {
        // 取消旧搜索,防止旧 Flow 的 withContext(Main) 在新 clear() 后追加结果导致竞态
        searchJob?.cancel()

        // 根因 A 修复:生成新令牌,滞后的旧 EDT 任务通过令牌校验自检丢弃
        activeSearchToken++

        val pattern = searchField.text.trim()
        if (pattern.isEmpty()) {
            // 自动触发搜索时若搜索框为空,不显示错误,仅清空旧结果
            treeModel.clear()
            statusLabel.text = RustSearchBundle.message("search.status.empty.input")
            return
        }

        // 根据作用域解析搜索根目录
        val roots = resolveSearchRoots()
        if (roots.isEmpty()) {
            statusLabel.text = RustSearchBundle.message("search.status.no.roots")
            return
        }

        // 从勾选的后缀构建 includeGlobs;不勾选任何项=搜索全部文件
        val includeGlobs = extensionCheckBoxes
            .filter { it.isSelected }
            .map { "*.${it.text.removePrefix(".")}" }

        val config = SearchConfig(
            roots = roots,
            pattern = pattern,
            isRegex = regexButton.isSelected,
            caseSensitive = caseSensitiveButton.isSelected,
            wholeWords = wholeWordsButton.isSelected,
            includeGlobs = includeGlobs,
            excludeGlobs = emptyList(),
            contextLines = 2
        )

        // 清空旧结果
        treeModel.clear()
        // 需求 2:设置当前搜索词供 renderer 做关键字高亮
        // 正则模式传空字符串跳过高亮(避免元字符误匹配);字面量模式传原始 pattern
        treeModel.setCurrentPattern(if (regexButton.isSelected) "" else pattern)
        statusLabel.text = RustSearchBundle.message("search.status.searching")

        val startTime = System.currentTimeMillis()

        // 启动协程收集 Flow
        searchJob = searchScope.launch {
            // 根因 A 修复:捕获当前搜索令牌,用于 EDT 任务自检
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
                        // 根因修复:isRootVisible=false 时,JTree 默认不展开 root 的子节点,
                        // 导致所有文件节点处于折叠状态,JTree.rowCount=0 → 树显示空白。
                        // 修复:addResults 后展开 root 路径 + 所有文件节点路径,
                        // 让文件节点和匹配内容直接可见,无需用户手动点击展开。
                        val rootPath = javax.swing.tree.TreePath(treeModel.root)
                        resultTree.expandPath(rootPath)
                        // 展开所有文件节点,显示匹配内容
                        treeModel.getFileNodePaths().forEach { path ->
                            resultTree.expandPath(path)
                        }
                        val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
                        // 诊断日志:JTree 实际状态(验证渲染层数据)
                        logger.info(
                            "After addResults: treeRowCount=${resultTree.rowCount}, " +
                            "visibleRowCount=${resultTree.visibleRowCount}, " +
                            "treeModelRootChildCount=${(treeModel.root as? javax.swing.tree.DefaultMutableTreeNode)?.childCount ?: -1}"
                        )
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
    }

    /**
     * 根据作用域单选按钮解析搜索根目录列表
     *
     * - 项目:返回 [project.basePath]
     * - 模块:返回选中模块的所有 contentRoot
     *
     * M4:`ModuleManager.modules` 与 `ModuleRootManager.contentRoots` 调用需读锁保护,
     * 用 `ReadAction.compute(Computable { ... })` 包裹,避免并发写异常。
     *
     * @return 根目录路径列表;空列表表示无法确定
     */
    private fun resolveSearchRoots(): List<String> {
        return when {
            scopeProjectRadio.isSelected -> {
                val basePath = project.basePath
                if (basePath.isNullOrBlank()) emptyList() else listOf(basePath)
            }
            scopeModuleRadio.isSelected -> {
                val moduleName = moduleComboBox.selectedItem as? String
                if (moduleName.isNullOrBlank()) return emptyList()
                // M4:模块查询与 contentRoots 读取需读锁保护
                // Kotlin 重载解析匹配 ThrowableComputable 版本,显式指定 <T, E> 类型参数
                ReadAction.compute<List<String>, Exception> {
                    val module = ModuleManager.getInstance(project).modules
                        .firstOrNull { it.name == moduleName } ?: return@compute emptyList()
                    ModuleRootManager.getInstance(module).contentRoots.map { it.path }
                }
            }
            else -> emptyList()
        }
    }

    /**
     * 取消搜索
     */
    private fun cancelSearch() {
        searchJob?.cancel()
        statusLabel.text = RustSearchBundle.message("search.status.cancelled", treeModel.getTotalMatches())
    }

    /**
     * 双击结果树节点,跳转到对应文件行
     *
     * 使用 [OpenFileDescriptor.navigate] 打开文件并定位到匹配行号。
     * OpenFileDescriptor 的行号参数为 0-based,而 [MatchNodeData.lineNumber] 为 1-based(从 Rust 侧返回),
     * 需转换。navigate(true) 会自动请求焦点并滚动到目标位置。
     *
     * M5:VFS 缓存的文件可能已被删除(磁盘不一致),navigate 时会抛异常。
     * 用 isValid + try-catch 包裹,异常时显示友好提示而非红色错误气泡。
     *
     * IC-261 兼容性修复:IC-261 强化线程模型,VirtualFile.refresh() 必须在
     * WriteIntentReadAction 或 WriteAction 中执行。本方法由 EDT 双击事件触发,
     * 不在 WriteIntent 上下文中,故移除 file.refresh 调用,直接用 VFS 缓存的
     * isValid 判断;navigate 本身内部会处理 VFS 同步。
     */
    private fun navigateToSelectedResult() {
        val node = resultTree.lastSelectedPathComponent as? DefaultMutableTreeNode ?: return
        val data = node.userObject as? MatchNodeData ?: return

        val file = LocalFileSystem.getInstance().findFileByPath(data.filePath)
        if (file != null) {
            // 直接用 VFS 缓存的 isValid 判断,避免 refresh 触发 IC-261 线程断言
            if (!file.isValid) {
                statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
                return
            }
            // lineNumber 为 1-based,OpenFileDescriptor 需 0-based
            val descriptor = OpenFileDescriptor(project, file, data.lineNumber - 1, data.column)
            try {
                // M5:捕获 navigate 可能抛出的异常(文件已被删除/权限不足/编辑器冲突)
                descriptor.navigate(true)
            } catch (e: Exception) {
                logger.warn("Failed to navigate to ${data.filePath}:${data.lineNumber}: ${e.message}")
                statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
            }
        } else {
            statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
        }
    }

    /**
     * 释放资源(P1-5)
     *
     * 由 IntelliJ Disposer 在 ToolWindow 释放时调用,
     * 取消正在进行的搜索与协程作用域,防止内存泄漏。
     */
    override fun dispose() {
        searchJob?.cancel()
        searchScope.cancel()
        // P2-E:已知限制 — Flow finally(cancel+releaseSearch)在 IO 线程异步执行,
        // 可能在 dispose 返回后仍在运行。由于 searchId 全局递增不复用,
        // 旧搜索的 releaseSearch 不会误删新搜索的 session,实际风险极低。
        logger.info("RustSearchPanel disposed")
    }

    /**
     * 图标式 toggle 按钮,对齐 Find in Path 风格
     *
     * 选中时显示 Selected 图标(高亮),未选中显示 normalIcon,悬停显示 hoveredIcon。
     * 基于 JToggleButton(原生支持 selected 态),避免 JButton + ButtonModel.isSelected 的状态丢失问题。
     *
     * 用于正则 / 大小写 / 全字 三个开关,对齐 Android Studio Find in Path 的 `.*` `Aa` `|W|` 风格。
     *
     * 选中态视觉强化:
     * - 选中时显示 selectedIcon(AllIcons.Actions.RegexSelected 等,带蓝色高亮)
     * - 选中时背景填充浅色(与 Find in Path 一致),未选中透明
     */
    private class IconToggleButton(
        private val normalIcon: Icon,
        private val hoveredIcon: Icon,
        private val selectedIcon: Icon,
        tooltip: String
    ) : javax.swing.JToggleButton() {
        init {
            icon = normalIcon
            // 透明背景 + 无边框,选中态通过图标和手动绘制背景区分
            isContentAreaFilled = false
            isBorderPainted = false
            isFocusPainted = false
            isFocusable = false
            toolTipText = tooltip
            // 收紧尺寸:对齐 Find in Path 图标按钮(约 24x24)
            margin = java.awt.Insets(0, 0, 0, 0)
            preferredSize = java.awt.Dimension(24, 24)
            maximumSize = java.awt.Dimension(24, 24)
            minimumSize = java.awt.Dimension(24, 24)
            // 根据选中/悬停/普通三态切换图标
            model.addChangeListener {
                icon = when {
                    isSelected -> selectedIcon
                    model.isRollover -> hoveredIcon
                    else -> normalIcon
                }
                // 选中态切换后重绘,确保背景填充立即生效
                repaint()
            }
        }

        /** 选中态绘制浅色背景(与 Find in Path 一致) */
        override fun paintComponent(g: java.awt.Graphics) {
            if (isSelected) {
                val color = JBUI.CurrentTheme.ActionButton.hoverBackground()
                g.color = color
                g.fillRect(0, 0, width, height)
            }
            super.paintComponent(g)
        }
    }

    /**
     * JToggleButton 原生 isSelected() 由 ButtonModel.selected 支撑,
     * 点击自动切换,无需扩展属性;regexButton.isSelected 直接用父类 AbstractButton.isSelected。
     */
}

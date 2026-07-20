package com.example.rustsearch.action

import com.example.rustsearch.RustSearchBundle
import com.example.rustsearch.ui.RustSearchToolWindowFactory
import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindowManager

/**
 * 打开 RustSearch 工具窗口的 Action
 *
 * 注册在 plugin.xml 中,快捷键 `Ctrl+Shift+Alt+F`(Mac: `Cmd+Shift+Alt+F`)。
 *
 * 需求 1:对齐 IntelliJ Find in Path 体验
 * - 若编辑器有选中文本(长度 ≤ 200),打开 ToolWindow 后预填搜索框并自动触发搜索
 * - 无选中文本时仅打开 ToolWindow 并聚焦搜索框
 * - Action 文本动态切换:有选中显示"搜索选中文字",无选中显示"打开搜索"
 */
class RustSearchAction : AnAction() {

    private val logger = Logger.getInstance(RustSearchAction::class.java)

    override fun actionPerformed(e: AnActionEvent) {
        val project: Project = e.project ?: run {
            logger.warn("actionPerformed: project is null, abort")
            return
        }
        val toolWindow = ToolWindowManager.getInstance(project)
            .getToolWindow("RustSearch") ?: run {
            logger.warn("actionPerformed: RustSearch ToolWindow not found")
            return
        }

        // 需求 1:读取当前编辑器选中的文本(限制 ≤ 200 字符,避免大段选中卡顿)
        val editor = e.getData(CommonDataKeys.EDITOR)
        val selectedText = editor?.selectionModel?.selectedText
            ?.takeIf { it.isNotBlank() && it.length <= 200 }

        // 诊断日志:验证 Action 触发、选中文本读取、线程
        logger.info(
            "actionPerformed: hasEditor=${editor != null}, " +
            "hasSelection=${editor?.selectionModel?.hasSelection() == true}, " +
            "selectedTextLen=${selectedText?.length ?: 0}, " +
            "selectedTextPreview='${selectedText?.take(60) ?: ""}', " +
            "thread=${Thread.currentThread().name}, " +
            "isEDT=${javax.swing.SwingUtilities.isEventDispatchThread()}"
        )

        toolWindow.show {
            // ToolWindow 显示后,获取 panel 并预填
            // 231 SDK 中 ToolWindow 接口不继承 UserDataHolder,改从 Content 取(Content 继承 UserDataHolder)
            logger.info("toolWindow.show callback fired")
            ApplicationManager.getApplication().invokeLater {
                val contents = toolWindow.contentManager.contents
                val content = contents.firstOrNull()
                logger.info(
                    "invokeLater: contentCount=${contents.size}, " +
                    "hasContent=${content != null}, " +
                    "thread=${Thread.currentThread().name}"
                )
                if (content == null) {
                    logger.warn("invokeLater: content is null, abort")
                    return@invokeLater
                }
                val panel = content.getUserData(RustSearchToolWindowFactory.PANEL_KEY)
                if (panel == null) {
                    logger.warn("invokeLater: panel is null (PANEL_KEY not set on content)")
                    return@invokeLater
                }
                if (!selectedText.isNullOrBlank()) {
                    // 有选中文本:预填并自动搜索
                    logger.info("invokeLater: setInitialSearchText with selectedText, len=${selectedText.length}")
                    panel.setInitialSearchText(selectedText, autoTrigger = true)
                } else {
                    // 无选中文本:仅聚焦搜索框
                    logger.info("invokeLater: setInitialSearchText empty (no selection)")
                    panel.setInitialSearchText("", autoTrigger = false)
                }
            }
        }
    }

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabled = e.project != null
        // 需求 1:有选中文本时动态修改 Action 文本,提示用户可直接搜索选中内容
        val editor = e.getData(CommonDataKeys.EDITOR)
        val hasSelection = editor?.selectionModel?.hasSelection() == true
        e.presentation.text = if (hasSelection) {
            RustSearchBundle.message("action.rustsearch.search.selection")
        } else {
            RustSearchBundle.message("action.rustsearch.open.text")
        }
    }
}

package com.example.rustsearch.action

import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindowManager

/**
 * 打开 RustSearch 工具窗口的 Action
 *
 * 注册在 plugin.xml 中,快捷键 `Ctrl+Shift+Alt+F`(Mac: `Cmd+Shift+Alt+F`)。
 * 触发后显示 RustSearch 工具窗口。
 */
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

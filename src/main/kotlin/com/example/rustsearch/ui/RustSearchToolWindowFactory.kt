package com.example.rustsearch.ui

import com.example.rustsearch.RustSearchBundle
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.intellij.openapi.util.Key
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.content.ContentFactory

/**
 * RustSearch 工具窗口工厂
 *
 * 注册在 plugin.xml 的 `<toolWindow>` 扩展点,
 * 当用户首次打开 RustSearch 工具窗口时调用 [createToolWindowContent] 创建面板。
 *
 * 实现了 [DumbAware],允许在 dumb mode(索引构建中)也能使用搜索功能,
 * 因为 Rust 搜索不依赖 IntelliJ 的索引。
 */
class RustSearchToolWindowFactory : ToolWindowFactory, DumbAware {

    companion object {
        /**
         * ToolWindow UserData Key,缓存 RustSearchPanel 引用
         *
         * 需求 1:供 RustSearchAction 在 toolWindow.show 回调中获取 panel,
         * 调用 setInitialSearchText 预填选中文本并自动触发搜索。
         * 生命周期跟随 ToolWindow,ToolWindow 销毁时自动回收。
         */
        val PANEL_KEY: Key<RustSearchPanel> = Key.create("RustSearch.Panel")
    }

    /**
     * 创建工具窗口内容
     *
     * @param project 当前项目
     * @param toolWindow 工具窗口实例
     */
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val panel = RustSearchPanel(project)
        val content = ContentFactory.getInstance()
            .createContent(panel, RustSearchBundle.message("toolwindow.content.name"), false)
        toolWindow.contentManager.addContent(content)

        // 需求 1:缓存 panel 引用到 Content userdata(231 SDK 中 ToolWindow 接口
        // 不继承 UserDataHolder,但 Content 继承 UserDataHolder),供 RustSearchAction 获取
        content.putUserData(PANEL_KEY, panel)

        // P1-5:注册 Disposable,ToolWindow 释放时调用 panel.dispose() 清理协程
        Disposer.register(toolWindow.disposable, panel)
    }

    override fun shouldBeAvailable(project: Project): Boolean = true
}

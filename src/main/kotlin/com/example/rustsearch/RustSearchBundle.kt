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

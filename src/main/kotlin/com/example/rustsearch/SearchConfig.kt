package com.example.rustsearch

/**
 * 搜索配置(Kotlin 侧)
 *
 * 封装用户在 UI 中输入的搜索参数,通过 [RustSearchService] 转换为 JNI 调用参数。
 * 字段与 Rust 侧 `SearchConfig` 一一对应,但此处用可空类型与默认值简化 UI 层使用。
 *
 * @param roots 搜索根目录列表
 * @param pattern 搜索模式(字面量或正则)
 * @param isRegex 是否为正则模式,默认 false
 * @param caseSensitive 大小写敏感,默认 false
 * @param wholeWords 全字匹配,默认 false
 * @param includeGlobs 包含文件通配符(如 "*.kt"),空列表表示不过滤
 * @param excludeGlobs 排除文件通配符,空列表表示不排除
 * @param contextLines 上下文行数,默认 0(不提取上下文)
 */
data class SearchConfig(
    val roots: List<String>,
    val pattern: String,
    val isRegex: Boolean = false,
    val caseSensitive: Boolean = false,
    val wholeWords: Boolean = false,
    val includeGlobs: List<String> = emptyList(),
    val excludeGlobs: List<String> = emptyList(),
    val contextLines: Int = 0
) {
    /**
     * 转换为 JNI 调用所需的数组参数
     *
     * Kotlin List<String> → Java Array<String>,供 [RustSearchEngine.startSearch] 使用
     */
    fun toJniArgs(): JniSearchArgs = JniSearchArgs(
        roots = roots.toTypedArray(),
        pattern = pattern,
        isRegex = isRegex,
        caseSensitive = caseSensitive,
        wholeWords = wholeWords,
        includeGlobs = includeGlobs.toTypedArray(),
        excludeGlobs = excludeGlobs.toTypedArray(),
        contextLines = contextLines
    )
}

/**
 * JNI 调用参数(数组形式)
 *
 * 由 [SearchConfig.toJniArgs] 转换而来,直接传给 [RustSearchEngine.startSearch]
 */
data class JniSearchArgs(
    val roots: Array<String>,
    val pattern: String,
    val isRegex: Boolean,
    val caseSensitive: Boolean,
    val wholeWords: Boolean,
    val includeGlobs: Array<String>,
    val excludeGlobs: Array<String>,
    val contextLines: Int
)

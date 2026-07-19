package com.example.rustsearch

import com.example.rustsearch.service.RustSearchService

/**
 * Rust 搜索引擎 JNI 入口
 *
 * 对应 Rust 侧 `rust-search/src/jni/bridge.rs` 中的 JNI 函数声明。
 * 函数名前缀 `Java_com_example_rustsearch_RustSearchEngine_` 必须与此类的全限定名严格对应,
 * 否则 JVM 加载动态库后调用会抛出 `UnsatisfiedLinkError`。
 *
 * 使用流程:
 * 1. 首次访问时触发 `init` 块,通过 [RustSearchService] 加载 native 动态库
 * 2. 调用 [startSearch] 启动异步搜索,获得 searchId
 * 3. 循环调用 [pollResults] 获取批量结果,配合 [isSearchComplete] 判断是否结束
 * 4. 搜索完成后必须调用 [releaseSearch] 释放资源,防止内存泄漏
 * 5. 如需中途取消,调用 [cancel]
 */
object RustSearchEngine {

    init {
        // 通过 application service 获取实例后加载 native 库
        // loadNativeLibrary() 是实例方法,需通过 getInstance() 调用
        RustSearchService.getInstance().loadNativeLibrary()
    }

    /**
     * 启动异步搜索,立即返回 searchId
     *
     * 搜索在 Rust 后台线程执行,通过 [pollResults] 轮询获取结果。
     *
     * @param roots 搜索根目录数组
     * @param pattern 搜索模式(字面量或正则)
     * @param isRegex 是否为正则模式
     * @param caseSensitive 大小写敏感
     * @param wholeWords 全字匹配
     * @param includeGlobs 包含文件通配符(如 "*.kt")
     * @param excludeGlobs 排除文件通配符
     * @param contextLines 上下文行数
     * @return searchId > 0 表示成功,0 表示失败(异常已抛出)
     */
    @JvmStatic
    external fun startSearch(
        roots: Array<String>,
        pattern: String,
        isRegex: Boolean,
        caseSensitive: Boolean,
        wholeWords: Boolean,
        includeGlobs: Array<String>,
        excludeGlobs: Array<String>,
        contextLines: Int
    ): Long

    /**
     * 轮询获取一批搜索结果
     *
     * 阻塞等待 timeoutMs 或拿到一批结果后返回。
     * 返回空数组表示暂无结果或搜索已完成(需配合 [isSearchComplete] 判断)。
     *
     * @param searchId [startSearch] 返回的搜索 ID
     * @param timeoutMs 最大等待时间(毫秒),建议 100~500
     * @return 匹配结果数组(可能为空)
     */
    @JvmStatic
    external fun pollResults(searchId: Long, timeoutMs: Int): Array<SearchResult>

    /**
     * 检查搜索是否完成
     *
     * @param searchId [startSearch] 返回的搜索 ID
     * @return true 表示搜索已完成(正常结束、取消或出错)
     */
    @JvmStatic
    external fun isSearchComplete(searchId: Long): Boolean

    /**
     * 取消指定 ID 的搜索
     *
     * 取消后,正在进行的搜索会在下一个检查点停止,[pollResults] 将返回空数组,
     * [isSearchComplete] 将返回 true。
     *
     * @param searchId [startSearch] 返回的搜索 ID
     */
    @JvmStatic
    external fun cancel(searchId: Long)

    /**
     * 释放搜索会话资源
     *
     * 必须在搜索结束后调用,清理 Rust 侧的 SearchSession(engine + receiver),
     * 防止内存泄漏。建议放在 `finally` 块中确保执行。
     *
     * @param searchId [startSearch] 返回的搜索 ID
     */
    @JvmStatic
    external fun releaseSearch(searchId: Long)

    /**
     * 单条搜索匹配结果
     *
     * **注意**:此类必须作为 [RustSearchEngine] 的内部类,且构造函数参数顺序与类型
     * 必须与 Rust 侧 `result.rs` 中的 `build_single_result` 严格一致:
     * `(Ljava/lang/String;IILjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V`
     * 即 (String, int, int, String, String[], String[])
     *
     * @param filePath 匹配文件绝对路径
     * @param lineNumber 行号(从 1 开始)
     * @param column 列号(从 0 开始,MVP 阶段暂为 0)
     * @param matchedText 匹配的文本内容
     * @param contextBefore 上下文前导行(按行分割)
     * @param contextAfter 上下文后续行(按行分割)
     */
    data class SearchResult(
        val filePath: String,
        val lineNumber: Int,
        val column: Int,
        val matchedText: String,
        val contextBefore: Array<String>,
        val contextAfter: Array<String>
    ) {
        // data class 中 Array 字段不会自动生成 equals/hashCode,需手动实现
        override fun equals(other: Any?): Boolean {
            if (this === other) return true
            if (other !is SearchResult) return false
            return filePath == other.filePath &&
                    lineNumber == other.lineNumber &&
                    column == other.column &&
                    matchedText == other.matchedText &&
                    contextBefore.contentEquals(other.contextBefore) &&
                    contextAfter.contentEquals(other.contextAfter)
        }

        override fun hashCode(): Int {
            var result = filePath.hashCode()
            result = 31 * result + lineNumber
            result = 31 * result + column
            result = 31 * result + matchedText.hashCode()
            result = 31 * result + contextBefore.contentHashCode()
            result = 31 * result + contextAfter.contentHashCode()
            return result
        }

        override fun toString(): String {
            return "SearchResult(filePath='$filePath', line=$lineNumber, col=$column, matched='$matchedText')"
        }
    }
}

package com.example.rustsearch.service

import com.example.rustsearch.RustSearchEngine
import com.example.rustsearch.RustSearchEngine.SearchResult
import com.example.rustsearch.RustSearchBundle
import com.example.rustsearch.SearchConfig
import com.example.rustsearch.SearchException
import com.intellij.openapi.Disposable
import com.intellij.openapi.diagnostic.Logger
import com.intellij.openapi.application.ApplicationManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.flowOn
import java.io.File
import java.io.IOException
import java.nio.file.Files
import java.nio.file.StandardCopyOption

/**
 * Rust 搜索服务
 *
 * 职责:
 * 1. **动态库加载**:从插件 classpath 提取 native 动态库到临时目录,通过 `System.load` 加载
 * 2. **搜索会话管理**:封装 [RustSearchEngine.startSearch] + [RustSearchEngine.pollResults] + [RustSearchEngine.releaseSearch] 生命周期
 * 3. **流式 API**:提供 Kotlin Flow 供 UI 层消费,自动管理 searchId 的获取与释放
 *
 * 注册为 IntelliJ applicationService,全局单例,插件卸载时通过 [Disposable] 清理。
 */
class RustSearchService : Disposable {

    private val logger = Logger.getInstance(RustSearchService::class.java)

    /**
     * 动态库是否已加载成功
     */
    @Volatile
    private var nativeLoaded = false

    companion object {
        /** 动态库在插件资源中的路径前缀 */
        private const val NATIVE_RESOURCE_PREFIX = "/native/"

        /**
         * 临时目录名(P2-1:加用户名后缀,多用户系统隔离避免权限冲突)
         */
        private val TEMP_DIR_NAME: String =
            "rustsearch-${System.getProperty("user.name", "default")}"

        /**
         * 获取全局单例实例
         */
        fun getInstance(): RustSearchService =
            ApplicationManager.getApplication().getService(RustSearchService::class.java)
    }

    /**
     * 加载 native 动态库
     *
     * 从插件 classpath 读取 `/native/librust_search.{dylib|so|dll}`,
     * 拷贝到 `${java.io.tmpdir}/rustsearch/` 临时目录,然后 `System.load`。
     *
     * 必须在首次调用 [RustSearchEngine.startSearch] 前完成加载,
     * 由 [RustSearchEngine] 的 `init` 块触发。
     *
     * @throws UnsatisfiedLinkError 加载失败时抛出
     */
    @Synchronized
    fun loadNativeLibrary() {
        if (nativeLoaded) return

        val libName = getNativeLibName()
        val resourcePath = NATIVE_RESOURCE_PREFIX + libName

        logger.info("Loading Rust native library: $resourcePath")

        // 从 classpath 读取动态库资源
        val resourceStream = javaClass.getResourceAsStream(resourcePath)
            ?: throw UnsatisfiedLinkError(RustSearchBundle.message("service.error.library.not.found", resourcePath))

        // 拷贝到临时目录
        val tempDir = File(System.getProperty("java.io.tmpdir"), TEMP_DIR_NAME)
        if (!tempDir.exists()) {
            tempDir.mkdirs()
        }

        val tempLibFile = File(tempDir, libName)
        try {
            Files.copy(resourceStream, tempLibFile.toPath(), StandardCopyOption.REPLACE_EXISTING)
            resourceStream.close()
        } catch (e: IOException) {
            throw UnsatisfiedLinkError(RustSearchBundle.message("service.error.copy.failed", e.message ?: ""))
        }

        // 设置可执行权限(macOS/Linux 需要)
        tempLibFile.setExecutable(true)

        // 加载动态库
        try {
            System.load(tempLibFile.absolutePath)
            nativeLoaded = true
            logger.info("Rust native library loaded: ${tempLibFile.absolutePath}")
        } catch (e: UnsatisfiedLinkError) {
            logger.error("Failed to load Rust native library: ${tempLibFile.absolutePath}", e)
            throw e
        }
    }

    /**
     * 执行流式搜索
     *
     * 封装 startSearch + pollResults + releaseSearch 生命周期,
     * 通过 Kotlin Flow 流式返回批量结果。
     *
     * 使用示例:
     * ```kotlin
     * val service = RustSearchService.getInstance()
     * service.search(config).collect { batch ->
     *     // 更新 UI 展示 batch
     * }
     * ```
     *
     * @param config 搜索配置
     * @return 搜索结果 Flow,每次 emit 一批匹配结果
     */
    fun search(config: SearchConfig): Flow<List<SearchResult>> = flow {
        if (!nativeLoaded) {
            loadNativeLibrary()
        }

        val args = config.toJniArgs()
        val searchId = RustSearchEngine.startSearch(
            args.roots, args.pattern, args.isRegex, args.caseSensitive, args.wholeWords,
            args.includeGlobs, args.excludeGlobs, args.contextLines,
            args.skipComments, args.skipImports, args.skipPackages
        )

        if (searchId == 0L) {
            throw SearchException(RustSearchBundle.message("service.error.search.start"))
        }

        logger.info("Search started: searchId=$searchId, pattern='${config.pattern}'")

        try {
            // 轮询获取结果直到搜索完成
            var pollCount = 0
            while (!RustSearchEngine.isSearchComplete(searchId)) {
                val batch = RustSearchEngine.pollResults(searchId, 200)
                pollCount++
                logger.info(
                    "Poll#$pollCount: searchId=$searchId, batchSize=${batch.size}, " +
                    "isComplete=${RustSearchEngine.isSearchComplete(searchId)}"
                )
                if (batch.isNotEmpty()) {
                    val sample = batch.first()
                    logger.info(
                        "Emit batch: searchId=$searchId, batchSize=${batch.size}, " +
                        "sample filePath='${sample.filePath}', line=${sample.lineNumber}, " +
                        "matched='${sample.matchedText.take(50)}', " +
                        "contextBefore=${sample.contextBefore.size}, contextAfter=${sample.contextAfter.size}"
                    )
                    emit(batch.toList())
                }
            }
            logger.info("Polling loop ended: searchId=$searchId, totalPolls=$pollCount")

            // 最后再 poll 一次,确保拿到剩余结果
            val finalBatch = RustSearchEngine.pollResults(searchId, 50)
            logger.info("Final poll: searchId=$searchId, batchSize=${finalBatch.size}")
            if (finalBatch.isNotEmpty()) {
                logger.info("Emit final batch: searchId=$searchId, batchSize=${finalBatch.size}")
                emit(finalBatch.toList())
            }
        } finally {
            // 关键修复:先触发 Rust 侧 cancel,让后台线程在下一个检查点退出
            // 这样 releaseSearch 移除 session 后,后台线程不会继续运行
            try {
                RustSearchEngine.cancel(searchId)
            } catch (e: Throwable) {
                logger.warn("Failed to cancel search $searchId: ${e.message}")
            }
            // 短暂等待后台线程退出(避免 tx.send 失败时的日志噪音)
            Thread.sleep(50)
            RustSearchEngine.releaseSearch(searchId)
            logger.info("Search session released: searchId=$searchId")
        }
    }.flowOn(Dispatchers.IO)

    /**
     * 取消指定搜索
     *
     * @param searchId [RustSearchEngine.startSearch] 返回的搜索 ID
     */
    fun cancel(searchId: Long) {
        if (searchId > 0) {
            RustSearchEngine.cancel(searchId)
            logger.info("Search cancelled: searchId=$searchId")
        }
    }

    /**
     * 根据 OS 选择动态库文件名
     *
     * macOS: librust_search.dylib
     * Linux: librust_search.so
     * Windows: rust_search.dll
     */
    private fun getNativeLibName(): String {
        val osName = System.getProperty("os.name").lowercase()
        return when {
            osName.contains("mac") || osName.contains("darwin") -> "librust_search.dylib"
            osName.contains("linux") -> "librust_search.so"
            osName.contains("windows") -> "rust_search.dll"
            else -> throw UnsatisfiedLinkError(RustSearchBundle.message("service.error.unsupported.os", osName))
        }
    }

    /**
     * 插件卸载时清理资源
     *
     * 注意:动态库一旦加载无法卸载,这里仅清理状态
     */
    override fun dispose() {
        logger.info("RustSearchService disposed")
    }
}

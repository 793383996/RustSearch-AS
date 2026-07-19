import org.jetbrains.kotlin.gradle.dsl.JvmTarget
import org.jetbrains.kotlin.gradle.tasks.KotlinCompile

plugins {
    id("org.jetbrains.intellij.platform") version "2.2.0"
    kotlin("jvm") version "2.2.20"
}

group = providers.gradleProperty("pluginGroup").get()
version = providers.gradleProperty("pluginVersion").get()

// repositories:阿里云镜像优先 + IntelliJ Platform releases + 本地 IDE 仓库
// (settings.gradle.kts 的 PREFER_SETTINGS 模式允许此处追加)
repositories {
    maven("https://maven.aliyun.com/repository/public")
    maven("https://maven.aliyun.com/repository/central")
    mavenCentral()
    intellijPlatform {
        // 使用默认仓库集合(releases + snapshots + marketplace)
        // 确保坐标映射规则正确注册
        defaultRepositories()
    }
}

dependencies {
    implementation("org.jetbrains.kotlin:kotlin-stdlib-jdk8")
    // 注意:kotlinx-coroutines-core 已由 IntelliJ Platform 内置提供,
    // 显式声明会触发 verifyPluginConfiguration 警告,已移除。

    // IntelliJ Platform 2.0 语法:远程 IC-2023.1 作为编译依赖源
    // 与 plugin.xml since-build=231 严格对齐
    // (本地 AS 261 的新模块化 jar 布局未被插件 2.0.1 正确提取为 classes,
    //  改用远程 IC-2023.1 稳定版,通过阿里云镜像规避 SSL 问题)
    intellijPlatform {
        intellijIdeaCommunity("2023.1")
    }
}

// ============================================================================
// IntelliJ Platform 配置(2.0 语法)
// ============================================================================

intellijPlatform {
    buildSearchableOptions = false

    // 显式声明 sinceBuild/untilBuild,覆盖 patchPluginXml 的自动 patch 行为
    // 不显式声明时,IntelliJ Platform Gradle Plugin 2.x 会基于编译平台版本
    // (IC-2023.1 = build 231) 自动将 until-build 限制为 231.*,
    // 导致插件无法在 AS 261 上安装。此处强制覆盖为 261.* 以支持目标 IDE。
    pluginConfiguration {
        ideaVersion {
            sinceBuild = providers.gradleProperty("pluginSinceBuild")
            untilBuild = providers.gradleProperty("pluginUntilBuild")
        }
    }
}

tasks {
    withType<KotlinCompile> {
        compilerOptions {
            // 使用 Java 17 字节码:IC-2023.1 运行于 JDK 17,
            // AS 261 运行于 JDK 21(JDK 21 向后兼容 Java 17 字节码)。
            // 使用 jvmTarget=17 确保 since-build=231 兼容性。
            jvmTarget.set(JvmTarget.JVM_17)
            freeCompilerArgs.add("-Xjsr305=strict")
        }
    }

    withType<JavaCompile> {
        sourceCompatibility = "17"
        targetCompatibility = "17"
    }

    // runIde 开发调试配置:
    // 1. 跳过 EUA/启动提示,避免阻塞自动化验证
    // 2. 支持 -PideProjectPath=/path/to/project 自动打开项目(激活 Tool Window)
    named<JavaExec>("runIde") {
        // 跳过 End User Agreement 和启动提示
        jvmArgs("-Djb.consents.confirmation.enabled=false")
        jvmArgs("-Dide.show.tips.on.startup=false")

        // 通过 -PideProjectPath=<path> 自动打开指定项目目录
        val ideProjectPath = providers.gradleProperty("ideProjectPath")
        if (ideProjectPath.isPresent) {
            args(ideProjectPath.get())
        }
    }
}

// ============================================================================
// Rust 编译集成:自动 cargo build --release 并拷贝动态库到插件资源目录
// ============================================================================

val rustDir = file("rust-search")
val nativeDir = file("src/main/resources/native")

/**
 * 根据 OS 选择动态库文件名
 * macOS: librust_search.dylib
 * Linux: librust_search.so
 * Windows: rust_search.dll
 */
val nativeLibName: String by lazy {
    val osName = System.getProperty("os.name").lowercase()
    when {
        osName.contains("mac") || osName.contains("darwin") -> "librust_search.dylib"
        osName.contains("linux") -> "librust_search.so"
        osName.contains("windows") -> "rust_search.dll"
        else -> throw GradleException("不支持的操作系统: $osName")
    }
}

/**
 * 编译 Rust 动态库(release 模式)
 * 输入增量检测:仅在 rust-search/src 或 Cargo.toml 变更时重新编译
 */
val buildRust by tasks.registering(Exec::class) {
    group = "rust"
    description = "编译 Rust 动态库(cargo build --release)"

    workingDir = rustDir
    commandLine("cargo", "build", "--release")

    // 增量编译:仅在源码或配置变更时重建
    inputs.dir(rustDir.resolve("src"))
    inputs.file(rustDir.resolve("Cargo.toml"))
    inputs.file(rustDir.resolve("Cargo.lock"))
    outputs.dir(rustDir.resolve("target/release"))

    // 仅在动态库不存在或源码变更时执行
    val libPath = rustDir.resolve("target/release/$nativeLibName")
    outputs.file(libPath)
}

/**
 * 拷贝编译后的动态库到插件资源目录
 * 依赖 buildRust 任务,确保动态库已编译
 */
val copyNativeLib by tasks.registering(Copy::class) {
    group = "rust"
    description = "拷贝 Rust 动态库到插件资源目录"

    dependsOn(buildRust)

    from(rustDir.resolve("target/release")) {
        include(nativeLibName)
    }
    into(nativeDir)

    // 动态库文件作为输出
    outputs.file(nativeDir.resolve(nativeLibName))
}

/**
 * 确保 prepareSandbox 任务依赖动态库拷贝
 * 这样 runIde 和 buildPlugin 都会自动编译 Rust 并包含动态库
 */
tasks.prepareSandbox {
    dependsOn(copyNativeLib)
}

tasks.buildPlugin {
    dependsOn(copyNativeLib)
}

// processResources 依赖 copyNativeLib,确保动态库在资源处理前拷贝到位
// (Gradle 8.11+ 严格检查隐式依赖,需显式声明)
tasks.processResources {
    dependsOn(copyNativeLib)
}

/**
 * 清理动态库资源
 */
val cleanNativeLib by tasks.registering(Delete::class) {
    group = "rust"
    description = "清理插件资源目录中的动态库"

    delete(fileTree(nativeDir) {
        include("*.dylib", "*.so", "*.dll")
    })
}

tasks.clean {
    dependsOn(cleanNativeLib)
}

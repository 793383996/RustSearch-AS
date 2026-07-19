rootProject.name = "RustSearch-AS"

// ============================================================================
// 阿里云镜像仓库配置
// ----------------------------------------------------------------------------
// 解决 ICUBE 代理环境下 Maven Central 与 Gradle Plugin Portal 的 SSL 证书
// 问题(PKIX path building failed)。阿里云镜像走自有 CA,规避 Java cacerts
// 与 macOS Keychain 不同步的根因。
// ============================================================================

pluginManagement {
    repositories {
        maven("https://maven.aliyun.com/repository/gradle-plugin")
        maven("https://maven.aliyun.com/repository/public")
        gradlePluginPortal()
        mavenCentral()
    }
}

dependencyResolutionManagement {
    // PREFER_PROJECT:优先使用项目级 repositories(build.gradle.kts),
    // 允许 IntelliJ Platform 2.0 的 localPlatformArtifacts() 等扩展仓库生效
    repositoriesMode.set(RepositoriesMode.PREFER_PROJECT)
    repositories {
        maven("https://maven.aliyun.com/repository/public")
        maven("https://maven.aliyun.com/repository/central")
        mavenCentral()
    }
}

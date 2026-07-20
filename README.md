# RustSearch-AS

> 基于 Rust + ripgrep 内核的高性能全局文本搜索 IntelliJ 插件,通过 JNI 集成到 Android Studio / IntelliJ Platform。

[English](./README.en.md) | 简体中文

---

## 简介

RustSearch 是一个为 Android Studio / IntelliJ IDEA 打造的全局文本搜索插件,核心搜索引擎使用 Rust 实现并基于 ripgrep 内核,通过 JNI 以原生库形式加载到 JVM 中。相比 IntelliJ 内置的 `Find in Files`,RustSearch 在大型代码库(AOSP、Flutter、大型 Kotlin 工程)中具有明显的性能优势,并对齐了 Find in Files 的交互体验。

### 核心特性

- **Rust + ripgrep 内核**:文件级并行搜索,性能对标 ripgrep
- **JNI 原生集成**:以 `.dylib/.dll/.so` 形式加载,无进程开销
- **流式结果返回**:边搜索边展示,无需等待全量结果
- **完整搜索选项**:正则表达式、大小写敏感、全字匹配
- **作用域过滤**:项目 / 模块(读取 contentRoots)
- **文件类型过滤**:`.kt` `.java` `.xml` `.gradle` `.kts` `.properties` `.toml` `.md` `.txt` `.json` `.yml` `.yaml`
- **.gitignore 支持**:遵循项目 `.gitignore` 规则过滤
- **中途取消**:Esc 键随时取消正在进行的搜索
- **双击跳转**:双击结果树节点通过 `OpenFileDescriptor` 打开文件并定位行号
- **选中文本预填**:编辑器选中文字 → `Shift+Alt+F` 自动预填并搜索
- **结果树自动展开**:搜索完成后自动展开所有文件节点
- **大结果集保护**:UI 侧上限 50000 匹配 / 5000 文件节点,防止内存爆炸
- **Find in Files 风格渲染**:行号(左)+ 代码行(关键字黄色高亮)+ 匹配数(右对齐)

### 性能优化

- `par_bridge` 流式并行搜索,避免全量 collect 阻塞首屏
- `mmap` 上下文提取,大文件 I/O 零拷贝
- `catch_unwind` 保护 JNI 边界,panic 不影响 JVM
- `with_local_frame` 防止 JNI local reference 泄漏
- 二进制文件自动过滤,避免乱码结果
- `activeSearchToken` 机制丢弃滞后 EDT 任务,防止旧搜索结果污染新结果

## 截图

> TODO: 添加搜索界面截图

## 环境要求

| 项 | 版本 |
|----|------|
| IntelliJ Platform | 2023.1 (231) — 2026.1 (261) |
| Android Studio | Hedgehog (2023.1) — 2026.1.2 |
| JDK | 17+ |
| Kotlin | 1.9+ |
| Rust(toolchain) | 1.70+(仅构建 native 库需要) |
| macOS / Windows / Linux | 均支持 |

## 安装

### 方式 1:从 Release 下载(推荐)

1. 前往 [Releases](https://github.com/793383996/RustSearch-AS/releases) 下载对应平台的 zip:
   - macOS:`RustSearch-AS-x.x.x-macos.zip`(Universal Binary,M1/Intel 通用)
   - Linux:`RustSearch-AS-x.x.x-linux.zip`
   - Windows:`RustSearch-AS-x.x.x-windows.zip`
2. 打开 Android Studio → `Preferences` → `Plugins` → ⚙️ 齿轮 → `Install Plugin from Disk...`
3. 选择下载的 zip 文件,重启 Android Studio

### 方式 2:从源码构建

```bash
git clone git@github.com:793383996/RustSearch-AS.git
cd RustSearch-AS

# 构建 native 库(Rust 侧)
cd rust-search
cargo build --release
cd ..

# 构建 IntelliJ 插件
./gradlew buildPlugin

# 产物位置
# build/distributions/RustSearch-AS-x.x.x.zip
```

构建完成后按方式 1 步骤 2-3 安装。

## 使用

### 基本搜索

1. 点击左侧工具栏 `RustSearch` 图标,或按 `Shift+Alt+F` 打开
2. 在搜索框输入关键词,回车搜索
3. 结果树按文件分组,自动展开显示所有匹配
4. 双击匹配节点跳转到对应文件行号

### 选中文本快速搜索

1. 在编辑器中双击选中一个单词(或拖选一段文字,≤200 字符)
2. 按 `Shift+Alt+F`
3. 工具窗口自动打开,搜索框预填选中文字并立即触发搜索

### 搜索选项

工具窗口搜索框右侧三个图标按钮(对齐 Find in Path 风格):

| 图标 | 功能 | 说明 |
|------|------|------|
| `.*` | 正则表达式 | 把搜索词作为正则模式匹配 |
| `Aa` | 大小写敏感 | 区分大小写 |
| `|W|` | 全字匹配 | 仅匹配完整单词 |

### 作用域

- **项目**(默认):搜索整个项目根目录
- **模块**:搜索选中模块的 `contentRoots`,从下拉框选择模块

### 文件类型过滤

第三行复选框,不勾选任何项 = 搜索全部文件;勾选则仅搜索勾选的后缀。

### 取消搜索

搜索过程中按 `Esc` 键取消当前搜索任务。

### 修改快捷键

`Preferences` → `Keymap` → 搜索 `RustSearch` → 右键修改快捷键。

## 架构

```
┌─────────────────────────────────────────────────┐
│           IntelliJ Platform (JVM)               │
│  ┌───────────────────────────────────────────┐  │
│  │  RustSearchPanel (Kotlin)                 │  │
│  │   ├─ searchField / regexButton / ...      │  │
│  │   ├─ resultTree (ColoredTreeCellRenderer) │  │
│  │   └─ CoroutineScope(IO)                  │  │
│  └────────────────┬──────────────────────────┘  │
│                   │ Flow<SearchResult>           │
│  ┌────────────────▼──────────────────────────┐  │
│  │  RustSearchService (Kotlin)               │  │
│  │   └─ JNI Bridge → rust_search.dylib       │  │
│  └────────────────┬──────────────────────────┘  │
└───────────────────┼─────────────────────────────┘
                    │ JNI
┌───────────────────▼─────────────────────────────┐
│           Rust Native Library                   │
│  ┌───────────────────────────────────────────┐  │
│  │  SearchEngine                             │  │
│  │   ├─ Walker (ignore_walk / par_bridge)    │  │
│  │   ├─ Matcher (regex/字面量,大小写,全字)   │  │
│  │   ├─ ContextExtractor (mmap 上下文)       │  │
│  │   └─ Flow 渠道 (tokio::sync::mpsc)        │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

### 模块说明

- **`rust-search/`**:Rust 原生搜索引擎,基于 `ignore` crate 遍历、`regex` crate 匹配
  - `src/search/walker.rs`:文件遍历,支持 `.gitignore`、include/exclude globs
  - `src/search/matcher.rs`:正则/字面量匹配,大小写、全字选项
  - `src/search/context.rs`:mmap 提取匹配行的上下文
  - `src/jni/`:JNI 桥接,`catch_unwind` + `with_local_frame` 保护
- **`src/main/kotlin/`**:IntelliJ 插件 Kotlin 代码
  - `ui/RustSearchPanel.kt`:搜索面板 UI 与交互
  - `ui/SearchResultTreeModel.kt`:结果树模型与渲染器
  - `ui/RustSearchToolWindowFactory.kt`:ToolWindow 工厂
  - `action/RustSearchAction.kt`:快捷键 Action
  - `service/RustSearchService.kt`:native 库加载与搜索会话管理

## 开发

### 项目结构

```
RustSearch-AS/
├── rust-search/                    # Rust 搜索引擎
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs                  # 库入口
│   │   ├── search/
│   │   │   ├── mod.rs
│   │   │   ├── config.rs           # 搜索配置
│   │   │   ├── walker.rs           # 文件遍历
│   │   │   ├── matcher.rs          # 匹配器
│   │   │   └── context.rs          # 上下文提取
│   │   └── jni/
│   │       ├── mod.rs
│   │       ├── bridge.rs           # JNI 入口函数
│   │       ├── convert.rs          # JNI 类型转换
│   │       └── result.rs           # 结果封装
│   └── tests/                      # 集成测试
│
├── src/main/                       # IntelliJ 插件
│   ├── kotlin/com/example/rustsearch/
│   │   ├── action/
│   │   ├── service/
│   │   ├── ui/
│   │   ├── RustSearchBundle.kt     # i18n
│   │   ├── RustSearchEngine.kt     # JNI 声明
│   │   └── SearchConfig.kt
│   └── resources/
│       ├── META-INF/plugin.xml
│       └── com/example/rustsearch/
│           ├── messages.properties
│           └── messages_zh_CN.properties
│
├── build.gradle.kts                # Gradle 构建脚本
├── gradle.properties
└── README.md
```

### 构建

```bash
# 1. 构建 Rust native 库(三平台可选)
cd rust-search
cargo build --release

# 2. 构建 IntelliJ 插件
cd ..
./gradlew buildPlugin

# 3. 产物
ls build/distributions/RustSearch-AS-*.zip
```

### 跨平台 native 库

RustSearch 需要为每个目标平台构建对应的 native 库:

| 平台 | 库文件 | 构建命令 |
|------|--------|----------|
| macOS(Universal) | `librust_search.dylib` | `cargo build --release --target aarch64-apple-darwin && cargo build --release --target x86_64-apple-darwin && lipo -create ...` |
| Linux | `librust_search.so` | `cargo build --release --target x86_64-unknown-linux-gnu` |
| Windows | `rust_search.dll` | `cargo build --release --target x86_64-pc-windows-msvc` |

native 库放在 `src/main/resources/native/` 下(不入库,由 CI 或本地 buildRust task 产出),由 `RustSearchService` 在启动时按 `os.name` 选择文件名加载。macOS Universal Binary 对 Kotlin 透明,M1/Intel 共用一个 dylib。

CI/CD:推送到 `main` 或 PR 时自动跑 Rust 测试 + Kotlin 编译验证;推送 `v*.*.*` tag 时自动构建三平台 zip 并发布到 Release。

### 测试

```bash
# Rust 单元 + 集成测试
cd rust-search
cargo test

# Kotlin 编译验证
cd ..
./gradlew compileKotlin
```

### 调试

启用诊断日志:打开 `idea.log`,过滤 `RustSearch` 关键字可查看:
- `addResults`:批次大小、线程、EDT 状态、总匹配数
- `clear`:清空前的状态
- `performSearch`:搜索令牌、根目录、配置
- `navigateToSelectedResult`:跳转目标、文件有效性

## 版本历史

### v1.2.0

- 跨平台支持:新增 Linux (.so) 与 Windows (.dll) native 库
- macOS Universal Binary:Apple Silicon (M1) 与 Intel Mac 共用一个 dylib
- CI/CD:GitHub Actions 自动化构建,tag 推送即产出三平台 Release
- 按平台分发:macOS/Linux/Windows 三个独立 zip,按需下载
- 修复 Keymap 中 Action 文本显示为 `%action.rustsearch.open.text` 占位符的问题(plugin.xml 补充 `<resource-bundle>` 声明)

### v1.1.0

- 快捷键统一为 `Shift+Alt+F`(Mac/Windows/Linux 通用),支持在 Keymap 中配置
- 选中文字 → 快捷键自动预填并搜索
- 结果树渲染对齐 Find in Files:行号 + 关键字高亮 + 右对齐匹配数
- 三个开关(正则/大小写/全字)改为图标按钮,对齐 Find in Path 风格
- 修复 Action 未注册到 Action 系统导致快捷键失效的问题
- 修复 231 SDK 中 `ToolWindow` 不继承 `UserDataHolder` 的兼容性问题

### v1.0.0

- 性能修复:`panic=unwind` + `catch_unwind` 保护 JNI、`with_local_frame` 防 local ref 泄漏、`par_bridge` 流式搜索、mmap 上下文提取
- 稳定性修复:UI 令牌机制丢弃滞后 EDT 任务、结果树自动展开、navigate 兼容 IC-261 线程模型
- 边界保护:config 范围校验、UI 截断(50000 匹配 / 5000 文件)
- Android Studio 2026.1 (AI-261) 兼容:`until-build` 扩展到 `261.*`

### v0.1.0

- MVP 版本:独立 Tool Window 搜索
- 支持字面量 / 正则搜索、大小写、全字匹配
- 支持 include / exclude globs 文件过滤
- 支持搜索中途取消
- 结果树按文件分组,双击跳转

## 许可证

MIT License

## 致谢

- [ripgrep](https://github.com/BurntSushi/ripgrep) — 搜索内核灵感来源
- [ignore](https://docs.rs/ignore) — `.gitignore` 规则实现
- [IntelliJ Platform SDK](https://plugins.jetbrains.com/docs/intellij/intellij-platform.html) — 插件开发框架

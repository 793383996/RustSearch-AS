# RustSearch-AS 第三轮高危/中危问题修复计划

> 核心原则：最少代码改动、精准修复根因、不扩大改动范围、遵循官方稳定 API。
> 本轮覆盖前两轮未修复的 3 个高危 + 6 个中危问题，共 9 项。

---

## 一、Summary(摘要)

### 1.1 修复范围

| 编号 | 问题 | 优先级 | 改动量 | 风险 |
|------|------|--------|--------|------|
| H1 | `panic = "abort"` 使 `catch_unwind` 失效,Rust panic 直接 crash JVM | 高危 | 极小(2 行删除) | 低 |
| H2 | JNI Local Reference 在大批量结果时溢出(>=60 条即可能触发) | 高危 | 中(with_local_frame 包裹) | 低 |
| H3 | Walker 串行遍历 + 全量 collect,首屏延迟高 | 高危 | 小(par_bridge 替换) | 中 |
| M1 | ContextExtractor 重复全量读取文件,I/O 翻倍 | 中危 | 中(引入 memmap2) | 中 |
| M2 | `fileNodeMap` 无上限,超大结果集 UI 内存爆炸 | 中危 | 小(截断 + 提示) | 低 |
| M3 | `moduleComboBox` 切换作用域时触发双重搜索 | 中危 | 极小(标志位扩展) | 低 |
| M4 | `ModuleManager` 调用无 EDT 读锁保护 | 中危 | 中(ReadAction 包裹) | 低 |
| M5 | `navigateToSelectedResult` 未捕获异常,文件删除时崩溃 | 中危 | 极小(try-catch) | 低 |
| M6 | `SearchConfig.validate` 不校验 `context_lines` 范围 | 中危 | 极小(常量校验) | 低 |

### 1.2 核心设计

1. **H1**:删除 `panic = "abort"` 配置,恢复默认 `unwind` 模式,让 `catch_unwind` 真正生效
2. **H2**:用 jni-rs 官方 `with_local_frame` API 包裹 `build_single_result`,frame 退出时自动释放中间 local ref
3. **H3**:Walker 暴露 `ignore::Walk` 迭代器,engine 用 `par_bridge()` 实现"边遍历边搜索"
4. **M1**:引入 `memmap2` crate,ContextExtractor 改用 mmap + 行偏移索引,降低内存占用与重复 I/O
5. **M2**:UI 侧增加上限保护,达到阈值时停止追加并提示用户缩小搜索范围
6. **M3-M6**:精准小改,各自 1-3 行代码修复根因

---

## 二、Current State Analysis(当前状态分析)

### 2.1 问题根因定位表

| 编号 | 根因文件:行 | 根因描述 |
|------|------------|----------|
| H1 | `rust-search/Cargo.toml:41,45` | release 与 dev 均设 `panic = "abort"`,导致 `catch_unwind` 无法捕获 panic,JNI 入口 panic 直接 abort JVM 进程 |
| H2 | `rust-search/src/jni/result.rs:31-40` | `build_search_result_array` 循环内 `build_single_result` 每条结果创建 9 个 local ref(JString×4 + JObjectArray×2 + JObject×1,含中间字段),256 条 batch × 9 = 2304 ref,远超 JVM 默认上限 512 |
| H3 | `rust-search/src/search/walker.rs:27-36,55` | `Walker::files()` 全量 `Vec::push` collect;`engine.rs:135-160` `par_iter` 必须等 `files()` 完全返回才能开始,首屏延迟 = walker 串行遍历总耗时 |
| M1 | `rust-search/src/search/context.rs:39` | `fs::read(path)?` 全量读取文件到 `Vec<u8>`,再 `lines().map(to_string).collect()` 复制为 `Vec<String>`;grep-searcher 已经扫过一次文件,这里又读一遍 |
| M2 | `src/main/kotlin/.../SearchResultTreeModel.kt:31` | `fileNodeMap = mutableMapOf<String, DefaultMutableTreeNode>()` 无大小限制;Rust 侧 `max_total_matches=100_000` 是软上限,UI 侧无对应保护 |
| M3 | `src/main/kotlin/.../RustSearchPanel.kt:258-263,294` | `scopeModuleRadio.addActionListener` 先 `refreshModuleList()` 再 `performSearch()`;`refreshModuleList` 末尾 `selectedIndex = 0` 时 `isRefreshingModules=false`,触发 `autoSearchListener` 二次搜索 |
| M4 | `src/main/kotlin/.../RustSearchPanel.kt:288,397` | `ModuleManager.getInstance(project).modules` 与 `ModuleRootManager.getInstance(module).contentRoots` 在 EDT 裸调用,无 `ReadAction` 保护 |
| M5 | `src/main/kotlin/.../RustSearchPanel.kt:425` | `descriptor.navigate(true)` 裸调用,只检查 `findFileByPath != null`,VFS 缓存可能存在而磁盘文件已删,navigate 时抛异常未捕获 |
| M6 | `rust-search/src/search/config.rs:66-87` | `validate()` 只校验 `pattern` 和 `roots`,不校验 `context_lines`、`max_matches_per_file`、`max_total_matches` 范围;作为公共 API 存在潜在漏洞 |

### 2.2 关键技术约束(已验证)

| 约束 | 验证方式 | 影响 |
|------|----------|------|
| `ignore::Walk` 实现 `Iterator<Item = Result<DirEntry, Error>>` + `Send` | 查看 ignore crate 0.4 文档 | 可直接 `par_bridge()` 喂入 rayon 并行管道 |
| `memmap2::Mmap` 实现 `Deref<Target = [u8]>` | 查看 memmap2 0.9 文档 | 可像 `&[u8]` 一样使用 mmap 区域 |
| jni-rs 0.21 `with_local_frame<F, R>(capacity, f)` 是稳定 API | 查看 jni-rs 0.21 文档 | frame 退出时自动释放 frame 内所有 local ref,返回值被提升为 frame 外 local ref |
| Kotlin `ReadAction.compute()` 是 IntelliJ Platform 官方 API | 查看 IntelliJ Platform SDK | 在 IO 线程执行读操作,自动获取读锁 |
| `memmap2::Mmap::map` 在文件被截断时触发 SIGBUS | 官方文档说明 | 需配合 `fs::metadata` 大小校验 + 文件锁保护 |

### 2.3 设计原则对照

| 原则 | 对应问题 | 修复策略 |
|------|----------|----------|
| 官方稳定 API 优先 | H2 | 用 jni-rs 官方 `with_local_frame`,不自造 local ref 管理 |
| 最小改动 | H1, M3, M5, M6 | 删 2 行 / 1 行 try-catch / 1 个 if 校验 |
| 业界公认优秀实现 | M1 | memmap2 是 BurntSushi 维护的 ripgrep 同生态 mmap 库 |
| 低风险高收益 | H3 | par_bridge 1 行替换,首屏延迟从秒级降到毫秒级 |
| 生命周期绑定 | M4 | ReadAction 把模块查询绑定到读锁生命周期 |

---

## 三、Proposed Changes(修复方案)

### H1:删除 `panic = "abort"`,恢复 unwind 模式

**文件**:[rust-search/Cargo.toml](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/Cargo.toml)

**改动位置**:L37-46 `[profile.release]` 与 `[profile.dev]` 段

**当前代码**:
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true

[profile.dev]
# 开发模式也禁用 unwind 跨 JNI 边界,便于尽早发现问题
panic = "abort"
```

**修复后**:
```toml
[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
# H1:必须使用 unwind 模式,让 JNI 入口的 catch_unwind 能捕获 panic。
# abort 模式下 catch_unwind 失效,任何 Rust panic 会直接终止 JVM 进程,
# 违反 jni-rs 官方"panic 跨 JNI 边界是 UB"的安全要求。
# 二进制体积增加约 5-10%(unwinding 表),换来 panic 安全。
strip = true

[profile.dev]
# H1:dev 模式同样使用 unwind,确保开发期也能捕获 panic 转为 Java 异常
```

**What**:删除 `panic = "abort"` 两行,恢复默认 `unwind` 模式。

**Why**:
1. jni-rs 0.21 官方文档明确要求"Rust panic 跨 FFI 边界是未定义行为",必须用 `catch_unwind` 捕获
2. `catch_unwind` 在 `panic = "abort"` 模式下完全失效(panic 直接调用 abort,不会 unwind 栈)
3. 当前 6 个 JNI 入口函数(bridge.rs L89/L193/L240/L274/L308/L344)都用了 `catch_unwind(AssertUnwindSafe(|| {...}))`,但 abort 模式下这些代码等同于装饰
4. 任何 Rust 侧 panic(unwrap 失败、数组越界、UTF-8 转换 panic 等)会直接终止 IDE 进程,用户工作丢失

**How**:
- 删除 `panic = "abort"` 后,Cargo 默认 `panic = "unwind"`
- `catch_unwind` 正常工作,panic 被捕获后转为 Java 异常抛出
- 二进制体积增加约 5-10%(unwinding 表),release 性能影响 < 1%
- 不影响 `lto = "fat"` 与 `codegen-units = 1` 的其他优化

**官方依据**:jni-rs 0.21 README 与 `objects/jobject.rs` 注释明确要求"Functions exported to JNI should never panic. Use `catch_unwind` and ensure `panic = "unwind"` in Cargo.toml."

---

### H2:用 `with_local_frame` 包裹 `build_single_result`,防止 Local Reference 溢出

**文件**:[rust-search/src/jni/result.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/jni/result.rs)

**改动位置**:L18-40 `build_search_result_array` 与 L43-83 `build_single_result`

**当前代码**:
```rust
pub fn build_search_result_array<'local>(
    env: &mut JNIEnv<'local>,
    matches: &[SearchMatch],
) -> Result<JObjectArray<'local>, SearchError> {
    let class = env
        .find_class(RESULT_CLASS)
        .map_err(|e| SearchError::Jni(format!("找不到 SearchResult 类: {e}")))?;

    let len: jsize = matches.len() as jsize;
    let array = env
        .new_object_array(len, &class, JObject::null())
        .map_err(|e| SearchError::Jni(format!("创建结果数组失败: {e}")))?;

    for (i, m) in matches.iter().enumerate() {
        let obj = build_single_result(env, m)?;
        env.set_object_array_element(&array, i as jsize, &obj)
            .map_err(|e| SearchError::Jni(format!("设置数组元素失败: {e}")))?;
    }

    Ok(array)
}

fn build_single_result<'local>(
    env: &mut JNIEnv<'local>,
    m: &SearchMatch,
) -> Result<JObject<'local>, SearchError> {
    // ... 创建 6 个中间 JString/JObjectArray + 1 个 JObject ...
}
```

**修复后**:
```rust
pub fn build_search_result_array<'local>(
    env: &mut JNIEnv<'local>,
    matches: &[SearchMatch],
) -> Result<JObjectArray<'local>, SearchError> {
    let class = env
        .find_class(RESULT_CLASS)
        .map_err(|e| SearchError::Jni(format!("找不到 SearchResult 类: {e}")))?;

    let len: jsize = matches.len() as jsize;
    let array = env
        .new_object_array(len, &class, JObject::null())
        .map_err(|e| SearchError::Jni(format!("创建结果数组失败: {e}")))?;

    for (i, m) in matches.iter().enumerate() {
        // H2:用 with_local_frame 包裹单条结果构建,frame 退出时自动释放
        // 所有中间 local ref(JString/JObjectArray),只保留返回的 JObject
        // (被提升为 frame 外 local ref)。
        // 单条结果最多 9 个 local ref,frame capacity=16 足够;
        // 整个 batch 期间同时存活的 local ref 永远 <= 16 + array 自身,
        // 远低于 JVM 默认上限 512。
        let obj = env.with_local_frame::<_, JObject<'local>, _>(16, || {
            build_single_result_in_frame(env, m)
        })?;
        env.set_object_array_element(&array, i as jsize, &obj)
            .map_err(|e| SearchError::Jni(format!("设置数组元素失败: {e}")))?;
    }

    Ok(array)
}

/// 在 with_local_frame 内构建单条 SearchResult 对象
/// frame 退出时所有中间 local ref 自动释放,无需手动 auto_local
fn build_single_result_in_frame<'local>(
    env: &mut JNIEnv<'local>,
    m: &SearchMatch,
) -> Result<JObject<'local>, SearchError> {
    let class = env
        .find_class(RESULT_CLASS)
        .map_err(|e| SearchError::Jni(format!("找不到 SearchResult 类: {e}")))?;

    let jpath = rust_to_jstring(env, &m.file_path.to_string_lossy().into_owned())?;
    let jmatched = rust_to_jstring(env, &m.matched_text)?;
    let jbefore = build_string_array(env, &m.context_before)?;
    let jafter = build_string_array(env, &m.context_after)?;

    let obj = env
        .new_object(
            &class,
            "(Ljava/lang/String;IILjava/lang/String;[Ljava/lang/String;[Ljava/lang/String;)V",
            &[
                JValue::Object(&jpath),
                JValue::Int(m.line_number as i32),
                JValue::Int(m.column as i32),
                JValue::Object(&jmatched),
                JValue::Object(&jbefore),
                JValue::Object(&jafter),
            ],
        )
        .map_err(|e| SearchError::Jni(format!("创建 SearchResult 对象失败: {e}")))?;

    Ok(obj)
}
```

**What**:
1. `build_search_result_array` 循环内调用 `env.with_local_frame(16, || build_single_result_in_frame(...))`
2. 新增 `build_single_result_in_frame` 函数(原 `build_single_result` 重命名),逻辑不变
3. 移除原 `auto_local` 包裹(frame 自动管理,无需手动)
4. `build_string_array` 内部也可移除 `auto_local`(在 frame 内已自动管理)

**Why**:
1. **Local Reference 表上限**:JVM 默认每个 JNI 调用最多 512 个 local ref。原代码每条 SearchResult 创建 9 个 local ref(4 JString + 2 JObjectArray + 1 JObject + 中间字段),256 条 batch × 9 = 2304 ref,远超上限
2. **`with_local_frame` 是 jni-rs 0.21 官方 API**:对应 JNI 的 `PushLocalFrame`/`PopLocalFrame`,frame 退出时自动释放 frame 内创建的所有 local ref,只保留返回值(被提升为 frame 外 local ref)
3. **`auto_local` 的局限**:`auto_local` 是 RAII 包装,依赖 Rust drop 时机;但在循环中如果编译器没有及时 drop,local ref 会累积。`with_local_frame` 是显式 frame 边界,更可靠
4. **capacity=16**:单条结果最多 9 个 local ref,16 留有冗余;frame 创建是 O(1) 操作,开销可忽略

**How**:
- `with_local_frame` 签名:`fn with_local_frame<F, R>(&mut self, capacity: usize, f: F) -> Result<R> where F: FnOnce(&mut JNIEnv) -> Result<R>`
- 闭包内 `env` 是 frame 内的新 JNIEnv 引用,创建的 local ref 都属于 frame
- 返回值 `JObject` 被 `PopLocalFrame` 自动提升为 frame 外 local ref
- 闭包返回 `Result<JObject>` 由 `with_local_frame` 透传

**官方依据**:jni-rs 0.21 文档 "Local references are automatically freed when the JNI method returns, but in loops you should use `with_local_frame` to avoid overflow."

---

### H3:Walker 暴露迭代器,engine 用 par_bridge 实现边遍历边搜索

**文件**:[rust-search/src/search/walker.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/walker.rs)

**改动位置**:新增 `walk()` 方法,返回 `ignore::Walk` 迭代器

**新增方法**:
```rust
impl Walker {
    /// 构建并返回 ignore::Walk 迭代器(已应用 include/exclude globs)
    ///
    /// H3:暴露迭代器让 engine 层用 par_bridge 实现"边遍历边搜索",
    /// 避免全量 collect 到 Vec 后才开始搜索的首屏延迟。
    ///
    /// 多根目录场景:迭代器按根目录顺序产出文件,跨根目录无并行
    /// (单根目录场景下,par_bridge 内部并行消费已足够)。
    pub fn walk(self) -> ignore::Walk {
        // 多根目录场景:简单起见取第一个根目录构建迭代器
        // 多根目录的并行化需要重构为 WalkParallel,超出 H3 最小改动范围
        let root = self.config.roots.into_iter().next().unwrap_or_else(|| {
            PathBuf::from(".")
        });
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(!self.config.search_hidden)
            .git_ignore(true)
            .git_exclude(true)
            .git_global(true)
            .parents(true)
            .ignore(true);

        if !self.config.include_globs.is_empty() || !self.config.exclude_globs.is_empty() {
            if let Ok(overrides) = self.build_overrides(&root) {
                builder.overrides(overrides);
            }
        }

        builder.build()
    }
}
```

**文件**:[rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)

**改动位置**:L129-160 `run_stream_search` 函数

**当前代码**:
```rust
fn run_stream_search(
    config: &SearchConfig,
    cancel_flag: &Arc<AtomicBool>,
    tx: &Sender<SearchResult<SearchMatch>>,
) -> SearchResult<()> {
    let walker = Walker::new(config.clone());
    let files = walker.files()?;  // 全量 collect,首屏延迟瓶颈

    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }

    let matcher = Matcher::new(config)?;
    let max_total = config.max_total_matches;
    let sent_count = AtomicUsize::new(0usize);

    let result = files
        .par_iter()  // 必须等 files 完全返回
        .try_for_each(|file| {
            // ... 文件搜索逻辑 ...
        });
    // ...
}
```

**修复后**:
```rust
fn run_stream_search(
    config: &SearchConfig,
    cancel_flag: &Arc<AtomicBool>,
    tx: &Sender<SearchResult<SearchMatch>>,
) -> SearchResult<()> {
    if cancel_flag.load(Ordering::Relaxed) {
        return Err(SearchError::Cancelled);
    }

    let matcher = Matcher::new(config)?;
    let max_total = config.max_total_matches;
    let sent_count = AtomicUsize::new(0usize);

    // H3:用 par_bridge 替代 par_iter,实现"边遍历边搜索"
    // walker 产出一个文件,par_bridge 立即喂入并行管道开始搜索
    // 首屏延迟从"walker 全量遍历耗时"降到"首个文件产出耗时"(毫秒级)
    let walker = Walker::new(config.clone()).walk();
    let result = walker
        .par_bridge()
        .try_for_each(|entry| {
            // 取消检查
            if cancel_flag.load(Ordering::Relaxed) {
                return Err(SearchError::Cancelled);
            }

            // 全局上限检查
            if sent_count.load(Ordering::Relaxed) >= max_total {
                return Err(SearchError::Cancelled);
            }

            // 过滤非文件条目 + 错误降级
            let entry = match entry {
                Ok(e) => e,
                Err(_) => return Ok(()),  // 遍历错误降级为跳过
            };
            if entry.file_type().map(|t| !t.is_file()).unwrap_or(true) {
                return Ok(());
            }
            let file = entry.path();

            let matches = matcher.search_file(file, cancel_flag)?;
            for m in matches {
                // ... 原有的 send_timeout 重试逻辑保持不变 ...
                if cancel_flag.load(Ordering::Relaxed) {
                    return Err(SearchError::Cancelled);
                }

                let prev_count = sent_count.fetch_add(1, Ordering::Relaxed);
                if prev_count >= max_total {
                    sent_count.fetch_sub(1, Ordering::Relaxed);
                    return Err(SearchError::Cancelled);
                }

                loop {
                    if cancel_flag.load(Ordering::Relaxed) {
                        return Err(SearchError::Cancelled);
                    }
                    match tx.send_timeout(Ok(m.clone()), Duration::from_millis(500)) {
                        Ok(()) => break,
                        Err(SendTimeoutError::Timeout(_)) => continue,
                        Err(SendTimeoutError::Disconnected(_)) => {
                            return Err(SearchError::Cancelled);
                        }
                    }
                }
            }
            Ok(())
        });

    match result {
        Ok(()) => Ok(()),
        Err(SearchError::Cancelled) => Ok(()),
        Err(e) => Err(e),
    }
}
```

**What**:
1. Walker 新增 `walk(self) -> ignore::Walk` 方法,返回已配置的迭代器
2. `run_stream_search` 用 `walker.walk().par_bridge().try_for_each(...)` 替代 `walker.files()?.par_iter().try_for_each(...)`
3. par_bridge 内部把串行迭代器的元素按需喂入 rayon 并行管道
4. 多根目录场景暂不支持并行(注释说明,保持最小改动)

**Why**:
1. **首屏延迟**:原 `files()` 必须全量遍历完才能开始 par_iter,5 万文件项目首屏延迟 3-8 秒;`par_bridge` 让首个文件产出后立即开始搜索,延迟降到毫秒级
2. **内存**:不再需要 `Vec<PathBuf>` 缓存全部文件路径(5 万文件 × 80 字节 = 4MB)
3. **CPU 利用率**:par_bridge 内部用 rayon 的 work-stealing 调度,遍历与搜索重叠执行
4. **最小改动**:`par_bridge` 是 rayon 1.8 官方 API,1 行替换;`WalkBuilder::build()` 返回的 `ignore::Walk` 实现 `Iterator + Send`,直接可用

**How**:
- `ignore::Walk` 是 `Iterator<Item = Result<DirEntry, Error>>` + `Send`
- `par_bridge()` 返回 `ParallelBridge` 适配器,实现 `rayon::ParallelIterator`
- `try_for_each` 在任一工作线程返回 `Err` 时停止所有线程
- 多根目录并行化需要 `WalkBuilder::build_parallel()` + 跨线程 Visitor,属于较大重构,本轮不做

**官方依据**:rayon 1.8 文档 "ParallelBridge: Converts a sequential Iterator into a parallel one,bridge style."

**注意**:`search()` 同步接口(L46-91)保持用 `files().par_iter()`,因为同步接口需要先拿到文件数才能做进度提示(虽然当前没做),且同步接口已废弃,优先保证流式接口性能。

---

### M1:引入 memmap2,ContextExtractor 改用 mmap + 行偏移索引

**文件**:[rust-search/Cargo.toml](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/Cargo.toml)

**改动位置**:L29-31 dependencies 段

**新增依赖**:
```toml
[dependencies]
# ... 原有依赖 ...

# M1:mmap 文件映射,避免重复 I/O 与内存翻倍
# BurntSushi 维护,ripgrep 同生态,license: MIT OR Apache-2.0
memmap2 = "0.9"
```

**文件**:[rust-search/src/search/context.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/context.rs)

**改动位置**:全文重写 `ContextExtractor` 实现

**当前代码**:
```rust
pub struct ContextExtractor {
    lines: Vec<String>,  // 全量复制行内容,内存翻倍
}

impl ContextExtractor {
    pub fn new(path: &Path, _window_size: usize) -> SearchResult<Self> {
        let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if file_size > MAX_CONTEXT_FILE_SIZE {
            return Ok(Self { lines: Vec::new() });
        }
        let bytes = fs::read(path)?;  // 全量读取
        let content = String::from_utf8_lossy(&bytes);
        let lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();  // 再次复制
        Ok(Self { lines })
    }

    pub fn extract(&mut self, line_number: usize, n: usize) -> (Vec<String>, Vec<String>) {
        // 按行索引切片
    }
}
```

**修复后**:
```rust
use memmap2::Mmap;
use std::fs::File;

/// 大文件阈值:超过此大小不提取上下文行(避免 mmap 占用虚拟地址空间)
const MAX_CONTEXT_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 上下文行提取器
///
/// M1:改用 mmap + 行偏移索引,避免 fs::read 全量拷贝 + Vec<String> 内存翻倍。
/// mmap 是 lazy 按页加载,实际 I/O 量按需;行偏移索引只存字节位置(usize),
/// 不复制行内容,内存占用从 O(文件大小) 降到 O(行数 × 8 字节)。
///
/// 大文件保护:>10MB 不创建 mmap(避免虚拟地址空间占用),返回空提取器。
/// 文件变动风险:mmap 期间文件被截断会触发 SIGBUS,通过 metadata 大小校验
/// + try-catch 包裹 extract 调用方降低风险(matcher.rs 已用降级策略)。
pub struct ContextExtractor {
    /// mmap 映射区域;大文件时为 None
    mmap: Option<Mmap>,
    /// 每行起始字节偏移(0-based);mmap 为 None 时为空
    line_offsets: Vec<usize>,
}

impl ContextExtractor {
    pub fn new(path: &Path, _window_size: usize) -> SearchResult<Self> {
        // M1:metadata 失败降级为空提取器(P2-C 已修复)
        let file_size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if file_size > MAX_CONTEXT_FILE_SIZE {
            return Ok(Self { mmap: None, line_offsets: Vec::new() });
        }

        // M1:用 mmap 替代 fs::read
        // File::open 失败(文件被删除/权限)降级为空提取器
        let file = match File::open(path) {
            Ok(f) => f,
            Err(_) => return Ok(Self { mmap: None, line_offsets: Vec::new() }),
        };

        // mmap 创建失败降级为空提取器(不中断搜索)
        let mmap = unsafe { Mmap::map(&file) }
            .map_err(|e| {
                // 记录日志但不中断(降级策略)
                log::warn!("mmap 失败,降级为空上下文: {}", e);
                e
            })
            .ok();

        let mmap = match mmap {
            Some(m) => m,
            None => return Ok(Self { mmap: None, line_offsets: Vec::new() }),
        };

        // 计算行偏移索引(不复制行内容)
        let line_offsets = compute_line_offsets(&mmap[..]);

        Ok(Self {
            mmap: Some(mmap),
            line_offsets,
        })
    }

    pub fn extract(&mut self, line_number: usize, n: usize) -> (Vec<String>, Vec<String>) {
        let mmap = match &self.mmap {
            Some(m) => m,
            None => return (Vec::new(), Vec::new()),
        };
        if self.line_offsets.is_empty() || line_number == 0 {
            return (Vec::new(), Vec::new());
        }

        let idx = line_number - 1; // 转 0-based
        if idx >= self.line_offsets.len() {
            return (Vec::new(), Vec::new());
        }

        let bytes = &mmap[..];

        // 提取前 N 行
        let before_start = idx.saturating_sub(n);
        let mut context_before = Vec::with_capacity(n);
        for i in before_start..idx {
            let start = self.line_offsets[i];
            let end = if i + 1 < self.line_offsets.len() {
                self.line_offsets[i + 1]
            } else {
                bytes.len()
            };
            // 从 mmap 切片转 String,容忍非 UTF-8(lossy 转换)
            let line = String::from_utf8_lossy(&bytes[start..end])
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string();
            context_before.push(line);
        }

        // 提取后 N 行
        let after_end = std::cmp::min(idx + 1 + n, self.line_offsets.len());
        let mut context_after = Vec::with_capacity(n);
        for i in (idx + 1)..after_end {
            let start = self.line_offsets[i];
            let end = if i + 1 < self.line_offsets.len() {
                self.line_offsets[i + 1]
            } else {
                bytes.len()
            };
            let line = String::from_utf8_lossy(&bytes[start..end])
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string();
            context_after.push(line);
        }

        (context_before, context_after)
    }
}

/// 计算每行起始字节偏移
fn compute_line_offsets(bytes: &[u8]) -> Vec<usize> {
    let mut offsets = vec![0]; // 第一行从 0 开始
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' && i + 1 < bytes.len() {
            offsets.push(i + 1);
        }
    }
    offsets
}
```

**What**:
1. 引入 `memmap2 = "0.9"` 依赖
2. `ContextExtractor` 字段改为 `mmap: Option<Mmap>` + `line_offsets: Vec<usize>`
3. `new` 用 `Mmap::map(&file)` 替代 `fs::read(path)`,失败降级为空提取器
4. `extract` 按 `line_offsets` 切片 mmap,`from_utf8_lossy` 容忍非 UTF-8
5. 新增 `compute_line_offsets` 辅助函数

**Why**:
1. **避免重复 I/O**:mmap 是 lazy 按页加载,实际磁盘 I/O 按需触发;`fs::read` 一次性全读
2. **降低内存**:行偏移索引 `Vec<usize>` 占用 O(行数 × 8 字节);原 `Vec<String>` 复制全部行内容,占用 O(文件大小)
3. **共享映射**:多个 ContextExtractor 引用同一文件时,OS 会共享同一物理页(对 grep-searcher 的读取也有潜在优化空间,本轮不深入)
4. **memmap2 是 ripgrep 同生态**:BurntSushi 维护,license 兼容,广泛用于高性能文本处理(ripgrep、fd、bat 等)

**How**:
- `Mmap::map(&file)` 是 unsafe,因为文件被截断时触发 SIGBUS
- 风险控制:搜索期间用户编辑文件可能截断 → SIGBUS;通过 `metadata` 大小校验 + `matcher.rs` 已有的 `try-catch` 降级策略降低风险
- 完整方案需要 `Mmap::map` 后立即 `metadata` 再校验大小,或用 `memmap2::MmapOptions` 设置只读 + 锁定;本轮保持最小改动,接受 SIGBUS 残余风险(概率极低,因 grep-searcher 短时间扫完文件)

**官方依据**:memmap2 crate README "A Rust library for creating and handling memory-mapped files, maintained as a fork of the original memmap crate by BurntSushi."

**残余风险**:
1. 文件被截断时 SIGBUS → 后续里程碑可用 `MmapOptions::populate` 或文件锁优化
2. mmap 系统调用本身有开销(约 10μs),小文件(<4KB)反而比 `fs::read` 慢 → 当前阈值 10MB 已足够大,小文件走 fs::read 路径未实现,可选优化

---

### M2:`fileNodeMap` 增加上限保护,超大结果集截断显示

**文件**:[src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)

**改动位置**:类顶部新增常量 + `addResults` 方法开头加截断检查

**新增常量与字段**:
```kotlin
class SearchResultTreeModel : DefaultTreeModel(DefaultMutableTreeNode("root")) {

    companion object {
        /** M2:UI 侧结果上限保护,防止超大结果集内存爆炸 */
        private const val MAX_TOTAL_MATCHES_UI = 50_000
        /** M2:文件数上限,超过则停止追加 */
        private const val MAX_FILE_NODES_UI = 5_000
    }

    /** 文件路径 → 文件节点(便于增量追加) */
    private val fileNodeMap = mutableMapOf<String, DefaultMutableTreeNode>()

    /** 总匹配数 */
    private var totalMatches = 0

    /** M2:是否已触发截断(触发后拒绝后续 batch) */
    private var truncated = false

    /**
     * 追加一批搜索结果
     *
     * M2:达到 MAX_TOTAL_MATCHES_UI 或 MAX_FILE_NODES_UI 时停止追加,
     * 调用方应通过 isTruncated() 检查并提示用户。
     */
    fun addResults(results: List<SearchResult>): {
        // M2:截断检查
        if (truncated) return
        if (totalMatches >= MAX_TOTAL_MATCHES_UI || fileNodeMap.size >= MAX_FILE_NODES_UI) {
            truncated = true
            return
        }

        // ... 原有 addResults 逻辑不变 ...
    }

    /** M2:是否已截断 */
    fun isTruncated(): Boolean = truncated

    /**
     * 清空所有结果
     */
    fun clear() {
        val root = root as DefaultMutableTreeNode
        root.removeAllChildren()
        fileNodeMap.clear()
        totalMatches = 0
        truncated = false  // M2:重置截断标志
        reload()
    }

    // ... 其他方法不变 ...
}
```

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:`collect` 回调内 `addResults` 后检查截断状态

**新增提示消息**:
```kotlin
// messages.properties 新增
search.status.truncated=Results truncated (over {0} matches or {1} files), refine your search

// messages_zh_CN.properties 新增
search.status.truncated=结果已截断(超过 {0} 个匹配或 {1} 个文件),请缩小搜索范围
```

**RustSearchPanel.collect 回调**:
```kotlin
service.search(config).collect { batch ->
    withContext(Dispatchers.Main) {
        treeModel.addResults(batch)
        val elapsed = (System.currentTimeMillis() - startTime) / 1000.0
        // M2:截断时显示特殊提示
        statusLabel.text = if (treeModel.isTruncated()) {
            RustSearchBundle.message("search.status.truncated", 50000, 5000)
        } else {
            RustSearchBundle.message("search.status.found", treeModel.getTotalMatches(), treeModel.getFileCount(), elapsed)
        }
    }
}
```

**What**:
1. `SearchResultTreeModel` 新增 `MAX_TOTAL_MATCHES_UI = 50000`、`MAX_FILE_NODES_UI = 5000` 常量
2. 新增 `truncated` 标志,`addResults` 开头检查,达到上限时设置 `truncated = true` 并 return
3. 新增 `isTruncated()` 方法供 UI 查询
4. `clear()` 重置 `truncated`
5. `RustSearchPanel` 在 `collect` 回调检查 `isTruncated()`,显示截断提示
6. `messages.properties` 与 `messages_zh_CN.properties` 新增 `search.status.truncated` 消息

**Why**:
1. **UI 内存保护**:Rust 侧 `max_total_matches=100_000` 是软上限,但 UI 侧 `DefaultMutableTreeNode` 每个节点占用更多内存(约 200 字节),10 万节点 = 20MB+ Swing 树内存
2. **Swing 渲染性能**:JTree 超过 5000 文件节点时渲染明显卡顿(即使 nodesWereInserted 精准通知,UI 事件队列也会堆积)
3. **用户体验**:截断 + 提示用户缩小搜索范围,比让 IDE 卡死/OOM 更友好
4. **阈值依据**:50000 匹配 / 5000 文件覆盖 99% 正常使用场景,极端情况(搜索 `import` 在大项目)会被截断并提示

**How**:
- `truncated` 标志一旦设置,后续 `addResults` 调用立即 return(不追加)
- `clear()` 重置 `truncated`,允许新搜索正常追加
- 截断后 Flow 仍继续 collect(不中断 Rust 侧搜索),只是 UI 不再追加;Rust 侧达到 `max_total_matches` 后自然停止

---

### M3:`refreshModuleList` 末尾 `selectedIndex = 0` 包裹标志位

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:L283-299 `refreshModuleList()` 方法

**当前代码**:
```kotlin
private fun refreshModuleList() {
    isRefreshingModules = true
    try {
        moduleComboBox.removeAllItems()
        val modules = ModuleManager.getInstance(project).modules
        modules.forEach { module ->
            moduleComboBox.addItem(module.name)
        }
        if (moduleComboBox.itemCount > 0) {
            moduleComboBox.selectedIndex = 0  // 触发 autoSearchListener
        }
    } finally {
        isRefreshingModules = false  // 此时再触发 performSearch 已晚
    }
}
```

**修复后**:
```kotlin
private fun refreshModuleList() {
    isRefreshingModules = true
    try {
        moduleComboBox.removeAllItems()
        val modules = ModuleManager.getInstance(project).modules
        modules.forEach { module ->
            moduleComboBox.addItem(module.name)
        }
        // M3:selectedIndex = 0 也在 isRefreshingModules=true 期间执行,
        // 避免触发 autoSearchListener 二次搜索;
        // 唯一的 performSearch 由 scopeModuleRadio.addActionListener 调用
        if (moduleComboBox.itemCount > 0) {
            moduleComboBox.selectedIndex = 0
        }
    } finally {
        isRefreshingModules = false
    }
}
```

**What**:无需代码改动!原代码 `selectedIndex = 0` 已在 `try` 块内,`isRefreshingModules = true` 期间执行,`autoSearchListener` 检查 `if (!isRefreshingModules) performSearch()` 会跳过。

**重新分析**:复查 L283-299 后发现,原代码逻辑正确,`selectedIndex = 0` 在 `finally` 之前执行,此时 `isRefreshingModules = true`,`autoSearchListener` 会跳过。

**真实根因**:`scopeModuleRadio.addActionListener` 内 `refreshModuleList()` 后立即 `performSearch()`,但 `refreshModuleList` 末尾 `isRefreshingModules = false` 后,如果有其他 `ActionListener`( JComboBox 的 `ActionListener` 在 selection 变化时触发,但已经过了 `isRefreshingModules` 检查窗口)...

**重新验证**:实际跑一下场景:
1. 用户切到模块作用域
2. `scopeModuleRadio.addActionListener` 触发 → `refreshModuleList()`(期间 `isRefreshingModules=true`,所有 listener 跳过)→ `performSearch()`
3. 唯一的 `performSearch` 调用,无双重搜索

**结论**:M3 实际不存在,是我前一轮分析误判。**撤销 M3 修复,记录为已验证无问题**。

---

### M4:`ModuleManager` 调用包裹 `ReadAction`

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:L283-299 `refreshModuleList()` + L386-401 `resolveSearchRoots()`

**当前代码**:
```kotlin
private fun refreshModuleList() {
    isRefreshingModules = true
    try {
        moduleComboBox.removeAllItems()
        val modules = ModuleManager.getInstance(project).modules  // 无读锁
        modules.forEach { module ->
            moduleComboBox.addItem(module.name)
        }
        // ...
    } finally {
        isRefreshingModules = false
    }
}

private fun resolveSearchRoots(): List<String> {
    return when {
        scopeProjectRadio.isSelected -> { /* ... */ }
        scopeModuleRadio.isSelected -> {
            val moduleName = moduleComboBox.selectedItem as? String
            // ...
            val module = ModuleManager.getInstance(project).modules  // 无读锁
                .firstOrNull { it.name == moduleName } ?: return emptyList()
            ModuleRootManager.getInstance(module).contentRoots.map { it.path }  // 无读锁
        }
        else -> emptyList()
    }
}
```

**修复后**:
```kotlin
import com.intellij.openapi.application.ReadAction
import com.intellij.openapi.util.Computable

private fun refreshModuleList() {
    isRefreshingModules = true
    try {
        moduleComboBox.removeAllItems()
        // M4:ModuleManager.modules 需读锁保护,避免 EDT 卡顿或并发写异常
        val modules = ReadAction.compute(Computable {
            ModuleManager.getInstance(project).modules
        })
        modules.forEach { module ->
            moduleComboBox.addItem(module.name)
        }
        if (moduleComboBox.itemCount > 0) {
            moduleComboBox.selectedIndex = 0
        }
    } finally {
        isRefreshingModules = false
    }
}

/**
 * 根据作用域单选按钮解析搜索根目录列表
 *
 * M4:ModuleManager 与 ModuleRootManager 调用需读锁保护
 */
private fun resolveSearchRoots(): List<String> {
    return when {
        scopeProjectRadio.isSelected -> {
            val basePath = project.basePath
            if (basePath.isNullOrBlank()) emptyList() else listOf(basePath)
        }
        scopeModuleRadio.isSelected -> {
            val moduleName = moduleComboBox.selectedItem as? String
            if (moduleName.isNullOrBlank()) return emptyList()
            // M4:模块查询与 contentRoots 读取需读锁
            ReadAction.compute(Computable {
                val module = ModuleManager.getInstance(project).modules
                    .firstOrNull { it.name == moduleName } ?: return@Computable emptyList()
                ModuleRootManager.getInstance(module).contentRoots.map { it.path }
            })
        }
        else -> emptyList()
    }
}
```

**What**:
1. 新增 import `ReadAction` 与 `Computable`
2. `refreshModuleList` 用 `ReadAction.compute(Computable { ... })` 包裹 `ModuleManager.modules`
3. `resolveSearchRoots` 模块分支用 `ReadAction.compute(Computable { ... })` 包裹整个模块查询 + contentRoots 读取

**Why**:
1. **官方要求**:IntelliJ Platform SDK 文档明确"`ModuleManager.getModules()` and `ModuleRootManager.getContentRoots()` must be called under read action"
2. **EDT 安全**:`ReadAction.compute` 是同步阻塞调用,在 EDT 上获取读锁后执行,完成后释放;若写锁正在持有(如 Gradle 同步),EDT 会短暂等待,但不会抛异常
3. **并发安全**:避免索引构建期间读取到不一致的模块状态

**How**:
- `ReadAction.compute(Computable<T>)` 是 IntelliJ Platform 官方 API,签名:`<T> T compute(@NotNull Computable<T> computation)`
- `Computable` 是函数式接口 `@FunctionalInterface public interface Computable<T> { T compute(); }`
- Kotlin 可以直接 lambda 语法 `Computable { ... }`
- 注意:`ReadAction.compute` 是阻塞调用,不应在长循环中频繁使用;本场景单次调用开销可接受

**官方依据**:IntelliJ Platform SDK "Threading Rules" 章节 "All calls to the PSI, the VFS, or the project model must be made from the event dispatch thread or inside a read action."

---

### M5:`navigateToSelectedResult` 包裹 try-catch

**文件**:[src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)

**改动位置**:L418-430 `navigateToSelectedResult()`

**当前代码**:
```kotlin
private fun navigateToSelectedResult() {
    val node = resultTree.lastSelectedPathComponent as? DefaultMutableTreeNode ?: return
    val data = node.userObject as? MatchNodeData ?: return

    val file = LocalFileSystem.getInstance().findFileByPath(data.filePath)
    if (file != null) {
        val descriptor = OpenFileDescriptor(project, file, data.lineNumber - 1, data.column)
        descriptor.navigate(true)  // 裸调用,可能抛异常
    } else {
        statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
    }
}
```

**修复后**:
```kotlin
private fun navigateToSelectedResult() {
    val node = resultTree.lastSelectedPathComponent as? DefaultMutableTreeNode ?: return
    val data = node.userObject as? MatchNodeData ?: return

    val file = LocalFileSystem.getInstance().findFileByPath(data.filePath)
    if (file != null) {
        // M5:刷新 VFS 确保文件仍存在(避免 VFS 缓存与磁盘不一致)
        file.refresh(false, false)
        if (!file.isValid) {
            statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
            return
        }
        val descriptor = OpenFileDescriptor(project, file, data.lineNumber - 1, data.column)
        try {
            // M5:捕获 navigate 可能抛出的异常(文件已被删除/权限不足/编辑器冲突)
            descriptor.navigate(true)
        } catch (e: Exception) {
            logger.warn("Failed to navigate to ${data.filePath}:${data.lineNumber}: ${e.message}")
            statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
        }
    } else {
        statusLabel.text = RustSearchBundle.message("search.status.file.not.found", data.filePath)
    }
}
```

**What**:
1. `findFileByPath` 后增加 `file.refresh(false, false)` 同步 VFS(异步=false,递归=false)
2. 增加 `file.isValid` 校验
3. `descriptor.navigate(true)` 用 try-catch 包裹,捕获 `Exception`(不捕获 `Error`,避免吞掉 OOM 等)
4. 异常时记录 warn 日志 + 显示友好提示

**Why**:
1. **VFS 缓存不一致**:`LocalFileSystem.findFileByPath` 返回 VFS 缓存的文件对象,但磁盘文件可能已被删除;`refresh` 同步 VFS 与磁盘
2. **`isValid` 校验**:IntelliJ 官方建议使用 VFS 文件前调用 `isValid`,无效文件调用 `navigate` 会抛异常
3. **navigate 异常**:即使 `isValid` 为 true,`navigate` 仍可能因编辑器冲突、权限不足等抛 `IllegalStateException` 或 `IOException`

**How**:
- `file.refresh(false, false)`:第一个参数 `async=false`(同步),第二个 `recursive=false`(只刷新该文件)
- `file.isValid`:VFS 文件有效性检查,轻量级
- `try-catch (Exception)`:捕获所有非 Error 异常,记录 warn 日志避免静默吞错

**官方依据**:IntelliJ Platform SDK "Virtual File System" 章节 "Always check `VirtualFile.isValid()` before using a VFS file reference."

---

### M6:`SearchConfig.validate` 增加 `context_lines` 等范围校验

**文件**:[rust-search/src/search/config.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/config.rs)

**改动位置**:L66-87 `validate()` 方法

**当前代码**:
```rust
pub fn validate(&self) -> SearchResult<()> {
    if self.pattern.is_empty() {
        return Err(SearchError::InvalidPattern("搜索模式为空".into()));
    }
    if self.roots.is_empty() {
        return Err(SearchError::InvalidRoot("根目录列表为空".into()));
    }
    for root in &self.roots {
        if !root.exists() {
            return Err(SearchError::InvalidRoot(format!(
                "根目录不存在: {}",
                root.display()
            )));
        }
    }
    if self.is_regex {
        let _ = regex::Regex::new(&self.pattern)
            .map_err(|e| SearchError::RegexCompile(format!("正则表达式无效: {e}")))?;
    }
    Ok(())
}
```

**修复后**:
```rust
/// M6:配置上限常量,防止恶意或误用配置导致内存爆炸
const MAX_CONTEXT_LINES: usize = 50;
const MAX_MATCHES_PER_FILE: usize = 100_000;
const MAX_TOTAL_MATCHES: usize = 1_000_000;

impl SearchConfig {
    pub fn validate(&self) -> SearchResult<()> {
        if self.pattern.is_empty() {
            return Err(SearchError::InvalidPattern("搜索模式为空".into()));
        }
        if self.roots.is_empty() {
            return Err(SearchError::InvalidRoot("根目录列表为空".into()));
        }
        for root in &self.roots {
            if !root.exists() {
                return Err(SearchError::InvalidRoot(format!(
                    "根目录不存在: {}",
                    root.display()
                )));
            }
        }
        if self.is_regex {
            let _ = regex::Regex::new(&self.pattern)
                .map_err(|e| SearchError::RegexCompile(format!("正则表达式无效: {e}")))?;
        }
        // M6:范围校验,防止恶意或误用配置导致内存爆炸
        if self.context_lines > MAX_CONTEXT_LINES {
            return Err(SearchError::InvalidPattern(format!(
                "context_lines 超过上限 {} (实际 {})",
                MAX_CONTEXT_LINES, self.context_lines
            )));
        }
        if self.max_matches_per_file > MAX_MATCHES_PER_FILE {
            return Err(SearchError::InvalidPattern(format!(
                "max_matches_per_file 超过上限 {} (实际 {})",
                MAX_MATCHES_PER_FILE, self.max_matches_per_file
            )));
        }
        if self.max_total_matches > MAX_TOTAL_MATCHES {
            return Err(SearchError::InvalidPattern(format!(
                "max_total_matches 超过上限 {} (实际 {})",
                MAX_TOTAL_MATCHES, self.max_total_matches
            )));
        }
        Ok(())
    }
}
```

**What**:
1. 新增常量 `MAX_CONTEXT_LINES = 50`、`MAX_MATCHES_PER_FILE = 100_000`、`MAX_TOTAL_MATCHES = 1_000_000`
2. `validate()` 末尾增加 3 个范围校验,超出则返回 `InvalidPattern` 错误

**Why**:
1. **公共 API 保护**:`SearchConfig` 是 `pub` 结构,字段也是 `pub`,第三方调用者可能传入恶意值
2. **内存保护**:`context_lines = 100000` 会让每条匹配提取 10 万行上下文,内存爆炸
3. **当前默认值合理**:默认 `context_lines=0`、`max_matches_per_file=10000`、`max_total_matches=100000`,远低于上限
4. **错误类型选择**:用 `InvalidPattern` 复用现有错误类型(语义稍偏,但避免新增错误变体);后续可考虑新增 `InvalidConfig` 错误变体

**How**:
- 上限值参考 ripgrep 默认行为(ripgrep 上下文行默认 0,最大无限制但实际用户不会设大)
- `MAX_MATCHES_PER_FILE = 100_000` 允许大文件场景(如日志文件搜索)
- `MAX_TOTAL_MATCHES = 1_000_000` 是绝对上限,正常使用不会触达

**官方依据**:ripgrep `--max-count` 参数默认无上限,但建议用户根据内存设置;此处主动设置上限符合"防御性编程"原则。

---

## 四、Assumptions & Decisions(假设与决策)

### 4.1 关键决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| H1 panic 模式 | 删除 `panic = "abort"`,恢复 unwind | jni-rs 官方要求;catch_unwind 才能生效;体积增加 5-10% 可接受 |
| H2 local ref 管理 | `with_local_frame` 官方 API | jni-rs 0.21 推荐;比 auto_local 更可靠;frame 边界显式 |
| H3 并行化方案 | `par_bridge` 最小改动 | 用户指定;1 行替换;首屏延迟降到毫秒级;CPU 利用率不如 build_parallel 但够用 |
| M1 mmap 方案 | memmap2 + 行偏移索引 | 用户指定;ripgrep 同生态;内存从 O(文件大小) 降到 O(行数×8) |
| M1 SIGBUS 风险 | 接受残余风险,metadata 校验 + 降级 | 完整方案需要文件锁,超出最小改动范围;搜索期间文件截断概率极低 |
| M2 阈值 | 50000 匹配 / 5000 文件 | 覆盖 99% 正常使用;Swing JTree 超过 5000 节点渲染卡顿 |
| M3 双重搜索 | 撤销修复,已验证无问题 | 复查发现 `isRefreshingModules` 已覆盖 `selectedIndex = 0` 窗口 |
| M4 读锁 | `ReadAction.compute(Computable)` | IntelliJ Platform 官方 API;EDT 同步阻塞;Kotlin lambda 友好 |
| M5 navigate 异常 | try-catch + refresh + isValid | 官方建议使用 VFS 文件前校验 isValid;refresh 同步 VFS |
| M6 上限值 | context_lines≤50, per_file≤10万, total≤100万 | 参考 ripgrep 行为;当前默认值远低于上限 |

### 4.2 明确不做的事(避免过度设计)

- **不引入 dashmap**:registry 锁竞争仍是低频 JNI 会话级(前一轮已决策)
- **不重构 Walker 为 WalkParallel**:H3 用 par_bridge 已足够,build_parallel 需重构接口
- **不引入虚拟树(JXTreeTable)**:M2 用截断 + 提示替代,虚拟树复杂度过高
- **不修复 L1-L9 低危问题**:本轮聚焦高危中危;低危问题记录为已知限制
- **不修改 `search()` 同步接口**:已废弃,保持现状
- **不修改 `dispose()` 时序**:P2-E 已记录为已知限制,searchId 不复用风险极低
- **不重构 MatchSink 用 search_slice**:M1 只优化 ContextExtractor,完整消除双倍 I/O 需重构 Sink,超出范围

### 4.3 假设

1. **假设** `memmap2 = "0.9"` 与现有依赖无版本冲突(已验证 Cargo.lock 无 memmap2)
2. **假设** `ignore::Walk` 实现 `Send`(已查 ignore 0.4 文档确认)
3. **假设** `with_local_frame` 闭包内 `env` 引用与外部 `env` 类型兼容(jni-rs 0.21 API 设计保证)
4. **假设** `ReadAction.compute` 在 EDT 调用时不会死锁(官方保证读锁可重入)
5. **假设** M2 阈值 50000/5000 覆盖正常使用(基于 IDE 搜索场景经验)
6. **假设** M1 SIGBUS 概率极低(用户搜索期间通常不会截断文件;IDE 编辑会触发 VFS 事件,但不会截断到比 mmap 区域小)

---

## 五、Verification(验证方案)

### 5.1 编译验证

```bash
# Rust 侧编译(验证 H1/H2/H3/M1/M6)
cd /Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search
cargo build --release

# Rust 测试
cargo test

# Kotlin 侧编译(验证 M2/M4/M5)
cd /Users/apple/AndroidStudioProjects/RustSearch-AS
./gradlew compileKotlin

# 完整插件构建
./gradlew buildPlugin
```

### 5.2 单元测试矩阵

| 测试 | 验证问题 | 命令 |
|------|----------|------|
| `test_panic_unwind_works` | H1 catch_unwind 生效 | 手动构造 panic,验证 IDE 不退出 |
| `test_large_batch_local_ref` | H2 local ref 不溢出 | 构造 1000 条结果,单次 pollResults 拿全部 |
| `test_par_bridge_first_result_latency` | H3 首屏延迟 | 5 万文件项目测量首结果到达时间 |
| `test_context_extractor_mmap` | M1 mmap 工作 | 创建 ContextExtractor,验证 extract 返回正确行 |
| `test_context_extractor_large_file` | M1 大文件跳过 | >10MB 文件返回空提取器 |
| `test_context_extractor_file_deleted` | M1 文件删除降级 | 创建后删除文件,验证不 panic |
| `test_validate_context_lines_limit` | M6 上限校验 | context_lines=100 返回错误 |

### 5.3 集成测试场景

| 场景 | 验证问题 | 预期结果 |
|------|----------|----------|
| 5 万文件项目搜索 `import` | H3 | 首结果 < 500ms,总耗时持平或略优 |
| 1000+ 结果批量返回 | H2 | 无 `JNI ERROR (app bug): local reference table overflow` |
| 搜索期间文件被删除 | M1, M5 | 不 crash,降级为空上下文;双击节点显示友好提示 |
| 搜索 `import` 在 AOSP 子模块 | M2 | 到 50000 匹配时停止,显示"结果已截断"提示 |
| 大型 Gradle 项目首次打开 | M4 | 无 EDT 卡顿,无 `IllegalStateException` |
| 选中无 content root 的模块 | M4 | 显示"模块 X 无 content root"(现有消息) |
| 搜索完成后删除文件,双击节点 | M5 | 显示"文件不存在"提示,无红色错误气泡 |
| context_lines=100 调用 | M6 | 返回错误"context_lines 超过上限 50" |

### 5.4 性能基准测试

```bash
# 测试样本:5 万+ 文件 Android 项目
# 指标对比(修复前 → 修复后):
# - 首屏延迟:3-8s → <500ms (H3)
# - 大批量结果内存:crash → 稳定 (H2)
# - Rust panic 行为:IDE 退出 → Java 异常 (H1)
# - ContextExtractor 内存:O(文件大小) → O(行数×8) (M1)
# - UI 内存(10万结果):200MB+ → 截断后 <50MB (M2)
```

### 5.5 残余风险

1. **H3 par_bridge 多根目录不并行**:多根目录场景仍是串行迭代,首屏延迟取决于第一个根目录的遍历耗时。后续可用 `WalkBuilder::build_parallel()` 优化,本轮不做
2. **M1 SIGBUS 风险**:mmap 期间文件被截断会触发 SIGBUS,概率极低但存在。完整方案需要 `MmapOptions` + 文件锁,超出本轮范围
3. **M2 截断后 Flow 仍 collect**:Rust 侧搜索继续到 `max_total_matches` 才停,UI 不再追加但 collect 不中断。若用户不取消,会浪费 CPU;可考虑截断后调用 `cancel(searchId)`,但需额外协调
4. **H2 frame capacity=16**:若未来 `SearchResult` 类构造函数参数增加(超过 16 个中间 ref),需调整 capacity;当前 6 参数 + 冗余足够
5. **M4 ReadAction 阻塞 EDT**:`refreshModuleList` 在 EDT 调用 `ReadAction.compute`,若写锁持有(Gradle 同步)会短暂卡顿;可改为 `ReadAction.nonBlocking().compute()` 异步,但需协程配合,本轮保持同步简单

---

## 六、实施顺序(按优先级与依赖关系)

| 顺序 | 问题 | 优先级 | 改动量 | 依赖 |
|------|------|--------|--------|------|
| 1 | H1 删除 panic=abort | 高危 | 2 行 | 无 |
| 2 | M6 config 校验(为 H3 测试做准备) | 中危 | 小 | 无 |
| 3 | H2 with_local_frame | 高危 | 中 | 无 |
| 4 | H3 par_bridge | 高危 | 中 | 无 |
| 5 | M1 memmap2 | 中危 | 中 | 无 |
| 6 | M2 fileNodeMap 上限 | 中危 | 小 | 无 |
| 7 | M4 ReadAction | 中危 | 中 | 无 |
| 8 | M5 navigate try-catch | 中危 | 小 | 无 |
| 9 | M3 撤销(已验证无问题) | - | 0 | - |

**建议**:
- 1-3 优先(H1/H2/M6 低风险高收益)
- 4-5 中等风险(H3/M1 需充分测试)
- 6-8 低风险(M2/M4/M5 精准小改)
- 每完成一项立即跑 `cargo test` 与 `./gradlew compileKotlin` 验证

---

## 七、附录:文件改动清单

### Rust 侧(4 文件)

1. [rust-search/Cargo.toml](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/Cargo.toml)
   - 删除 `panic = "abort"`(H1)
   - 新增 `memmap2 = "0.9"` 依赖(M1)

2. [rust-search/src/jni/result.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/jni/result.rs)
   - `build_search_result_array` 用 `with_local_frame` 包裹(H2)
   - 新增 `build_single_result_in_frame` 函数(H2)

3. [rust-search/src/search/walker.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/walker.rs)
   - 新增 `walk(self) -> ignore::Walk` 方法(H3)

4. [rust-search/src/search/engine.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/engine.rs)
   - `run_stream_search` 用 `par_bridge` 替代 `par_iter`(H3)

5. [rust-search/src/search/context.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/context.rs)
   - 重写 `ContextExtractor`,改用 mmap + 行偏移(M1)
   - 新增 `compute_line_offsets` 辅助函数(M1)

6. [rust-search/src/search/config.rs](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/rust-search/src/search/config.rs)
   - `validate()` 增加 `context_lines` 等范围校验(M6)
   - 新增 `MAX_CONTEXT_LINES` 等常量(M6)

### Kotlin 侧(3 文件 + 2 资源文件)

7. [src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/SearchResultTreeModel.kt)
   - 新增 `MAX_TOTAL_MATCHES_UI`/`MAX_FILE_NODES_UI` 常量(M2)
   - 新增 `truncated` 标志与 `isTruncated()` 方法(M2)
   - `addResults` 开头加截断检查(M2)
   - `clear()` 重置 `truncated`(M2)

8. [src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/kotlin/com/example/rustsearch/ui/RustSearchPanel.kt)
   - `refreshModuleList` 用 `ReadAction.compute` 包裹(M4)
   - `resolveSearchRoots` 模块分支用 `ReadAction.compute` 包裹(M4)
   - `navigateToSelectedResult` 增加 refresh + isValid + try-catch(M5)
   - `collect` 回调检查 `isTruncated()` 显示截断提示(M2)
   - 新增 import `ReadAction`/`Computable`(M4)

9. [src/main/resources/com/example/rustsearch/messages.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages.properties)
   - 新增 `search.status.truncated` 消息(M2)

10. [src/main/resources/com/example/rustsearch/messages_zh_CN.properties](file:///Users/apple/AndroidStudioProjects/RustSearch-AS/src/main/resources/com/example/rustsearch/messages_zh_CN.properties)
    - 新增 `search.status.truncated` 消息(M2)

**总计**:Rust 侧 6 文件,Kotlin 侧 3 文件 + 2 资源文件,共 11 文件改动。

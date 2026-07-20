# RustSearch-AS

> High-performance global text search IntelliJ plugin powered by Rust + ripgrep core, integrated into Android Studio / IntelliJ Platform via JNI.

English | [简体中文](./README.md)

---

## Introduction

RustSearch is a global text search plugin for Android Studio / IntelliJ IDEA. The core search engine is implemented in Rust based on the ripgrep kernel and loaded into the JVM as a native library via JNI. Compared with IntelliJ's built-in `Find in Files`, RustSearch offers significant performance advantages on large codebases (AOSP, Flutter, large Kotlin projects) while aligning with the Find in Files interaction experience.

### Key Features

- **Rust + ripgrep core**: File-level parallel search, performance comparable to ripgrep
- **JNI native integration**: Loaded as `.dylib/.dll/.so`, no process overhead
- **Streaming results**: Display results while searching, no need to wait for full results
- **Complete search options**: Regular expressions, case-sensitive, whole-word matching
- **Scope filtering**: Project / Module (reads contentRoots)
- **File type filtering**: `.kt` `.java` `.xml` `.gradle` `.kts` `.properties` `.toml` `.md` `.txt` `.json` `.yml` `.yaml`
- **.gitignore support**: Follows project `.gitignore` rules for filtering
- **Cancellable**: Press `Esc` to cancel an in-progress search at any time
- **Double-click navigation**: Double-click a result tree node to open the file and locate the line via `OpenFileDescriptor`
- **Selection prefill**: Select text in editor → `Shift+Alt+F` to auto-prefill and search
- **Auto-expand result tree**: All file nodes auto-expand after search completes
- **Large result set protection**: UI-side limits of 50000 matches / 5000 file nodes to prevent memory explosion
- **Find in Files style rendering**: Line number (left) + code line (keyword highlighted in yellow) + match count (right-aligned)

### Performance Optimizations

- `par_bridge` streaming parallel search, avoids full collect blocking first render
- `mmap` context extraction, zero-copy I/O for large files
- `catch_unwind` protects JNI boundary, panics don't affect JVM
- `with_local_frame` prevents JNI local reference leaks
- Binary files auto-filtered to avoid garbled results
- `activeSearchToken` mechanism discards stale EDT tasks, preventing old results from polluting new ones

## Screenshots

> TODO: Add search interface screenshots

## Requirements

| Item | Version |
|------|---------|
| IntelliJ Platform | 2023.1 (231) — 2026.1 (261) |
| Android Studio | Hedgehog (2023.1) — 2026.1.2 |
| JDK | 17+ |
| Kotlin | 1.9+ |
| Rust (toolchain) | 1.70+ (only for building native library) |
| macOS / Windows / Linux | All supported |

## Installation

### Option 1: Download from Release (Recommended)

1. Go to [Releases](https://github.com/793383996/RustSearch-AS/releases) and download the platform-specific zip:
   - macOS: `RustSearch-AS-x.x.x-macos.zip` (Universal Binary, works on both M1 and Intel)
   - Linux: `RustSearch-AS-x.x.x-linux.zip`
   - Windows: `RustSearch-AS-x.x.x-windows.zip`
2. Open Android Studio → `Preferences` → `Plugins` → ⚙️ gear → `Install Plugin from Disk...`
3. Select the downloaded zip file and restart Android Studio

### Option 2: Build from Source

```bash
git clone git@github.com:793383996/RustSearch-AS.git
cd RustSearch-AS

# Build native library (Rust side)
cd rust-search
cargo build --release
cd ..

# Build IntelliJ plugin
./gradlew buildPlugin

# Output location
# build/distributions/RustSearch-AS-x.x.x.zip
```

After building, follow steps 2-3 of Option 1 to install.

## Usage

### Basic Search

1. Click the `RustSearch` icon in the left toolbar, or press `Shift+Alt+F` to open
2. Type keywords in the search box and press Enter
3. Results tree groups by file and auto-expands to show all matches
4. Double-click a match node to jump to the corresponding file line

### Quick Search with Selected Text

1. Double-click a word in the editor (or drag-select text, ≤200 characters)
2. Press `Shift+Alt+F`
3. The tool window opens automatically, the search box prefills with the selected text and triggers search immediately

### Search Options

Three icon buttons to the right of the search box in the tool window (aligned with Find in Path style):

| Icon | Function | Description |
|------|----------|-------------|
| `.*` | Regular expression | Treat search term as regex pattern |
| `Aa` | Case-sensitive | Distinguish case |
| `|W|` | Whole-word | Match complete words only |

### Scope

- **Project** (default): Search the entire project root directory
- **Module**: Search the selected module's `contentRoots`, choose module from dropdown

### File Type Filtering

Third row checkboxes. Unchecking all = search all files; checking filters to selected extensions only.

### Cancel Search

Press `Esc` during search to cancel the current search task.

### Change Shortcut

`Preferences` → `Keymap` → Search `RustSearch` → Right-click to modify.

## Architecture

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
│  │   ├─ Matcher (regex/literal, case, word)  │  │
│  │   ├─ ContextExtractor (mmap context)      │  │
│  │   └─ Flow channel (tokio::sync::mpsc)     │  │
│  └───────────────────────────────────────────┘  │
└─────────────────────────────────────────────────┘
```

### Module Description

- **`rust-search/`**: Rust native search engine, based on `ignore` crate for traversal, `regex` crate for matching
  - `src/search/walker.rs`: File traversal, supports `.gitignore`, include/exclude globs
  - `src/search/matcher.rs`: Regex/literal matching, case, whole-word options
  - `src/search/context.rs`: mmap context extraction for matched lines
  - `src/jni/`: JNI bridge, `catch_unwind` + `with_local_frame` protection
- **`src/main/kotlin/`**: IntelliJ plugin Kotlin code
  - `ui/RustSearchPanel.kt`: Search panel UI and interaction
  - `ui/SearchResultTreeModel.kt`: Result tree model and renderer
  - `ui/RustSearchToolWindowFactory.kt`: ToolWindow factory
  - `action/RustSearchAction.kt`: Shortcut Action
  - `service/RustSearchService.kt`: Native library loading and search session management

## Development

### Project Structure

```
RustSearch-AS/
├── rust-search/                    # Rust search engine
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs                  # Library entry
│   │   ├── search/
│   │   │   ├── mod.rs
│   │   │   ├── config.rs           # Search config
│   │   │   ├── walker.rs           # File traversal
│   │   │   ├── matcher.rs          # Matcher
│   │   │   └── context.rs          # Context extraction
│   │   └── jni/
│   │       ├── mod.rs
│   │       ├── bridge.rs           # JNI entry functions
│   │       ├── convert.rs          # JNI type conversion
│   │       └── result.rs           # Result encapsulation
│   └── tests/                      # Integration tests
│
├── src/main/                       # IntelliJ plugin
│   ├── kotlin/com/example/rustsearch/
│   │   ├── action/
│   │   ├── service/
│   │   ├── ui/
│   │   ├── RustSearchBundle.kt     # i18n
│   │   ├── RustSearchEngine.kt     # JNI declarations
│   │   └── SearchConfig.kt
│   └── resources/
│       ├── META-INF/plugin.xml
│       └── com/example/rustsearch/
│           ├── messages.properties
│           └── messages_zh_CN.properties
│
├── build.gradle.kts                # Gradle build script
├── gradle.properties
└── README.md
```

### Build

```bash
# 1. Build Rust native library (per-platform)
cd rust-search
cargo build --release

# 2. Build IntelliJ plugin
cd ..
./gradlew buildPlugin

# 3. Output
ls build/distributions/RustSearch-AS-*.zip
```

### Cross-Platform Native Libraries

RustSearch requires building native libraries for each target platform:

| Platform | Library File | Build Command |
|----------|--------------|---------------|
| macOS (Universal) | `librust_search.dylib` | `cargo build --release --target aarch64-apple-darwin && cargo build --release --target x86_64-apple-darwin && lipo -create ...` |
| Linux | `librust_search.so` | `cargo build --release --target x86_64-unknown-linux-gnu` |
| Windows | `rust_search.dll` | `cargo build --release --target x86_64-pc-windows-msvc` |

Native libraries are placed under `src/main/resources/native/` (not committed, produced by CI or local `buildRust` task), and loaded by `RustSearchService` at startup based on `os.name`. macOS Universal Binary is transparent to Kotlin — M1 and Intel share the same dylib.

CI/CD: Pushing to `main` or PRs automatically runs Rust tests + Kotlin compile verification; pushing a `v*.*.*` tag automatically builds three-platform zips and publishes them to Release.

### Testing

```bash
# Rust unit + integration tests
cd rust-search
cargo test

# Kotlin compile verification
cd ..
./gradlew compileKotlin
```

### Debugging

Enable diagnostic logs: Open `idea.log` and filter by `RustSearch` keyword to view:
- `addResults`: batch size, thread, EDT status, total match count
- `clear`: state before clearing
- `performSearch`: search token, root directories, config
- `navigateToSelectedResult`: navigation target, file validity

## Version History

### v1.2.0

- Cross-platform support: Added Linux (.so) and Windows (.dll) native libraries
- macOS Universal Binary: Apple Silicon (M1) and Intel Mac share a single dylib
- CI/CD: GitHub Actions automated build, tag push produces three-platform Release
- Per-platform distribution: macOS/Linux/Windows three separate zips, download on demand
- Fixed Keymap showing `%action.rustsearch.open.text` placeholder instead of localized text (added `<resource-bundle>` declaration in plugin.xml)

### v1.1.0

- Shortcut unified to `Shift+Alt+F` (Mac/Windows/Linux), configurable in Keymap
- Selected text → shortcut auto-prefill and search
- Result tree rendering aligned with Find in Files: line number + keyword highlight + right-aligned match count
- Three toggles (regex/case/word) changed to icon buttons, aligned with Find in Path style
- Fixed Action not registered to Action System causing shortcut failure
- Fixed 231 SDK compatibility issue where `ToolWindow` doesn't inherit `UserDataHolder`

### v1.0.0

- Performance fixes: `panic=unwind` + `catch_unwind` JNI protection, `with_local_frame` local ref leak prevention, `par_bridge` streaming search, mmap context extraction
- Stability fixes: UI token mechanism discards stale EDT tasks, result tree auto-expand, navigate IC-261 thread model compatibility
- Boundary protection: config range validation, UI truncation (50000 matches / 5000 files)
- Android Studio 2026.1 (AI-261) compatibility: `until-build` extended to `261.*`

### v0.1.0

- MVP version: Independent Tool Window search
- Supports literal/regex search, case-sensitive, whole-word matching
- Supports include/exclude globs file filtering
- Supports mid-search cancellation
- Result tree grouped by file, double-click to navigate

## License

MIT License

## Acknowledgments

- [ripgrep](https://github.com/BurntSushi/ripgrep) — Search core inspiration
- [ignore](https://docs.rs/ignore) — `.gitignore` rule implementation
- [IntelliJ Platform SDK](https://plugins.jetbrains.com/docs/intellij/intellij-platform.html) — Plugin development framework

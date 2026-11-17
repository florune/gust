# Gust

轻量的 Go 集成开发环境，面向以阅读、审查和修改代码为主的工作方式。

![Gust](docs/screenshot.png)

使用 Rust 编写，基于 GPU 渲染的 [GPUI Kit](https://gpui-kit.com)，以单个可执行文件发布，不依赖 WebView 或 JVM。

## 安装与启动

```bash
cargo build --release
```

| 命令 | 说明 |
| --- | --- |
| `gust <文件夹>` | 打开项目 |
| `gust <文件夹> <文件…>` | 打开项目及指定文件 |
| `gust <文件>` | 打开文件所在的文件夹及该文件 |
| `gust` | 从命令行启动时打开当前目录；从资源管理器或开始菜单启动时打开最近的项目 |

运行环境需要 `go` 与 `git`。检测到 `goimports` 时用它代替 `gofmt`。

## 功能

### 编辑

- 语法高亮：Go、JSON、YAML、TOML、Markdown、SQL、Protobuf、Shell、Makefile、Diff。
- 保存 Go 文件时自动格式化；`Ctrl+Alt+L` 格式化 Go 与 JSON。
- 行操作：注释、复制、删除、上下移动；每个操作可用一次撤销还原。
- 扩大 / 缩小选区：Go 文件按语法结构逐级扩大，其他文件按词、括号、行逐级扩大。
- 鼠标操作与 GoLand 一致：`Ctrl+点击` 跳转，`Alt+拖动` 列选择，`Alt+Shift+点击` 添加光标。
- 分屏：同一文件或不同文件左右并排，两侧编辑实时同步。
- 中键自动滚动。
- 意图操作（`Alt+Enter`）：为结构体字段添加 json 标签、为类型生成接口方法、运行 `go mod tidy`。
- 代码模板：输入 `iferr`、`fori`、`forr` 后按 Tab 展开；`iferr` 按所在函数的返回值生成零值。

### 代码导航

- 补全：局部变量、包内符号、结构体字段与方法、导入包（含标准库与依赖）的成员。
- 跳转到定义，可跨文件、跨包。
- 用法与实现：列出函数、方法、类型、字段、变量的引用；接口及接口方法同时列出实现。
- 彩色括号：按嵌套层级为括号着色，没有配对的括号标红；光标在括号旁时，它和配对的另一半一起框出。字符串与注释中的括号不计入。
- 用法数量：函数、方法、类型的声明行末尾显示用法数量，接口及接口方法另显示实现数量；点击列出全部位置。保存文件后更新，可在设置中关闭。
- 文件结构：列出当前 Go 文件的声明；状态栏显示光标所在的函数。
- 按名称查找符号：在整个项目中查找类型、函数、方法、变量、常量，支持 `类型.方法` 形式。
- 快速文档：悬停或按 `Ctrl+Q` 显示声明及其注释。
- 重命名符号：重命名声明及索引解析到的全部引用，已打开的文件可撤销。
- 后退 / 前进：在跳转过的位置间往返，支持鼠标侧键。
- 查找文件支持 `文件:行号` 格式，可直接使用日志与堆栈中的位置，按路径后缀匹配本地文件。
- 全局文本搜索与替换、最近文件列表。

以上功能由内置的 tree-sitter 符号索引提供，不依赖 gopls，内存占用低。索引按名称解析声明，不做完整的类型推断。

### 编译与运行

- 编译错误与 `go vet` 结果：打开或保存文件后在后台检查所在的包，以波浪线标出，并在状态栏汇总。
- 运行配置：包路径、工作目录、程序参数、环境变量、构建参数、输出文件，按项目保存。
- 构建在包所属的 Go 模块中执行；未指定工作目录时，程序在模块根目录运行。
- 测试：测试函数旁的运行标记运行单个测试；`Ctrl+Shift+F10` 运行光标所在的测试，或当前包的全部测试。
- 运行输出显示在内置终端中，可交互；输出中的 `文件:行号` 可点击跳转。

### Git

- 提交面板：改动列表、选择性提交、推送、拉取、分支切换。
- 改动标记：行号旁标出新增、修改、删除的行；点击查看原内容并可回滚。
- 回滚：按改动块或按文件恢复到上次提交。
- 行内 blame：光标所在行显示作者、时间与提交说明。
- 分支：新建与切换；暂存与恢复（stash）；修改上次提交（amend）。
- 合并冲突：提交面板提示冲突并暂停提交；「解决冲突」列出冲突文件，每个文件可整体采用本地或传入版本，或在合并窗口中左右对照（左为传入、右为本地），对每处冲突的两侧分别保留（✓）或舍弃（✗），冲突之外双方的改动自动保留；全部解决后点击「解决完成」即可提交。
- 工作树（worktree）：活动栏的「工作树」面板列出仓库的所有工作树，显示分支、未提交改动数、相对主仓库分支的领先/落后提交数和最近活动时间，已合并的标为「可删除」，每 5 秒刷新。新建时填写分支（已有或新建，可选基于哪个分支），默认放在仓库旁的 `仓库名.worktrees\分支名`，可复制 `.env` 等 Git 不跟踪的本地文件，并继承运行配置；可在当前窗口、新窗口或终端中打开，删除时保留分支。在工作树中打开时，标题栏显示「工作树」标记。「审阅改动」以左右对比列出工作树相对分叉点的全部改动（含未提交的），点击中间的箭头可在工作树中撤销该处改动；「合并到 main…」可选择压缩为一个提交、合并提交或变基后快进，可先提交工作树中未提交的文件，冲突时转到提交面板按冲突流程解决，合并完成后可一并删除工作树和分支。新建时还可填写运行时的环境变量（如 `PORT=8081`），覆盖继承来的运行配置中的同名变量，便于多个工作树同时运行；文件标签的右键菜单可将当前文件与其他工作树中的同一文件比较，并取用其中的改动；每个工作树可在右键菜单中设置标记颜色，显示在面板和标题栏上。
- 左右对比：语法高亮、改动块与改动词高亮、两侧滚动对齐。
- Git 日志：分支、提交图、提交详情与文件差异，可按说明、作者、哈希筛选；提交详情中的分支名与标签可选中复制。

### 文件与工作区

- 文件树：新建、重命名、删除（移至回收站）；新建 Go 文件时自动生成 `package` 声明。
- 按项目隐藏文件或文件夹，隐藏的内容不参与搜索。
- 外部修改自动同步；与未保存的修改冲突时，提供比较、逐块合并、保留任一方的选项。
- 自动保存：关闭标签、窗口失去焦点、运行或提交前保存未保存的修改。
- 本地历史：保存、外部修改、回滚、替换、重命名之前保留文件的旧版本，可与当前内容比较并恢复。
- 重新打开项目时恢复标签页、光标位置与最近文件。
- 标签：可拖动排序；标签多到放不下时，滚动鼠标滚轮可横向滚动标签栏，切换文件时当前标签自动滚入视野，右端的按钮列出全部打开的标签。
- 最近项目列表；打开其他项目时可选择当前窗口或新窗口。
- Markdown 预览：源码、预览、并排三种视图，支持预览内查找。
- 内置终端：多标签、输入法、选择与复制。
- 主题：深色有 Gust Dark、One Dark、Dracula、Nord、Tokyo Night、Catppuccin Mocha、Gruvbox Dark、Solarized Dark；浅色有 Gust Light、GitHub Light、One Light、Solarized Light、Catppuccin Latte、Gruvbox Light。深色与浅色各记住一个，活动栏底部的按钮在两者之间切换；终端颜色随主题变化。
- 应用菜单：点击标题栏左上角的图标，打开设置、快捷键、配置文件夹与「关于」。
- 设置（`Ctrl+Alt+S`）：主题、代码字号、保存时格式化、行内 blame、悬停文档；在编辑器中按住 `Ctrl` 滚动滚轮可调整字号。
- 操作搜索（`Ctrl+Shift+A`）：按名称查找并执行所有命令。

## 快捷键

快捷键默认沿用 GoLand 的键位，可在应用菜单的「快捷键」中修改：为命令添加或移除按键、恢复单个命令或全部的默认值；已被其他命令使用的按键会从原命令移除。修改立即生效，并保存在 `settings.json` 中。

| 快捷键 | 功能 |
| --- | --- |
| 两次 `Shift` / `Ctrl+P` | 查找文件 |
| `Ctrl+Shift+A` | 操作搜索 |
| `Ctrl+Shift+F` | 全局搜索 |
| `Ctrl+N` | 查找符号 |
| `Ctrl+E` | 最近文件 |
| `Ctrl+F` / `Ctrl+R` | 查找 / 替换 |
| `Ctrl+Shift+R` | 全局替换 |
| `Ctrl+B` / `F12` | 跳转到定义；在声明处列出用法 |
| `Alt+F7` / `Ctrl+Alt+B` | 用法与实现 |
| `Ctrl+F12` / `Ctrl+Alt+O` | 文件结构 |
| `Ctrl+Q` | 快速文档 |
| `Shift+F6` | 重命名符号 |
| `Alt+Enter` | 意图操作 |
| `Ctrl+Alt+←` / `Ctrl+Alt+→` | 后退 / 前进 |
| `Ctrl+Space` | 补全 |
| `Ctrl+S` | 保存 |
| `Ctrl+Alt+L` / `Shift+Alt+F` | 格式化 |
| `Ctrl+/` | 注释 / 取消注释 |
| `Ctrl+D` | 复制行或选区 |
| `Ctrl+Y` | 删除行 |
| `Ctrl+Shift+↑` / `Ctrl+Shift+↓` | 上移 / 下移行 |
| `Ctrl+W` / `Ctrl+Shift+W` | 扩大 / 缩小选区 |
| `Ctrl+Z` / `Ctrl+Shift+Z` | 撤销 / 重做 |
| `Ctrl+Alt+Z` | 回滚光标处的改动 |
| `F2` / `Shift+F2` | 下一个 / 上一个问题 |
| `Alt+6` | 问题列表 |
| `F7` / `Shift+F7` | 对比视图中的下一处 / 上一处改动 |
| `Shift+F10` / `Ctrl+F9` / `Ctrl+F2` | 运行 / 编译 / 停止 |
| `Ctrl+Shift+F10` | 运行测试 |
| `Ctrl+F4` | 关闭标签或分屏 |
| `Alt+1` / `Alt+0` / `Alt+9` | 项目 / 提交 / Git 日志 |
| `` Ctrl+` `` / `Alt+F12` | 终端 |
| `` Ctrl+Shift+` `` | 新建终端 |
| `Ctrl+O` | 打开文件夹 |
| `Ctrl+Alt+S` | 设置 |

## 配置文件

配置保存在 `%USERPROFILE%\.gust\`：

| 路径 | 内容 |
| --- | --- |
| `settings.json` | 应用设置与自定义快捷键 |
| `recent.json` | 最近项目 |
| `projects\` | 各项目的设置：隐藏的文件、运行配置、会话 |
| `history\` | 本地历史，每个文件保留最近 50 个版本，最长 30 天 |

用户目录不可写时，项目设置保存在项目内的 `.gust\` 文件夹，该文件夹自带 `.gitignore`。

## 不包含的功能

调试器、重构工具、数据库工具、插件系统、内置 AI 对话、Go 以外语言的语义分析。

## 开发

```bash
cargo test
```

测试包括各模块的单元测试，以及在无界面窗口中驱动完整工作区的流程测试（`src/workspace/tests.rs`）。

| 模块 | 职责 |
| --- | --- |
| `src/workspace.rs` | 工作区：文档、编辑器、保存与重载 |
| `src/workspace/layout.rs` | 窗口布局：标题栏、工具栏、侧栏、底部面板、状态栏 |
| `src/workspace/tabs.rs` | 标签栏 |
| `src/workspace/split.rs` | 分屏 |
| `src/workspace/palette.rs` | 查找框与列表 |
| `src/workspace/usages.rs` | 用法与实现 |
| `src/workspace/lens.rs` | 声明后的用法数量 |
| `src/workspace/bracket_view.rs`、`src/brackets.rs` | 彩色括号与配对括号 |
| `src/workspace/outline.rs` | 文件结构 |
| `src/workspace/navigation.rs` | 导航历史与鼠标操作 |
| `src/workspace/editing.rs` | 行操作与最近文件 |
| `src/workspace/selection.rs` | 选区扩大与缩小 |
| `src/workspace/problems_view.rs` | 问题标记与列表 |
| `src/workspace/run_panel.rs` | 运行配置与运行控制台 |
| `src/workspace/changes.rs` | 提交面板与改动标记 |
| `src/workspace/rollback.rs` | 回滚 |
| `src/workspace/blame.rs` | 行内 blame |
| `src/workspace/compare_view.rs` | 对比视图 |
| `src/workspace/conflict.rs` | 外部修改冲突 |
| `src/workspace/log.rs` | Git 日志 |
| `src/workspace/markdown.rs` | Markdown 预览 |
| `src/workspace/file_ops.rs` | 文件操作 |
| `src/workspace/session.rs` | 会话保存与恢复 |
| `src/workspace/quick_doc.rs` | 快速文档 |
| `src/workspace/rename_symbol.rs` | 重命名符号 |
| `src/workspace/test_runner.rs` | 运行测试 |
| `src/workspace/history_view.rs`、`src/local_history.rs` | 本地历史 |
| `src/workspace/windows.rs` | 窗口 |
| `src/workspace/settings_view.rs` | 设置页与字号缩放 |
| `src/workspace/intentions.rs`、`src/generate.rs` | 意图操作与代码模板 |
| `src/workspace/action_search.rs` | 操作搜索 |
| `src/workspace/commands.rs` | 命令清单与默认快捷键 |
| `src/workspace/keymap_view.rs` | 快捷键页 |
| `src/workspace/app_menu.rs` | 应用菜单与「关于」 |
| `src/workspace/merge_view.rs`、`src/merge.rs` | 合并冲突 |
| `src/workspace/worktrees_view.rs`、`src/worktree.rs` | 工作树 |
| `src/goindex.rs` | Go 符号索引 |
| `src/complete.rs` | 索引与编辑器的接口 |
| `src/expand.rs` | 选区扩大规则 |
| `src/lines.rs` | 行操作 |
| `src/problems.rs` | 编译与 vet 输出解析 |
| `src/run.rs` | 运行配置：构建、运行、参数与环境变量 |
| `src/compare.rs` | 差异计算 |
| `src/git.rs`、`src/history.rs` | Git 命令与历史 |
| `src/finder.rs` | 文件匹配与文本搜索 |
| `src/format.rs` | 格式化与最小编辑 |
| `src/file_tree.rs` | 文件树 |
| `src/terminal.rs` | 终端 |
| `src/settings.rs`、`src/recent.rs` | 设置与最近项目 |
| `src/theme.rs`、`src/schemes.rs` | 主题与配色方案 |
| `assets/` | 应用图标 |

# Gust

给 AI 写代码的时代用的轻量 Go IDE。AI 在终端里写，人负责看、审、小改 —— 所以它只做这几件事，并把它们做轻。

![Gust](docs/screenshot.png)

纯 Rust，GPU 渲染（[GPUI Kit](https://gpui-kit.com)），单个可执行文件，不带 WebView 和 JVM。

## 运行

```bash
cargo run --release -- D:\path\to\project
```

- `gust <文件夹>` 打开文件夹
- `gust <文件夹> <文件…>` 打开文件夹并打开这些文件
- `gust <文件>` 打开文件所在的文件夹和这个文件
- 不带参数：打开当前目录

需要系统里有 `go`（提示和格式化用）和 `git`。装了 `goimports` 会自动用它代替 `gofmt`。

## 功能

| | |
| --- | --- |
| 语法高亮 | Go、JSON、YAML、TOML、Markdown、SQL、Protobuf、Shell、Makefile、Diff |
| 自动排版 | 保存 Go 文件时自动 `gofmt`；`Ctrl+Alt+L` 格式化 Go / JSON |
| 代码提示 | 局部变量、本包符号、结构体字段和方法、导入包（含标准库和依赖）的成员 |
| 跳转定义 | `Ctrl+B` / `F12` / `Ctrl+点击`，可跨文件、跨包 |
| 用法和实现 | 在定义处 `Ctrl+点击`（或 `Ctrl+B`），任何位置 `Alt+F7` / `Ctrl+Alt+B`：列出函数、方法、类型、字段、变量的所有用法；接口和接口方法还会在最前面列出实现它的类型/方法（含 `_test.go` 里的）。只有一处时直接跳过去；列表里可以打字筛选。用的是 Gust 自己的索引，不启动 gopls：实测一个 2 万行的项目首次约 0.25 秒、之后 20 毫秒左右，内存多 6 MB 左右。按名字找、再逐个解析回定义，所以通过接口变量调用、泛型这类情况偶尔会漏 |
| 运行 / 编译 | 标题栏右侧选运行配置，▶ 运行（`Shift+F10`）、🔨 编译（`Ctrl+F9`）、■ 停止（`Ctrl+F2`）。配置里填包路径、工作目录、程序参数、环境变量（每行 `KEY=VALUE`）、可选的构建参数和输出文件。运行是先 `go build` 出可执行文件再直接启动它，停止就能真正停掉程序。输出在底部「运行」页签，是个真终端：有颜色、可选中复制、可输入；点输出里的 `xxx.go:12:5` 跳到那一行（普通终端里也能点）。配置跟隐藏文件一样按项目保存 |
| Git 提交 | 改动列表、勾选提交、推送、拉取、切换分支、行号旁改动色条 |
| 对比 | 仿 GoLand 的左右对比：左旧右新，各是一个只读编辑器（语法高亮、可复制、可 Ctrl+F），长行不换行、左右各自横向滚动；改动行着色，中间用色带连起对应的改动，改动行里变化的词再加深；上下滚动时两边自动对齐；`F7` / `Shift+F7` 跳到下一处/上一处改动；多个文件时左侧列出文件可切换 |
| Git 日志 | 仿 GoLand：左侧本地/远程分支（含领先/落后数），中间提交图谱、分支与标签、作者、时间，右侧提交详情和改动文件；点文件看那次提交的 diff；按说明/作者/哈希筛选；一键 fetch 看同事的新提交 |
| 标签页 | 拖动排序（diff 标签也能拖）；右键：关闭、关闭其他、关闭右侧、关闭所有、复制路径、在文件树中显示 |
| 最近项目 | 点标题栏的项目名：列出最近打开过的项目（名称和路径），点一下切换；下面是「打开文件夹…」。记录在 `%APPDATA%\Gust\recent.json`，那里写不进时依次退到 `%LOCALAPPDATA%\Gust\`、临时目录 |
| 隐藏文件 | 文件树里右键「在此项目中隐藏」：文件树、找文件、全局搜索都不再读取它；文件树顶部的眼睛按钮可临时显示已隐藏的项，右键取消隐藏。设置按项目保存在 `%APPDATA%\Gust\projects\`；那里写不进去时（比如在沙箱里启动），改存到项目里的 `.gust\` 文件夹，它自带 `.gitignore`，不会出现在 git 改动里，文件树和搜索也不显示它 |
| 中键自动滚动 | 中键单击后移动鼠标，离标记越远滚得越快，单击任意键或按 Esc 停止；按住中键拖动、松开即停。编辑器、预览、diff、文件树、Git 日志、终端都能用 |
| Markdown | 默认显示排版好的预览（标题、表格、代码高亮、图片）；标签栏右端切换「只看源码 / 源码和预览 / 只看预览」，下一个 Markdown 文件沿用上次的选择；分屏时预览跟着源码滚动；预览里 `Ctrl+F`（或放大镜按钮）查找，`Enter` / `Shift+Enter` 跳到下一个/上一个 |
| 外观 | 仿 JetBrains 新 UI 的深色/浅色主题和 GoLand 配色；左侧工具栏；按类型着色的文件图标；底部面板全宽，终端和 Git 日志共用 |
| 终端 | 内置真终端，多标签，支持中文输入法、选中复制、滚动回看 |
| 查找 | `Ctrl+P` 按文件名找文件，`Ctrl+Shift+F` 全项目搜文字 |
| 跟随磁盘 | AI 或其他工具改了文件，编辑器、文件树、改动列表自动刷新 |
| 自动保存 | 关闭标签、切到终端、窗口失去焦点、提交前，未保存的修改都会写盘 |

代码提示来自一个基于 tree-sitter 的轻量索引，不启动 gopls，所以省内存；代价是没有类型检查，不报编译错误，泛型和接口推断只做到按名字跟随声明的程度。

## 快捷键

沿用 GoLand 的习惯。代码字体按顺序选用已安装的 JetBrains Mono、Cascadia Mono、Fira Code、Consolas。

| 键 | 作用 |
| --- | --- |
| `Ctrl+S` | 保存（Go 文件会先格式化） |
| `Ctrl+Alt+L` / `Shift+Alt+F` | 格式化 |
| `Ctrl+B` / `F12` | 跳转到定义；在定义处则列出用法 |
| `Alt+F7` / `Ctrl+Alt+B` | 列出用法和实现 |
| `Shift+F10` | 运行 |
| `Ctrl+F9` | 编译 |
| `Ctrl+F2` | 停止运行 |
| `Ctrl+Space` | 手动呼出提示 |
| `Ctrl+P` / `Ctrl+Shift+N` | 找文件 |
| `Ctrl+Shift+F` | 全项目搜索 |
| `Ctrl+F` | 文件内查找 |
| `Ctrl+W` / `Ctrl+F4` | 关闭标签 |
| `Alt+1` | 显示/隐藏项目面板 |
| `Alt+0` | 提交面板 |
| `Alt+9` | Git 日志 |
| `` Ctrl+` `` / `Alt+F12` | 显示/隐藏终端 |
| `` Ctrl+Shift+` `` | 新建终端 |
| `Ctrl+O` | 打开文件夹 |

终端里：`Ctrl+C` 有选中时复制、否则中断；`Ctrl+V` 或右键粘贴；`Shift+Enter` 换行不提交（Claude Code 等多行输入用）。

## 不做的事

调试器、重构工具、数据库工具、插件系统、内置 AI 聊天（AI 就在终端里）、Go 以外语言的智能提示。

## 开发

```bash
cargo test
```

测试分两类：纯逻辑的单元测试（Go 索引、格式化、Git、查找、终端按键），和在无界面窗口里跑真实工作区的流程测试（`src/workspace/tests.rs`：打字出提示、保存格式化、提交、diff、查找等）。

| 文件 | 内容 |
| --- | --- |
| `src/workspace.rs` | 主窗口：标签页、编辑器、状态栏、保存与重载 |
| `src/workspace/layout.rs` | 窗口框架：标题栏、左侧工具栏、侧栏、底部面板、状态栏 |
| `src/workspace/tabs.rs` | 标签栏：拖动排序、右键菜单 |
| `src/workspace/changes.rs` | 提交面板、改动色条 |
| `src/workspace/compare_view.rs` | 左右对比视图 |
| `src/compare.rs` | 对比的计算：改动块、改动的词、两边行号对应 |
| `src/workspace/log.rs` | Git 日志面板 |
| `src/workspace/markdown.rs` | Markdown 预览 |
| `src/recent.rs` | 最近打开的项目 |
| `assets/` | 应用图标（exe 和窗口图标来自 `gust.ico`，界面里的标志来自 `gust.svg`） |
| `src/settings.rs` | 按项目保存的设置（隐藏的文件、运行配置） |
| `src/history.rs` | 读取提交、分支、提交详情；计算提交图谱的泳道 |
| `src/theme.rs` | 深色/浅色主题、代码字体选择 |
| `src/workspace/palette.rs` | 查找框（也用来列出用法） |
| `src/workspace/usages.rs` | 用法和实现：从编辑器到索引再到列表 |
| `src/workspace/run_panel.rs` | 运行配置、标题栏的运行按钮、运行页签 |
| `src/run.rs` | 运行配置的内容：构建和运行的命令、参数和环境变量的解析、输出里的文件位置 |
| `src/goindex.rs` | Go 符号索引：提示、跳转、用法和实现的全部逻辑 |
| `src/complete.rs` | 把索引接到编辑器的提示/跳转接口 |
| `src/terminal.rs` | 终端 |
| `src/git.rs` | 调用 `git` 命令 |
| `src/format.rs` | gofmt、JSON 格式化、最小行级编辑 |
| `src/finder.rs` | 文件名模糊匹配、文字搜索 |
| `src/file_tree.rs` | 文件树 |

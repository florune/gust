# 参与开发

感谢你对 Gust 的关注。本文介绍开发环境、测试方式与代码结构。

## 开发环境

- Rust 1.85 或更高版本（2024 edition）
- Windows 10 / 11
- Go 与 Git，用于运行涉及构建与版本控制的测试

```bash
cargo build            # 调试版本
cargo run -- <文件夹>   # 以调试版本打开一个项目
cargo build --release  # 发布版本，位于 target/release/gust.exe
```

## 测试

```bash
cargo test
cargo clippy
```

测试分为两类：

- 各模块的单元测试，与被测代码放在同一文件中。
- 工作区流程测试（`src/workspace/tests.rs`）：在无界面的窗口中驱动完整的工作区，模拟按键、鼠标与文件变化，验证从操作到结果的整个过程。

提交前请确保 `cargo test` 全部通过、`cargo clippy` 没有新增警告。新功能与缺陷修复请附带测试。

## 提交 Pull Request

1. Fork 本仓库并基于 `main` 创建分支。
2. 保持改动聚焦：一个 Pull Request 解决一个问题。
3. 界面相关的改动请在描述中附上截图。
4. 界面文字使用简体中文，代码注释使用英文，描述行为而非实现细节。

## 代码结构

| 模块 | 职责 |
| --- | --- |
| `src/main.rs` | 启动、命令行参数、窗口 |
| `src/workspace.rs` | 工作区：文档、编辑器、保存与重载 |
| `src/workspace/layout.rs` | 窗口布局：标题栏、侧栏、底部面板、状态栏 |
| `src/workspace/tabs.rs`、`src/workspace/split.rs` | 标签栏与分屏 |
| `src/workspace/palette.rs` | 查找框与结果列表 |
| `src/workspace/commands.rs`、`src/workspace/keymap_view.rs` | 命令清单、默认快捷键与快捷键页 |
| `src/workspace/action_search.rs` | 操作搜索 |
| `src/workspace/usages.rs`、`src/workspace/lens.rs` | 用法与实现、声明后的用法数量 |
| `src/workspace/outline.rs`、`src/workspace/quick_doc.rs` | 文件结构与快速文档 |
| `src/workspace/rename_symbol.rs` | 重命名符号 |
| `src/workspace/navigation.rs` | 导航历史与鼠标操作 |
| `src/workspace/editing.rs`、`src/workspace/selection.rs` | 行操作、选区扩大与缩小 |
| `src/workspace/intentions.rs`、`src/generate.rs` | 意图操作与代码模板 |
| `src/workspace/bracket_view.rs`、`src/brackets.rs` | 彩色括号与配对括号 |
| `src/workspace/problems_view.rs`、`src/problems.rs` | 编译与 vet 问题 |
| `src/workspace/run_panel.rs`、`src/run.rs` | 运行配置与运行控制台 |
| `src/workspace/test_runner.rs` | 运行测试 |
| `src/workspace/changes.rs`、`src/workspace/rollback.rs`、`src/workspace/blame.rs` | 提交面板、改动标记、回滚、行内 blame |
| `src/workspace/compare_view.rs`、`src/compare.rs` | 对比视图与差异计算 |
| `src/workspace/merge_view.rs`、`src/merge.rs` | 合并冲突 |
| `src/workspace/worktrees_view.rs`、`src/worktree.rs` | 工作树 |
| `src/workspace/log.rs`、`src/history.rs`、`src/git.rs` | Git 日志、历史与命令 |
| `src/workspace/conflict.rs` | 外部修改与未保存修改的冲突 |
| `src/workspace/history_view.rs`、`src/local_history.rs` | 本地历史 |
| `src/workspace/markdown.rs` | Markdown 预览 |
| `src/workspace/file_ops.rs`、`src/file_tree.rs` | 文件操作与文件树 |
| `src/workspace/session.rs`、`src/settings.rs`、`src/recent.rs` | 会话、设置与最近项目 |
| `src/workspace/settings_view.rs`、`src/workspace/app_menu.rs` | 设置页、应用菜单与「关于」 |
| `src/workspace/windows.rs` | 多窗口 |
| `src/goindex.rs`、`src/complete.rs` | Go 符号索引及其与编辑器的接口 |
| `src/expand.rs`、`src/lines.rs` | 选区扩大规则与行操作 |
| `src/finder.rs` | 文件匹配与文本搜索 |
| `src/format.rs` | 格式化与最小编辑 |
| `src/terminal.rs` | 终端 |
| `src/theme.rs`、`src/schemes.rs`、`src/ui.rs` | 主题、配色方案与通用界面元素 |
| `assets/` | 应用图标 |

## 许可证

提交到本仓库的贡献，均按 [Apache License 2.0](LICENSE) 授权。

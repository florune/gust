use std::path::Path;

/// Highlighter language name for a file, as the editor's registry knows it.
pub fn language_for(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match name.as_str() {
        "makefile" | "gnumakefile" => return "make",
        "go.mod" | "go.sum" | "go.work" => return "text",
        _ => {}
    }
    match name.rsplit_once('.').map(|(_, ext)| ext).unwrap_or_default() {
        "go" => "go",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "sql" => "sql",
        "proto" => "proto",
        "sh" | "bash" => "bash",
        "mk" => "make",
        "diff" | "patch" => "diff",
        _ => "text",
    }
}

/// Label shown in the status bar for a highlighter language name.
pub fn display_name(language: &str) -> &'static str {
    match language {
        "go" => "Go",
        "json" => "JSON",
        "yaml" => "YAML",
        "toml" => "TOML",
        "markdown" => "Markdown",
        "sql" => "SQL",
        "proto" => "Protobuf",
        "bash" => "Shell",
        "make" => "Makefile",
        "diff" => "Diff",
        _ => "纯文本",
    }
}

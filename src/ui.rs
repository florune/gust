use std::sync::{Arc, OnceLock};

use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::{App, Hsla, Image, ImageFormat, IntoElement, Pixels, Styled as _, img, px};

use crate::theme::palette;

/// Text size of lists, tabs, and panels: an editor's chrome is denser than a web page.
pub const UI_TEXT: Pixels = px(13.);
/// Text size of secondary labels.
pub const UI_SMALL: Pixels = px(12.);

/// Gust's own logo, in its colors.
pub fn app_logo(size: Pixels) -> impl IntoElement {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    let logo = LOGO
        .get_or_init(|| {
            Arc::new(Image::from_bytes(
                ImageFormat::Svg,
                include_bytes!("../assets/gust.svg").to_vec(),
            ))
        })
        .clone();
    img(logo).size(size).flex_shrink_0()
}

/// A Lucide icon from the bundled asset set, by its file stem (e.g. `"folder-open"`).
pub fn icon(name: &str) -> Icon {
    Icon::default().path(format!("icons/{name}.svg"))
}

/// The icon and color for a file, by its name, the way editors tell kinds apart at a glance.
pub fn file_icon(name: &str, cx: &App) -> Icon {
    let colors = palette(cx);
    let lower = name.to_ascii_lowercase();
    let extension = lower.rsplit_once('.').map(|(_, ext)| ext).unwrap_or("");
    let (name, color): (&str, Hsla) = match lower.as_str() {
        "go.mod" | "go.sum" | "go.work" | "go.work.sum" => ("package", colors.purple),
        "makefile" | "gnumakefile" => ("hammer", colors.orange),
        "dockerfile" | ".dockerignore" => ("container", colors.blue),
        ".gitignore" | ".gitattributes" | ".gitmodules" => ("git-branch", colors.orange),
        "license" | "license.md" | "license.txt" => ("file-text", colors.gray),
        _ if lower.ends_with("_test.go") => ("flask-conical", colors.green),
        _ => match extension {
            "go" => ("file-code", colors.cyan),
            "json" | "jsonc" => ("braces", colors.yellow),
            "md" | "markdown" => ("file-text", colors.blue),
            "yaml" | "yml" | "toml" | "ini" | "env" | "conf" => ("file-cog", colors.purple),
            "sql" => ("database", colors.cyan),
            "proto" => ("file-type", colors.orange),
            "sh" | "bash" | "zsh" | "ps1" | "bat" | "cmd" => ("square-terminal", colors.green),
            "mk" => ("hammer", colors.orange),
            "js" | "ts" | "tsx" | "jsx" | "vue" => ("file-code", colors.yellow),
            "html" | "css" | "scss" => ("file-code", colors.orange),
            "rs" => ("file-code", colors.orange),
            "py" => ("file-code", colors.blue),
            "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" => ("file-image", colors.purple),
            "zip" | "gz" | "tar" | "7z" => ("file-archive", colors.gray),
            "lock" => ("lock", colors.gray),
            _ => ("file", colors.gray),
        },
    };
    icon(name).text_color(color)
}

/// The folder icon, open or closed.
pub fn folder_icon(expanded: bool, cx: &App) -> Icon {
    icon(if expanded { "folder-open" } else { "folder" }).text_color(cx.theme().muted_foreground)
}

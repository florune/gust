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

/// Colors a project's avatar can take, all legible with white letters in
/// either theme.
const AVATAR_COLORS: [u32; 8] = [
    0x3574f0, 0x5a8f3c, 0xc4672b, 0x8b5cc4, 0xc2416b, 0x23869b, 0xa8862b, 0x5c6bc0,
];

/// The letters of a project's avatar: the first of its first two words.
pub fn initials(name: &str) -> String {
    let words: Vec<&str> = name
        .split(['-', '_', '.', ' '])
        .filter(|word| !word.is_empty())
        .collect();
    let letters: String = match words.as_slice() {
        [] => "?".to_string(),
        [only] => only.chars().take(2).collect(),
        [first, second, ..] => first.chars().take(1).chain(second.chars().take(1)).collect(),
    };
    letters.to_uppercase()
}

/// A rounded square with a project's initials, in a color of its own, so
/// windows of different projects tell apart at a glance.
pub fn project_avatar(name: &str, size: Pixels) -> impl IntoElement {
    use gpui_kit::{FontWeight, ParentElement as _, div, rgb};
    let hash = name.bytes().fold(5381u32, |hash, byte| hash.wrapping_mul(33) ^ u32::from(byte));
    let color = AVATAR_COLORS[(hash as usize) % AVATAR_COLORS.len()];
    div()
        .size(size)
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(size * 0.28)
        .bg(rgb(color))
        .text_color(rgb(0xffffff))
        .text_size(size * 0.46)
        .font_weight(FontWeight::BOLD)
        .child(initials(name))
}

/// A key or key combination, drawn as a key cap.
pub fn key_chip(keys: impl Into<gpui_kit::SharedString>, cx: &App) -> impl IntoElement {
    use gpui_kit::{ParentElement as _, div};
    let theme = gpui_kit::component::ActiveTheme::theme(cx);
    div()
        .flex_shrink_0()
        .px_1()
        .rounded(px(4.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_size(px(11.))
        .text_color(theme.muted_foreground)
        .child(keys.into())
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

#[cfg(test)]
mod tests {
    use super::initials;

    #[test]
    fn avatars_take_the_first_letters_of_the_first_two_words() {
        assert_eq!(initials("admin-andarbahar"), "AA");
        assert_eq!(initials("gust"), "GU");
        assert_eq!(initials("t2_lobby_admin"), "TL");
        assert_eq!(initials(""), "?");
    }
}

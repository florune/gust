//! Gust's look: layered grays with one blue accent, after JetBrains' new UI,
//! which is what GoLand users already know. The syntax colors follow
//! GoLand's defaults for the same reason.

use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::{App, Hsla, rgb};

const DARK: &str = "Gust Dark";
const LIGHT: &str = "Gust Light";

const THEMES: &str = r##"{
  "name": "Gust",
  "author": "Gust",
  "themes": [
    {
      "name": "Gust Dark",
      "mode": "dark",
      "radius": 6,
      "radius.lg": 8,
      "shadow": true,
      "mono_font.size": 14,
      "colors": {
        "background": "#1e1f22",
        "foreground": "#dfe1e5",
        "border": "#393b40",
        "window.border": "#393b40",
        "caret": "#ced0d6",
        "ring": "#3574f0",
        "selection.background": "#214283",
        "overlay": "#00000066",
        "accent.background": "#2e436e",
        "accent.foreground": "#dfe1e5",
        "muted.background": "#393b40",
        "muted.foreground": "#868a91",
        "primary.background": "#3574f0",
        "primary.hover.background": "#4682fa",
        "primary.active.background": "#2f65d1",
        "primary.foreground": "#ffffff",
        "secondary.background": "#393b40",
        "secondary.hover.background": "#43454a",
        "secondary.active.background": "#4e5157",
        "secondary.foreground": "#dfe1e5",
        "input.border": "#4e5157",
        "popover.background": "#2b2d30",
        "popover.foreground": "#dfe1e5",
        "list.background": "#2b2d30",
        "list.hover.background": "#35373b",
        "list.active.background": "#2e436e",
        "list.active.border": "#3574f0",
        "list.even.background": "#2b2d30",
        "list.head.background": "#2b2d30",
        "sidebar.background": "#2b2d30",
        "sidebar.foreground": "#dfe1e5",
        "sidebar.border": "#1e1f22",
        "sidebar.accent.background": "#35373b",
        "sidebar.accent.foreground": "#dfe1e5",
        "sidebar.primary.background": "#3574f0",
        "sidebar.primary.foreground": "#ffffff",
        "tab.background": "#00000000",
        "tab.foreground": "#a8adb6",
        "tab.active.background": "#1e1f22",
        "tab.active.foreground": "#dfe1e5",
        "tab_bar.background": "#2b2d30",
        "tab_bar.segmented.background": "#1e1f22",
        "title_bar.background": "#2b2d30",
        "title_bar.border": "#1e1f22",
        "status_bar.background": "#2b2d30",
        "status_bar.border": "#1e1f22",
        "scrollbar.background": "#00000000",
        "scrollbar.thumb.background": "#4e515799",
        "scrollbar.thumb.hover.background": "#5a5d63",
        "drag_border": "#3574f0",
        "drop_target.background": "#3574f033",
        "danger.background": "#f75464",
        "danger.foreground": "#f75464",
        "success.background": "#5fb865",
        "success.foreground": "#5fb865",
        "warning.background": "#f2c55c",
        "warning.foreground": "#f2c55c",
        "info.background": "#548af7",
        "info.foreground": "#548af7",
        "link.foreground": "#548af7",
        "link.hover.foreground": "#6b9bfa",
        "link.active.foreground": "#548af7",
        "switch.background": "#4e5157",
        "base.red": "#f75464",
        "base.red.light": "#fa6675",
        "base.green": "#5fb865",
        "base.green.light": "#73bd79",
        "base.blue": "#548af7",
        "base.blue.light": "#6b9bfa",
        "base.yellow": "#f2c55c",
        "base.yellow.light": "#f5d273",
        "base.magenta": "#c77dbb",
        "base.magenta.light": "#d491c9",
        "base.cyan": "#2aacb8",
        "base.cyan.light": "#3fc0cc"
      },
      "highlight": {
        "editor.background": "#1e1f22",
        "editor.foreground": "#bcbec4",
        "editor.active_line.background": "#26282e",
        "editor.line_number": "#4b5059",
        "editor.active_line_number": "#a1a3ab",
        "editor.invisible": "#6f737a66",
        "error": "#f75464",
        "error.background": "#402929",
        "error.border": "#f75464",
        "warning": "#f2c55c",
        "warning.background": "#3d3223",
        "warning.border": "#c29e4a",
        "info.background": "#25324d",
        "info.border": "#3574f0",
        "hint": "#868a91",
        "created": "#5fb865",
        "modified": "#548af7",
        "deleted": "#f75464",
        "syntax": {
          "keyword": { "color": "#cf8e6d" },
          "boolean": { "color": "#cf8e6d" },
          "string": { "color": "#6aab73" },
          "string.escape": { "color": "#cf8e6d" },
          "string.special": { "color": "#6aab73" },
          "string.regex": { "color": "#42c3d4" },
          "number": { "color": "#2aacb8" },
          "comment": { "color": "#7a7e85", "font_style": "italic" },
          "comment.doc": { "color": "#5f826b", "font_style": "italic" },
          "function": { "color": "#56a8f5" },
          "type": { "color": "#6fafbd" },
          "constructor": { "color": "#6fafbd" },
          "constant": { "color": "#c77dbb" },
          "property": { "color": "#c77dbb" },
          "attribute": { "color": "#b3ae60" },
          "tag": { "color": "#d5b778" },
          "label": { "color": "#32b8af" },
          "variable": { "color": "#bcbec4" },
          "variable.special": { "color": "#94558d" },
          "operator": { "color": "#bcbec4" },
          "punctuation": { "color": "#bcbec4" },
          "punctuation.bracket": { "color": "#bcbec4" },
          "punctuation.delimiter": { "color": "#bcbec4" },
          "title": { "color": "#cf8e6d", "font_weight": 700 },
          "emphasis": { "font_style": "italic" },
          "emphasis.strong": { "font_weight": 700 },
          "link_text": { "color": "#548af7" },
          "link_uri": { "color": "#548af7", "font_style": "italic" },
          "text.literal": { "color": "#6aab73" },
          "text.code.span": { "color": "#6aab73" },
          "embedded": { "color": "#bcbec4" }
        }
      }
    },
    {
      "name": "Gust Light",
      "mode": "light",
      "radius": 6,
      "radius.lg": 8,
      "shadow": true,
      "mono_font.size": 14,
      "colors": {
        "background": "#ffffff",
        "foreground": "#1e1f22",
        "border": "#dfe1e5",
        "window.border": "#c9ccd6",
        "caret": "#000000",
        "ring": "#3574f0",
        "selection.background": "#a6d2ff",
        "overlay": "#00000033",
        "accent.background": "#d5e1ff",
        "accent.foreground": "#1e1f22",
        "muted.background": "#ebecf0",
        "muted.foreground": "#6c707e",
        "primary.background": "#3574f0",
        "primary.hover.background": "#4682fa",
        "primary.active.background": "#2f65d1",
        "primary.foreground": "#ffffff",
        "secondary.background": "#ebecf0",
        "secondary.hover.background": "#dfe1e5",
        "secondary.active.background": "#d3d5db",
        "secondary.foreground": "#1e1f22",
        "input.border": "#c9ccd6",
        "popover.background": "#ffffff",
        "popover.foreground": "#1e1f22",
        "list.background": "#f7f8fa",
        "list.hover.background": "#ebecf0",
        "list.active.background": "#d5e1ff",
        "list.active.border": "#3574f0",
        "list.even.background": "#f7f8fa",
        "list.head.background": "#f7f8fa",
        "sidebar.background": "#f7f8fa",
        "sidebar.foreground": "#1e1f22",
        "sidebar.border": "#ebecf0",
        "sidebar.accent.background": "#ebecf0",
        "sidebar.accent.foreground": "#1e1f22",
        "sidebar.primary.background": "#3574f0",
        "sidebar.primary.foreground": "#ffffff",
        "tab.background": "#00000000",
        "tab.foreground": "#494b57",
        "tab.active.background": "#ffffff",
        "tab.active.foreground": "#1e1f22",
        "tab_bar.background": "#f7f8fa",
        "tab_bar.segmented.background": "#ebecf0",
        "title_bar.background": "#f7f8fa",
        "title_bar.border": "#ebecf0",
        "status_bar.background": "#f7f8fa",
        "status_bar.border": "#ebecf0",
        "scrollbar.background": "#00000000",
        "scrollbar.thumb.background": "#b9bdc999",
        "scrollbar.thumb.hover.background": "#a8adbd",
        "drag_border": "#3574f0",
        "drop_target.background": "#3574f022",
        "danger.background": "#e55765",
        "danger.foreground": "#db3b4b",
        "success.background": "#208a3c",
        "success.foreground": "#208a3c",
        "warning.background": "#ffaf0f",
        "warning.foreground": "#a46704",
        "info.background": "#3574f0",
        "info.foreground": "#3574f0",
        "link.foreground": "#2e55a3",
        "link.hover.foreground": "#3574f0",
        "link.active.foreground": "#2e55a3",
        "switch.background": "#c9ccd6",
        "base.red": "#db3b4b",
        "base.red.light": "#e55765",
        "base.green": "#208a3c",
        "base.green.light": "#3b9b53",
        "base.blue": "#3574f0",
        "base.blue.light": "#588cf3",
        "base.yellow": "#c27d04",
        "base.yellow.light": "#d99a1e",
        "base.magenta": "#871094",
        "base.magenta.light": "#9a35a6",
        "base.cyan": "#0e7c86",
        "base.cyan.light": "#1f8f99"
      },
      "highlight": {
        "editor.background": "#ffffff",
        "editor.foreground": "#080808",
        "editor.active_line.background": "#f5f8fe",
        "editor.line_number": "#aeb3c2",
        "editor.active_line_number": "#767a8a",
        "editor.invisible": "#adadad66",
        "error": "#db3b4b",
        "error.background": "#fff7f7",
        "error.border": "#e55765",
        "warning": "#a46704",
        "warning.background": "#fff6de",
        "warning.border": "#ffaf0f",
        "info.background": "#edf3ff",
        "info.border": "#3574f0",
        "hint": "#6c707e",
        "created": "#208a3c",
        "modified": "#3574f0",
        "deleted": "#db3b4b",
        "syntax": {
          "keyword": { "color": "#0033b3" },
          "boolean": { "color": "#0033b3" },
          "string": { "color": "#067d17" },
          "string.escape": { "color": "#0037a6" },
          "string.special": { "color": "#067d17" },
          "string.regex": { "color": "#264eff" },
          "number": { "color": "#1750eb" },
          "comment": { "color": "#8c8c8c", "font_style": "italic" },
          "comment.doc": { "color": "#8c8c8c", "font_style": "italic" },
          "function": { "color": "#00627a" },
          "type": { "color": "#008080" },
          "constructor": { "color": "#008080" },
          "constant": { "color": "#871094" },
          "property": { "color": "#871094" },
          "attribute": { "color": "#9e880d" },
          "tag": { "color": "#0033b3" },
          "label": { "color": "#2a7d77" },
          "variable": { "color": "#080808" },
          "variable.special": { "color": "#871094" },
          "operator": { "color": "#080808" },
          "punctuation": { "color": "#080808" },
          "punctuation.bracket": { "color": "#080808" },
          "punctuation.delimiter": { "color": "#080808" },
          "title": { "color": "#0033b3", "font_weight": 700 },
          "emphasis": { "font_style": "italic" },
          "emphasis.strong": { "font_weight": 700 },
          "link_text": { "color": "#2e55a3" },
          "link_uri": { "color": "#2e55a3", "font_style": "italic" },
          "text.literal": { "color": "#067d17" },
          "text.code.span": { "color": "#067d17" },
          "embedded": { "color": "#080808" }
        }
      }
    }
  ]
}"##;

/// The coding font: the first of these that is installed. GoLand's font
/// first, for people coming from it; Windows 11 ships Cascadia Mono.
const MONO_FONTS: &[&str] = &[
    "JetBrains Mono",
    "Cascadia Mono",
    "Cascadia Code",
    "Fira Code",
    "Consolas",
];

/// Register Gust's themes and switch to `mode`.
pub fn init(mode: ThemeMode, cx: &mut App) {
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(THEMES)
        .expect("Gust's own themes parse");
    let installed = cx.text_system().all_font_names();
    let mono = MONO_FONTS
        .iter()
        .find(|font| installed.iter().any(|name| name == *font))
        .map(|font| font.to_string());

    let registry = ThemeRegistry::global(cx);
    let mut dark = (*registry.themes()[DARK]).clone();
    let mut light = (*registry.themes()[LIGHT]).clone();
    if let Some(mono) = mono {
        dark.mono_font_family = Some(mono.clone().into());
        light.mono_font_family = Some(mono.into());
    }
    Theme::update(cx, |theme| {
        theme.dark_theme = dark.into();
        theme.light_theme = light.into();
    });
    change(mode, cx);
    // Scrollbars stay in view, as in GoLand, instead of fading after a scroll.
    Theme::set_scrollbar_mode(gpui_kit::component::scroll::ScrollbarMode::Always, cx);
}

/// Switch between the dark and the light theme.
pub fn change(mode: ThemeMode, cx: &mut App) {
    Theme::change(mode, None, cx);
    let size = f32::from_bits(FONT_SIZE.load(std::sync::atomic::Ordering::Relaxed));
    // Loading a theme caps the selection at 30% opacity, which leaves it
    // faint next to GoLand's; selections are drawn under the text, so a
    // stronger one hides nothing.
    Theme::update(cx, |theme| {
        if size > 0. {
            theme.mono_font_size = gpui_kit::px(size);
        }
        theme.selection = if theme.is_dark() {
            Hsla::from(rgb(0x2d5fbf)).opacity(0.5)
        } else {
            Hsla::from(rgb(0x3d8ef0)).opacity(0.4)
        };
    });
    cx.refresh_windows();
}

/// The code font size chosen, kept across theme changes; 0 is the theme's own.
static FONT_SIZE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Set the size of the code font, in editors and terminals.
pub fn set_font_size(size: f32, cx: &mut App) {
    FONT_SIZE.store(size.to_bits(), std::sync::atomic::Ordering::Relaxed);
    Theme::update(cx, |theme| theme.mono_font_size = gpui_kit::px(size));
    cx.refresh_windows();
}

/// Colors that mean the same thing in both themes: file kinds and git states.
pub struct Palette {
    pub blue: Hsla,
    pub green: Hsla,
    pub red: Hsla,
    pub yellow: Hsla,
    pub purple: Hsla,
    pub cyan: Hsla,
    pub orange: Hsla,
    pub gray: Hsla,
}

pub fn palette(cx: &App) -> Palette {
    let theme = gpui_kit::component::ActiveTheme::theme(cx);
    let dark = theme.is_dark();
    let pick = |dark_color: u32, light_color: u32| -> Hsla {
        rgb(if dark { dark_color } else { light_color }).into()
    };
    Palette {
        blue: pick(0x548af7, 0x3574f0),
        green: pick(0x5fb865, 0x208a3c),
        red: pick(0xf75464, 0xdb3b4b),
        yellow: pick(0xf2c55c, 0xc27d04),
        purple: pick(0xc77dbb, 0x871094),
        cyan: pick(0x2aacb8, 0x0e7c86),
        orange: pick(0xe08855, 0xc25e0c),
        gray: pick(0x868a91, 0x6c707e),
    }
}

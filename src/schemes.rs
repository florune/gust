//! The color themes, each described by a few base colors from which every
//! color of the interface and the editor is derived. Gust Dark and Gust
//! Light are written out in full in `theme.rs`; the rest are generated here.

use serde_json::{Value, json};

/// Colors of code.
pub struct Syntax {
    pub keyword: u32,
    pub string: u32,
    pub number: u32,
    pub comment: u32,
    pub function: u32,
    pub ty: u32,
    pub constant: u32,
    pub property: u32,
}

pub struct Scheme {
    pub name: &'static str,
    pub dark: bool,
    /// The editor and tab background.
    pub background: u32,
    /// Panels around the editor: sidebar, title bar, status bar, popups.
    pub panel: u32,
    pub border: u32,
    pub foreground: u32,
    /// Secondary text: hints, line details.
    pub muted: u32,
    /// Buttons, focus rings, the active item.
    pub accent: u32,
    /// Background of the caret's line.
    pub line: u32,
    pub selection: u32,
    pub red: u32,
    pub green: u32,
    pub yellow: u32,
    pub blue: u32,
    pub magenta: u32,
    pub cyan: u32,
    pub orange: u32,
    pub syntax: Syntax,
    /// Written out in full elsewhere rather than generated.
    pub handmade: bool,
}

pub const SCHEMES: &[Scheme] = &[
    Scheme {
        name: "Gust Dark",
        dark: true,
        background: 0x1e1f22,
        panel: 0x2b2d30,
        border: 0x393b40,
        foreground: 0xdfe1e5,
        muted: 0x868a91,
        accent: 0x3574f0,
        line: 0x26282e,
        selection: 0x2d5fbf,
        red: 0xf75464,
        green: 0x5fb865,
        yellow: 0xf2c55c,
        blue: 0x548af7,
        magenta: 0xc77dbb,
        cyan: 0x2aacb8,
        orange: 0xe08855,
        syntax: Syntax {
            keyword: 0xcf8e6d,
            string: 0x6aab73,
            number: 0x2aacb8,
            comment: 0x7a7e85,
            function: 0x56a8f5,
            ty: 0x6fafbd,
            constant: 0xc77dbb,
            property: 0xc77dbb,
        },
        handmade: true,
    },
    Scheme {
        name: "One Dark",
        dark: true,
        background: 0x282c34,
        panel: 0x21252b,
        border: 0x181a1f,
        foreground: 0xabb2bf,
        muted: 0x7f848e,
        accent: 0x4d78cc,
        line: 0x2c313c,
        selection: 0x3e4451,
        red: 0xe06c75,
        green: 0x98c379,
        yellow: 0xe5c07b,
        blue: 0x61afef,
        magenta: 0xc678dd,
        cyan: 0x56b6c2,
        orange: 0xd19a66,
        syntax: Syntax {
            keyword: 0xc678dd,
            string: 0x98c379,
            number: 0xd19a66,
            comment: 0x7f848e,
            function: 0x61afef,
            ty: 0xe5c07b,
            constant: 0xd19a66,
            property: 0xe06c75,
        },
        handmade: false,
    },
    Scheme {
        name: "Dracula",
        dark: true,
        background: 0x282a36,
        panel: 0x21222c,
        border: 0x191a21,
        foreground: 0xf8f8f2,
        muted: 0x6272a4,
        accent: 0xbd93f9,
        line: 0x323443,
        selection: 0x44475a,
        red: 0xff5555,
        green: 0x50fa7b,
        yellow: 0xf1fa8c,
        blue: 0x8be9fd,
        magenta: 0xff79c6,
        cyan: 0x8be9fd,
        orange: 0xffb86c,
        syntax: Syntax {
            keyword: 0xff79c6,
            string: 0xf1fa8c,
            number: 0xbd93f9,
            comment: 0x6272a4,
            function: 0x50fa7b,
            ty: 0x8be9fd,
            constant: 0xbd93f9,
            property: 0xf8f8f2,
        },
        handmade: false,
    },
    Scheme {
        name: "Nord",
        dark: true,
        background: 0x2e3440,
        panel: 0x3b4252,
        border: 0x434c5e,
        foreground: 0xd8dee9,
        muted: 0x7b88a1,
        accent: 0x5e81ac,
        line: 0x353b49,
        selection: 0x434c5e,
        red: 0xbf616a,
        green: 0xa3be8c,
        yellow: 0xebcb8b,
        blue: 0x81a1c1,
        magenta: 0xb48ead,
        cyan: 0x88c0d0,
        orange: 0xd08770,
        syntax: Syntax {
            keyword: 0x81a1c1,
            string: 0xa3be8c,
            number: 0xb48ead,
            comment: 0x616e88,
            function: 0x88c0d0,
            ty: 0x8fbcbb,
            constant: 0xb48ead,
            property: 0xd8dee9,
        },
        handmade: false,
    },
    Scheme {
        name: "Tokyo Night",
        dark: true,
        background: 0x1a1b26,
        panel: 0x16161e,
        border: 0x101014,
        foreground: 0xc0caf5,
        muted: 0x565f89,
        accent: 0x3d59a1,
        line: 0x1e202e,
        selection: 0x283457,
        red: 0xf7768e,
        green: 0x9ece6a,
        yellow: 0xe0af68,
        blue: 0x7aa2f7,
        magenta: 0xbb9af7,
        cyan: 0x7dcfff,
        orange: 0xff9e64,
        syntax: Syntax {
            keyword: 0xbb9af7,
            string: 0x9ece6a,
            number: 0xff9e64,
            comment: 0x565f89,
            function: 0x7aa2f7,
            ty: 0x2ac3de,
            constant: 0xff9e64,
            property: 0x73daca,
        },
        handmade: false,
    },
    Scheme {
        name: "Catppuccin Mocha",
        dark: true,
        background: 0x1e1e2e,
        panel: 0x181825,
        border: 0x11111b,
        foreground: 0xcdd6f4,
        muted: 0x7f849c,
        accent: 0x89b4fa,
        line: 0x252536,
        selection: 0x45475a,
        red: 0xf38ba8,
        green: 0xa6e3a1,
        yellow: 0xf9e2af,
        blue: 0x89b4fa,
        magenta: 0xcba6f7,
        cyan: 0x94e2d5,
        orange: 0xfab387,
        syntax: Syntax {
            keyword: 0xcba6f7,
            string: 0xa6e3a1,
            number: 0xfab387,
            comment: 0x7f849c,
            function: 0x89b4fa,
            ty: 0xf9e2af,
            constant: 0xfab387,
            property: 0xb4befe,
        },
        handmade: false,
    },
    Scheme {
        name: "Gruvbox Dark",
        dark: true,
        background: 0x282828,
        panel: 0x32302f,
        border: 0x3c3836,
        foreground: 0xebdbb2,
        muted: 0x928374,
        accent: 0x458588,
        line: 0x32302f,
        selection: 0x504945,
        red: 0xfb4934,
        green: 0xb8bb26,
        yellow: 0xfabd2f,
        blue: 0x83a598,
        magenta: 0xd3869b,
        cyan: 0x8ec07c,
        orange: 0xfe8019,
        syntax: Syntax {
            keyword: 0xfb4934,
            string: 0xb8bb26,
            number: 0xd3869b,
            comment: 0x928374,
            function: 0x8ec07c,
            ty: 0xfabd2f,
            constant: 0xd3869b,
            property: 0x83a598,
        },
        handmade: false,
    },
    Scheme {
        name: "Solarized Dark",
        dark: true,
        background: 0x002b36,
        panel: 0x00252e,
        border: 0x073642,
        foreground: 0x93a1a1,
        muted: 0x657b83,
        accent: 0x268bd2,
        line: 0x073642,
        selection: 0x0f4a5a,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        magenta: 0xd33682,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
        syntax: Syntax {
            keyword: 0x859900,
            string: 0x2aa198,
            number: 0xd33682,
            comment: 0x586e75,
            function: 0x268bd2,
            ty: 0xb58900,
            constant: 0xcb4b16,
            property: 0x6c71c4,
        },
        handmade: false,
    },
    Scheme {
        name: "Gust Light",
        dark: false,
        background: 0xffffff,
        panel: 0xf7f8fa,
        border: 0xdfe1e5,
        foreground: 0x1e1f22,
        muted: 0x6c707e,
        accent: 0x3574f0,
        line: 0xf5f8fe,
        selection: 0x3d8ef0,
        red: 0xdb3b4b,
        green: 0x208a3c,
        yellow: 0xc27d04,
        blue: 0x3574f0,
        magenta: 0x871094,
        cyan: 0x0e7c86,
        orange: 0xc25e0c,
        syntax: Syntax {
            keyword: 0x0033b3,
            string: 0x067d17,
            number: 0x1750eb,
            comment: 0x8c8c8c,
            function: 0x00627a,
            ty: 0x008080,
            constant: 0x871094,
            property: 0x871094,
        },
        handmade: true,
    },
    Scheme {
        name: "GitHub Light",
        dark: false,
        background: 0xffffff,
        panel: 0xf6f8fa,
        border: 0xd0d7de,
        foreground: 0x1f2328,
        muted: 0x656d76,
        accent: 0x0969da,
        line: 0xf6f8fa,
        selection: 0x54aeff,
        red: 0xcf222e,
        green: 0x1a7f37,
        yellow: 0x9a6700,
        blue: 0x0969da,
        magenta: 0x8250df,
        cyan: 0x1b7c83,
        orange: 0xbc4c00,
        syntax: Syntax {
            keyword: 0xcf222e,
            string: 0x0a3069,
            number: 0x0550ae,
            comment: 0x6e7781,
            function: 0x8250df,
            ty: 0x953800,
            constant: 0x0550ae,
            property: 0x0550ae,
        },
        handmade: false,
    },
    Scheme {
        name: "One Light",
        dark: false,
        background: 0xfafafa,
        panel: 0xf0f0f1,
        border: 0xdbdbdc,
        foreground: 0x383a42,
        muted: 0x8e8f96,
        accent: 0x4078f2,
        line: 0xf0f0f1,
        selection: 0x82a7f5,
        red: 0xe45649,
        green: 0x50a14f,
        yellow: 0xc18401,
        blue: 0x4078f2,
        magenta: 0xa626a4,
        cyan: 0x0184bc,
        orange: 0x986801,
        syntax: Syntax {
            keyword: 0xa626a4,
            string: 0x50a14f,
            number: 0x986801,
            comment: 0xa0a1a7,
            function: 0x4078f2,
            ty: 0xc18401,
            constant: 0x986801,
            property: 0xe45649,
        },
        handmade: false,
    },
    Scheme {
        name: "Solarized Light",
        dark: false,
        background: 0xfdf6e3,
        panel: 0xeee8d5,
        border: 0xddd6c1,
        foreground: 0x586e75,
        muted: 0x839496,
        accent: 0x268bd2,
        line: 0xf5efdc,
        selection: 0x9fc7e8,
        red: 0xdc322f,
        green: 0x859900,
        yellow: 0xb58900,
        blue: 0x268bd2,
        magenta: 0xd33682,
        cyan: 0x2aa198,
        orange: 0xcb4b16,
        syntax: Syntax {
            keyword: 0x859900,
            string: 0x2aa198,
            number: 0xd33682,
            comment: 0x93a1a1,
            function: 0x268bd2,
            ty: 0xb58900,
            constant: 0xcb4b16,
            property: 0x6c71c4,
        },
        handmade: false,
    },
    Scheme {
        name: "Catppuccin Latte",
        dark: false,
        background: 0xeff1f5,
        panel: 0xe6e9ef,
        border: 0xccd0da,
        foreground: 0x4c4f69,
        muted: 0x7c7f93,
        accent: 0x1e66f5,
        line: 0xe6e9ef,
        selection: 0x9fb8f5,
        red: 0xd20f39,
        green: 0x40a02b,
        yellow: 0xdf8e1d,
        blue: 0x1e66f5,
        magenta: 0x8839ef,
        cyan: 0x179299,
        orange: 0xfe640b,
        syntax: Syntax {
            keyword: 0x8839ef,
            string: 0x40a02b,
            number: 0xfe640b,
            comment: 0x8c8fa1,
            function: 0x1e66f5,
            ty: 0xdf8e1d,
            constant: 0xfe640b,
            property: 0x7287fd,
        },
        handmade: false,
    },
    Scheme {
        name: "Gruvbox Light",
        dark: false,
        background: 0xfbf1c7,
        panel: 0xf2e5bc,
        border: 0xd5c4a1,
        foreground: 0x3c3836,
        muted: 0x7c6f64,
        accent: 0x076678,
        line: 0xf2e5bc,
        selection: 0xd5c4a1,
        red: 0x9d0006,
        green: 0x79740e,
        yellow: 0xb57614,
        blue: 0x076678,
        magenta: 0x8f3f71,
        cyan: 0x427b58,
        orange: 0xaf3a03,
        syntax: Syntax {
            keyword: 0x9d0006,
            string: 0x79740e,
            number: 0x8f3f71,
            comment: 0x928374,
            function: 0x427b58,
            ty: 0xb57614,
            constant: 0x8f3f71,
            property: 0x076678,
        },
        handmade: false,
    },
];

pub fn find(name: &str) -> Option<&'static Scheme> {
    SCHEMES.iter().find(|scheme| scheme.name == name)
}

fn channels(color: u32) -> [f32; 3] {
    [(color >> 16) as f32, ((color >> 8) & 0xff) as f32, (color & 0xff) as f32]
}

/// `a` moved toward `b` by `t`, from 0 (all `a`) to 1 (all `b`).
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let (a, b) = (channels(a), channels(b));
    let channel = |ix: usize| (a[ix] + (b[ix] - a[ix]) * t).round().clamp(0., 255.) as u32;
    channel(0) << 16 | channel(1) << 8 | channel(2)
}

/// Relative brightness, 0 to 1.
fn luminance(color: u32) -> f32 {
    let [r, g, b] = channels(color);
    (0.299 * r + 0.587 * g + 0.114 * b) / 255.
}

fn hex(color: u32) -> String {
    format!("#{color:06x}")
}

/// `color` with `alpha`, 0 to 255.
fn hexa(color: u32, alpha: u32) -> String {
    format!("#{color:06x}{alpha:02x}")
}

impl Scheme {
    /// The theme as the theme registry reads it.
    fn to_json(&self) -> Value {
        let (bg, panel, fg) = (self.background, self.panel, self.foreground);
        // Interface surfaces lean toward the text color by `t`.
        let shade = |t: f32| hex(mix(panel, fg, t));
        let on_accent = if luminance(self.accent) > 0.6 { 0x1e1f22 } else { 0xffffff };
        let active = mix(bg, self.accent, if self.dark { 0.28 } else { 0.18 });
        let light = |color: u32| hex(mix(color, fg, 0.15));
        let s = &self.syntax;
        let colors: serde_json::Map<String, Value> = [
            ("background", hex(bg)),
            ("foreground", hex(fg)),
            ("border", hex(self.border)),
            ("window.border", hex(self.border)),
            ("caret", hex(fg)),
            ("ring", hex(self.accent)),
            ("selection.background", hex(self.selection)),
            ("overlay", (if self.dark { "#00000066" } else { "#00000033" }).to_string()),
            ("accent.background", hex(active)),
            ("accent.foreground", hex(fg)),
            ("muted.background", shade(0.08)),
            ("muted.foreground", hex(self.muted)),
            ("primary.background", hex(self.accent)),
            ("primary.hover.background", hex(mix(self.accent, fg, 0.12))),
            ("primary.active.background", hex(mix(self.accent, bg, 0.15))),
            ("primary.foreground", hex(on_accent)),
            ("secondary.background", shade(0.08)),
            ("secondary.hover.background", shade(0.13)),
            ("secondary.active.background", shade(0.18)),
            ("secondary.foreground", hex(fg)),
            ("input.border", hex(mix(bg, fg, 0.22))),
            ("popover.background", hex(panel)),
            ("popover.foreground", hex(fg)),
            ("list.background", hex(panel)),
            ("list.hover.background", shade(0.06)),
            ("list.active.background", hex(active)),
            ("list.active.border", hex(self.accent)),
            ("list.even.background", hex(panel)),
            ("list.head.background", hex(panel)),
            ("sidebar.background", hex(panel)),
            ("sidebar.foreground", hex(fg)),
            ("sidebar.border", hex(self.border)),
            ("sidebar.accent.background", shade(0.06)),
            ("sidebar.accent.foreground", hex(fg)),
            ("sidebar.primary.background", hex(self.accent)),
            ("sidebar.primary.foreground", hex(on_accent)),
            ("tab.background", "#00000000".to_string()),
            ("tab.foreground", hex(mix(fg, bg, 0.3))),
            ("tab.active.background", hex(bg)),
            ("tab.active.foreground", hex(fg)),
            ("tab_bar.background", hex(panel)),
            ("tab_bar.segmented.background", hex(bg)),
            ("title_bar.background", hex(panel)),
            ("title_bar.border", hex(self.border)),
            ("status_bar.background", hex(panel)),
            ("status_bar.border", hex(self.border)),
            ("scrollbar.background", "#00000000".to_string()),
            ("scrollbar.thumb.background", hexa(mix(bg, fg, 0.3), 0x99)),
            ("scrollbar.thumb.hover.background", hex(mix(bg, fg, 0.4))),
            ("drag_border", hex(self.accent)),
            ("drop_target.background", hexa(self.accent, 0x33)),
            ("danger.background", hex(self.red)),
            ("danger.foreground", hex(self.red)),
            ("success.background", hex(self.green)),
            ("success.foreground", hex(self.green)),
            ("warning.background", hex(self.yellow)),
            ("warning.foreground", hex(self.yellow)),
            ("info.background", hex(self.blue)),
            ("info.foreground", hex(self.blue)),
            ("link.foreground", hex(self.blue)),
            ("link.hover.foreground", light(self.blue)),
            ("link.active.foreground", hex(self.blue)),
            ("switch.background", hex(mix(bg, fg, 0.25))),
            ("base.red", hex(self.red)),
            ("base.red.light", light(self.red)),
            ("base.green", hex(self.green)),
            ("base.green.light", light(self.green)),
            ("base.blue", hex(self.blue)),
            ("base.blue.light", light(self.blue)),
            ("base.yellow", hex(self.yellow)),
            ("base.yellow.light", light(self.yellow)),
            ("base.magenta", hex(self.magenta)),
            ("base.magenta.light", light(self.magenta)),
            ("base.cyan", hex(self.cyan)),
            ("base.cyan.light", light(self.cyan)),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), Value::String(value)))
        .collect();
        let syntax = json!({
                    "keyword": { "color": hex(s.keyword) },

                    "boolean": { "color": hex(s.keyword) },
                    "string": { "color": hex(s.string) },
                    "string.escape": { "color": hex(s.keyword) },
                    "string.special": { "color": hex(s.string) },
                    "string.regex": { "color": hex(self.cyan) },
                    "number": { "color": hex(s.number) },
                    "comment": { "color": hex(s.comment), "font_style": "italic" },
                    "comment.doc": { "color": hex(s.comment), "font_style": "italic" },
                    "function": { "color": hex(s.function) },
                    "type": { "color": hex(s.ty) },
                    "constructor": { "color": hex(s.ty) },
                    "constant": { "color": hex(s.constant) },
                    "property": { "color": hex(s.property) },
                    "attribute": { "color": hex(self.yellow) },
                    "tag": { "color": hex(s.keyword) },
                    "label": { "color": hex(self.cyan) },
                    "variable": { "color": hex(fg) },
                    "variable.special": { "color": hex(s.constant) },
                    "operator": { "color": hex(fg) },
                    "punctuation": { "color": hex(fg) },
                    "punctuation.bracket": { "color": hex(fg) },
                    "punctuation.delimiter": { "color": hex(fg) },
                    "title": { "color": hex(s.keyword), "font_weight": 700 },
                    "emphasis": { "font_style": "italic" },
                    "emphasis.strong": { "font_weight": 700 },
                    "link_text": { "color": hex(self.blue) },
                    "link_uri": { "color": hex(self.blue), "font_style": "italic" },
                    "text.literal": { "color": hex(s.string) },
                    "text.code.span": { "color": hex(s.string) },
                    "embedded": { "color": hex(fg) }
        });
        let mut highlight = json!({
                "editor.background": hex(bg),
                "editor.foreground": hex(fg),
                "editor.active_line.background": hex(self.line),
                "editor.line_number": hex(mix(bg, fg, 0.3)),
                "editor.active_line_number": hex(mix(bg, fg, 0.7)),
                "editor.invisible": hexa(mix(bg, fg, 0.35), 0x66),
                "error": hex(self.red),
                "error.background": hex(mix(bg, self.red, 0.12)),
                "error.border": hex(self.red),
                "warning": hex(self.yellow),
                "warning.background": hex(mix(bg, self.yellow, 0.12)),
                "warning.border": hex(self.yellow),
                "info.background": hex(mix(bg, self.blue, 0.12)),
                "info.border": hex(self.blue),
                "hint": hex(self.muted),
                "created": hex(self.green),
                "modified": hex(self.blue),
                "deleted": hex(self.red)
        });
        highlight["syntax"] = syntax;
        json!({
            "name": self.name,
            "mode": if self.dark { "dark" } else { "light" },
            "radius": 6,
            "radius.lg": 8,
            "shadow": true,
            "mono_font.size": 14,
            "colors": colors,
            "highlight": highlight,
        })
    }
}

/// The generated themes, as one theme set for the registry.
pub fn generated() -> String {
    let themes: Vec<Value> = SCHEMES
        .iter()
        .filter(|scheme| !scheme.handmade)
        .map(Scheme::to_json)
        .collect();
    json!({ "name": "Gust Schemes", "author": "Gust", "themes": themes }).to_string()
}

#[cfg(test)]
mod tests {
    use super::{SCHEMES, generated, mix};

    #[test]
    fn colors_mix_by_channel() {
        assert_eq!(mix(0x000000, 0xffffff, 0.5), 0x808080);
        assert_eq!(mix(0x102030, 0x102030, 0.7), 0x102030);
        assert_eq!(mix(0xff0000, 0x0000ff, 1.), 0x0000ff);
    }

    #[test]
    fn every_theme_has_its_own_name_and_both_kinds_exist() {
        let mut names: Vec<&str> = SCHEMES.iter().map(|scheme| scheme.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), SCHEMES.len());
        assert!(SCHEMES.iter().filter(|scheme| scheme.dark).count() >= 5);
        assert!(SCHEMES.iter().filter(|scheme| !scheme.dark).count() >= 5);
        assert!(generated().contains("\"Nord\""));
    }
}

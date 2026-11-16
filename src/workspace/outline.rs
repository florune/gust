//! The declarations of the open Go file: a list to jump through (Ctrl+F12),
//! and the one the caret is in, shown in the status bar.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::*;

use super::Workspace;
use super::palette::Mode;
use crate::finder::Hit;
use crate::goindex::{self, Kind, OutlineItem};

actions!(gust, [ShowOutline]);

pub(super) fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("ctrl-f12", ShowOutline, workspace),
        KeyBinding::new("ctrl-alt-o", ShowOutline, workspace),
    ]);
}

pub(super) fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Func => "函数",
        Kind::Method => "方法",
        Kind::Type => "类型",
        Kind::Field => "字段",
        Kind::Var => "变量",
        Kind::Const => "常量",
        _ => "",
    }
}

impl Workspace {
    /// Read the outline of the file at `ix` again, after typing pauses
    /// unless `now`.
    pub(super) fn refresh_outline(&mut self, ix: usize, now: bool, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        if doc.language != "go" {
            return;
        }
        let text = doc.editor.read(cx).text().to_string();
        if now {
            doc.outline = Rc::new(goindex::outline(&text));
            return;
        }
        let path = doc.path.clone();
        doc.outline_task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let items = cx
                .background_executor()
                .spawn(async move { goindex::outline(&text) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(doc) = this.docs.iter_mut().find(|doc| doc.path == path) {
                    doc.outline = Rc::new(items);
                    cx.notify();
                }
            })
            .ok();
        });
    }

    /// The declaration the caret of the active file is in, as `Type.method`.
    pub(super) fn current_symbol(&self, cx: &App) -> Option<String> {
        let doc = self.active.and_then(|ix| self.docs.get(ix))?;
        let row = doc.editor.read(cx).cursor_position().line;
        goindex::enclosing(&doc.outline, row).map(OutlineItem::full_name)
    }

    pub(super) fn on_show_outline(&mut self, _: &ShowOutline, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active.and_then(|ix| self.docs.get(ix)) else {
            return;
        };
        if doc.language != "go" || self.diff_active {
            return;
        }
        let path = super::relative_path(&self.root, &doc.path);
        let row = doc.editor.read(cx).cursor_position().line;
        let current = goindex::enclosing(&doc.outline, row).cloned();
        let hits: Vec<Hit> = doc
            .outline
            .iter()
            .map(|item| Hit {
                path: path.clone(),
                row: item.row,
                col: item.col,
                // Members sit under their type.
                preview: if item.owner.is_some() && item.kind != Kind::Method {
                    format!("    {}", item.name)
                } else {
                    item.full_name()
                },
                tag: kind_label(item.kind),
                detail: item.detail.clone(),
            })
            .collect();
        let selected = current
            .and_then(|current| {
                doc.outline
                    .iter()
                    .position(|item| item.row == current.row && item.col == current.col)
            })
            .unwrap_or(0);
        let title = format!("{} · {} 个声明", doc.title, hits.len());
        self.open_palette(Mode::Outline, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.title = Some(title.into());
            palette.found = hits.clone();
            palette.hits = hits;
            palette.selected = selected;
            palette.scroll.scroll_to_item(selected);
        }
        cx.notify();
    }
}

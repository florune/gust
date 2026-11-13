//! Connects the Go index to the editor's completion and go-to-definition hooks.

use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::component::Rope;
use gpui_kit::component::input::{CompletionProvider, DefinitionProvider};
use gpui_kit::{App, Task, Window};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    LocationLink, Position, Range, TextEdit, Uri,
};

use crate::goindex::{self, Index, Kind};

/// Completion and definitions for one open Go file.
pub struct GoFile {
    pub index: Arc<Index>,
    pub path: PathBuf,
}

/// The LSP position of a byte offset: line, and UTF-16 units into the line.
fn position(source: &str, offset: usize) -> Position {
    let before = &source[..offset];
    let line_start = before.rfind('\n').map_or(0, |ix| ix + 1);
    Position::new(
        before.matches('\n').count() as u32,
        before[line_start..].encode_utf16().count() as u32,
    )
}

fn item_kind(kind: Kind) -> CompletionItemKind {
    match kind {
        Kind::Local | Kind::Var => CompletionItemKind::VARIABLE,
        Kind::Func | Kind::Builtin => CompletionItemKind::FUNCTION,
        Kind::Method => CompletionItemKind::METHOD,
        Kind::Type => CompletionItemKind::STRUCT,
        Kind::Const => CompletionItemKind::CONSTANT,
        Kind::Field => CompletionItemKind::FIELD,
        Kind::Package => CompletionItemKind::MODULE,
        Kind::Keyword => CompletionItemKind::KEYWORD,
    }
}

impl CompletionProvider for GoFile {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _: CompletionContext,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let source = text.to_string();
        let index = self.index.clone();
        let path = self.path.clone();
        cx.background_executor().spawn(async move {
            let completions = goindex::complete(&index, &path, &source, offset);
            // Each item says exactly what it replaces: the word typed so far.
            let range = Range::new(
                position(&source, completions.start),
                position(&source, offset.min(source.len())),
            );
            let items = completions
                .items
                .into_iter()
                .map(|item| CompletionItem {
                    kind: Some(item_kind(item.kind)),
                    detail: (!item.detail.is_empty()).then_some(item.detail),
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: item.name.clone(),
                    })),
                    label: item.name,
                    ..Default::default()
                })
                .collect();
            Ok(CompletionResponse::Array(items))
        })
    }

    fn is_completion_trigger(&self, _: usize, new_text: &str, _: &mut App) -> bool {
        let mut chars = new_text.chars();
        matches!(
            (chars.next(), chars.next()),
            (Some(ch), None) if ch.is_alphanumeric() || ch == '_' || ch == '.'
        )
    }
}

impl DefinitionProvider for GoFile {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        let source = text.to_string();
        let index = self.index.clone();
        let path = self.path.clone();
        cx.background_executor().spawn(async move {
            // On a declaration there is nowhere to go but to its usages,
            // which the workspace lists when it is shown this kind of link.
            let (location, uri) = match goindex::definition(&index, &path, &source, offset) {
                Some(location) => {
                    let uri = path_to_uri(&location.file);
                    (location, uri)
                }
                None => {
                    let Some(location) = goindex::declaration_at(&index, &path, &source, offset)
                    else {
                        return Ok(Vec::new());
                    };
                    let uri = usages_uri(&location.file);
                    (location, uri)
                }
            };
            let Some(uri) = uri else {
                return Ok(Vec::new());
            };
            // The column is in bytes; `uri_to_target` hands it back unchanged
            // and the workspace converts it against the target file's text.
            let at = Position::new(location.row, location.col);
            let range = Range::new(at, at);
            Ok(vec![LocationLink {
                origin_selection_range: None,
                target_uri: uri,
                target_range: range,
                target_selection_range: range,
            }])
        })
    }
}

/// A `file:` URI for `path`, with everything but plain path characters escaped.
fn path_to_uri(path: &Path) -> Option<Uri> {
    let mut uri = String::from("file:///");
    for byte in path.to_str()?.replace('\\', "/").trim_start_matches('/').bytes() {
        if byte.is_ascii_alphanumeric() || b"/._-~:".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    Uri::from_str(&uri).ok()
}

const USAGES_SCHEME: &str = "gust-usages";

/// A link that lists the usages of the declaration it points at.
fn usages_uri(path: &Path) -> Option<Uri> {
    let file = path_to_uri(path)?;
    let rest = file.as_str().strip_prefix("file:")?;
    Uri::from_str(&format!("{USAGES_SCHEME}:{rest}")).ok()
}

/// The file a link made by [`usages_uri`] lists usages in.
pub fn usages_target(uri: &Uri) -> Option<PathBuf> {
    let rest = uri.as_str().strip_prefix(USAGES_SCHEME)?.strip_prefix(':')?;
    uri_to_path(&Uri::from_str(&format!("file:{rest}")).ok()?)
}

/// The path a URI made by [`path_to_uri`] stands for.
pub fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    let encoded = uri.as_str().strip_prefix("file:///")?;
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut rest = encoded.as_bytes();
    while let [first, tail @ ..] = rest {
        if *first == b'%' && tail.len() >= 2 {
            let hex = std::str::from_utf8(&tail[..2]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            rest = &tail[2..];
        } else {
            bytes.push(*first);
            rest = tail;
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // `C:/…` is already absolute; a Unix path lost its leading slash above.
    if path.as_bytes().get(1) == Some(&b':') {
        Some(PathBuf::from(path))
    } else {
        Some(PathBuf::from(format!("/{path}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_survive_the_trip_through_a_uri() {
        for path in [r"C:\work\my project\主程序.go", "/home/me/a b/main.go"] {
            let uri = path_to_uri(Path::new(path)).unwrap();
            assert!(uri.as_str().is_ascii());
            assert_eq!(
                uri_to_path(&uri).unwrap(),
                PathBuf::from(path.replace('\\', "/"))
            );
        }
    }

    #[test]
    fn usages_links_are_told_apart_from_file_links() {
        let path = Path::new(r"C:\work\my project\main.go");
        let usages = usages_uri(path).unwrap();
        assert_eq!(usages_target(&usages), Some(PathBuf::from("C:/work/my project/main.go")));
        assert_eq!(usages_target(&path_to_uri(path).unwrap()), None);
    }

    #[test]
    fn positions_count_utf16_units_within_the_line() {
        let source = "a\n“b” := c";
        assert_eq!(position(source, 0), Position::new(0, 0));
        assert_eq!(position(source, source.find(":=").unwrap()), Position::new(1, 4));
    }
}

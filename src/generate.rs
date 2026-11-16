//! Code generation for Go: struct tags, live templates.

use tree_sitter::{Node, Parser, Tree};

fn parse(source: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    parser.parse(source, None)
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or_default()
}

/// A Go field name as a JSON key: `ConfigKey` → `configKey`, `ID` → `id`,
/// `HTTPServer` → `httpServer`.
pub fn json_name(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let upper = chars.iter().take_while(|ch| ch.is_uppercase()).count();
    let lower = match upper {
        0 => return name.to_string(),
        _ if upper == chars.len() => upper,
        1 => 1,
        // The last capital starts the next word.
        _ => upper - 1,
    };
    chars[..lower]
        .iter()
        .flat_map(|ch| ch.to_lowercase())
        .chain(chars[lower..].iter().copied())
        .collect()
}

/// The struct the caret at `offset` is in or names.
fn struct_at<'a>(tree: &'a Tree, offset: usize) -> Option<Node<'a>> {
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset);
    while let Some(current) = node {
        if current.kind() == "struct_type" {
            return Some(current);
        }
        if current.kind() == "type_spec" {
            return current
                .child_by_field_name("type")
                .filter(|ty| ty.kind() == "struct_type");
        }
        node = current.parent();
    }
    None
}

/// Whether the caret at `offset` is in a struct with a field that has no
/// `json` tag.
pub fn can_add_json_tags(source: &str, offset: usize) -> bool {
    json_tags(source, offset).is_some_and(|tagged| tagged != source)
}

/// `source` with a `json` tag on every named field of the struct at
/// `offset` that has none.
pub fn json_tags(source: &str, offset: usize) -> Option<String> {
    let tree = parse(source)?;
    let structure = struct_at(&tree, offset)?;
    let mut edits: Vec<(usize, String)> = Vec::new();
    let mut stack = vec![structure];
    while let Some(node) = stack.pop() {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "field_declaration_list" => stack.push(child),
                "field_declaration" => {
                    let mut names = child.children_by_field_name("name", &mut child.walk()).collect::<Vec<_>>();
                    // An embedded field, or several names sharing one tag.
                    if names.len() != 1 {
                        continue;
                    }
                    let name = names.remove(0);
                    let key = json_name(text(name, source));
                    match child.child_by_field_name("tag") {
                        None => {
                            let at = child.child_by_field_name("type").map_or(child.end_byte(), |ty| ty.end_byte());
                            edits.push((at, format!(" `json:\"{key}\"`")));
                        }
                        Some(tag) => {
                            let written = text(tag, source);
                            if written.contains("json:\"") {
                                continue;
                            }
                            // Inside the quotes, first.
                            edits.push((tag.start_byte() + 1, format!("json:\"{key}\" ")));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    if edits.is_empty() {
        return Some(source.to_string());
    }
    edits.sort_by_key(|(at, _)| *at);
    let mut out = String::with_capacity(source.len() + edits.len() * 16);
    let mut at = 0;
    for (offset, insert) in edits {
        out.push_str(&source[at..offset]);
        out.push_str(&insert);
        at = offset;
    }
    out.push_str(&source[at..]);
    Some(out)
}

/// The zero value of a type as written, for a `return` before `err`.
fn zero_value(ty: &str) -> String {
    let ty = ty.trim();
    match ty {
        "string" => "\"\"".into(),
        "bool" => "false".into(),
        "error" | "any" => "nil".into(),
        "int" | "int8" | "int16" | "int32" | "int64" | "uint" | "uint8" | "uint16" | "uint32"
        | "uint64" | "uintptr" | "float32" | "float64" | "byte" | "rune" | "complex64"
        | "complex128" => "0".into(),
        _ if ["*", "[]", "map[", "chan ", "func(", "interface"]
            .iter()
            .any(|prefix| ty.starts_with(prefix)) =>
        {
            "nil".into()
        }
        _ => format!("{ty}{{}}"),
    }
}

/// The result types of the function around `offset`.
fn results_at(source: &str, offset: usize) -> Vec<String> {
    let Some(tree) = parse(source) else {
        return Vec::new();
    };
    let mut node = tree.root_node().descendant_for_byte_range(offset, offset);
    while let Some(current) = node {
        if matches!(current.kind(), "function_declaration" | "method_declaration" | "func_literal") {
            let Some(result) = current.child_by_field_name("result") else {
                return Vec::new();
            };
            if result.kind() != "parameter_list" {
                return vec![text(result, source).to_string()];
            }
            let mut types = Vec::new();
            let mut cursor = result.walk();
            for parameter in result.named_children(&mut cursor) {
                let Some(ty) = parameter.child_by_field_name("type") else {
                    continue;
                };
                let names = parameter.children_by_field_name("name", &mut parameter.walk()).count().max(1);
                for _ in 0..names {
                    types.push(text(ty, source).to_string());
                }
            }
            return types;
        }
        node = current.parent();
    }
    Vec::new()
}

/// What the template `word` typed before `offset` expands to, with `$`
/// where the caret goes; lines after the first are indented by `indent`.
pub fn expand_template(word: &str, source: &str, offset: usize, indent: &str) -> Option<String> {
    let body = match word {
        "iferr" => {
            let results = results_at(source, offset);
            let values: Vec<String> = match results.split_last() {
                Some((last, rest)) if last.trim() == "error" => {
                    rest.iter().map(|ty| zero_value(ty)).chain(["err".to_string()]).collect()
                }
                Some(_) => results.iter().map(|ty| zero_value(ty)).collect(),
                None => Vec::new(),
            };
            let ret = if values.is_empty() {
                "return".to_string()
            } else {
                format!("return {}", values.join(", "))
            };
            format!("if err != nil {{\n\t{ret}$\n}}")
        }
        "fori" => "for i := 0; i < $; i++ {\n\t\n}".to_string(),
        "forr" => "for _, v := range $ {\n\t\n}".to_string(),
        "forv" => "for _, v := range $ {\n\t\n}".to_string(),
        _ => return None,
    };
    Some(body.replace('\n', &format!("\n{indent}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_names_follow_go_initialisms() {
        assert_eq!(json_name("ConfigKey"), "configKey");
        assert_eq!(json_name("ID"), "id");
        assert_eq!(json_name("UserID"), "userID");
        assert_eq!(json_name("HTTPServer"), "httpServer");
        assert_eq!(json_name("name"), "name");
    }

    #[test]
    fn json_tags_go_on_fields_without_one() {
        let source = "package p\n\ntype Config struct {\n\tKey string\n\tValue int `db:\"value\"`\n\tName string `json:\"n\"`\n\tA, B int\n\tEmbedded\n}\n";
        let offset = source.find("Key").unwrap();
        let tagged = json_tags(source, offset).unwrap();
        assert_eq!(
            tagged,
            "package p\n\ntype Config struct {\n\tKey string `json:\"key\"`\n\tValue int `json:\"value\" db:\"value\"`\n\tName string `json:\"n\"`\n\tA, B int\n\tEmbedded\n}\n"
        );
        // On the type's name too; nothing left to add the second time.
        let on_name = source.find("Config").unwrap();
        assert!(can_add_json_tags(source, on_name));
        assert!(!can_add_json_tags(&tagged, on_name));
        assert!(!can_add_json_tags("package p\n\nfunc f() {}\n", 12));
    }

    #[test]
    fn iferr_returns_zero_values_and_the_error() {
        let source = "package p\n\nfunc load() (*Item, int, string, error) {\n\t\n}\n";
        let offset = source.find("\t\n").unwrap() + 1;
        assert_eq!(
            expand_template("iferr", source, offset, "\t").unwrap(),
            "if err != nil {\n\t\treturn nil, 0, \"\", err$\n\t}"
        );
        let plain = "package p\n\nfunc run() {\n\t\n}\n";
        let offset = plain.find("\t\n").unwrap() + 1;
        assert_eq!(expand_template("iferr", plain, offset, "").unwrap(), "if err != nil {\n\treturn$\n}");
        assert!(expand_template("nothing", plain, offset, "").is_none());
    }
}

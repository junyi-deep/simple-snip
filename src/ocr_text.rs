use gpui_kit::component::highlighter::LanguageRegistry;
use snip_core::editor_preferences::OcrLanguage;
use std::ops::Range;
use tree_sitter::{Node, Parser};

pub const MAX_FORMAT_BYTES: usize = 256 * 1024;
const MAX_FORMAT_OUTPUT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub enum FormatError {
    Invalid(serde_json::Error),
    InvalidCode(usize, usize),
    UnsafeLayout,
    Unsupported,
    TooLarge,
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(error) => write!(
                f,
                "JSON 格式无效：第 {} 行，第 {} 列",
                error.line(),
                error.column()
            ),
            Self::InvalidCode(row, col) => {
                write!(f, "第 {row} 行，第 {col} 列有语法错误，请先修正识别结果")
            }
            Self::UnsafeLayout => f.write_str("该片段无法安全排版，已保留原文"),
            Self::Unsupported => f.write_str("请选择识别结果的代码语言"),
            Self::TooLarge => f.write_str("内容或排版结果太大，无法在 OCR 面板中格式化"),
        }
    }
}

pub const LANGUAGES: &[(OcrLanguage, &str)] = &[
    (OcrLanguage::Auto, "自动"),
    (OcrLanguage::PlainText, "纯文本"),
    (OcrLanguage::Json, "JSON"),
    (OcrLanguage::JavaScript, "JavaScript"),
    (OcrLanguage::TypeScript, "TypeScript"),
    (OcrLanguage::Python, "Python"),
    (OcrLanguage::Rust, "Rust"),
    (OcrLanguage::Yaml, "YAML"),
    (OcrLanguage::Go, "Go"),
    (OcrLanguage::Java, "Java"),
];

pub fn label(language: OcrLanguage) -> &'static str {
    LANGUAGES
        .iter()
        .find(|(mode, _)| *mode == language)
        .map_or("纯文本", |(_, label)| label)
}

pub fn syntax(language: OcrLanguage, text: &str) -> &'static str {
    match language {
        OcrLanguage::Auto => {
            if text.len() <= MAX_FORMAT_BYTES
                && text.trim_start().starts_with(['{', '['])
                && validate_json(text).is_ok()
            {
                "json"
            } else {
                "plaintext"
            }
        }
        OcrLanguage::PlainText => "plaintext",
        OcrLanguage::Json => "json",
        OcrLanguage::JavaScript => "javascript",
        OcrLanguage::TypeScript => "typescript",
        OcrLanguage::Python => "python",
        OcrLanguage::Rust => "rust",
        OcrLanguage::Yaml => "yaml",
        OcrLanguage::Go => "go",
        OcrLanguage::Java => "java",
    }
}

fn validate_json(text: &str) -> Result<(), serde_json::Error> {
    // Validate without converting numbers or collecting object keys.
    serde_json::from_str::<serde::de::IgnoredAny>(text).map(|_| ())
}

/// Once validated, change only JSON whitespace outside string literals.
/// Keeping the tokens intact also preserves large numbers, escapes, key order,
/// duplicate keys, and interpolation placeholders in translation catalogs.
pub fn format_json(text: &str) -> Result<String, FormatError> {
    if text.len() > MAX_FORMAT_BYTES {
        return Err(FormatError::TooLarge);
    }
    validate_json(text).map_err(FormatError::Invalid)?;
    let mut out = String::with_capacity(text.len());
    let mut depth: usize = 0;
    let mut previous = None;
    let mut quoted = false;
    let mut escaped = false;
    let next_line = |out: &mut String, depth| {
        out.push('\n');
        out.extend(std::iter::repeat_n(' ', depth * 2));
    };
    for ch in text.chars() {
        if out.len() >= MAX_FORMAT_OUTPUT_BYTES {
            return Err(FormatError::TooLarge);
        }
        if quoted {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
                previous = Some(ch);
            }
            continue;
        }
        if matches!(ch, ' ' | '\t' | '\r' | '\n') {
            continue;
        }
        if matches!(ch, '}' | ']') {
            depth -= 1;
            if !matches!(previous, Some('{' | '[')) {
                next_line(&mut out, depth);
            }
        } else if matches!(previous, Some('{' | '[' | ',')) {
            next_line(&mut out, depth);
        } else if previous == Some(':') {
            out.push(' ');
        }
        out.push(ch);
        match ch {
            '{' | '[' => depth += 1,
            '"' => quoted = true,
            _ => {}
        }
        previous = Some(ch);
    }
    Ok(out)
}

pub fn format(text: &str, syntax: &str) -> Result<String, FormatError> {
    if text.len() > MAX_FORMAT_BYTES {
        return Err(FormatError::TooLarge);
    }
    if syntax == "json" {
        return format_json(text);
    }
    let language = LanguageRegistry::singleton()
        .language(syntax)
        .and_then(|config| config.language)
        .ok_or(FormatError::Unsupported)?;
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|_| FormatError::Unsupported)?;
    let original = parser.parse(text, None).ok_or(FormatError::UnsafeLayout)?;
    let root = original.root_node();
    if root.has_error() {
        let point = invalid_node(root).start_position();
        return Err(FormatError::InvalidCode(point.row + 1, point.column + 1));
    }

    let tokens = layout_tokens(root);
    let brace_layout = !matches!(syntax, "python" | "yaml");
    let mut out = String::with_capacity(text.len());
    let mut end = 0;
    for (index, token) in tokens.iter().enumerate() {
        let gap = &text[end..token.bytes.start];
        let previous = index.checked_sub(1).map(|index| &tokens[index]);
        if let Some(previous) = previous.filter(|_| gap.chars().all(char::is_whitespace)) {
            let left = &text[previous.bytes.clone()];
            let right = &text[token.bytes.clone()];
            let same_empty_block = previous.opens && token.closes && previous.depth == token.depth;
            if brace_layout && !same_empty_block && (previous.opens || token.closes) {
                if gap.contains('\n') {
                    push_indented_gap(&mut out, gap, token.depth)?;
                } else {
                    push_indented_gap(&mut out, "\n", token.depth)?;
                }
            } else if brace_layout && gap.contains('\n') {
                push_indented_gap(&mut out, gap, token.depth)?;
            } else if !gap.contains('\n') {
                out.push_str(inline_gap(syntax, previous, token, left, right, gap));
            } else {
                out.push_str(gap);
            }
        } else {
            out.push_str(gap);
        }
        out.push_str(&text[token.bytes.clone()]);
        if out.len() > MAX_FORMAT_OUTPUT_BYTES {
            return Err(FormatError::TooLarge);
        }
        end = token.bytes.end;
    }
    out.push_str(&text[end..]);
    if out.len() > MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::TooLarge);
    }
    // Verify the entire syntax tree and token contents after the whitespace edit.
    // In particular this protects Python/YAML layout, Go and JS semicolon
    // insertion, compound operators, comments, and OCR strings.
    let formatted = parser.parse(&out, None).ok_or(FormatError::UnsafeLayout)?;
    let after = formatted.root_node();
    if after.has_error()
        || root.to_sexp() != after.to_sexp()
        || !tokens
            .iter()
            .map(|token| &text[token.bytes.clone()])
            .eq(layout_tokens(after)
                .into_iter()
                .map(|token| &out[token.bytes]))
    {
        return Err(FormatError::UnsafeLayout);
    }
    Ok(out)
}

fn invalid_node(mut node: Node<'_>) -> Node<'_> {
    while !node.is_error() && !node.is_missing() {
        let mut cursor = node.walk();
        let Some(child) = node
            .children(&mut cursor)
            .find(|child| child.has_error() || child.is_missing())
        else {
            break;
        };
        node = child;
    }
    node
}

struct LayoutToken {
    bytes: Range<usize>,
    parent: &'static str,
    depth: usize,
    opens: bool,
    closes: bool,
}

fn layout_tokens(root: Node<'_>) -> Vec<LayoutToken> {
    let mut tokens = Vec::new();
    let mut pending = vec![root];
    let mut depth = 0usize;
    while let Some(node) = pending.pop() {
        let kind = node.kind();
        if node.child_count() != 0
            && !matches!(
                kind,
                "string"
                    | "string_literal"
                    | "raw_string_literal"
                    | "interpreted_string_literal"
                    | "template_string"
                    | "concatenated_string"
                    | "regex"
                    | "raw_string"
                    | "char_literal"
                    | "character_literal"
                    | "rune_literal"
                    | "text_block"
                    | "block_scalar"
                    | "plain_scalar"
                    | "single_quote_scalar"
                    | "double_quote_scalar"
                    | "line_comment"
                    | "block_comment"
                    | "comment"
            )
        {
            pending.extend(
                (0..node.child_count())
                    .rev()
                    .filter_map(|i| node.child(i as u32)),
            );
            continue;
        }
        if node.byte_range().is_empty() {
            continue;
        }
        let parent = node.parent().map_or("", |parent| parent.kind());
        let block = matches!(
            parent,
            "statement_block"
                | "class_body"
                | "block"
                | "declaration_list"
                | "constructor_body"
                | "interface_body"
                | "enum_body"
                | "switch_block"
        );
        let opens = block && kind == "{";
        let closes = block && kind == "}";
        depth = depth.saturating_sub(usize::from(closes));
        tokens.push(LayoutToken {
            bytes: node.byte_range(),
            depth,
            parent,
            opens,
            closes,
        });
        depth += usize::from(opens);
    }
    tokens
}

fn push_indented_gap(out: &mut String, gap: &str, depth: usize) -> Result<(), FormatError> {
    let indentation = depth * 4;
    if out.len() + gap.len() + indentation > MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::TooLarge);
    }
    out.extend(std::iter::repeat_n(
        '\n',
        gap.bytes().filter(|&byte| byte == b'\n').count(),
    ));
    out.extend(std::iter::repeat_n(' ', indentation));
    Ok(())
}

fn inline_gap<'a>(
    syntax: &str,
    left_token: &LayoutToken,
    right_token: &LayoutToken,
    left: &str,
    right: &str,
    old: &'a str,
) -> &'a str {
    if syntax == "yaml" {
        return match (left, right) {
            (":" | ",", _) | ("-", _) if left_token.parent != "plain_scalar" => " ",
            (_, "," | "]" | "}") => "",
            _ => old,
        };
    }
    let operator = |value: &str, parent: &str| {
        matches!(
            value,
            "=" | ":="
                | "=>"
                | "=="
                | "!="
                | "==="
                | "!=="
                | "+="
                | "-="
                | "&&"
                | "||"
                | "<="
                | ">="
        ) || matches!(value, "+" | "-" | "*" | "/" | "%" | "<" | ">")
            && matches!(
                parent,
                "binary_expression" | "binary_operator" | "comparison_operator"
            )
    };
    if right_token.opens && !matches!(left, "(" | "[") {
        " "
    } else if matches!(right, "," | ";" | ")" | "]") || matches!(left, "(" | "[") {
        ""
    } else if matches!(left, "," | ":" | ";")
        || right == "(" && matches!(left, "if" | "for" | "while" | "switch" | "catch")
        || operator(left, left_token.parent)
        || operator(right, right_token.parent)
    {
        " "
    } else {
        old
    }
}

#[cfg(test)]
mod tests {
    use super::{FormatError, OcrLanguage, format, format_json, syntax};

    #[test]
    fn language_formatting_is_available_offline_and_preserves_strings() {
        for (syntax, raw, expected) in [
            (
                "go",
                "package main\nfunc demo(){return}\n",
                "func demo() {\n    return\n}",
            ),
            (
                "java",
                "class App{int value=1;}",
                "class App {\n    int value = 1;\n}",
            ),
            (
                "javascript",
                "const f=(x)=>{return `a  ${x}`;}",
                "const f = (x) => {\n    return `a  ${x}`;\n}",
            ),
            (
                "typescript",
                "let value:number=1;",
                "let value: number = 1;",
            ),
            (
                "rust",
                "fn foo(){let x=1;}",
                "fn foo() {\n    let x = 1;\n}",
            ),
            (
                "python",
                "if True:\n    text='保留  空格'\n",
                "if True:\n    text = '保留  空格'\n",
            ),
            (
                "yaml",
                "values: [one,two]\ntext: 'a  b'\n",
                "values: [one, two]\ntext: 'a  b'\n",
            ),
        ] {
            let formatted = format(raw, syntax).unwrap_or_else(|error| panic!("{syntax}: {error}"));
            assert!(formatted.contains(expected), "{syntax}: {formatted:?}");
            assert_eq!(format(&formatted, syntax).unwrap(), formatted);
        }
        assert!(matches!(
            format("public class {", "java"),
            Err(FormatError::InvalidCode(..))
        ));
    }

    #[test]
    fn json_formatting_preserves_translation_tokens_and_exact_numbers() {
        let raw = r#"{"z":"你好，{name} {{count}} \u00e9","ar":"مرحبا","z":[-0,900719925474099312345,1.2300e+30,{},[]],"s":"\" \\"}"#;
        let pretty = format_json(raw).unwrap();
        assert_eq!(pretty.lines().count(), 12);
        assert!(pretty.contains(r#""z": "你好，{name} {{count}} \u00e9""#));
        assert!(pretty.contains(r#""ar": "مرحبا""#));
        assert_eq!(pretty.matches(r#""z":"#).count(), 2);
        assert!(pretty.contains("900719925474099312345"));
        assert!(pretty.contains("1.2300e+30"));
        assert!(pretty.contains(r#""s": "\" \\""#));
        assert_eq!(format_json(&pretty).unwrap(), pretty);
    }

    #[test]
    fn auto_highlights_strict_json_and_leaves_ambiguous_ocr_as_plain_text() {
        let invalid = "{\n  “key”: 1\n}";
        let FormatError::Invalid(error) = format_json(invalid).unwrap_err() else {
            panic!("expected validation error");
        };
        assert_eq!((error.line(), error.column()), (2, 3));
        assert_eq!(syntax(OcrLanguage::Auto, invalid), "plaintext");
        assert_eq!(syntax(OcrLanguage::Auto, r#"[{"key":1}]"#), "json");
        assert_eq!(syntax(OcrLanguage::Json, invalid), "json");
        assert_eq!(format_json(" {   } ").unwrap(), "{}");
        assert!(format_json(r#"{"key":1,}"#).is_err());
    }
}

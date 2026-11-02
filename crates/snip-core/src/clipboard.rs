//! Bounded clipboard-to-RTF conversion. No browser, scripts, URLs or objects.
use anyhow::{Result, ensure};
use html5ever::{parse_document, tendril::TendrilSink};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

pub const MAX_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PIXELS: u64 = 16_000_000;

#[derive(Clone)]
pub struct RichText {
    pub rtf: Vec<u8>,
    pub width: u32,
}
#[derive(Clone)]
struct Style {
    font: String,
    size: f32,
    color: [u8; 3],
    background: Option<[u8; 3]>,
    bold: bool,
    italic: bool,
    underline: bool,
    align: &'static str,
    pre: bool,
    border: bool,
    border_width: f32,
    border_color: [u8; 3],
    border_kind: &'static str,
    width: Option<f32>,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            font: "Arial".into(),
            size: 18.,
            color: [24, 24, 27],
            background: None,
            bold: false,
            italic: false,
            underline: false,
            align: "ql",
            pre: false,
            border: false,
            border_width: 1.,
            border_color: [96, 96, 96],
            border_kind: "brdrs",
            width: None,
        }
    }
}
fn attr(node: &Handle, key: &str) -> Option<String> {
    if let NodeData::Element { attrs, .. } = &node.data {
        attrs
            .borrow()
            .iter()
            .find(|a| a.name.local.as_ref() == key)
            .map(|a| a.value.to_string())
    } else {
        None
    }
}
fn tag(node: &Handle) -> &str {
    if let NodeData::Element { name, .. } = &node.data {
        name.local.as_ref()
    } else {
        ""
    }
}
fn escape(text: &str) -> String {
    let mut out = String::new();
    for ch in text.encode_utf16() {
        match ch {
            10 => out.push_str("\\line "),
            13 => {}
            9 => out.push_str("\\tab "),
            92 | 123 | 125 => {
                out.push('\\');
                out.push(char::from_u32(ch as u32).unwrap());
            }
            32..=126 => out.push(char::from_u32(ch as u32).unwrap()),
            _ => out.push_str(&format!("\\u{}?", ch as i16)),
        }
    }
    out
}
fn dimension(s: &str) -> Option<f32> {
    let s = s.trim();
    let (value, factor) = if let Some(v) = s.strip_suffix("pt") {
        (v, 4. / 3.)
    } else if let Some(v) = s.strip_suffix("rem") {
        (v, 16.)
    } else {
        (s.strip_suffix("px").unwrap_or(s), 1.)
    };
    value
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite() && *n >= 0.)
        .map(|n| n * factor)
}
fn color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        let hex = if hex.len() == 3 {
            hex.chars().flat_map(|c| [c, c]).collect()
        } else {
            hex.to_string()
        };
        if hex.len() >= 6 && hex.is_ascii() {
            let n = u32::from_str_radix(&hex[..6], 16).ok()?;
            return Some([(n >> 16) as u8, (n >> 8) as u8, n as u8]);
        }
    }
    if s.starts_with("rgb(") || s.starts_with("rgba(") {
        let args = s.split_once('(')?.1.trim_end_matches(')');
        let numbers: Vec<_> = args
            .split(',')
            .take(3)
            .map(|n| n.trim().parse::<u8>())
            .collect();
        if numbers.len() == 3 {
            return Some([
                *numbers[0].as_ref().ok()?,
                *numbers[1].as_ref().ok()?,
                *numbers[2].as_ref().ok()?,
            ]);
        }
    }
    Some(match s.as_str() {
        "black" => [0, 0, 0],
        "white" => [255, 255, 255],
        "red" => [255, 0, 0],
        "blue" => [0, 0, 255],
        "green" => [0, 128, 0],
        "yellow" => [255, 255, 0],
        "gray" | "grey" => [128, 128, 128],
        "orange" => [255, 165, 0],
        "purple" => [128, 0, 128],
        "navy" => [0, 0, 128],
        "teal" => [0, 128, 128],
        _ => return None,
    })
}
fn declarations(style: &mut Style, css: &str) {
    for declaration in css.split(';') {
        let Some((key, value)) = declaration.split_once(':') else {
            continue;
        };
        let value = value.trim().trim_end_matches("!important").trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "font-family" => {
                style.font = value
                    .split(',')
                    .next()
                    .unwrap_or("Arial")
                    .trim()
                    .trim_matches(['\'', '"'])
                    .to_string()
            }
            "font-size" => {
                if let Some(size) = dimension(value) {
                    style.size = size.clamp(8., 144.);
                }
            }
            "font-weight" => {
                style.bold = value == "bold"
                    || value == "bolder"
                    || value.parse::<u32>().is_ok_and(|v| v >= 600)
            }
            "font-style" => style.italic = value == "italic" || value == "oblique",
            "text-decoration" | "text-decoration-line" => {
                style.underline = value.contains("underline")
            }
            "color" => {
                if let Some(c) = color(value) {
                    style.color = c;
                }
            }
            "background-color" | "background" => {
                if let Some(c) = color(value) {
                    style.background = Some(c);
                }
            }
            "text-align" => {
                style.align = match value {
                    "center" => "qc",
                    "right" => "qr",
                    _ => "ql",
                }
            }
            "white-space" => style.pre = value.starts_with("pre"),
            "width" => style.width = dimension(value).map(|v| v.clamp(24., 1200.)),
            "border" | "border-width" | "border-top" | "border-bottom" | "border-left"
            | "border-right" => {
                style.border = value != "0" && value != "none" && !value.starts_with("0px");
                for part in value.split_whitespace() {
                    if let Some(width) = dimension(part) {
                        style.border_width = width.clamp(0., 12.);
                    }
                    if let Some(c) = color(part) {
                        style.border_color = c;
                    }
                    match part {
                        "dashed" => style.border_kind = "brdrdash",
                        "dotted" => style.border_kind = "brdrdot",
                        "double" => style.border_kind = "brdrdb",
                        _ => {}
                    }
                }
            }
            "border-color" => {
                if let Some(c) = color(value) {
                    style.border_color = c;
                }
            }
            "border-style" => {
                style.border_kind = match value {
                    "dashed" => "brdrdash",
                    "dotted" => "brdrdot",
                    "double" => "brdrdb",
                    _ => "brdrs",
                }
            }
            _ => {}
        }
    }
}
struct Writer {
    body: String,
    fonts: Vec<String>,
    colors: Vec<[u8; 3]>,
    rules: Vec<(String, String)>,
    nodes: usize,
    characters: usize,
    width: u32,
}
impl Writer {
    fn new() -> Self {
        Self {
            body: String::new(),
            fonts: vec![],
            colors: vec![],
            rules: vec![],
            nodes: 0,
            characters: 0,
            width: 720,
        }
    }
    fn color(&mut self, value: [u8; 3]) -> usize {
        if let Some(i) = self.colors.iter().position(|c| *c == value) {
            i + 1
        } else {
            self.colors.push(value);
            self.colors.len()
        }
    }
    fn style(&self, node: &Handle, parent: &Style) -> Style {
        let mut style = parent.clone();
        style.width = None;
        let name = tag(node);
        match name {
            "b" | "strong" | "th" => style.bold = true,
            "i" | "em" => style.italic = true,
            "u" | "a" => style.underline = true,
            "pre" | "code" => {
                style.font = "Consolas".into();
                style.pre = name == "pre";
            }
            "h1" => {
                style.size = 32.;
                style.bold = true;
            }
            "h2" => {
                style.size = 28.;
                style.bold = true;
            }
            "h3" | "h4" => {
                style.size = 24.;
                style.bold = true;
            }
            _ => {}
        }
        if let Some(c) = attr(node, "color").and_then(|c| color(&c)) {
            style.color = c;
        }
        if let Some(c) = attr(node, "bgcolor").and_then(|c| color(&c)) {
            style.background = Some(c);
        }
        if let Some(font) = attr(node, "face") {
            style.font = font;
        }
        if let Some(border) = attr(node, "border") {
            style.border = border != "0";
        }
        let classes = attr(node, "class").unwrap_or_default();
        let id = attr(node, "id").unwrap_or_default();
        for (selector, css) in &self.rules {
            let matched = if let Some(class) = selector.strip_prefix('.') {
                classes.split_whitespace().any(|c| c == class)
            } else if let Some(selector_id) = selector.strip_prefix('#') {
                id == selector_id
            } else if let Some((element, class)) = selector.split_once('.') {
                element == name && classes.split_whitespace().any(|c| c == class)
            } else {
                selector == name || selector == "*"
            };
            if matched {
                declarations(&mut style, css);
            }
        }
        if let Some(width) = attr(node, "width").and_then(|v| dimension(&v)) {
            style.width = Some(width.clamp(24., 1200.));
        }
        if let Some(css) = attr(node, "style") {
            declarations(&mut style, &css);
        }
        style
    }
    fn text(&mut self, value: &str, style: &Style) {
        let value = if style.pre {
            value.to_string()
        } else {
            let mut out = String::new();
            let mut space = false;
            for ch in value.chars() {
                if ch.is_whitespace() && ch != '\u{a0}' {
                    if !space {
                        out.push(' ');
                    }
                    space = true;
                } else {
                    out.push(ch);
                    space = false;
                }
            }
            out
        };
        if value.trim().is_empty() && self.body.ends_with("\\par\n") {
            return;
        }
        self.characters += value.chars().filter(|c| !c.is_whitespace()).count();
        let font = if let Some(i) = self.fonts.iter().position(|f| *f == style.font) {
            i
        } else {
            self.fonts.push(style.font.clone());
            self.fonts.len() - 1
        };
        let fg = self.color(style.color);
        let bg = style.background.map(|c| self.color(c)).unwrap_or(0);
        self.body.push_str(&format!(
            "{{\\f{font}\\fs{}\\cf{fg}\\highlight{bg}\\cb{bg}\\b{}\\i{}\\ul{} {}}}",
            (style.size * 1.5).round() as u32,
            u8::from(style.bold),
            u8::from(style.italic),
            u8::from(style.underline),
            escape(&value)
        ));
    }
    fn visit(&mut self, node: &Handle, parent: &Style, depth: usize) -> Result<()> {
        self.nodes += 1;
        ensure!(
            depth <= 64 && self.nodes <= 20_000,
            "剪贴板内容结构过于复杂，请分段复制"
        );
        if let NodeData::Text { contents } = &node.data {
            self.text(&contents.borrow(), parent);
            return Ok(());
        }
        let name = tag(node);
        if matches!(
            name,
            "head" | "script" | "style" | "iframe" | "object" | "svg" | "img" | "link" | "meta"
        ) {
            return Ok(());
        }
        if attr(node, "hidden").is_some()
            || attr(node, "style").is_some_and(|s| s.replace(' ', "").contains("display:none"))
        {
            return Ok(());
        }
        let style = self.style(node, parent);
        if name == "table" {
            return self.table(node, &style, depth + 1);
        }
        if name == "br" {
            self.body.push_str("\\line ");
            return Ok(());
        }
        let block = matches!(
            name,
            "p" | "div" | "h1" | "h2" | "h3" | "h4" | "pre" | "li" | "blockquote"
        );
        if block {
            self.body.push_str(&format!("\\par\n\\{} ", style.align));
        }
        if name == "li" {
            self.text("• ", &style);
        }
        for child in node.children.borrow().iter() {
            self.visit(child, &style, depth + 1)?;
        }
        if block {
            self.body.push_str("\\par\n");
        }
        Ok(())
    }
    fn table(&mut self, table: &Handle, style: &Style, depth: usize) -> Result<()> {
        fn rows(node: &Handle, out: &mut Vec<Handle>) {
            for child in node.children.borrow().iter() {
                match tag(child) {
                    "tr" => out.push(child.clone()),
                    "thead" | "tbody" | "tfoot" => rows(child, out),
                    _ => {}
                }
            }
        }
        ensure!(depth < 64, "剪贴板表格嵌套过深");
        let mut rows_out = vec![];
        rows(table, &mut rows_out);
        ensure!(rows_out.len() <= 1000, "剪贴板表格过长，请分段复制");
        #[derive(Clone)]
        struct Cell {
            node: Option<Handle>,
            style: Style,
            column: usize,
            span: usize,
            merge: &'static str,
        }
        let mut pending: Vec<Option<(usize, Cell)>> = vec![None; 64];
        let mut grid = Vec::new();
        let mut columns = 1;
        for row in rows_out {
            let rs = self.style(&row, style);
            let source: Vec<_> = row
                .children
                .borrow()
                .iter()
                .filter(|c| matches!(tag(c), "td" | "th"))
                .cloned()
                .collect();
            let mut source = source.into_iter().peekable();
            let mut cells = Vec::new();
            let mut column = 0;
            while source.peek().is_some() || pending[column.min(64)..].iter().any(Option::is_some) {
                ensure!(column < 64, "剪贴板表格列数过多，请分段复制");
                if let Some((remaining, mut cell)) = pending[column].take() {
                    cell.node = None;
                    cell.merge = "\\clvmrg";
                    if remaining > 1 {
                        pending[column] = Some((remaining - 1, cell.clone()));
                    }
                    column += cell.span;
                    cells.push(cell);
                    continue;
                }
                let node = source.next();
                let span = node
                    .as_ref()
                    .and_then(|n| attr(n, "colspan"))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 64);
                ensure!(column + span <= 64, "剪贴板表格列数过多，请分段复制");
                let rowspan = node
                    .as_ref()
                    .and_then(|n| attr(n, "rowspan"))
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(1)
                    .clamp(1, 1000);
                let cs = node
                    .as_ref()
                    .map(|n| self.style(n, &rs))
                    .unwrap_or_else(|| rs.clone());
                let cell = Cell {
                    node,
                    style: cs,
                    column,
                    span,
                    merge: if rowspan > 1 { "\\clvmgf" } else { "" },
                };
                if rowspan > 1 {
                    pending[column] = Some((rowspan - 1, cell.clone()));
                }
                column += span;
                cells.push(cell);
            }
            columns = columns.max(column);
            grid.push(cells);
        }
        let mut widths = vec![0f32; columns];
        fn cols(node: &Handle, out: &mut Vec<Handle>) {
            for child in node.children.borrow().iter() {
                match tag(child) {
                    "col" => out.push(child.clone()),
                    "colgroup" => cols(child, out),
                    _ => {}
                }
            }
        }
        let mut col_nodes = vec![];
        cols(table, &mut col_nodes);
        let mut column = 0;
        for col in col_nodes {
            let span = attr(&col, "span")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 64);
            if let Some(width) = self.style(&col, style).width {
                for w in widths.iter_mut().skip(column).take(span) {
                    *w = width;
                }
            }
            column += span;
        }
        for cells in &grid {
            for cell in cells {
                for w in &mut widths[cell.column..cell.column + cell.span] {
                    if let Some(width) = cell.style.width {
                        *w = w.max(width / cell.span as f32);
                    } else if *w == 0. {
                        *w = 150.;
                    }
                }
            }
        }
        let total = widths.iter().sum::<f32>().max(1.);
        let scale = style.width.unwrap_or(total).min(1150.) / total;
        self.width = self.width.max((total * scale).ceil() as u32 + 32).min(1200);
        for cells in grid {
            self.body.push_str("\\par\\trowd\\trgaph90\\trleft0 ");
            for cell in &cells {
                let cs = &cell.style;
                self.body.push_str(cell.merge);
                if cs.border {
                    let border_color = self.color(cs.border_color);
                    for edge in ["t", "b", "l", "r"] {
                        self.body.push_str(&format!(
                            "\\clbrdr{edge}\\{}\\brdrw{}\\brdrcf{border_color}",
                            cs.border_kind,
                            (cs.border_width * 15.).round() as u32
                        ));
                    }
                }
                if let Some(bg) = cs.background {
                    let c = self.color(bg);
                    self.body
                        .push_str(&format!("\\clcbpat{c}\\clcfpat{c}\\clshdng10000"));
                }
                let right = widths[..cell.column + cell.span].iter().sum::<f32>() * scale;
                self.body
                    .push_str(&format!("\\cellx{} ", (right * 15.).round() as u32));
            }
            for cell in &cells {
                self.body
                    .push_str(&format!("\\pard\\intbl\\{} ", cell.style.align));
                if let Some(node) = &cell.node {
                    for child in node.children.borrow().iter() {
                        self.visit(child, &cell.style, depth + 1)?;
                    }
                }
                self.body.push_str("\\cell ");
            }
            self.body.push_str("\\row\n");
        }
        self.body.push_str("\\pard\\par\n");
        Ok(())
    }
    fn finish(self) -> Result<RichText> {
        ensure!(self.characters <= 100_000, "剪贴板文字过长，请分段复制");
        ensure!(self.characters > 0, "剪贴板中没有可贴出的文字");
        let fonts = self
            .fonts
            .iter()
            .enumerate()
            .map(|(i, f)| format!("{{\\f{i}\\fnil\\fcharset0 {};}}", escape(f)))
            .collect::<String>();
        let colors = self
            .colors
            .iter()
            .map(|c| format!("\\red{}\\green{}\\blue{};", c[0], c[1], c[2]))
            .collect::<String>();
        let rtf=format!("{{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1{{\\fonttbl{fonts}}}{{\\colortbl;{colors}}}\\viewkind4\\pard\\fs27 {} }}",self.body).into_bytes();
        ensure!(rtf.len() <= MAX_BYTES, "剪贴板内容过大，请分段复制");
        Ok(RichText {
            rtf,
            width: self.width,
        })
    }
}
fn dom(html: &str) -> RcDom {
    parse_document(RcDom::default(), Default::default()).one(html)
}
fn rules(node: &Handle, out: &mut Vec<(String, String)>, depth: usize) {
    if depth > 64 || out.len() >= 1024 {
        return;
    }
    if tag(node) == "style" {
        for child in node.children.borrow().iter() {
            if let NodeData::Text { contents } = &child.data {
                let raw = contents.borrow().replace("<!--", "").replace("-->", "");
                let mut remaining = raw.as_str();
                let mut cleaned = String::new();
                while let Some(start) = remaining.find("/*") {
                    cleaned.push_str(&remaining[..start]);
                    let Some(end) = remaining[start + 2..].find("*/") else {
                        remaining = "";
                        break;
                    };
                    remaining = &remaining[start + end + 4..];
                }
                cleaned.push_str(remaining);
                for rule in cleaned.split('}') {
                    if let Some((selectors, css)) = rule.split_once('{') {
                        for selector in selectors.split(',') {
                            if out.len() >= 1024 {
                                return;
                            }
                            out.push((selector.trim().to_string(), css.to_string()));
                        }
                    }
                }
            }
        }
    }
    for child in node.children.borrow().iter() {
        rules(child, out, depth + 1);
    }
}
/// CF_HTML offsets are UTF-8 byte positions, not character offsets.
pub fn from_html(bytes: &[u8]) -> Result<RichText> {
    ensure!(bytes.len() <= MAX_BYTES, "剪贴板 HTML 过大，请分段复制");
    let text = std::str::from_utf8(bytes)?.trim_end_matches('\0');
    let offset = |key: &str| -> Option<usize> {
        text.lines()
            .take(12)
            .find_map(|l| l.strip_prefix(key).and_then(|s| s.trim().parse().ok()))
    };
    let context = offset("StartHTML:")
        .zip(offset("EndHTML:"))
        .and_then(|(a, b)| text.get(a..b))
        .unwrap_or_else(|| text.get(text.find('<').unwrap_or(0)..).unwrap_or(text));
    let fragment = offset("StartFragment:")
        .zip(offset("EndFragment:"))
        .and_then(|(a, b)| text.get(a..b))
        .or_else(|| {
            context
                .split_once("<!--StartFragment-->")
                .and_then(|(_, r)| r.split_once("<!--EndFragment-->").map(|(s, _)| s))
        })
        .unwrap_or(context);
    let context_dom = dom(context);
    let mut writer = Writer::new();
    rules(&context_dom.document, &mut writer.rules, 0);
    // Recover inherited formatting from enclosing tags outside StartFragment.
    fn inherited(node: &Handle, parent: &Style, w: &Writer, depth: usize) -> Option<Style> {
        if depth > 64 {
            return None;
        }
        if let NodeData::Comment { contents } = &node.data
            && contents.trim() == "StartFragment"
        {
            return Some(parent.clone());
        }
        let style = w.style(node, parent);
        node.children
            .borrow()
            .iter()
            .find_map(|c| inherited(c, &style, w, depth + 1))
    }
    let base = inherited(&context_dom.document, &Style::default(), &writer, 0).unwrap_or_default();
    let trim = fragment.trim_start();
    let wrapped = if trim.starts_with("<tr") || trim.starts_with("<tbody") {
        format!("<table>{fragment}</table>")
    } else if trim.starts_with("<td") || trim.starts_with("<th") {
        format!("<table><tr>{fragment}</tr></table>")
    } else {
        fragment.to_string()
    };
    writer.visit(&dom(&wrapped).document, &base, 0)?;
    writer.finish()
}
pub fn from_text(text: &str) -> Result<RichText> {
    ensure!(
        text.len() <= MAX_BYTES && text.chars().count() <= 100_000,
        "剪贴板文字过长，请分段复制"
    );
    if text.contains('\t') {
        let mut html = "<table border='1'>".to_string();
        for row in text.lines() {
            html.push_str("<tr>");
            for cell in row.split('\t') {
                html.push_str("<td>");
                html.push_str(
                    &cell
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                        .replace(' ', "&nbsp;"),
                );
                html.push_str("</td>");
            }
            html.push_str("</tr>");
        }
        html.push_str("</table>");
        return from_html(html.as_bytes());
    }
    let mut writer = Writer::new();
    writer.text(
        text,
        &Style {
            pre: true,
            ..Default::default()
        },
    );
    writer.finish()
}
/// Keep native rich text formatting, remove embedded images/OLE/field instructions.
pub fn from_rtf(bytes: &[u8]) -> Result<RichText> {
    ensure!(
        bytes.len() <= MAX_BYTES && bytes.starts_with(b"{\\rtf"),
        "剪贴板 RTF 格式无效或过大"
    );
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut depth = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            depth += 1;
            ensure!(depth <= 64, "剪贴板 RTF 嵌套过深");
            let tail = &bytes[index + 1..];
            if [
                b"\\pict".as_slice(),
                b"\\object",
                b"\\*\\objdata",
                b"\\*\\fldinst",
            ]
            .iter()
            .any(|p| tail.starts_with(p))
            {
                let mut nested = 1;
                index += 1;
                while index < bytes.len() && nested > 0 {
                    match bytes[index] {
                        b'\\' => {
                            index += 1;
                        }
                        b'{' => nested += 1,
                        b'}' => nested -= 1,
                        _ => {}
                    }
                    index += 1;
                }
                depth -= 1;
                continue;
            }
        } else if bytes[index] == b'}' {
            ensure!(depth > 0, "剪贴板 RTF 括号不匹配");
            depth -= 1;
        } else if bytes[index] == b'\\'
            && bytes
                .get(index + 1)
                .is_some_and(|b| matches!(*b, b'\\' | b'{' | b'}'))
        {
            out.push(bytes[index]);
            index += 1;
        }
        out.push(bytes[index]);
        index += 1;
    }
    ensure!(depth == 0, "剪贴板 RTF 不完整");
    let text = String::from_utf8_lossy(&out);
    let mut width = 1000u32;
    for value in text.split("\\cellx").skip(1) {
        let value: String = value
            .chars()
            .take_while(char::is_ascii_digit)
            .take(9)
            .collect();
        if let Ok(twips) = value.parse::<u32>() {
            width = width.max(twips.saturating_add(14) / 15 + 32);
        }
    }
    ensure!(width <= 1200, "剪贴板表格过宽，请分列复制");
    Ok(RichText { rtf: out, width })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn styled_table_keeps_unicode_colors_fonts_and_column_spans() {
        let rich=from_html(br#"<style>.title{color:#ff0000;font-size:24pt}td{border:1px solid black}</style><table><tr><td class='title' colspan='2'><b>Hello</b></td></tr><tr><td style='background-color:#12ab34;font-family:Consolas'>42</td><td><i>world</i></td></tr></table>"#).unwrap();
        let rtf = String::from_utf8(rich.rtf).unwrap();
        assert!(rtf.contains("\\trowd"));
        assert!(rtf.contains("\\clcbpat"));
        assert!(rtf.contains("\\red255\\green0\\blue0"));
        assert!(rtf.contains("\\fs48"));
        assert!(rtf.contains("\\b1"));
        assert!(rtf.contains("\\i1"));
        assert!(rtf.contains("Consolas"));
        assert_eq!(rtf.matches("\\row\n").count(), 2);
    }
    #[test]
    fn merged_cells_and_excel_column_widths_keep_the_table_grid() {
        let doc=from_html(br"<table border='1'><colgroup><col width='80'><col width='160'></colgroup><tr><td rowspan='2'>A</td><td>B</td></tr><tr><td>C</td></tr></table>").unwrap();
        let rtf = String::from_utf8(doc.rtf).unwrap();
        assert!(rtf.contains("\\clvmgf"));
        assert!(rtf.contains("\\clvmrg"));
        assert_eq!(rtf.matches("\\cellx1200").count(), 2);
        assert_eq!(rtf.matches("\\cellx3600").count(), 2);
    }
    #[test]
    fn html_fragment_excludes_neighbors_but_inherits_formatting() {
        let html = "<div style='color:#123456'>not selected<!--StartFragment-->中文😀<!--EndFragment-->also excluded</div>";
        let rtf = String::from_utf8(from_html(html.as_bytes()).unwrap().rtf).unwrap();
        assert!(!rtf.contains("selected"));
        assert!(!rtf.contains("excluded"));
        assert!(rtf.contains("\\u20013?"));
        assert!(rtf.contains("\\red18\\green52\\blue86"));
        let fragment = "中文";
        let header = format!(
            "Version:1.0\r\nStartFragment:{:010}\r\nEndFragment:{:010}\r\n",
            0, 0
        );
        let start = header.len();
        let header = format!(
            "Version:1.0\r\nStartFragment:{start:010}\r\nEndFragment:{:010}\r\n",
            start + fragment.len()
        );
        assert!(from_html(format!("{header}{fragment}").as_bytes()).is_ok());
    }
    #[test]
    fn text_and_tsv_are_escaped_and_untrusted_objects_never_render() {
        let rich = from_text("{hello}\\\n中文").unwrap();
        let rtf = String::from_utf8(rich.rtf).unwrap();
        assert!(rtf.contains("\\{hello\\}\\\\\\line"));
        assert!(
            String::from_utf8(from_text("a\tb\n1\t2").unwrap().rtf)
                .unwrap()
                .contains("\\trowd")
        );
        let html = "<script>evil()</script><iframe src='https://example.com'></iframe><img src='file:///secret'>safe";
        let rtf = String::from_utf8(from_html(html.as_bytes()).unwrap().rtf).unwrap();
        assert!(rtf.contains("safe"));
        assert!(!rtf.contains("evil"));
        assert!(!rtf.contains("secret"));
        let rtf = from_rtf(br"{\rtf1 text {\object{\*\objdata bad}}{\pict bad} \b bold}").unwrap();
        assert!(!String::from_utf8(rtf.rtf).unwrap().contains("bad"));
        assert!(from_text(" \n\t").is_err());
        assert!(from_html("<div>".repeat(100).as_bytes()).is_err());
        assert!(from_rtf(br"{\rtf1 incomplete").is_err());
    }
}

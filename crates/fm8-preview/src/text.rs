//! Light syntax highlighting: one generic tokenizer driven by a small
//! per-language table. Not a parser; good enough to make code readable.

use crate::{Opts, Preview};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Keyword,
    Str,
    Comment,
    Number,
    Heading,
    Punct,
}

#[derive(Clone, Debug)]
pub struct HighlightedText {
    pub lines: Vec<Vec<(Tok, String)>>,
    pub truncated: bool,
    pub total_bytes: u64,
    pub lang: &'static str,
}

struct Lang {
    name: &'static str,
    line_comment: &'static [&'static str],
    block: Option<(&'static str, &'static str)>,
    keywords: &'static [&'static str],
}

const C_KW: &[&str] = &["if", "else", "for", "while", "do", "switch", "case", "break", "continue", "return", "struct", "enum", "union", "typedef", "static", "const", "void", "int", "char", "long", "short", "unsigned", "signed", "float", "double", "sizeof", "extern", "inline", "class", "public", "private", "protected", "new", "delete", "template", "typename", "namespace", "using", "bool", "true", "false", "nullptr", "NULL", "goto", "default", "auto", "volatile", "include", "define"];
const RUST_KW: &[&str] = &["fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl", "trait", "for", "while", "loop", "if", "else", "match", "return", "break", "continue", "const", "static", "type", "where", "as", "in", "ref", "move", "async", "await", "dyn", "unsafe", "self", "Self", "super", "crate", "true", "false", "Some", "None", "Ok", "Err"];
const PY_KW: &[&str] = &["def", "class", "if", "elif", "else", "for", "while", "return", "import", "from", "as", "with", "try", "except", "finally", "raise", "lambda", "pass", "break", "continue", "in", "is", "not", "and", "or", "None", "True", "False", "yield", "global", "async", "await", "self"];
const JS_KW: &[&str] = &["function", "const", "let", "var", "if", "else", "for", "while", "return", "class", "new", "import", "export", "from", "default", "async", "await", "try", "catch", "finally", "throw", "typeof", "this", "null", "undefined", "true", "false", "switch", "case", "break", "continue", "interface", "type", "extends", "implements"];
const SH_KW: &[&str] = &["if", "then", "else", "elif", "fi", "for", "while", "do", "done", "case", "esac", "function", "return", "local", "export", "in", "echo", "exit", "set", "source"];
const GO_KW: &[&str] = &["func", "package", "import", "var", "const", "type", "struct", "interface", "map", "chan", "go", "defer", "if", "else", "for", "range", "switch", "case", "return", "break", "continue", "nil", "true", "false", "select"];
const NONE: &[&str] = &[];

fn lang_for(ext: &str) -> Lang {
    match ext {
        "rs" => Lang { name: "Rust", line_comment: &["//"], block: Some(("/*", "*/")), keywords: RUST_KW },
        "c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "java" | "cs" | "kt" | "swift" | "dart" | "zig" | "glsl" | "wgsl" | "vert" | "frag" | "hlsl" | "gdshader" => Lang { name: "C-like", line_comment: &["//"], block: Some(("/*", "*/")), keywords: C_KW },
        "go" => Lang { name: "Go", line_comment: &["//"], block: Some(("/*", "*/")), keywords: GO_KW },
        "js" | "mjs" | "ts" | "tsx" | "jsx" | "json5" => Lang { name: "JavaScript", line_comment: &["//"], block: Some(("/*", "*/")), keywords: JS_KW },
        "py" | "gd" | "mojo" | "rb" => Lang { name: "Python-like", line_comment: &["#"], block: None, keywords: PY_KW },
        "sh" | "bash" | "zsh" | "fish" | "env" | "mk" => Lang { name: "Shell", line_comment: &["#"], block: None, keywords: SH_KW },
        "toml" | "ini" | "conf" | "cfg" | "yaml" | "yml" | "desktop" | "service" | "theme" | "gitignore" => Lang { name: "Config", line_comment: &["#", ";"], block: None, keywords: NONE },
        "json" => Lang { name: "JSON", line_comment: &[], block: None, keywords: &["true", "false", "null"] },
        "lua" | "sql" => Lang { name: "Lua/SQL", line_comment: &["--"], block: Some(("/*", "*/")), keywords: &["function", "local", "end", "if", "then", "else", "for", "while", "do", "return", "select", "from", "where", "insert", "into", "update", "delete", "create", "table", "null", "true", "false", "and", "or", "not"] },
        "html" | "htm" | "xml" => Lang { name: "Markup", line_comment: &[], block: Some(("<!--", "-->")), keywords: NONE },
        "md" | "markdown" => Lang { name: "Markdown", line_comment: &[], block: None, keywords: NONE },
        _ => Lang { name: "Text", line_comment: &[], block: None, keywords: NONE },
    }
}

pub fn load(path: &Path, ext: &str, o: &Opts) -> Preview {
    use std::io::Read;
    let mut f = match std::fs::File::open(path) { Ok(f) => f, Err(e) => return Preview::Error(e.to_string()) };
    let total = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut buf = vec![0u8; o.max_text_bytes];
    let n = match f.read(&mut buf) { Ok(n) => n, Err(e) => return Preview::Error(e.to_string()) };
    buf.truncate(n);
    if buf.contains(&0) {
        return Preview::Info(vec!["Binary file".into(), format!("{total} bytes")]);
    }
    let s = String::from_utf8_lossy(&buf).into_owned();
    Preview::Text(highlight(&s, ext, total, total > n as u64))
}

pub fn highlight(src: &str, ext: &str, total: u64, truncated: bool) -> HighlightedText {
    let lang = lang_for(ext);
    let mut lines = vec![];
    let mut in_block = false;
    for raw in src.lines().take(2000) {
        let line = raw.replace('\t', "    ");
        lines.push(tokenize_line(&line, &lang, &mut in_block, ext));
    }
    HighlightedText { lines, truncated, total_bytes: total, lang: lang.name }
}

fn push(out: &mut Vec<(Tok, String)>, t: Tok, s: &str) {
    if s.is_empty() { return; }
    if let Some(last) = out.last_mut() {
        if last.0 == t { last.1.push_str(s); return; }
    }
    out.push((t, s.to_string()));
}

fn tokenize_line(line: &str, l: &Lang, in_block: &mut bool, ext: &str) -> Vec<(Tok, String)> {
    let mut out = vec![];
    if l.name == "Markdown" {
        let t = line.trim_start();
        if t.starts_with('#') { push(&mut out, Tok::Heading, line); }
        else if t.starts_with("```") || t.starts_with("- ") || t.starts_with("* ") { push(&mut out, Tok::Keyword, line); }
        else { push(&mut out, Tok::Plain, line); }
        return out;
    }
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let starts = |i: usize, pat: &str| -> bool { pat.chars().enumerate().all(|(k, c)| chars.get(i + k) == Some(&c)) };
    while i < chars.len() {
        if *in_block {
            let end = l.block.map(|b| b.1).unwrap_or("");
            let mut j = i;
            let mut closed = false;
            while j < chars.len() {
                if starts(j, end) { j += end.chars().count(); closed = true; break; }
                j += 1;
            }
            push(&mut out, Tok::Comment, &chars[i..j].iter().collect::<String>());
            i = j;
            if closed { *in_block = false; }
            continue;
        }
        if let Some((open, _)) = l.block {
            if starts(i, open) { *in_block = true; push(&mut out, Tok::Comment, open); i += open.chars().count(); continue; }
        }
        if l.line_comment.iter().any(|c| starts(i, c)) && !(ext == "toml" && false) {
            push(&mut out, Tok::Comment, &chars[i..].iter().collect::<String>());
            break;
        }
        let c = chars[i];
        if c == '"' || c == '\'' || (c == '`' && l.name != "Rust") {
            // Rust lifetimes ('a) are not strings
            if l.name == "Rust" && c == '\'' && !(chars.get(i + 2) == Some(&'\'') || chars.get(i + 1) == Some(&'\\')) {
                push(&mut out, Tok::Punct, "'");
                i += 1;
                continue;
            }
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                if chars[j] == '\\' { j += 1; }
                j += 1;
            }
            let j = (j + 1).min(chars.len());
            push(&mut out, Tok::Str, &chars[i..j].iter().collect::<String>());
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_') { j += 1; }
            push(&mut out, Tok::Number, &chars[i..j].iter().collect::<String>());
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') { j += 1; }
            let w: String = chars[i..j].iter().collect();
            let kw = l.keywords.contains(&w.as_str());
            // markup: tag names
            push(&mut out, if kw { Tok::Keyword } else { Tok::Plain }, &w);
            i = j;
        } else if "{}[]()<>=+-*/%&|!;:,.".contains(c) {
            push(&mut out, Tok::Punct, &c.to_string());
            i += 1;
        } else {
            push(&mut out, Tok::Plain, &c.to_string());
            i += 1;
        }
    }
    out
}

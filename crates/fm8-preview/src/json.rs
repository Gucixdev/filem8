//! Minimal std-only JSON reader (RFC 8259) with a serde_json-like accessor API.

use std::ops::Index;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Array(Vec<Value>),
    /// Insertion-ordered members.
    Object(Vec<(String, Value)>),
}

static NULL: Value = Value::Null;

impl Value {
    pub fn get(&self, k: &str) -> Option<&Value> {
        match self { Value::Object(m) => m.iter().find(|(n, _)| n == k).map(|(_, v)| v), _ => None }
    }
    /// JSON pointer, e.g. `/meta/image`.
    pub fn pointer(&self, p: &str) -> Option<&Value> {
        let mut cur = self;
        for seg in p.split('/').skip(1) {
            cur = match cur { Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?, _ => cur.get(seg)? };
        }
        Some(cur)
    }
    pub fn is_array(&self) -> bool { matches!(self, Value::Array(_)) }
    pub fn as_array(&self) -> Option<&Vec<Value>> { if let Value::Array(a) = self { Some(a) } else { None } }
    pub fn as_str(&self) -> Option<&str> { if let Value::Str(s) = self { Some(s) } else { None } }
    pub fn as_f64(&self) -> Option<f64> { if let Value::Num(n) = self { Some(*n) } else { None } }
    pub fn as_i64(&self) -> Option<i64> { self.as_f64().filter(|n| n.fract() == 0.0).map(|n| n as i64) }
    pub fn as_u64(&self) -> Option<u64> { self.as_f64().filter(|n| n.fract() == 0.0 && *n >= 0.0).map(|n| n as u64) }
    pub fn as_bool(&self) -> Option<bool> { if let Value::Bool(b) = self { Some(*b) } else { None } }
}

impl Index<&str> for Value {
    type Output = Value;
    fn index(&self, k: &str) -> &Value { self.get(k).unwrap_or(&NULL) }
}

pub fn parse(src: &[u8]) -> Option<Value> {
    let mut p = P { b: src, i: 0, depth: 0 };
    p.ws();
    if p.b.starts_with(&[0xEF, 0xBB, 0xBF]) { p.i = 3; p.ws(); }
    let v = p.value()?;
    p.ws();
    if p.i == p.b.len() { Some(v) } else { None }
}

struct P<'a> { b: &'a [u8], i: usize, depth: u32 }

impl P<'_> {
    fn ws(&mut self) { while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) { self.i += 1 } }
    fn eat(&mut self, c: u8) -> bool { if self.b.get(self.i) == Some(&c) { self.i += 1; true } else { false } }
    fn lit(&mut self, s: &str, v: Value) -> Option<Value> {
        if self.b[self.i..].starts_with(s.as_bytes()) { self.i += s.len(); Some(v) } else { None }
    }
    fn value(&mut self) -> Option<Value> {
        self.depth += 1;
        if self.depth > 256 { return None }
        self.ws();
        let r = match *self.b.get(self.i)? {
            b'n' => self.lit("null", Value::Null),
            b't' => self.lit("true", Value::Bool(true)),
            b'f' => self.lit("false", Value::Bool(false)),
            b'"' => self.string().map(Value::Str),
            b'[' => {
                self.i += 1;
                let mut a = vec![];
                self.ws();
                if !self.eat(b']') {
                    loop {
                        a.push(self.value()?);
                        self.ws();
                        if self.eat(b',') { continue }
                        if self.eat(b']') { break }
                        return None;
                    }
                }
                Some(Value::Array(a))
            }
            b'{' => {
                self.i += 1;
                let mut m = vec![];
                self.ws();
                if !self.eat(b'}') {
                    loop {
                        self.ws();
                        let k = self.string()?;
                        self.ws();
                        if !self.eat(b':') { return None }
                        m.push((k, self.value()?));
                        self.ws();
                        if self.eat(b',') { continue }
                        if self.eat(b'}') { break }
                        return None;
                    }
                }
                Some(Value::Object(m))
            }
            _ => self.number(),
        };
        self.depth -= 1;
        r
    }
    fn number(&mut self) -> Option<Value> {
        let s = self.i;
        while matches!(self.b.get(self.i), Some(b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E')) { self.i += 1 }
        std::str::from_utf8(&self.b[s..self.i]).ok()?.parse::<f64>().ok().map(Value::Num)
    }
    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.b.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }
    fn string(&mut self) -> Option<String> {
        if !self.eat(b'"') { return None }
        let mut out: Vec<u8> = vec![];
        loop {
            let c = *self.b.get(self.i)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.b.get(self.i)?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"', b'\\' => '\\', b'/' => '/', b'b' => '\u{8}', b'f' => '\u{c}', b'n' => '\n', b'r' => '\r', b't' => '\t',
                        b'u' => {
                            let mut u = self.hex4()?;
                            if (0xD800..0xDC00).contains(&u) && self.b[self.i..].starts_with(b"\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                u = 0x10000 + ((u - 0xD800) << 10) + (lo.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            char::from_u32(u).unwrap_or('\u{FFFD}')
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0..=0x1f => return None,
                _ => out.push(c),
            }
        }
        String::from_utf8(out).ok()
    }
}

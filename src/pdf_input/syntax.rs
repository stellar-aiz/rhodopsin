//! Bounded syntax reader for the documented classic-xref PDF subset.
use anyhow::{Result, bail, ensure};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Value {
    Number(f64),
    Name(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Dict(BTreeMap<String, Value>),
    Ref(u32),
    Bool,
    Null,
    Word(String),
}
impl Value {
    pub fn number(&self) -> Result<f64> {
        if let Self::Number(n) = self {
            Ok(*n)
        } else {
            bail!("Expected number")
        }
    }
    pub fn integer(&self) -> Result<usize> {
        let n = self.number()?;
        ensure!(
            n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64,
            "Invalid integer"
        );
        Ok(n as usize)
    }
    pub fn name(&self) -> Result<&str> {
        if let Self::Name(s) = self {
            Ok(s)
        } else {
            bail!("Expected name")
        }
    }
    pub fn dict(&self) -> Result<&BTreeMap<String, Value>> {
        if let Self::Dict(d) = self {
            Ok(d)
        } else {
            bail!("Expected dictionary")
        }
    }
    pub fn array(&self) -> Result<&[Value]> {
        if let Self::Array(a) = self {
            Ok(a)
        } else {
            bail!("Expected array")
        }
    }
}
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub tokens: usize,
    pub string_bytes: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            tokens: 0,
            string_bytes: 0,
        }
    }
    fn reserve_string(&mut self, count: usize) -> Result<()> {
        self.string_bytes = self
            .string_bytes
            .checked_add(count)
            .ok_or_else(|| anyhow::anyhow!("String byte limit"))?;
        ensure!(self.string_bytes <= 64 * 1024 * 1024, "String byte limit");
        Ok(())
    }
    pub fn space(&mut self) {
        loop {
            while self.pos < self.data.len() && self.data[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
            if self.data.get(self.pos) == Some(&b'%') {
                while self.pos < self.data.len() && self.data[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }
    pub fn word(&mut self) -> Result<&'a [u8]> {
        self.space();
        let start = self.pos;
        while self.pos < self.data.len()
            && !self.data[self.pos].is_ascii_whitespace()
            && !b"()<>[]{}/%".contains(&self.data[self.pos])
        {
            self.pos += 1;
        }
        ensure!(self.pos > start, "Missing token");
        Ok(&self.data[start..self.pos])
    }
    pub fn expect(&mut self, s: &[u8]) -> Result<()> {
        ensure!(self.word()? == s, "Unexpected token");
        Ok(())
    }
    pub fn value(&mut self, depth: usize, refs: bool) -> Result<Value> {
        ensure!(depth < 64, "Syntax depth limit exceeded");
        self.tokens += 1;
        ensure!(self.tokens <= 1_000_000, "Syntax token limit exceeded");
        self.space();
        let c = *self
            .data
            .get(self.pos)
            .ok_or_else(|| anyhow::anyhow!("Truncated value"))?;
        match c {
            b'[' => {
                self.pos += 1;
                let mut a = Vec::new();
                loop {
                    self.space();
                    if self.data.get(self.pos) == Some(&b']') {
                        self.pos += 1;
                        break;
                    }
                    a.push(self.value(depth + 1, refs)?);
                }
                Ok(Value::Array(a))
            }
            b'<' if self.data.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                let mut d = BTreeMap::new();
                loop {
                    self.space();
                    if self.data.get(self.pos..self.pos + 2) == Some(b">>") {
                        self.pos += 2;
                        break;
                    }
                    let key = self.value(depth + 1, false)?.name()?.to_owned();
                    let v = self.value(depth + 1, refs)?;
                    ensure!(d.insert(key, v).is_none(), "Duplicate dictionary key");
                }
                Ok(Value::Dict(d))
            }
            b'/' => {
                self.pos += 1;
                let start = self.pos;
                while self.pos < self.data.len()
                    && !self.data[self.pos].is_ascii_whitespace()
                    && !b"()<>[]{}/%".contains(&self.data[self.pos])
                {
                    self.pos += 1;
                }
                self.reserve_string(self.pos - start)?;
                let raw = &self.data[start..self.pos];
                ensure!(!raw.contains(&b'#'), "Escaped names are unsupported");
                Ok(Value::Name(
                    std::str::from_utf8(raw)
                        .map_err(|_| anyhow::anyhow!("Invalid name"))?
                        .into(),
                ))
            }
            b'<' => {
                self.pos += 1;
                let mut bytes = Vec::new();
                let mut nibble = None;
                loop {
                    let b = *self
                        .data
                        .get(self.pos)
                        .ok_or_else(|| anyhow::anyhow!("Truncated hex string"))?;
                    self.pos += 1;
                    if b == b'>' {
                        break;
                    }
                    if b.is_ascii_whitespace() {
                        continue;
                    }
                    let n = (b as char)
                        .to_digit(16)
                        .ok_or_else(|| anyhow::anyhow!("Invalid hex string"))?
                        as u8;
                    if let Some(h) = nibble.take() {
                        self.reserve_string(1)?;
                        bytes.push(h * 16 + n);
                    } else {
                        nibble = Some(n);
                    }
                }
                if let Some(h) = nibble {
                    self.reserve_string(1)?;
                    bytes.push(h * 16);
                }
                Ok(Value::Bytes(bytes))
            }
            b'(' => {
                self.pos += 1;
                let mut bytes = Vec::new();
                let mut nesting = 1;
                while nesting > 0 {
                    let mut b = *self
                        .data
                        .get(self.pos)
                        .ok_or_else(|| anyhow::anyhow!("Truncated literal string"))?;
                    self.pos += 1;
                    if b == b'\\' {
                        b = *self
                            .data
                            .get(self.pos)
                            .ok_or_else(|| anyhow::anyhow!("Truncated escape"))?;
                        self.pos += 1;
                        b = match b {
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            b'b' => 8,
                            b'f' => 12,
                            b'\n' => continue,
                            b'\r' => {
                                if self.data.get(self.pos) == Some(&b'\n') {
                                    self.pos += 1;
                                }
                                continue;
                            }
                            b'0'..=b'7' => {
                                let mut n = (b - b'0') as u16;
                                for _ in 0..2 {
                                    if let Some(v @ b'0'..=b'7') = self.data.get(self.pos) {
                                        n = n * 8 + (v - b'0') as u16;
                                        self.pos += 1;
                                    } else {
                                        break;
                                    }
                                }
                                n as u8
                            }
                            _ => b,
                        };
                    } else if b == b'(' {
                        nesting += 1;
                        ensure!(nesting < 64, "String nesting limit");
                    } else if b == b')' {
                        nesting -= 1;
                        if nesting == 0 {
                            break;
                        }
                    }
                    self.reserve_string(1)?;
                    bytes.push(b);
                }
                Ok(Value::Bytes(bytes))
            }
            _ => {
                let w = self.word()?;
                let s = std::str::from_utf8(w).map_err(|_| anyhow::anyhow!("Invalid token"))?;
                if let Ok(n) = s.parse::<f64>() {
                    ensure!(n.is_finite() && n.abs() <= 1e12, "Invalid number");
                    if refs && n >= 0.0 && n.fract() == 0.0 && n <= u32::MAX as f64 {
                        let save = self.pos;
                        if self.word().is_ok_and(|w| w == b"0")
                            && self.word().is_ok_and(|w| w == b"R")
                        {
                            return Ok(Value::Ref(n as u32));
                        }
                        self.pos = save;
                    }
                    Ok(Value::Number(n))
                } else {
                    Ok(match s {
                        "true" => Value::Bool,
                        "false" => Value::Bool,
                        "null" => Value::Null,
                        _ => Value::Word(s.into()),
                    })
                }
            }
        }
    }
}
pub struct Object {
    pub value: Value,
    pub stream: Option<Vec<u8>>,
}
pub struct Document {
    pub objects: BTreeMap<u32, Object>,
    pub root: Value,
}
impl Document {
    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(data.starts_with(b"%PDF-1."), "Unsupported PDF header");
        let marker = b"startxref";
        let start = data
            .windows(marker.len())
            .rposition(|w| w == marker)
            .ok_or_else(|| anyhow::anyhow!("Missing cross-reference offset"))?;
        let mut end = Reader::new(&data[start + marker.len()..]);
        let offset = end.value(0, false)?.integer()?;
        ensure!(offset < data.len(), "Invalid cross-reference offset");
        let mut r = Reader::new(&data[offset..]);
        r.expect(b"xref")?;
        let mut offsets = BTreeMap::new();
        loop {
            r.space();
            if r.data.get(r.pos..r.pos + 7) == Some(b"trailer") {
                r.pos += 7;
                break;
            }
            let first = r.value(0, false)?.integer()?;
            let count = r.value(0, false)?.integer()?;
            ensure!(
                count <= 100_000 && offsets.len() + count <= 100_000,
                "Object count limit"
            );
            for i in 0..count {
                let off = r.value(0, false)?.integer()?;
                let generation = r.value(0, false)?.integer()?;
                let status = r.word()?;
                ensure!(
                    status == b"n" || status == b"f",
                    "Invalid cross-reference entry"
                );
                if status == b"n" {
                    ensure!(
                        generation == 0 && first + i <= u32::MAX as usize,
                        "Unsupported generation"
                    );
                    ensure!(
                        offsets.insert((first + i) as u32, off).is_none(),
                        "Duplicate object"
                    );
                }
            }
        }
        let trailer = r.value(0, true)?;
        let t = trailer.dict()?;
        ensure!(!t.contains_key("Encrypt"), "Encrypted PDF is unsupported");
        ensure!(
            !t.contains_key("Prev") && !t.contains_key("XRefStm"),
            "Incremental or hybrid PDF is unsupported"
        );
        let root = t
            .get("Root")
            .ok_or_else(|| anyhow::anyhow!("Missing catalog"))?
            .clone();
        let mut objects = BTreeMap::new();
        let mut total_tokens = 0;
        let mut total_strings = 0;
        let mut total_stream = 0;
        for (id, off) in offsets {
            ensure!(off < offset, "Invalid object offset");
            let mut o = Reader::new(&data[off..offset]);
            o.tokens = total_tokens;
            o.string_bytes = total_strings;
            ensure!(
                o.value(0, false)?.integer()? == id as usize,
                "Object ID mismatch"
            );
            ensure!(o.value(0, false)?.integer()? == 0, "Unsupported generation");
            o.expect(b"obj")?;
            let value = o.value(0, true)?;
            o.space();
            let stream = if o.data.get(o.pos..o.pos + 6) == Some(b"stream") {
                o.pos += 6;
                if o.data.get(o.pos) == Some(&b'\r') {
                    o.pos += 1;
                }
                ensure!(
                    o.data.get(o.pos) == Some(&b'\n'),
                    "Invalid stream delimiter"
                );
                o.pos += 1;
                let len = value
                    .dict()?
                    .get("Length")
                    .ok_or_else(|| anyhow::anyhow!("Missing stream length"))?
                    .integer()?;
                total_stream += len;
                ensure!(total_stream <= 64 * 1024 * 1024, "Stream byte limit");
                let stop = o
                    .pos
                    .checked_add(len)
                    .filter(|n| *n <= o.data.len())
                    .ok_or_else(|| anyhow::anyhow!("Invalid stream length"))?;
                let bytes = o.data[o.pos..stop].to_vec();
                o.pos = stop;
                o.expect(b"endstream")?;
                Some(bytes)
            } else {
                None
            };
            o.expect(b"endobj")?;
            total_tokens = o.tokens;
            total_strings = o.string_bytes;
            objects.insert(id, Object { value, stream });
        }
        Ok(Self { objects, root })
    }
    pub fn resolve<'a>(&'a self, v: &'a Value) -> Result<&'a Value> {
        let mut v = v;
        for _ in 0..64 {
            if let Value::Ref(id) = v {
                v = &self
                    .objects
                    .get(id)
                    .ok_or_else(|| anyhow::anyhow!("Missing object"))?
                    .value;
            } else {
                return Ok(v);
            }
        }
        bail!("Reference depth or cycle limit")
    }
    pub fn dict<'a>(&'a self, v: &'a Value) -> Result<&'a BTreeMap<String, Value>> {
        self.resolve(v)?.dict()
    }
    pub fn object(&self, v: &Value) -> Result<&Object> {
        if let Value::Ref(id) = v {
            self.objects
                .get(id)
                .ok_or_else(|| anyhow::anyhow!("Missing stream object"))
        } else {
            bail!("Expected stream reference")
        }
    }
}

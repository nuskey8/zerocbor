use zerocbor::Value;

/// Compares two decoded values, treating any two `NaN`s as the same and ignoring
/// the float width, which is not part of the value.
pub fn same_value(a: &Value<'_>, b: &Value<'_>) -> bool {
    use Value::*;
    match (a, b) {
        (Null, Null) | (Undefined, Undefined) => true,
        (Bool(x), Bool(y)) => x == y,
        (Simple(x), Simple(y)) => x == y,
        (Integer(x), Integer(y)) => x == y,
        (Float(x), Float(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Bytes(x), Bytes(y)) => x == y,
        (Text(x), Text(y)) => x == y,
        (Tag(x, p), Tag(y, q)) => x == y && same_value(p, q),
        (Array(x), Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same_value(p, q))
        }
        (Map(x), Map(y)) => {
            // A key is compared with the same rule as a value, since a key is a
            // value: a `NaN` inside a key does not compare equal to itself, and
            // a map keyed by floats is legal.
            x.len() == y.len()
                && x.iter()
                    .zip(y)
                    .all(|((kx, vx), (ky, vy))| same_value(kx, ky) && same_value(vx, vy))
        }
        _ => false,
    }
}

/// A readable rendering of a value. `Value` has no `Display`, and a failure that
/// only says "these differ" costs more than the run that found it.
pub fn show(value: &Value<'_>) -> String {
    use Value::*;
    let mut out = String::new();
    fn go(value: &Value<'_>, out: &mut String, depth: usize) {
        if depth > 4 {
            out.push_str("...");
            return;
        }
        match value {
            Null => out.push_str("null"),
            Undefined => out.push_str("undefined"),
            Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Simple(n) => out.push_str(&format!("simple({n})")),
            Integer(n) => out.push_str(&format!("{n}")),
            Float(f) => out.push_str(&format!("{f:?}")),
            Bytes(b) => out.push_str(&format!("h'{}'", hex(b))),
            Text(t) => out.push_str(&format!("{t:?}")),
            Tag(n, inner) => {
                out.push_str(&format!("{n}("));
                go(inner, out, depth + 1);
                out.push(')');
            }
            Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(item, out, depth + 1);
                }
                out.push(']');
            }
            Map(entries) => {
                out.push('{');
                for (i, (k, v)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    go(k, out, depth + 1);
                    out.push_str(": ");
                    go(v, out, depth + 1);
                }
                out.push('}');
            }
            // `Value` is `#[non_exhaustive]`, so a shape added later is shown
            // by name rather than not at all.
            other => out.push_str(&format!("<{other:?}>")),
        }
    }
    go(value, &mut out, 0);
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where the rewriter is in the input.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Cursor { bytes, at: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let slice = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(slice)
    }

    fn done(&self) -> bool {
        self.at == self.bytes.len()
    }
}

/// The length argument of one item, and the width it occupied.
struct Head {
    major: u8,
    /// The payload length for a string or container, unused for the rest.
    len: usize,
    /// How many bytes the head itself took, including this struct's own.
    width: usize,
}

/// Reads one head, refusing anything that is not a single definite-length item.
fn read_head(c: &mut Cursor<'_>) -> Option<Head> {
    let initial = *c.take(1)?.first()?;
    let major = initial >> 5;
    let info = initial & 0x1f;
    let (len, arg) = match info {
        0..=23 => (info as usize, 0),
        24 => (*c.take(1)?.first()? as usize, 1),
        25 => {
            let b = c.take(2)?;
            (u16::from_be_bytes([b[0], b[1]]) as usize, 2)
        }
        26 => {
            let b = c.take(4)?;
            (u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize, 4)
        }
        27 => {
            let b = c.take(8)?;
            (
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize,
                8,
            )
        }
        // 28 through 30 are reserved and 31 is the indefinite form, which this
        // rewriter does not produce as an input.
        _ => return None,
    };
    Some(Head {
        major,
        len,
        width: 1 + arg,
    })
}

/// Writes a string chunk: a major type 2 or 3 head followed by its payload.
fn write_chunk(major: u8, payload: &[u8], out: &mut Vec<u8>) {
    let head = (major << 5)
        | match payload.len() {
            n if n < 24 => n as u8,
            n if n <= u8::MAX as usize => 24,
            n if n <= u16::MAX as usize => 25,
            _ => 26,
        };
    out.push(head);
    match head & 0x1f {
        24 => out.push(payload.len() as u8),
        25 => out.extend_from_slice(&(payload.len() as u16).to_be_bytes()),
        26 => out.extend_from_slice(&(payload.len() as u32).to_be_bytes()),
        _ => {}
    }
    out.extend_from_slice(payload);
}

/// Rewrites one definite-length value into the indefinite-length form.
///
/// `None` if the input is not one well-formed definite-length value, which is
/// not a failure: the target only claims that the rewrite of a value the crate
/// produced decodes to the same thing.
///
/// `split_strings` picks one chunk or two, so both the borrowed and the joined
/// paths are exercised, with the boundary placed between characters.
pub fn to_indefinite(bytes: &[u8], split_strings: bool) -> Option<Vec<u8>> {
    let mut c = Cursor::new(bytes);
    let mut out = Vec::with_capacity(bytes.len() + 8);
    rewrite(&mut c, &mut out, split_strings, 0)?;
    c.done().then_some(out)
}

/// `depth` bounds the recursion here. The crate has its own `MAX_DEPTH`, and a
/// value that deep is not interesting to re-encode.
fn rewrite(c: &mut Cursor<'_>, out: &mut Vec<u8>, split: bool, depth: usize) -> Option<()> {
    if depth > 24 {
        return None;
    }
    let head = read_head(c)?;
    match head.major {
        // An integer, a negative integer, or a simple value and float: the item
        // is self-contained, so the head and its argument are copied as they
        // are. `read_head` has already validated that the argument is there.
        0 | 1 | 7 => {
            let start = c.at - head.width;
            out.extend_from_slice(&c.bytes[start..c.at]);
            Some(())
        }
        // A tag head is copied and the value it applies to is rewritten.
        6 => {
            let start = c.at - head.width;
            out.extend_from_slice(&c.bytes[start..c.at]);
            rewrite(c, out, split, depth + 1)
        }
        // A byte string or a text string: one chunk, or two with the boundary
        // placed before the last character.
        2 | 3 => {
            let payload = c.take(head.len)?;
            out.push((head.major << 5) | 0x1f);
            if split && payload.len() > 1 {
                // Text chunks must each be valid UTF-8. Move the cut back to
                // the start of the final code point rather than splitting it.
                let mut cut = payload.len() - 1;
                if head.major == 3 {
                    while cut > 0 && (payload[cut] & 0xc0) == 0x80 {
                        cut -= 1;
                    }
                }
                write_chunk(head.major, &payload[..cut], out);
                write_chunk(head.major, &payload[cut..], out);
            } else {
                write_chunk(head.major, payload, out);
            }
            out.push(0xff);
            Some(())
        }
        // An array: an indefinite head, the items, and a break.
        4 => {
            out.push((4 << 5) | 0x1f);
            for _ in 0..head.len {
                rewrite(c, out, split, depth + 1)?;
            }
            out.push(0xff);
            Some(())
        }
        // A map: the same, with a key and a value per entry.
        5 => {
            out.push((5 << 5) | 0x1f);
            for _ in 0..head.len {
                rewrite(c, out, split, depth + 1)?;
                rewrite(c, out, split, depth + 1)?;
            }
            out.push(0xff);
            Some(())
        }
        _ => None,
    }
}

/// A `Value` rebuilt for the differential target. `ciborium` has no
/// `undefined` and no bare simple value, so those are out of the set both
/// describe and this is the largest one worth checking.
pub fn to_comparable(value: &Value<'_>) -> Option<ciborium::Value> {
    use ciborium::Value as C;
    Some(match value {
        Value::Null => C::Null,
        Value::Bool(b) => C::Bool(*b),
        // An integer wider than 64 bits is where a `serde`-based decoder's
        // assumption shows: `ciborium` reads a major type 1 head with an 8-byte
        // argument as a `i64`, so a magnitude that does not fit comes back as
        // something else. This crate keeps it in an `i128`, which RFC 8949
        // Section 3.1 allows, so there is no second opinion to compare against
        // here and the case is out of scope for this target.
        Value::Integer(n) => C::Integer(i64::try_from(*n).ok()?.into()),
        Value::Float(f) => C::Float(*f),
        Value::Text(t) => C::Text(t.to_string()),
        Value::Bytes(b) => C::Bytes(b.to_vec()),
        Value::Array(items) => C::Array(
            items
                .iter()
                .map(to_comparable)
                .collect::<Option<Vec<_>>>()?,
        ),
        Value::Map(entries) => C::Map(
            entries
                .iter()
                .map(|(k, v)| Some((to_comparable(k)?, to_comparable(v)?)))
                .collect::<Option<Vec<_>>>()?,
        ),
        _ => return None,
    })
}

/// Whether a `ciborium` value equals a `zerocbor` one. `ciborium::Value` has no
/// `Ord` and a `f64` in it does not compare equal to itself, so both are walked.
pub fn same_as_ciborium(ours: &Value<'_>, theirs: &ciborium::Value) -> bool {
    use ciborium::Value as C;
    match (ours, theirs) {
        (Value::Null, C::Null) => true,
        (Value::Bool(a), C::Bool(b)) => a == b,
        (Value::Integer(a), C::Integer(b)) => *a == i128::from(*b),
        (Value::Float(a), C::Float(b)) => a == b || (a.is_nan() && b.is_nan()),
        (Value::Text(a), C::Text(b)) => a == b,
        (Value::Bytes(a), C::Bytes(b)) => a == b,
        (Value::Array(a), C::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_as_ciborium(x, y))
        }
        // A map is matched by walking, not lookup: this crate's is a
        // `BTreeMap` in canonical order and theirs a `Vec` in wire order. Each
        // of ours pairs with a distinct one of theirs, so an entry appearing
        // twice cannot satisfy two.
        (Value::Map(a), C::Map(b)) => {
            if a.len() != b.len() {
                return false;
            }
            let mut taken = vec![false; b.len()];
            for (k, v) in a {
                let mut matched = false;
                for (i, (their_k, their_v)) in b.iter().enumerate() {
                    if !taken[i] && same_as_ciborium(k, their_k) && same_as_ciborium(v, their_v) {
                        taken[i] = true;
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return false;
                }
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn indefinite_text_rewriting_preserves_code_point_boundaries() {
        for text in ["", "a", "é", "日本", "aé日"] {
            let encoded = zerocbor::to_cbor_vec(text).unwrap();
            for split in [false, true] {
                let rewritten = super::to_indefinite(&encoded, split).unwrap();
                assert_eq!(zerocbor::from_cbor::<String>(&rewritten).unwrap(), text);
            }
        }
    }
}

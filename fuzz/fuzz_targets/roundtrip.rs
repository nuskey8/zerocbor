#![no_main]

use std::collections::BTreeMap;

use arbitrary::{Arbitrary, Result, Unstructured};
use libfuzzer_sys::fuzz_target;
use zerocbor::Value;
use zerocbor_fuzz::{same_value, show, to_indefinite};

#[derive(Debug)]
struct ArbitraryValue(Value<'static>);

impl<'a> Arbitrary<'a> for ArbitraryValue {
    fn arbitrary(u: &mut Unstructured<'a>) -> Result<Self> {
        const {
            assert!(
                3 <= zerocbor::MAX_DEPTH,
                "the reader's depth bound is too small"
            );
        }
        let depth = u.int_in_range(0..=3)?;
        Ok(Self(Self::at_depth(u, depth)?))
    }

    fn arbitrary_take_rest(mut u: Unstructured<'a>) -> Result<Self> {
        Self::arbitrary(&mut u)
    }
}

impl ArbitraryValue {
    fn at_depth(u: &mut Unstructured<'_>, depth: u8) -> Result<Value<'static>> {
        Ok(match u.int_in_range(0..=10u8)? {
            0 => Value::Null,
            1 => Value::Bool(u.arbitrary()?),
            2 => Value::Integer(i128::from(u.arbitrary::<i64>()?)),
            3 => Value::Float(u.arbitrary()?),
            4 => Value::Text(u.arbitrary::<String>()?.into()),
            5 => Value::Bytes(u.arbitrary::<Vec<u8>>()?.into()),
            6 => Value::Simple(u.int_in_range(0..=19u8)?),
            7 => Value::Undefined,
            // Tags consume the same depth budget as containers.
            10 if depth > 0 => Value::Tag(u.arbitrary()?, Box::new(Self::at_depth(u, depth - 1)?)),
            8 if depth > 0 => {
                let len = u.int_in_range(0..=3usize)?;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(Self::at_depth(u, depth - 1)?);
                }
                Value::Array(items)
            }
            9 if depth > 0 => {
                let len = u.int_in_range(0..=3usize)?;
                let mut entries = BTreeMap::new();
                for _ in 0..len {
                    let k = Self::at_depth(u, depth - 1)?;
                    let v = Self::at_depth(u, depth - 1)?;
                    // Duplicate keys do not define an unambiguous round trip.
                    if entries.insert(k, v).is_some() {
                        return Err(arbitrary::Error::IncorrectFormat);
                    }
                }
                Value::Map(entries)
            }
            // The leaf shapes, so a bounded generator still produces something.
            _ => match u.int_in_range(0..=3u8)? {
                0 => Value::Null,
                1 => Value::Integer(i128::from(u.arbitrary::<i64>()?)),
                2 => Value::Float(u.arbitrary()?),
                _ => Value::Text(u.arbitrary::<String>()?.into()),
            },
        })
    }
}

fuzz_target!(|generated: ArbitraryValue| {
    let ArbitraryValue(value) = generated;
    let encoded = zerocbor::to_cbor_vec(&value)
        .unwrap_or_else(|e| panic!("{} would not encode: {e}", show(&value)));

    let decoded: Value<'_> = zerocbor::from_cbor(&encoded)
        .unwrap_or_else(|e| panic!("our own output {} did not decode: {e}", hex(&encoded)));
    assert!(
        same_value(&value, &decoded),
        "a round trip changed {} into {}",
        show(&value),
        show(&decoded),
    );

    // Exercise both single-chunk and split-chunk indefinite forms.
    for split in [false, true] {
        let rewritten = to_indefinite(&encoded, split)
            .expect("generated value could not be rewritten as indefinite CBOR");
        let decoded: Value<'_> =
            zerocbor::from_cbor(&rewritten).expect("indefinite form did not decode");
        assert!(
            same_value(&value, &decoded),
            "indefinite form changed {} (split={split})",
            show(&value)
        );
        assert_eq!(encoded, zerocbor::to_cbor_vec(&decoded).unwrap());
        let from_stream: Value<'_> =
            zerocbor::read_cbor(&rewritten[..]).expect("stream reader rejected indefinite form");
        assert!(
            same_value(&value, &from_stream),
            "indefinite stream changed {} (split={split})",
            show(&value)
        );
    }

    let mut stream = Vec::new();
    zerocbor::write_cbor(&mut stream, &value).expect("the stream writer failed");
    assert_eq!(
        hex(&stream),
        hex(&encoded),
        "the stream writer disagreed for {}",
        show(&value),
    );
    let from_stream: Value<'_> =
        zerocbor::read_cbor(std::io::Cursor::new(stream)).expect("the stream reader failed");
    assert!(
        same_value(&value, &from_stream),
        "a trip through a stream changed {} into {}",
        show(&value),
        show(&from_stream),
    );
});

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

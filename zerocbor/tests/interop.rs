//! Cross-checks against `ciborium`, an independent implementation, so the
//! encoder is not merely self-consistent. A dev-dependency only.

use std::collections::BTreeMap;

use zerocbor::{ToCbor, Value};

/// Encodes with both implementations and requires the same bytes.
#[track_caller]
fn assert_matches_ciborium<T>(value: &T)
where
    T: ToCbor + serde::Serialize + std::fmt::Debug,
{
    let ours = zerocbor::to_cbor_vec(value).expect("zerocbor failed to encode");
    let mut theirs = Vec::new();
    ciborium::into_writer(value, &mut theirs).expect("ciborium failed to encode");
    assert_eq!(ours, theirs, "encodings differ for {value:?}");

    // Exercise the direct slice writer with exactly enough room, and require
    // the stream writer's scalar/bulk paths to agree with the Vec writer.
    let mut exact = vec![0; ours.len()];
    let written = zerocbor::to_cbor(value, &mut exact).unwrap();
    assert_eq!(written, ours.len());
    assert_eq!(exact, ours);
    let mut short = vec![0; ours.len() - 1];
    assert!(matches!(
        zerocbor::to_cbor(value, &mut short),
        Err(zerocbor::Error::BufferTooSmall)
    ));
    #[cfg(feature = "std")]
    {
        let mut stream = Vec::new();
        zerocbor::write_cbor(&mut stream, value).unwrap();
        assert_eq!(stream, ours, "stream encoding differs for {value:?}");
    }
}

/// The values are of different types, so the list is a macro rather than an
/// array: an array would have to pick one element type for the lot.
macro_rules! against_ciborium {
    ($($value:expr),* $(,)?) => {$(
        assert_matches_ciborium(&$value);
    )*};
}

#[test]
fn scalars_match_ciborium() {
    // The boundary of every argument width, and both signs, since major type 0
    // and major type 1 are the two ways a number can go wrong.
    against_ciborium!(
        0u64,
        23,
        24,
        255,
        256,
        65535,
        65536,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        u64::MAX
    );
    against_ciborium!(0i64, -1, -24, -25, i64::MIN, i64::MAX);
    against_ciborium!(true, false, ());
    against_ciborium!("hello", "a longer string that needs a two byte head");
}

#[test]
fn containers_match_ciborium() {
    against_ciborium!(
        Vec::<u64>::new(),
        vec![1u64, 2, 3],
        vec![0u8, 23, 24, 255],
        vec![String::from("a"), String::from("b")],
        vec![Some(1u64), None, Some(3)],
        BTreeMap::from([(1u64, 2u64), (3u64, 4u64)]),
        BTreeMap::from([(String::from("key"), vec![1u64, 2])]),
    );

    // Large enough to need the 4-byte length heads.
    let many: Vec<u64> = (0..1000).map(|i| i * 7).collect();
    assert_matches_ciborium(&many);
    for len in [0, 23, 24, 255, 256, 65_535, 65_536, 100_000] {
        assert_matches_ciborium(&"x".repeat(len));
    }

    // A byte string is written by asking for one. `#[cbor(as_bytes = true)]` is
    // the field-level spelling; a hand-written impl can call `Write::write_binary`
    // directly, which is what this does so the two agree byte for byte.
    struct ByteString<'a>(&'a [u8]);
    impl zerocbor::ToCbor for ByteString<'_> {
        fn write<W: zerocbor::Write>(&self, writer: &mut W) -> zerocbor::Result<()> {
            writer.write_binary(self.0)
        }
    }
    let ours = zerocbor::to_cbor_vec(&ByteString(&[1u8, 2, 3][..])).unwrap();
    let mut theirs = Vec::new();
    ciborium::into_writer(&serde_bytes::ByteBuf::from(vec![1u8, 2, 3]), &mut theirs).unwrap();
    assert_eq!(ours, theirs);
}

#[test]
fn floats_match_ciborium_at_the_narrowest_width() {
    // The shortest width that preserves the value is the preferred
    // serialization, so `0.0f64` goes out as a half. `ciborium` narrows too, and
    // a negative zero keeps its sign through the narrowing.
    assert_eq!(zerocbor::to_cbor_vec(&0.0f64).unwrap(), vec![0xf9, 0, 0]);
    assert_eq!(
        zerocbor::to_cbor_vec(&-0.0f64).unwrap(),
        vec![0xf9, 0x80, 0x00]
    );
    assert_eq!(zerocbor::from_cbor::<f64>(&[0xf9, 0, 0]).unwrap(), 0.0);
    for value in [
        0.0f64,
        -0.0,
        1.0,
        std::f64::consts::PI,
        f64::INFINITY,
        f64::MIN,
    ] {
        assert_matches_ciborium(&value);
    }

    // Narrowing on the way in widens on the way out, into whichever of the two
    // Rust float widths the caller asked for.
    let mut narrow = Vec::new();
    ciborium::into_writer(&1.5f64, &mut narrow).unwrap();
    assert_eq!(narrow, vec![0xf9, 0x3e, 0x00], "ciborium narrows 1.5");
    assert_eq!(zerocbor::from_cbor::<f64>(&narrow).unwrap(), 1.5);
    assert_eq!(zerocbor::from_cbor::<f32>(&narrow).unwrap(), 1.5);
}

#[test]
fn the_dynamic_value_type_round_trips_and_is_understood() {
    let original = (
        Value::Integer(1),
        Value::Text("two".into()),
        Value::Float(3.5),
        Value::Bool(true),
        Value::Null,
    );

    let ours = zerocbor::to_cbor_vec(&original).unwrap();

    // `ciborium` must accept the bytes, even though `Value::Float` uses the
    // 9-byte form where a narrower one would do.
    let mut theirs = Vec::new();
    ciborium::into_writer(
        &(1i64, "two".to_string(), 3.5f64, true, Option::<u8>::None),
        &mut theirs,
    )
    .unwrap();
    let _: serde::de::IgnoredAny = ciborium::from_reader(ours.as_slice())
        .unwrap_or_else(|e| panic!("ciborium rejected our Value encoding: {e}"));

    let decoded: (Value<'_>, Value<'_>, Value<'_>, Value<'_>, Value<'_>) =
        zerocbor::from_cbor(&ours).unwrap();
    assert_eq!(decoded, original, "Value did not round trip");

    // The `ciborium` encoding of the same data decodes to the same `Value`s.
    let from_theirs: (Value<'_>, Value<'_>, Value<'_>, Value<'_>, Value<'_>) =
        zerocbor::from_cbor(&theirs).unwrap();
    assert_eq!(from_theirs, original);
}

#[test]
fn malformed_input_is_rejected_rather_than_misread() {
    // Every proper prefix of a full encoding must be rejected, never decoded as
    // something else.
    let full = zerocbor::to_cbor_vec(&vec![1u64, 2, 3, 4, 5, 6, 7, 8]).unwrap();
    for len in 0..full.len() {
        let result = zerocbor::from_cbor::<Vec<u64>>(&full[..len]);
        assert!(
            result.is_err(),
            "a {len}-byte prefix decoded as {:?}",
            result.ok(),
        );
    }
    assert!(zerocbor::from_cbor::<Vec<u64>>(&full).is_ok());

    // Every byte that is not a valid major type 0 head must be refused rather
    // than reinterpreted.
    for byte in 0x40u8..=0xff {
        if byte & 0xe0 == 0x00 {
            continue; // That is a legitimate unsigned integer.
        }
        let result = zerocbor::from_cbor::<u64>(&[byte]);
        assert!(result.is_err(), "0x{byte:02x} was accepted as a u64");
    }
}

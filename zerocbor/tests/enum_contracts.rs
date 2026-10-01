//! Declared container lengths and Rust enum discriminants define the wire shape.
use zerocbor::{Error, FromCbor, FromCborOwned, ToCbor};

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
enum Array {
    One(u8),
    Named {
        value: u8,
        #[cbor(ignore)]
        cache: String,
    },
    Tuple(#[cbor(ignore)] String, u8),
    Empty(),
}
#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map)]
enum Map {
    Unit,
    Data(u8),
    Named { value: u8 },
    Tuple(#[cbor(ignore)] String, u8),
}

fn decode<T: FromCborOwned + PartialEq + std::fmt::Debug>(bytes: &[u8], expected: T) {
    assert_eq!(zerocbor::from_cbor::<T>(bytes).unwrap(), expected);
    #[cfg(feature = "std")]
    assert_eq!(zerocbor::read_cbor::<_, T>(bytes).unwrap(), expected);
}
fn rejected<T: FromCborOwned>(bytes: &[u8]) {
    assert!(
        zerocbor::from_cbor::<T>(bytes).is_err(),
        "accepted {bytes:02x?}"
    );
    #[cfg(feature = "std")]
    assert!(
        zerocbor::read_cbor::<_, T>(bytes).is_err(),
        "stream accepted {bytes:02x?}"
    );
}

#[test]
fn enum_arrays_enforce_the_selected_variants_element_count() {
    decode::<Array>(&[0x82, 0x00, 0x07], Array::One(7));
    decode::<Array>(&[0x9f, 0x00, 0x07, 0xff], Array::One(7));
    decode::<(Array, u8)>(&[0x82, 0x82, 0x00, 0x07, 0x08], (Array::One(7), 8));
    for bytes in [
        &[0x80, 0x00, 0x07][..],
        &[0x81, 0x00, 0x07][..],
        &[0x83, 0x00, 0x07, 0x08][..],
        &[0x9f, 0x00, 0xff][..],
        &[0x9f, 0x00, 0x07, 0x08, 0xff][..],
    ] {
        rejected::<Array>(bytes);
    }
    // The final byte belongs to the inner array according to its header.
    rejected::<(Array, u8)>(&[0x82, 0x83, 0x00, 0x07, 0x08]);
    decode::<Array>(&[0x81, 0x03], Array::Empty());
    for value in [
        Array::Named {
            value: 7,
            cache: String::new(),
        },
        Array::Tuple(String::new(), 7),
    ] {
        let bytes = zerocbor::to_cbor_vec(&value).unwrap();
        assert_eq!(bytes[0], 0x82);
        decode::<Array>(&bytes, value);
    }
}

#[test]
fn enum_maps_enforce_the_envelope_and_tuple_payload_lengths() {
    decode::<Map>(b"\xa1\x64Data\x81\x07", Map::Data(7));
    decode::<Map>(b"\xbf\x64Data\x9f\x07\xff\xff", Map::Data(7));
    decode::<Map>(b"\xa1\x65Named\xa1\x65value\x07", Map::Named { value: 7 });
    decode::<Map>(b"\x64Unit", Map::Unit);
    for bytes in [
        &b"\xa0\x64Data\x81\x07"[..],
        &b"\xa2\x64Data\x81\x07\x61x\x00"[..],
        &b"\xa1\x64Data\x80\x07"[..],
        &b"\xa1\x64Data\x82\x07\x08"[..],
        &b"\xbf\x64Data\x81\x07\x61x\x00\xff"[..],
        &b"\xa1\x64Unit\xf6"[..],
    ] {
        rejected::<Map>(bytes);
    }
    let value = Map::Tuple(String::new(), 7);
    let encoded = zerocbor::to_cbor_vec(&value).unwrap();
    decode::<Map>(&encoded, value);
}

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(c_enum)]
#[repr(i64)]
enum Signed {
    Negative = -2,
    NextNegative,
    Zero = 0,
    Explicit = 1 << 8,
    Next,
}
#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(c_enum)]
#[repr(u64)]
enum Wide {
    Max = u64::MAX,
}
#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(c_enum)]
#[repr(u128)]
enum TooWide {
    Max = u128::MAX,
}
#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(c_enum, tag = 42)]
enum Tagged {
    Four = 4,
}

#[test]
fn c_enum_uses_explicit_and_implicit_rust_discriminants() {
    for (value, bytes) in [
        (Signed::Negative, vec![0x21]),
        (Signed::NextNegative, vec![0x20]),
        (Signed::Zero, vec![0x00]),
        (Signed::Explicit, vec![0x19, 0x01, 0x00]),
        (Signed::Next, vec![0x19, 0x01, 0x01]),
    ] {
        assert_eq!(zerocbor::to_cbor_vec(&value).unwrap(), bytes);
        decode::<Signed>(&bytes, value);
    }
    assert!(matches!(
        zerocbor::from_cbor::<Signed>(&[0x03]),
        Err(Error::UnknownVariantDiscriminant(3))
    ));
    let bytes = [0x1b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(zerocbor::to_cbor_vec(&Wide::Max).unwrap(), bytes);
    decode::<Wide>(&bytes, Wide::Max);
    assert!(matches!(
        zerocbor::to_cbor_vec(&TooWide::Max),
        Err(Error::IntegerOutOfRange)
    ));
    rejected::<TooWide>(&[0x20]);
    let bytes = [0xd8, 0x2a, 0x04];
    assert_eq!(zerocbor::to_cbor_vec(&Tagged::Four).unwrap(), bytes);
    decode::<Tagged>(&bytes, Tagged::Four);
}

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(c_enum)]
#[repr(i128)]
enum NegativeWide {
    MinCbor = -18_446_744_073_709_551_616,
    TooNegative = -18_446_744_073_709_551_617,
}

#[test]
fn c_enum_respects_the_full_cbor_integer_range() {
    let bytes = [0x3b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
    assert_eq!(
        zerocbor::to_cbor_vec(&NegativeWide::MinCbor).unwrap(),
        bytes
    );
    decode::<NegativeWide>(&bytes, NegativeWide::MinCbor);
    assert!(matches!(
        zerocbor::to_cbor_vec(&NegativeWide::TooNegative),
        Err(Error::IntegerOutOfRange)
    ));
}

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map, allow_unknown_fields)]
enum Extensible {
    Named { value: u8 },
}

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map)]
enum TaggedNamed {
    #[cbor(tag = 42)]
    Named { value: u8 },
}

#[test]
fn named_enum_maps_validate_keys_and_tags() {
    let duplicate = b"\xa1\x65Named\xa2\x65value\x01\x65value\x02";
    rejected::<Map>(duplicate);
    rejected::<Extensible>(duplicate);
    let unknown = b"\xa1\x65Named\xa2\x65value\x01\x65extra\x02";
    rejected::<Map>(unknown);
    decode::<Extensible>(unknown, Extensible::Named { value: 1 });
    decode::<Map>(
        b"\xa1\x65Named\xbf\x65value\x01\xff",
        Map::Named { value: 1 },
    );
    rejected::<Map>(b"\xa1\x65Named\xbf\x65value\x01\x65value\x02\xff");
    rejected::<TaggedNamed>(b"\xd8\x2b\xa1\x65Named\xa1\x65value\x01");
    rejected::<TaggedNamed>(b"\xa1\x65Named\xa1\x65value\x01");
    decode::<TaggedNamed>(
        b"\xd8\x2a\xa1\x65Named\xa1\x65value\x01",
        TaggedNamed::Named { value: 1 },
    );
}

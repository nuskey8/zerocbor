//! Byte strings on the wire, whichever way the field is spelled.
//!
//! There is no `Bytes` newtype: `&[u8]` and `Vec<u8>` are claimed by the blanket
//! `[T]` and `Vec<T>` impls, which would make them arrays, so a byte string is
//! asked for instead — `#[cbor(as_bytes = true)]` per field, or
//! `Write::write_binary` / `Read::read_binary` by hand. Both have to mean the
//! same thing, and `Value` has to agree.

use std::borrow::Cow;

use zerocbor::{FromCbor, ToCbor, Value};

#[test]
fn the_attribute_and_the_trait_method_agree() {
    #[derive(ToCbor, FromCbor)]
    struct WithAttribute<'a> {
        #[cbor(as_bytes = true)]
        data: &'a [u8],
    }

    struct WithTraitMethod<'a>(&'a [u8]);

    impl ToCbor for WithTraitMethod<'_> {
        fn write<W: zerocbor::Write>(&self, writer: &mut W) -> zerocbor::Result<()> {
            writer.write_binary(self.0)
        }
    }

    let payload: &[u8] = &[0x01, 0x02, 0x03];
    let via_attribute = zerocbor::to_cbor_vec(&WithAttribute { data: payload }).unwrap();
    let via_method = zerocbor::to_cbor_vec(&WithTraitMethod(payload)).unwrap();

    // The attribute wraps the field in the struct's one-element array and the
    // hand-written impl encodes the string on its own, so the comparison drops
    // that wrapper. The payload is `43 010203` either way.
    assert_eq!(via_attribute, vec![0x81, 0x43, 0x01, 0x02, 0x03]);
    assert_eq!(via_method, vec![0x43, 0x01, 0x02, 0x03]);
    assert_eq!(via_attribute[1..], via_method[..]);
}

#[test]
fn the_field_shape_decides_whether_the_read_can_borrow() {
    // The case a newtype would have made more convenient, and the reason
    // removing it costs little: `Cow` already carries both.
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Borrowing<'a> {
        #[cbor(as_bytes = true)]
        payload: Cow<'a, [u8]>,
    }

    /// `Vec<u8>` has no lifetime to borrow into, so this is the one shape that
    /// cannot be zero-copy. `Cow` is the shape to reach for when that matters.
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Owning {
        #[cbor(as_bytes = true)]
        payload: Vec<u8>,
    }

    let encoded = zerocbor::to_cbor_vec(&Borrowing {
        payload: Cow::Borrowed(&[0xAA, 0xBB][..]),
    })
    .unwrap();
    let decoded: Borrowing<'_> = zerocbor::from_cbor(&encoded).unwrap();
    let base = encoded.as_ptr() as usize;
    match decoded.payload {
        Cow::Borrowed(slice) => assert!(
            (base..base + encoded.len()).contains(&(slice.as_ptr() as usize)),
            "Cow<[u8]> should borrow out of the input",
        ),
        Cow::Owned(_) => panic!("expected a borrow, got an owned copy"),
    }

    let value = Owning {
        payload: vec![0x01, 0x02],
    };
    let encoded = zerocbor::to_cbor_vec(&value).unwrap();
    assert_eq!(zerocbor::from_cbor::<Owning>(&encoded).unwrap(), value);
}

#[test]
fn the_dynamic_type_holds_a_plain_cow() {
    let encoded = vec![0x43, 0x01, 0x02, 0x03];
    let value: Value = zerocbor::from_cbor(&encoded).unwrap();
    assert_eq!(value.as_bytes(), Some(&[0x01u8, 0x02, 0x03][..]));

    // And it re-encodes as a byte string, not an array.
    assert_eq!(zerocbor::to_cbor_vec(&value).unwrap(), encoded);
}

#[test]
fn an_array_is_rejected_where_a_byte_string_is_required() {
    // `Value` is dynamic, so it reads an array as an array. The rejection
    // belongs to a *typed* field that asked for a byte string.
    let value: Value = zerocbor::from_cbor(&[0x83, 0x01, 0x02, 0x03]).unwrap();
    assert!(matches!(value, Value::Array(_)), "got {value:?}");

    #[derive(Debug, FromCbor)]
    #[expect(dead_code, reason = "the field is read by the derive")]
    struct NeedsBytes<'a> {
        #[cbor(as_bytes = true)]
        data: &'a [u8],
    }

    let err = zerocbor::from_cbor::<NeedsBytes<'_>>(&[0x81, 0x83, 0x01]).unwrap_err();
    assert!(
        matches!(err, zerocbor::Error::InvalidInitialByte(0x83)),
        "got {err:?}",
    );
}

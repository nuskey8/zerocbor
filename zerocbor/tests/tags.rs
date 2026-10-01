//! Tags, which RFC 8949 Section 3.4 gives meaning to.
//!
//! A tag on the wire is a number in front of a value and the format forces
//! nothing with it. What this crate offers is a name for each number and a way
//! for a type to say which one it expects, so a value read from the wrong tag
//! is reported rather than reinterpreted.

use zerocbor::tags;
use zerocbor::{Error, FromCbor, ToCbor, from_cbor, to_cbor_vec};

// A bignum is `2(h'...')`: a tag directly in front of a byte string, which is
// only expressible if a newtype can carry a tag.
#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(tag = 2)]
struct BigUint(#[cbor(as_bytes)] Vec<u8>);

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(tag = 3)]
struct BigInt(#[cbor(as_bytes)] Vec<u8>);

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(tag = 1)]
struct Epoch(f64);

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map, tag = 55799)]
struct SelfDescribed {
    a: u64,
}

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
struct Untagged(u64);

mod newtype {
    use super::*;

    #[test]
    fn a_newtype_is_its_inner_value_with_no_container_around_it() {
        assert_eq!(to_cbor_vec(&Untagged(7)).unwrap(), [0x07]);
        assert_eq!(from_cbor::<Untagged>(&[0x07]).unwrap(), Untagged(7));
    }

    #[test]
    fn a_newtype_can_borrow_through_its_inner_field() {
        #[derive(Debug, PartialEq, FromCbor)]
        struct Slice<'a>(#[cbor(as_bytes)] &'a [u8]);

        let data = [0x42, 0x01, 0x02];
        let decoded: Slice<'_> = from_cbor(&data[..]).unwrap();
        assert_eq!(decoded.0, [1, 2]);
        // A borrow, not a copy: the slice points into the input.
        assert!(core::ptr::eq(decoded.0.as_ptr(), data[1..].as_ptr()));
    }

    #[test]
    fn a_newtype_in_a_map_still_keys_by_name() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        struct Wrapper {
            id: Untagged,
        }
        assert_eq!(
            to_cbor_vec(&Wrapper { id: Untagged(3) }).unwrap(),
            [0xa1, 0x62, b'i', b'd', 0x03]
        );
    }
}

mod bignum {
    use super::*;

    #[test]
    fn a_bignum_is_a_tag_in_front_of_a_byte_string() {
        // 2(h'0100000000000000')
        let value = BigUint(vec![1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            to_cbor_vec(&value).unwrap(),
            [0xc2, 0x48, 0x01, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            from_cbor::<BigUint>(&[0xc2, 0x48, 1, 0, 0, 0, 0, 0, 0, 0]).unwrap(),
            value
        );
    }

    #[test]
    fn a_bignum_can_exceed_sixty_four_bits() {
        // The reason tag 2 exists at all: 0xffff_ffff_ffff_ffff does not fit a
        // CBOR integer, so it travels as bytes under a tag.
        let value = BigUint(vec![0xff; 10]);
        let encoded = to_cbor_vec(&value).unwrap();
        assert_eq!(encoded[0], 0xc2);
        assert_eq!(from_cbor::<BigUint>(&encoded).unwrap(), value);
    }

    #[test]
    fn a_negative_bignum_is_a_different_tag() {
        // 3(h'01') — the same byte string under tag 3 denotes -2.
        let value = BigInt(vec![1]);
        assert_eq!(to_cbor_vec(&value).unwrap(), [0xc3, 0x41, 0x01]);
        assert_eq!(from_cbor::<BigInt>(&[0xc3, 0x41, 0x01]).unwrap(), value);
    }
}

mod strictness {
    use super::*;

    /// Every way a tag can be wrong. `55799` is self-described CBOR, the one a
    /// document is most likely to claim.
    #[test]
    fn a_tag_mismatch_is_reported() {
        for (label, declared, found, bytes) in [
            // 1(1.0) declared, 2(1.0) found.
            (
                "a different tag",
                1u64,
                Some(2u64),
                vec![0xc2, 0xfb, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0],
            ),
            // 55799({a: 1}) declared, tag 1 found.
            (
                "self-describe replaced",
                tags::SELF_DESCRIBED_CBOR,
                Some(1),
                vec![0xc1, 0xa1, 0x61, b'a', 0x01],
            ),
            // 55799({a: 1}) declared, untagged.
            (
                "self-describe absent",
                tags::SELF_DESCRIBED_CBOR,
                None,
                vec![0xa1, 0x61, b'a', 0x01],
            ),
        ] {
            let err = match declared {
                1 => from_cbor::<Epoch>(&bytes).unwrap_err(),
                _ => from_cbor::<SelfDescribed>(&bytes).unwrap_err(),
            };
            assert!(
                matches!(err, Error::TagMismatch { expected, found: got } if expected == declared && got == found),
                "{label}: got {err:?}"
            );
        }
    }

    #[test]
    fn a_tag_on_an_untagged_type_is_rejected() {
        // A type error, not a tag error, and the right one: a type declaring no
        // tag does not look for one, so the tag head reaches the field reader.
        let err = from_cbor::<Untagged>(&[0xc1, 0x01]).unwrap_err();
        assert!(
            matches!(err, Error::InvalidInitialByte(0xc1)),
            "got {err:?}"
        );
    }
}

mod variants {
    use super::*;

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    enum Event {
        Nothing,
        #[cbor(tag = 24)]
        Blob(Vec<u8>),
        Moved {
            x: i32,
        },
    }

    #[test]
    fn a_variant_may_carry_its_own_tag() {
        // 24({"Blob": [h'0102']}) — the tag wraps the envelope, and the name
        // inside it says which variant.
        let value = Event::Blob(vec![1, 2]);
        assert_eq!(
            to_cbor_vec(&value).unwrap(),
            [
                0xd8, 0x18, 0xa1, 0x64, b'B', b'l', b'o', b'b', 0x81, 0x42, 0x01, 0x02
            ]
        );
        assert_eq!(
            from_cbor::<Event>(&to_cbor_vec(&value).unwrap()).unwrap(),
            value
        );
    }

    #[test]
    fn an_untagged_variant_is_still_a_bare_name() {
        // {"Moved": {"x": 7}} with no tag in front of it.
        let encoded = to_cbor_vec(&Event::Moved { x: 7 }).unwrap();
        assert_eq!(encoded[0], 0xa1);
        assert_eq!(from_cbor::<Event>(&encoded).unwrap(), Event::Moved { x: 7 });
    }

    #[test]
    fn a_variant_that_disagrees_about_its_tag_is_reported() {
        // The envelope resolved to `Blob`, which declares tag 24; neither the
        // presence nor the absence matched.
        let err = from_cbor::<Event>(&[0xd8, 0x18, 0x67, b'N', b'o', b't', b'h', b'i', b'n', b'g'])
            .unwrap_err();
        assert!(
            matches!(err, Error::UnexpectedTag { found: 24 }),
            "an untagged variant under a tag: got {err:?}"
        );

        let err = from_cbor::<Event>(&[0xa1, 0x64, b'B', b'l', b'o', b'b', 0x81, 0x42, 0x01, 0x02])
            .unwrap_err();
        assert!(
            matches!(
                err,
                Error::TagMismatch {
                    expected: 24,
                    found: None
                }
            ),
            "a tagged variant arriving untagged: got {err:?}"
        );
    }
}

mod hand_written {
    use super::*;
    use zerocbor::Read;

    /// Declares a tag the same way the derive does, the only way the
    /// attribute's promise holds for a type the macro cannot see.
    struct Epoch2(f64);

    impl<'de> FromCbor<'de> for Epoch2 {
        fn read<R: Read<'de>>(reader: &mut R) -> Result<Self, Error> {
            reader.check_tag(tags::EPOCH_DATE_TIME)?;
            Ok(Epoch2(f64::read(reader)?))
        }
    }

    impl ToCbor for Epoch2 {
        fn write<W: zerocbor::Write>(&self, writer: &mut W) -> Result<(), Error> {
            writer.write_tag(tags::EPOCH_DATE_TIME)?;
            self.0.write(writer)
        }
    }

    #[test]
    fn the_helper_is_usable_without_the_derive() {
        let encoded = to_cbor_vec(&Epoch2(1.0)).unwrap();
        assert_eq!(encoded[0], 0xc1);
        assert_eq!(from_cbor::<Epoch2>(&encoded).unwrap().0, 1.0);
        assert!(from_cbor::<Epoch2>(&[0x01]).is_err());
    }
}

//! Tests for the `ToCbor` and `FromCbor` derives.
//!
//! These cover the shapes the derive has to get right: the two representations,
//! ignored and renamed fields, and each enum form. The `zerompk_parity` module
//! covers the attribute names and the strictness rules.

#![allow(clippy::redundant_field_names)]

use zerocbor::{Error, FromCbor, ToCbor};

mod array_repr {
    use super::*;

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[test]
    fn a_struct_is_an_array_in_field_order() {
        let encoded = zerocbor::to_cbor_vec(&Point { x: 1, y: 2 }).unwrap();
        assert_eq!(encoded, vec![0x82, 0x01, 0x02]);
        assert_eq!(
            zerocbor::from_cbor::<Point>(&encoded).unwrap(),
            Point { x: 1, y: 2 }
        );
    }

    #[test]
    fn a_wrong_length_is_rejected() {
        let err = zerocbor::from_cbor::<Point>(&[0x83, 0x01, 0x02, 0x03]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::ArrayLengthMismatch {
                    expected: 2,
                    actual: 3
                }
            ),
            "got {err:?}"
        );
    }

    #[test]
    fn a_nested_struct_round_trips() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        struct Nested {
            origin: Point,
            points: Vec<Point>,
        }

        let value = Nested {
            origin: Point { x: 0, y: 0 },
            points: vec![Point { x: 1, y: 2 }, Point { x: -3, y: -4 }],
        };
        let encoded = zerocbor::to_cbor_vec(&value).unwrap();
        assert_eq!(zerocbor::from_cbor::<Nested>(&encoded).unwrap(), value);
    }

    #[test]
    fn a_fieldless_enum_is_a_bare_index() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[repr(u8)]
        enum Color {
            Red,
            Green,
            Blue,
        }

        // One integer per variant, with no length head: the cheapest form.
        for (value, expected) in [
            (Color::Red, 0x00u8),
            (Color::Green, 0x01),
            (Color::Blue, 0x02),
        ] {
            let encoded = zerocbor::to_cbor_vec(&value).unwrap();
            assert_eq!(encoded, vec![expected]);
            assert_eq!(zerocbor::from_cbor::<Color>(&encoded).unwrap(), value);
        }

        // An index the type does not name is reported rather than clamped.
        let err = zerocbor::from_cbor::<Color>(&[0x07]).unwrap_err();
        assert!(matches!(err, Error::UnknownVariantIndex(7)), "got {err:?}");
    }

    #[test]
    fn a_data_carrying_variant_carries_its_index_first() {
        // Whether the fields are named or positional makes no difference to the
        // encoding: each variant is an array whose element 0 is the index.
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        enum Shape {
            Circle(u32),
            Rect(u32, u32),
            Moved { dx: i32, dy: i32 },
        }

        for (value, expected) in [
            (Shape::Circle(5), vec![0x82, 0x00, 0x05]),
            (Shape::Rect(1, 2), vec![0x83, 0x01, 0x01, 0x02]),
            (Shape::Moved { dx: 1, dy: -1 }, vec![0x83, 0x02, 0x01, 0x20]),
        ] {
            let encoded = zerocbor::to_cbor_vec(&value).unwrap();
            assert_eq!(encoded, expected, "{value:?}");
            assert_eq!(zerocbor::from_cbor::<Shape>(&encoded).unwrap(), value);
        }
    }
}

mod map_repr {
    use super::*;

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[test]
    fn a_struct_is_a_map_keyed_by_field_name() {
        let encoded = zerocbor::to_cbor_vec(&Point { x: 1, y: 2 }).unwrap();
        assert_eq!(encoded, vec![0xa2, 0x61, b'x', 0x01, 0x61, b'y', 0x02]);
        assert_eq!(
            zerocbor::from_cbor::<Point>(&encoded).unwrap(),
            Point { x: 1, y: 2 }
        );
    }

    /// A three-entry map, so an unknown-key path can be exercised.
    fn wider_than_point() -> Vec<u8> {
        let mut wider = vec![0xa3];
        // x = 1
        wider.extend_from_slice(&[0x61, b'x', 0x01]);
        // y = 2
        wider.extend_from_slice(&[0x61, b'y', 0x02]);
        // z = "abc", an entry this type does not name
        wider.extend_from_slice(&[0x61, b'z', 0x63, b'a', b'b', b'c']);
        wider
    }

    /// Strict by default, so schema evolution is an explicit decision.
    #[test]
    fn a_wrong_map_is_reported() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map, allow_unknown_fields)]
        struct Tolerant {
            x: i32,
            y: i32,
        }

        // The count is checked before any key, so the keys never matter here.
        for (label, encoded, actual) in [
            ("one key too many", wider_than_point(), 3),
            ("one key too few", vec![0xa1, 0x61, b'x', 0x01], 1),
        ] {
            let err = zerocbor::from_cbor::<Point>(&encoded).unwrap_err();
            assert!(
                matches!(
                    err,
                    Error::MapLengthMismatch {
                        expected: 2,
                        actual: n
                    } if n == actual
                ),
                "{label}: got {err:?}"
            );
        }

        // With the count matching, the unrecognized key is named. Strict, so
        // nothing is skipped: `Renamed` is spelled so "z" is outside it.
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        struct Renamed {
            #[cbor(key = "a")]
            x: i32,
            #[cbor(key = "b")]
            y: i32,
        }
        let err = zerocbor::from_cbor::<Renamed>(&[0xa2, 0x61, b'a', 0x01, 0x61, b'z', 0x02])
            .unwrap_err();
        assert!(matches!(err, Error::UnknownVariant(_)), "got {err:?}");

        // Twice is a contradiction, not last-one-wins.
        let mut encoded = zerocbor::to_cbor_vec(&Tolerant { x: 1, y: 2 }).unwrap();
        encoded[0] = 0xa3;
        encoded.extend_from_slice(&[0x61, b'x', 0x09]);
        let err = zerocbor::from_cbor::<Tolerant>(&encoded).unwrap_err();
        assert!(matches!(err, Error::KeyDuplicated(_)), "got {err:?}");
    }

    #[test]
    fn allow_unknown_fields_opts_into_skipping_them() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map, allow_unknown_fields)]
        struct Tolerant {
            x: i32,
            y: i32,
        }

        // A newer writer's field does not break it, because it said so.
        assert_eq!(
            zerocbor::from_cbor::<Tolerant>(&wider_than_point()).unwrap(),
            Tolerant { x: 1, y: 2 },
        );
    }

    #[test]
    fn a_missing_key_is_reported() {
        // The length check fires before any key is read.
        let err = zerocbor::from_cbor::<Point>(&[0xa1, 0x61, b'x', 0x01]).unwrap_err();
        assert!(
            matches!(
                err,
                Error::MapLengthMismatch {
                    expected: 2,
                    actual: 1
                }
            ),
            "got {err:?}",
        );
    }

    #[test]
    fn default_fills_a_missing_key() {
        fn default_y() -> i32 {
            42
        }

        // The length is read, not checked, so `y` is filled rather than failing.
        // A bare `default` takes the type's own zero; a named one calls a function.
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        struct Zero {
            x: i32,
            #[cbor(default)]
            y: i32,
        }
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        struct Computed {
            x: i32,
            #[cbor(default = "default_y")]
            y: i32,
        }

        let encoded = [0xa1, 0x61, b'x', 0x07];
        assert_eq!(
            zerocbor::from_cbor::<Zero>(&encoded).unwrap(),
            Zero { x: 7, y: 0 },
        );
        assert_eq!(
            zerocbor::from_cbor::<Computed>(&encoded).unwrap(),
            Computed { x: 7, y: 42 },
        );
    }

    #[test]
    fn a_field_can_be_ignored() {
        #[derive(Debug, Default, PartialEq, ToCbor, FromCbor)]
        struct Skipped {
            kept: u8,
            #[cbor(ignore)]
            dropped: u8,
        }

        let value = Skipped {
            kept: 1,
            dropped: 2,
        };
        let encoded = zerocbor::to_cbor_vec(&value).unwrap();
        assert_eq!(encoded, vec![0x81, 0x01], "the ignored field is absent");
        assert_eq!(
            zerocbor::from_cbor::<Skipped>(&encoded).unwrap(),
            Skipped {
                kept: 1,
                dropped: 0,
            },
            "an ignored field falls back to Default"
        );
    }

    #[test]
    fn a_fieldless_enum_is_named() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        enum Color {
            Red,
            Green,
        }

        assert_eq!(
            zerocbor::to_cbor_vec(&Color::Green).unwrap(),
            vec![0x65, b'G', b'r', b'e', b'e', b'n'],
        );
        for value in [Color::Red, Color::Green] {
            let encoded = zerocbor::to_cbor_vec(&value).unwrap();
            assert_eq!(zerocbor::from_cbor::<Color>(&encoded).unwrap(), value);
        }
    }

    #[test]
    fn a_data_carrying_variant_is_wrapped_in_a_one_entry_map() {
        // The name keys, the fields are its value: tellable apart from a
        // fieldless variant's bare name.
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        enum Event {
            Nothing,
            Moved { x: i32, y: i32 },
            Renamed(String),
        }

        assert_eq!(
            zerocbor::to_cbor_vec(&Event::Moved { x: 1, y: 2 }).unwrap(),
            vec![
                0xa1, 0x65, b'M', b'o', b'v', b'e', b'd', 0xa2, 0x61, b'x', 0x01, 0x61, b'y', 0x02
            ],
        );
        // A tuple variant has no names, so an array under the name.
        assert_eq!(
            zerocbor::to_cbor_vec(&Event::Renamed("hi".into())).unwrap(),
            vec![
                0xa1, 0x67, b'R', b'e', b'n', b'a', b'm', b'e', b'd', 0x81, 0x62, b'h', b'i'
            ],
        );
        for value in [
            Event::Nothing,
            Event::Moved { x: 1, y: 2 },
            Event::Renamed("hi".into()),
        ] {
            let encoded = zerocbor::to_cbor_vec(&value).unwrap();
            assert_eq!(
                zerocbor::from_cbor::<Event>(&encoded).unwrap(),
                value,
                "round trip of {value:?}"
            );
        }
    }

    #[test]
    fn a_variant_name_that_does_not_resolve_is_reported() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        enum Event {
            Nothing,
            Moved { x: i32 },
        }

        for (label, encoded, expected) in [
            // A name the enum does not have.
            ("no such variant", vec![0x63, b'B', b'l', b'u'], "Blu"),
            // The name arrived bare, so there is nowhere for the fields to be.
            // The error names the variant rather than reporting a missing byte.
            (
                "a bare name with fields",
                vec![0x65, b'M', b'o', b'v', b'e', b'd'],
                "Moved",
            ),
        ] {
            let err = zerocbor::from_cbor::<Event>(&encoded).unwrap_err();
            assert!(
                matches!(err, Error::UnknownVariant(ref n) if n == expected),
                "{label}: got {err:?}"
            );
        }
    }
}

mod generics {
    use super::*;

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Wrapper<T> {
        value: T,
        count: u32,
    }

    #[test]
    fn a_generic_struct_round_trips() {
        let value = Wrapper {
            value: "hello".to_string(),
            count: 3,
        };
        let encoded = zerocbor::to_cbor_vec(&value).unwrap();
        assert_eq!(
            zerocbor::from_cbor::<Wrapper<String>>(&encoded).unwrap(),
            value
        );
    }

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Borrowed<'a> {
        text: &'a str,
        number: u32,
    }

    #[test]
    fn a_borrowing_struct_reads_without_copying() {
        let encoded = zerocbor::to_cbor_vec(&Borrowed {
            text: "borrowed",
            number: 5,
        })
        .unwrap();

        let decoded: Borrowed<'_> = zerocbor::from_cbor(&encoded).unwrap();
        assert_eq!(decoded.text, "borrowed");
        assert_eq!(decoded.number, 5);

        // The decoded string shares the input buffer rather than allocating.
        let base = encoded.as_ptr() as usize;
        let text = decoded.text.as_ptr() as usize;
        assert!(
            (base..base + encoded.len()).contains(&text),
            "the decoded str should point into the input slice",
        );
    }
}

mod option_fields {
    use super::*;

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Maybe {
        required: u32,
        optional: Option<u32>,
    }

    #[test]
    fn an_option_round_trips_in_both_states() {
        for value in [
            Maybe {
                required: 1,
                optional: None,
            },
            Maybe {
                required: 1,
                optional: Some(2),
            },
        ] {
            let encoded = zerocbor::to_cbor_vec(&value).unwrap();
            assert_eq!(zerocbor::from_cbor::<Maybe>(&encoded).unwrap(), value);
        }
    }
}

mod zerompk_parity {
    //! The attributes and their names follow the sibling crate, so code written
    //! for one reads the same in the other.
    use super::*;

    #[test]
    fn a_numeric_key_places_a_field_in_an_array() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        struct Sparse {
            #[cbor(key = 0)]
            a: u8,
            // `key = 2` leaves position 1 to be filled with `null`, so the
            // positions on the wire are the ones asked for.
            #[cbor(key = 2)]
            b: u8,
        }

        let encoded = zerocbor::to_cbor_vec(&Sparse { a: 1, b: 2 }).unwrap();
        assert_eq!(encoded, vec![0x83, 0x01, 0xf6, 0x02]);

        let decoded: Sparse = zerocbor::from_cbor(&encoded).unwrap();
        assert_eq!(decoded, Sparse { a: 1, b: 2 });
    }

    #[test]
    fn a_map_key_is_the_given_name() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(map)]
        struct Renamed {
            #[cbor(key = "the-x")]
            x: i32,
            y: i32,
        }

        // The keys are written in the order RFC 8949 Section 4.2.1 gives a
        // map's, which is length first, so the one-byte "y" leads even though
        // the field is declared second.
        let encoded = zerocbor::to_cbor_vec(&Renamed { x: 1, y: 2 }).unwrap();
        assert_eq!(
            encoded,
            vec![
                0xa2, 0x61, b'y', 0x02, 0x65, b't', b'h', b'e', b'-', b'x', 0x01
            ],
        );
        assert_eq!(
            zerocbor::from_cbor::<Renamed>(&encoded).unwrap(),
            Renamed { x: 1, y: 2 },
        );
    }

    #[test]
    fn c_enum_is_accepted_on_a_fieldless_enum() {
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        #[cbor(c_enum)]
        #[repr(u8)]
        enum Status {
            Ok = 0,
            NotFound = 1,
        }

        // A bare discriminant, which is what the attribute states.
        assert_eq!(
            zerocbor::to_cbor_vec(&Status::NotFound).unwrap(),
            vec![0x01]
        );
        for value in [Status::Ok, Status::NotFound] {
            let encoded = zerocbor::to_cbor_vec(&value).unwrap();
            assert_eq!(zerocbor::from_cbor::<Status>(&encoded).unwrap(), value);
        }
    }

    #[test]
    fn as_bytes_selects_a_byte_string_or_an_array_of_integers() {
        // The default follows the sibling crate: a byte-slice-shaped field is a
        // byte string unless it asks otherwise.
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        struct AsBlob {
            data: Vec<u8>,
        }
        #[derive(Debug, PartialEq, ToCbor, FromCbor)]
        struct AsArray {
            #[cbor(as_bytes = false)]
            data: Vec<u8>,
        }

        assert_eq!(
            zerocbor::to_cbor_vec(&AsBlob { data: vec![1, 2] }).unwrap(),
            vec![0x81, 0x42, 0x01, 0x02],
        );
        let encoded = zerocbor::to_cbor_vec(&AsArray { data: vec![1, 2] }).unwrap();
        assert_eq!(encoded, vec![0x81, 0x82, 0x01, 0x02]);
        assert_eq!(
            zerocbor::from_cbor::<AsArray>(&encoded).unwrap(),
            AsArray { data: vec![1, 2] },
        );
    }
}

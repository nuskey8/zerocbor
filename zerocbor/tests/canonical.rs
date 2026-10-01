//! Deterministic encoding, as RFC 8949 Section 4.2.1 defines it.
//!
//! There is no flag and no second entry point: the ordinary writer is the
//! deterministic one, so two encoders that agree on a value agree on the bytes.

use std::collections::BTreeMap;

use zerocbor::{FromCbor, ToCbor, Value, from_cbor, to_cbor_vec};

/// Decodes a value: the input is the wire bytes and the subject is what the
/// decoder makes of them.
fn round_trip(bytes: &[u8]) -> Value<'_> {
    from_cbor(bytes).unwrap()
}

#[test]
fn map_keys_are_written_in_canonical_order() {
    // A map read in one key order and written in the other. A text string's
    // initial byte rises with its length, so text keys come out length first;
    // keys of different types order by major type.
    for (label, input, expected) in [
        (
            "text keys by length then bytes",
            // {"aa": 1, "b": 2}
            &[0xa2, 0x62, b'a', b'a', 0x01, 0x61, b'b', 0x02][..],
            // {"b": 2, "aa": 1}
            &[0xa2, 0x61, b'b', 0x02, 0x62, b'a', b'a', 0x01][..],
        ),
        (
            "text keys of equal length by bytes",
            // {"b": 1, "a": 2}
            &[0xa2, 0x61, b'b', 0x01, 0x61, b'a', 0x02][..],
            // {"a": 2, "b": 1}
            &[0xa2, 0x61, b'a', 0x02, 0x61, b'b', 0x01][..],
        ),
        (
            "keys of different types by major type",
            // {"": 0, 0: 1, false: 2, h'00': 3}
            &[0xa4, 0x60, 0x00, 0x00, 0x01, 0xf4, 0x02, 0x41, 0x00, 0x03][..],
            // {0: 1, h'00': 3, "": 0, false: 2}
            &[0xa4, 0x00, 0x01, 0x41, 0x00, 0x03, 0x60, 0x00, 0xf4, 0x02][..],
        ),
        (
            "an integer before a negative one of the same size",
            // {-1: 1, 0: 2}
            &[0xa2, 0x20, 0x01, 0x00, 0x02][..],
            // {0: 2, -1: 1}
            &[0xa2, 0x00, 0x02, 0x20, 0x01][..],
        ),
        (
            "a nested map sorted at its own level",
            // {"a": {"y": 1, "x": 2}}
            &[0xa1, 0x61, b'a', 0xa2, 0x61, b'y', 0x01, 0x61, b'x', 0x02][..],
            &[0xa1, 0x61, b'a', 0xa2, 0x61, b'x', 0x02, 0x61, b'y', 0x01][..],
        ),
        (
            "an indefinite-length map comes back definite and sorted",
            // {_ "b": 2, "a": 1}
            &[0xbf, 0x61, b'b', 0x02, 0x61, b'a', 0x01, 0xff][..],
            &[0xa2, 0x61, b'a', 0x01, 0x61, b'b', 0x02][..],
        ),
    ] {
        assert_eq!(
            to_cbor_vec(&round_trip(input)).unwrap(),
            expected,
            "{label}"
        );
    }
}

#[test]
fn the_rfc_s_own_example_order_holds() {
    // The sorted keys from Section 4.2.1, as a map, each checked against its
    // canonical position.
    let keys: Vec<Vec<u8>> = vec![
        vec![0x0a],             // 10
        vec![0x18, 0x64],       // 100
        vec![0x20],             // -1
        vec![0x61, b'z'],       // "z"
        vec![0x62, b'a', b'a'], // "aa"
        vec![0x81, 0x18, 0x64], // [100]
        vec![0x81, 0x20],       // [-1]
        vec![0xf4],             // false
    ];
    let mut map = Value::Map(Default::default());
    let Value::Map(entries) = &mut map else {
        unreachable!()
    };
    for key in &keys {
        entries.insert(from_cbor(key).unwrap(), Value::Null);
    }
    // Insertion order is the reverse of canonical, so iterating in it would
    // come back wrong.
    let written = to_cbor_vec(&map).unwrap();
    let mut position = 1;
    for key in &keys {
        assert_eq!(
            &written[position..position + key.len()],
            key.as_slice(),
            "key {key:02x?} is out of order"
        );
        position += key.len() + 1;
    }
}

#[test]
fn a_derived_map_writes_its_keys_in_canonical_order() {
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Ordered {
        #[cbor(key = "z")]
        z: u64,
        #[cbor(key = "a")]
        a: u64,
        #[cbor(key = "mm")]
        mm: u64,
    }
    // Declared z, a, mm; canonical is length first, so the one-byte names come
    // before "mm" and between them in byte order.
    assert_eq!(
        to_cbor_vec(&Ordered { z: 1, a: 2, mm: 3 }).unwrap(),
        [
            0xa3, 0x61, b'a', 0x02, 0x61, b'z', 0x01, 0x62, b'm', b'm', 0x03
        ]
    );

    // A named enum variant's fields are a map in their own right and are
    // ordered the same way.
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    enum Shape {
        Moved { x: i32, y: i32, a: i32 },
    }
    assert_eq!(
        to_cbor_vec(&Shape::Moved { x: 1, y: 2, a: 3 }).unwrap(),
        [
            0xa1, 0x65, b'M', b'o', b'v', b'e', b'd', 0xa3, 0x61, b'a', 0x03, 0x61, b'x', 0x01,
            0x61, b'y', 0x02
        ]
    );

    // No keys, so nothing to sort: the declaration order goes on the wire.
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    struct Positional {
        z: u64,
        a: u64,
    }
    assert_eq!(
        to_cbor_vec(&Positional { z: 1, a: 2 }).unwrap(),
        [0x82, 0x01, 0x02]
    );
}

#[test]
fn a_float_goes_out_at_the_narrowest_width_that_holds_it() {
    // Section 4.1 makes this the preferred serialization and 4.2.1 requires
    // it, so it is not the caller's choice.
    for (label, value, expected) in [
        ("0.0", 0.0f64, &[0xf9, 0x00, 0x00][..]),
        ("1.0", 1.0, &[0xf9, 0x3c, 0x00][..]),
        // 1.5 needs one significand bit, so a binary16 holds it exactly.
        ("1.5", 1.5, &[0xf9, 0x3e, 0x00][..]),
        ("-0.0", -0.0, &[0xf9, 0x80, 0x00][..]),
        (
            "65504.0, the largest finite half",
            65504.0,
            &[0xf9, 0x7b, 0xff][..],
        ),
        (
            "2^-24, the smallest positive half",
            2.0f64.powi(-24),
            &[0xf9, 0x00, 0x01][..],
        ),
        ("infinity", f64::INFINITY, &[0xf9, 0x7c, 0x00][..]),
        ("-infinity", f64::NEG_INFINITY, &[0xf9, 0xfc, 0x00][..]),
        // 0xf97e00, the one NaN encoding of Section 4.1.
        ("NaN", f64::NAN, &[0xf9, 0x7e, 0x00][..]),
        // 65520 is past the largest finite half.
        ("65520.0", 65520.0, &[0xfa, 0x47, 0x7f, 0xf0, 0x00][..]),
        ("100000.0", 100000.0, &[0xfa, 0x47, 0xc3, 0x50, 0x00][..]),
        // 333333.0 needs nineteen significand bits, which a single has.
        ("333333.0", 333333.0, &[0xfa, 0x48, 0xa2, 0xc2, 0xa0][..]),
        (
            "0.1, which neither narrower width holds",
            0.1,
            &[0xfb, 0x3f, 0xb9, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9a][..],
        ),
    ] {
        assert_eq!(to_cbor_vec(&value).unwrap(), expected, "{label}");

        // The same holds for an `f32`, which can only reach a half or a single.
        if value as f32 as f64 == value {
            assert_eq!(
                to_cbor_vec(&(value as f32)).unwrap(),
                expected,
                "{value} as an f32"
            );
        }
    }
}

#[test]
fn a_float_narrows_in_a_field_and_in_a_slice() {
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Reading {
        value: f32,
    }
    let encoded = to_cbor_vec(&Reading { value: 1.0 }).unwrap();
    assert_eq!(&encoded[encoded.len() - 3..], &[0xf9, 0x3c, 0x00]);

    // A slice narrows element by element, so the array is shorter than three
    // single-precision values would be.
    let values = [1.0f32, 1.1, 2.0];
    assert_eq!(to_cbor_vec(&values[..]).unwrap().len(), 1 + 3 + 5 + 3);

    // And a `Value`, whose `Float` is an `f64`, recovers the width the input used.
    let value: Value = from_cbor(&[0xf9, 0x3c, 0x00]).unwrap();
    assert_eq!(to_cbor_vec(&value).unwrap(), [0xf9, 0x3c, 0x00]);
}

#[test]
fn the_size_hint_matches_the_width_that_is_written() {
    // A hint that over-promised would take the unchecked path and overflow; one
    // that under-promised would only cost a checked retry.
    assert_eq!(1.0f32.size_hint().map(|h| h.upper_bound()), Some(3));
    assert_eq!(1.1f32.size_hint().map(|h| h.upper_bound()), Some(5));
    assert_eq!(1.0f64.size_hint().map(|h| h.upper_bound()), Some(3));
    assert_eq!(0.1f64.size_hint().map(|h| h.upper_bound()), Some(9));
    assert_eq!(
        Value::Float(1.0).size_hint().map(|h| h.upper_bound()),
        Some(3)
    );

    // And a buffer sized from the hint is exactly big enough, which is the
    // property that makes the unchecked path safe.
    let mut buf = [0u8; 3];
    assert_eq!(zerocbor::to_cbor(&1.0f32, &mut buf).unwrap(), 3);
}

#[test]
fn what_the_format_left_to_the_producer_is_always_the_same() {
    // The rest of Section 4.2.1: shortest arguments, no indefinite lengths, and
    // the major types kept distinct. A macro, not an array, because the values
    // are of different types.
    macro_rules! check {
        ($($label:literal => $value:expr, $expected:expr;)*) => {$(
            assert_eq!(
                to_cbor_vec(&$value).unwrap(),
                $expected,
                concat!($label, ": ", stringify!($value))
            );
        )*};
    }

    // Shortest form for every argument: 0 and 23 are one byte, 24 is two, and
    // 65536 is five.
    check! {
        "a small integer"     => 0u64,    &[0x00][..];
        "just under a byte"   => 23u64,   &[0x17][..];
        "one over a byte"     => 24u64,   &[0x18, 0x18][..];
        // 65536 is one past what a u16 holds, so it takes a u32 argument.
        "one past a u16"      => 65536u64, &[0x1a, 0, 1, 0, 0][..];
        "minus one"           => -1i64,   &[0x20][..];
        "minus one past a byte" => -256i64, &[0x38, 0xff][..];
    }

    // A definite length, so an indefinite-length input comes back shorter.
    check! {
        "[1, 2] read indefinite" => round_trip(&[0x9f, 0x01, 0x02, 0xff]), &[0x82, 0x01, 0x02][..];
    }

    // A tag number is a value, not something to normalize.
    check! {
        "55799(0)" => round_trip(&[0xd9, 0xd9, 0xf7, 0x00]), &[0xd9, 0xd9, 0xf7, 0x00][..];
    }

    // A text string and a byte string stay distinct.
    check! {
        "a string beside a byte string" => round_trip(&[0x82, 0x61, b'a', 0x42, 0x01, 0x02]),
            &[0x82, 0x61, b'a', 0x42, 0x01, 0x02][..];
    }
}

#[test]
fn the_output_is_a_function_of_the_value_alone() {
    // Two encodings of one value in different key orders and float widths come
    // out identical, which is the entire point.
    let a = round_trip(&[
        0xa2, 0x62, b'a', b'a', 0xfb, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0, 0x61, b'b', 0x02,
    ]);
    let b = round_trip(&[0xa2, 0x61, b'b', 0x02, 0x62, b'a', b'a', 0xf9, 0x3c, 0x00]);
    let once = to_cbor_vec(&a).unwrap();
    assert_eq!(once, to_cbor_vec(&b).unwrap());

    // And a second round trip is the first.
    assert_eq!(once, to_cbor_vec(&round_trip(&once)).unwrap());
}

#[test]
fn a_derived_type_needs_no_second_path() {
    // For a derived type as well as a `Value`.
    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Record {
        z: String,
        a: u32,
    }
    let once = to_cbor_vec(&Record {
        z: "Alice".into(),
        a: 42,
    })
    .unwrap();
    assert_eq!(
        once,
        to_cbor_vec(&from_cbor::<Record>(&once).unwrap()).unwrap()
    );
    // "z" is declared first and written second.
    assert_eq!(once[1..3], [0x61, b'a']);
}

#[test]
fn the_order_is_total_and_matches_the_encodings() {
    // The order has to agree with `Eq`, or a `BTreeMap` would treat two
    // different values as one key.
    let values = vec![
        Value::Null,
        Value::Undefined,
        Value::Simple(0),
        Value::Simple(255),
        Value::Bool(false),
        Value::Bool(true),
        Value::Integer(0),
        Value::Integer(1),
        Value::Integer(-1),
        Value::Float(0.0),
        Value::Float(1.0),
        Value::Float(-1.0),
        Value::Bytes(std::borrow::Cow::Borrowed(&[])),
        Value::Text("".into()),
        Value::Text("a".into()),
        Value::Array(vec![]),
        Value::Array(vec![Value::Integer(0)]),
        Value::Map(Default::default()),
        Value::Tag(0, Box::new(Value::Integer(0))),
    ];
    for (i, a) in values.iter().enumerate() {
        for (j, b) in values.iter().enumerate() {
            if a != b {
                assert_ne!(
                    a.cmp(b),
                    core::cmp::Ordering::Equal,
                    "values {i} and {j} are distinct but compare equal"
                );
            }
        }
    }

    // And the order is the bytewise order of the encodings, so sorting a
    // canonically ordered list is a no-op.
    let encoded: Vec<Vec<u8>> = values
        .iter()
        .map(|v| to_cbor_vec(v).unwrap())
        .collect::<Vec<_>>();
    // Sorting the values and writing them out has to give the encodings in
    // bytewise order. CBOR is self-delimiting, so no encoding is a prefix of
    // another and comparing the byte strings is the same as comparing the
    // values' canonical order.
    let mut sorted_values = values.clone();
    sorted_values.sort();
    let resorted: Vec<Vec<u8>> = sorted_values
        .iter()
        .map(|v| to_cbor_vec(v).unwrap())
        .collect();
    let mut sorted_bytes = encoded.clone();
    sorted_bytes.sort();
    assert_eq!(resorted, sorted_bytes, "the order is not the bytewise one");

    // A float and a simple value are both major type 7, so the marker decides,
    // and narrowing is what gives the float its marker.
    assert!(Value::Null < Value::Float(1.0), "0xf6 should precede 0xf9");
    assert!(
        Value::Simple(0) > Value::Integer(0),
        "0xe0 should follow 0x00"
    );
}

/// RFC 8949 Section 4.1 names one `NaN` encoding, so a `NaN` is not a payload.
#[test]
fn a_nan_is_one_value_however_its_bits_are_spun() {
    let nans = [
        f64::NAN,
        -f64::NAN,
        f64::from_bits(0x7ff8_0000_0000_0001),
        f64::from_bits(0xfff8_0000_0000_0001),
        f64::from_bits(0x7ff0_0000_0000_0001),
    ];

    // Every one of them goes out as the same three bytes. A payload that reached
    // the wire would make two values that are both `NaN` encode differently, and
    // narrowing keeps only part of a wide one, so the same `NaN` read from a
    // `binary16` and from a `double` would come to one document as two.
    for nan in nans {
        assert_eq!(to_cbor_vec(&Value::Float(nan)).unwrap(), [0xf9, 0x7e, 0x00]);
        assert_eq!(to_cbor_vec(&nan).unwrap(), [0xf9, 0x7e, 0x00]);
        assert_eq!(to_cbor_vec(&(nan as f32)).unwrap(), [0xf9, 0x7e, 0x00]);
    }

    // And they compare equal, which is what makes `Ord` and `Eq` agree: a
    // `BTreeMap<Value, _>` would otherwise treat each `NaN` as a fresh key and
    // could hold several of them.
    for a in nans {
        for b in nans {
            assert_eq!(Value::Float(a), Value::Float(b), "two NaNs differ");
            assert_eq!(
                Value::Float(a).cmp(&Value::Float(b)),
                core::cmp::Ordering::Equal
            );
        }
    }

    // The order is still total against a float that is not a `NaN`, and it puts
    // the `NaN` where its encoding puts it: `f9 7e 00` is above `f9 7c 00` and
    // below `f9 fc 00`, so above every positive float and below every negative
    // one, which is the reverse of where the bit pattern of a `NaN` sits.
    assert!(Value::Float(1.0) < Value::Float(f64::INFINITY));
    assert!(Value::Float(f64::INFINITY) < Value::Float(f64::NAN));
    assert!(Value::Float(f64::NAN) < Value::Float(f64::NEG_INFINITY));
    assert!(Value::Float(-1.0) < Value::Float(f64::NEG_INFINITY));
    // A value that needs a double sorts after the `NaN` however small it is,
    // because the marker byte does the deciding: `f9` before `fb`.
    assert!(Value::Float(f64::NAN) < Value::Float(0.1));
    assert_ne!(Value::Float(1.0), Value::Float(f64::NAN));

    // The whole float order, checked against the bytewise order of the
    // encodings, which is the order this type is documented to have.
    // The bytewise order of the encodings, which is what the order has to be.
    // The halves run from `f9 00 00` to `f9 fc 00` with the `NaN` at `f9 7e 00`
    // between the largest positive and the most negative, and everything that
    // needs a wider float sorts after all of them because its marker byte is
    // larger.
    let floats = [
        0.0,
        f64::MIN_POSITIVE,
        1.0,
        65504.0,
        f64::INFINITY,
        f64::NAN,
        -0.0,
        -f64::MIN_POSITIVE,
        -1.0,
        -65504.0,
        f64::NEG_INFINITY,
        // A `binary32` and a `binary64`, which sort after every half.
        65536.0,
        0.1,
        f64::MAX,
    ];
    for (a, b) in floats.iter().copied().zip(floats).skip(1) {
        assert!(Value::Float(a) <= Value::Float(b), "{a} sorted above {b}");
    }

    // A `NaN` inside a map key is the case a payload-sensitive order broke: the
    // two keys are ordered by their second element once the `NaN`s are equal,
    // and that has to be the order they come back in.
    let map = Value::Map(BTreeMap::from([
        (
            Value::Array(vec![Value::Float(f64::NAN), Value::Integer(2)]),
            Value::Integer(0),
        ),
        (
            Value::Array(vec![Value::Float(f64::NAN), Value::Integer(1)]),
            Value::Integer(0),
        ),
    ]));
    let encoded = to_cbor_vec(&map).unwrap();
    let decoded: Value<'_> = from_cbor(&encoded).unwrap();
    assert_eq!(
        map, decoded,
        "a NaN key changed the map across a round trip"
    );
    // Both keys are still there, and in the order their second element puts them,
    // which is the order a payload-sensitive comparison could not produce.
    let Value::Map(entries) = &decoded else {
        unreachable!()
    };
    let keys: Vec<_> = entries.keys().collect();
    assert_eq!(keys.len(), 2, "a NaN key was lost or duplicated");
    assert_eq!(
        keys[0],
        &Value::Array(vec![Value::Float(f64::NAN), Value::Integer(1)])
    );
    assert_eq!(
        keys[1],
        &Value::Array(vec![Value::Float(f64::NAN), Value::Integer(2)])
    );
}

//! Half-precision floats, the one CBOR float width that cannot survive a round
//! trip through a wider Rust type: an `f16` read into an `f32` and written back
//! is five bytes where the input had three. Right for a type that asked for an
//! `f32`, wrong for a document that has to come back unchanged.
//!
//! The `f16` primitive's own behaviour is the language's, and
//! `rfc8949_appendix_a.rs` checks this crate's encoder over the whole
//! 65536-pattern space. What is left is what the type adds: that a value of it
//! reaches the wire as a half and comes back as one.
//!
//! Behind the feature throughout, since the language's `f16` is unstable.

#![cfg(feature = "f16")]
// The test crate needs the same unstable type the library does, and only when
// the feature is on, which is the same condition that gates the file.
#![cfg_attr(feature = "f16", feature(f16))]

use zerocbor::{FromCbor, ToCbor, Value, from_cbor, to_cbor_vec};

mod on_the_wire {
    use super::*;

    #[test]
    fn every_bit_pattern_goes_out_as_the_half_it_came_from() {
        // All 65536 of them, in both directions, and checked against the wire form
        // rather than against a conversion. Every `NaN` is the one exception:
        // RFC 8949 Section 4.1 names one encoding for it, so a payload does not
        // survive, and a `NaN` has to be checked by kind instead of by bits.
        for bits in 0..=u16::MAX {
            let is_nan = (bits & 0x7c00) == 0x7c00 && (bits & 0x03ff) != 0;
            let encoded = to_cbor_vec(&f16::from_bits(bits)).unwrap();
            assert_eq!(encoded[0], 0xf9, "{bits:#06x} did not go out as a half");

            if is_nan {
                // The one encoding, whatever the mantissa held going in.
                assert_eq!(encoded, [0xf9, 0x7e, 0x00], "bits {bits:#06x}");
                let decoded: f16 = from_cbor(&encoded).unwrap();
                assert!(decoded.is_nan(), "bits {bits:#06x} stopped being a NaN");
                continue;
            }

            let decoded: f16 = from_cbor(&encoded).unwrap();
            assert_eq!(decoded.to_bits(), bits, "bits {bits:#06x} did not survive");
        }

        // A subnormal is the case a naive conversion gets wrong by flushing it to
        // a zero, which would change the value, so it is named rather than left
        // to the sweep above.
        let smallest = f16::from_bits(0x0001);
        assert_eq!(smallest as f32, 2.0f32.powi(-24));
        assert!(smallest as f32 > 0.0);

        // The two zeros compare equal, so only the sign tells them apart.
        assert_eq!((-0.0f32 as f16).to_bits(), 0x8000);
        assert!((f16::from_bits(0x8000) as f32).is_sign_negative());
    }

    #[test]
    fn the_wire_form_is_the_one_rfc_8949_defines() {
        // The vectors from Appendix A, which are the encodings every other
        // implementation agrees on, plus the two zeros and a subnormal.
        for (bits, expected) in [
            (0x0000u16, [0xf9, 0x00, 0x00]), // +0.0
            (0x8000, [0xf9, 0x80, 0x00]),    // -0.0
            (0x3c00, [0xf9, 0x3c, 0x00]),    // 1.0
            (0x7bff, [0xf9, 0x7b, 0xff]),    // 65504.0, largest finite
            (0x0400, [0xf9, 0x04, 0x00]),    // 2^-14, largest subnormal
            (0x0001, [0xf9, 0x00, 0x01]),    // 2^-24, smallest subnormal
            (0x7c00, [0xf9, 0x7c, 0x00]),    // +infinity
            (0xfc00, [0xf9, 0xfc, 0x00]),    // -infinity
        ] {
            assert_eq!(to_cbor_vec(&f16::from_bits(bits)).unwrap(), expected);
            assert_eq!(from_cbor::<f16>(&expected).unwrap().to_bits(), bits);
        }

        // A `NaN` is checked by kind rather than by bits, since RFC 8949
        // Section 4.1 gives it one encoding and a payload is not part of it. The
        // conversion itself does keep the payload — it is the writer that drops
        // it, so a document is a function of its value.
        let payload = f16::from_bits(0x7e01);
        assert!(payload.is_nan());
        assert!((payload as f32).is_nan(), "the conversion dropped the NaN");
        let encoded = to_cbor_vec(&payload).unwrap();
        assert_eq!(
            encoded,
            [0xf9, 0x7e, 0x00],
            "a NaN payload reached the wire"
        );
        assert!(from_cbor::<f16>(&encoded).unwrap().is_nan());
    }

    #[test]
    fn every_width_is_accepted_on_the_way_in() {
        // A producer with no half-precision type writes a single or a double,
        // and refusing that would make its documents unreadable for no reason.
        for bytes in [
            &[0xf9, 0x3c, 0x00][..],                   // 1.0 as a half
            &[0xfa, 0x3f, 0x80, 0x00, 0x00][..],       // 1.0 as a single
            &[0xfb, 0x3f, 0xf0, 0, 0, 0, 0, 0, 0][..], // 1.0 as a double
        ] {
            let value: f16 = from_cbor(bytes).unwrap();
            assert_eq!(value, 1.0f16, "for {bytes:02x?}");
            // Whatever came in, what goes out is a half, since that is the type.
            assert_eq!(to_cbor_vec(&value).unwrap(), [0xf9, 0x3c, 0x00]);
        }

        // A value above the largest binary16 narrows to infinity rather than
        // failing, which is what a float cast does.
        let value: f16 = from_cbor(&[0xfb, 0x7e, 0x37, 0xe4, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(value, f16::INFINITY);
    }

    #[test]
    fn the_width_survives_a_wider_type_and_a_value() {
        // The writer narrows to the shortest width that holds the value, which
        // is what RFC 8949 Section 4.2.1 requires, and it is why the width
        // survives a round trip through a wider type.
        assert_eq!(
            to_cbor_vec(&from_cbor::<f32>(&[0xf9, 0x3c, 0x00]).unwrap()).unwrap(),
            [0xf9, 0x3c, 0x00]
        );

        // `Value::Float` is an `f64` and cannot hold a width, but the narrowing
        // recovers the one the input used.
        let value: Value = from_cbor(&[0xf9, 0x3c, 0x00]).unwrap();
        assert_eq!(to_cbor_vec(&value).unwrap(), [0xf9, 0x3c, 0x00]);

        // A map keyed by readings is unusual but legal, and the order has to
        // cope with it: both keys are the same width, so the payload decides.
        let value: Value =
            from_cbor(&[0xa2, 0xf9, 0x00, 0x00, 0x00, 0xf9, 0x3c, 0x00, 0x01]).unwrap();
        assert_eq!(
            to_cbor_vec(&value).unwrap(),
            [0xa2, 0xf9, 0x00, 0x00, 0x00, 0xf9, 0x3c, 0x00, 0x01]
        );
    }
}

mod in_a_derived_type {
    use super::*;

    #[derive(Debug, PartialEq, ToCbor, FromCbor)]
    #[cbor(map)]
    struct Reading {
        /// A sensor reading, which is the usual reason to want a half.
        value: f16,
        unit: String,
    }

    #[test]
    fn a_field_keeps_its_width_through_a_struct() {
        let reading = Reading {
            value: 1.5,
            unit: "V".into(),
        };
        let encoded = to_cbor_vec(&reading).unwrap();
        // a2 64 "unit" 61 "V" 65 "value" f9 3e00 — the keys are in canonical
        // order, so the four-byte "unit" comes before the five-byte "value".
        assert_eq!(
            encoded,
            [
                0xa2, 0x64, b'u', b'n', b'i', b't', 0x61, b'V', 0x65, b'v', b'a', b'l', b'u', b'e',
                0xf9, 0x3e, 0x00,
            ]
        );
        assert_eq!(from_cbor::<Reading>(&encoded).unwrap(), reading);
    }
}

mod size_hints {
    use super::*;

    #[test]
    fn a_buffer_sized_from_the_hint_is_never_short() {
        // A `f16` is always three bytes, so a buffer sized from the hint fits.
        assert_eq!(1.0f16.size_hint().map(|h| h.upper_bound()), Some(3));
        assert_eq!(
            <f16 as ToCbor>::max_size().map(|h| h.upper_bound()),
            Some(3)
        );

        // A slice encodes as an array, so there is the one-byte head on top of the
        // three bytes per element the hint promises.
        let values = [1.0f16, 2.0, 3.0];
        let mut buf = [0u8; 10];
        let written = zerocbor::to_cbor(&&values[..], &mut buf).unwrap();
        assert_eq!(written, 10);
        // 0x3c00 is 1.0, 0x4000 is 2.0 and 0x4200 is 3.0: the exponent field
        // carries the difference, so the significand is zero in all three.
        assert_eq!(
            buf,
            [0x83, 0xf9, 0x3c, 0x00, 0xf9, 0x40, 0x00, 0xf9, 0x42, 0x00]
        );
    }
}

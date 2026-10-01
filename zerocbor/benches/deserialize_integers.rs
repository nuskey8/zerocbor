//! Decoding side of the integer-width benchmark.
//!
//! The same record as `serialize_integers`, decoded rather than written.

#![feature(test)]
extern crate test;

use serde::{Deserialize, Serialize};

/// A record covering every integer width, so the benchmark shows how each
/// encoder handles the boundary between one-byte and multi-byte heads.
#[derive(
    zerocbor_derive::ToCbor,
    zerocbor_derive::FromCbor,
    Serialize,
    Deserialize,
    minicbor::Encode,
    minicbor::Decode,
)]
#[allow(dead_code)]
struct Integers {
    #[n(0)]
    u8_: u8,
    #[n(1)]
    u16_: u16,
    #[n(2)]
    u32_: u32,
    #[n(3)]
    u64_: u64,
    #[n(4)]
    i8_: i8,
    #[n(5)]
    i16_: i16,
    #[n(6)]
    i32_: i32,
    #[n(7)]
    i64_: i64,
}

const N: usize = 1000;

fn value() -> Integers {
    Integers {
        u8_: 200,
        u16_: 60_000,
        u32_: 4_000_000_000,
        u64_: 18_000_000_000_000_000_000,
        i8_: -100,
        i16_: -30_000,
        i32_: -2_000_000_000,
        i64_: -9_000_000_000_000_000_000,
    }
}

#[bench]
fn deserialize_integers_zerocbor(b: &mut test::Bencher) {
    let data = test::black_box(zerocbor::to_cbor_vec(&value()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(zerocbor::from_cbor::<Integers>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_integers_ciborium(b: &mut test::Bencher) {
    let mut bytes = Vec::new();
    ciborium::into_writer(&value(), &mut bytes).unwrap();
    let data = test::black_box(bytes);
    b.iter(|| {
        for _ in 0..N {
            let _: Integers = ciborium::from_reader(test::black_box(&data[..])).unwrap();
        }
    });
}

#[bench]
fn deserialize_integers_minicbor(b: &mut test::Bencher) {
    let data = test::black_box(minicbor::to_vec(value()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(minicbor::decode::<Integers>(test::black_box(&data)).unwrap());
        }
    });
}

//! Encoding side of the integer-width benchmark.
//!
//! A record with one field of every integer width, so the numbers show how each
//! encoder handles the boundary between a one-byte head and a multi-byte one.

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
fn serialize_integers_zerocbor(b: &mut test::Bencher) {
    let value = test::black_box(value());
    let mut buf = vec![0; 64];
    b.iter(|| {
        for _ in 0..N {
            let len = zerocbor::to_cbor(test::black_box(&value), &mut buf).unwrap();
            test::black_box(&buf[..len]);
        }
    });
}

#[bench]
fn serialize_integers_ciborium(b: &mut test::Bencher) {
    let value = test::black_box(value());
    let mut buf = Vec::with_capacity(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            ciborium::into_writer(&value, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_integers_minicbor(b: &mut test::Bencher) {
    let value = test::black_box(value());
    let mut buf = Vec::with_capacity(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            minicbor::encode(&value, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_integers_cbor2(b: &mut test::Bencher) {
    let value = test::black_box(value());
    let mut buf = Vec::with_capacity(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor2::to_writer(&value, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

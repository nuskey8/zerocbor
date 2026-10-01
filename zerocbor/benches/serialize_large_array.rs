#![feature(test)]
extern crate test;

/// A record holding 1000 integers, so the bulk encoder and decoder dominate.
///
/// A reusable buffer is used rather than a fresh allocation, so the measurement
/// covers encoding only.
#[derive(
    zerocbor_derive::ToCbor,
    zerocbor_derive::FromCbor,
    serde::Serialize,
    serde::Deserialize,
    minicbor::Encode,
    minicbor::Decode,
)]
#[allow(dead_code)]
struct Bulk {
    #[n(0)]
    x: i32,
    #[n(1)]
    y: i32,
}

const COUNT: usize = 1000;
const N: usize = 1000;

/// Ample room for the widest plausible encoding of 1000 `u64`s plus the record.
const CAPACITY: usize = 10 * COUNT + 64;

fn value() -> (Bulk, Vec<u64>) {
    (
        Bulk { x: 10, y: 20 },
        (0..COUNT as u64).map(|i| i * 1_000).collect(),
    )
}

#[bench]
fn serialize_large_array_zerocbor(b: &mut test::Bencher) {
    let (bulk, data) = test::black_box(value());
    // `to_cbor` takes `&mut [u8]`, so the buffer keeps its length and each call
    // overwrites from the start. The append-style encoders below clear and
    // refill an equally reserved `Vec`; either way nothing is allocated.
    let mut buf = vec![0u8; CAPACITY];
    b.iter(|| {
        for _ in 0..N {
            zerocbor::to_cbor(&bulk, &mut buf).unwrap();
            zerocbor::to_cbor(&data, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_large_array_ciborium(b: &mut test::Bencher) {
    let (bulk, data) = test::black_box(value());
    let mut buf = Vec::with_capacity(CAPACITY);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            ciborium::into_writer(&bulk, &mut buf).unwrap();
            ciborium::into_writer(&data, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_large_array_minicbor(b: &mut test::Bencher) {
    let (bulk, data) = test::black_box(value());
    let mut buf = Vec::with_capacity(CAPACITY);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            minicbor::encode(&bulk, &mut buf).unwrap();
            minicbor::encode(&data, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_large_array_cbor4ii(b: &mut test::Bencher) {
    let (bulk, data) = test::black_box(value());
    let mut buf = Vec::with_capacity(CAPACITY);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor4ii::serde::to_writer(&mut buf, &bulk).unwrap();
            cbor4ii::serde::to_writer(&mut buf, &data).unwrap();
            test::black_box(&buf);
        }
    });
}

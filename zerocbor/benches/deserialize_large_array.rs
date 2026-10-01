#![feature(test)]
extern crate test;

use serde::{Deserialize, Serialize};

#[derive(
    zerocbor_derive::ToCbor,
    zerocbor_derive::FromCbor,
    Serialize,
    Deserialize,
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

fn values() -> Vec<u64> {
    (0..COUNT as u64).map(|i| i * 1_000).collect()
}

#[bench]
fn deserialize_large_array_zerocbor(b: &mut test::Bencher) {
    // zerocbor writes a struct as a positional array, so the two values are
    // encoded separately and decoded separately.
    let bulk = test::black_box(zerocbor::to_cbor_vec(&Bulk { x: 10, y: 20 }).unwrap());
    let array = test::black_box(zerocbor::to_cbor_vec(&values()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(zerocbor::from_cbor::<Bulk>(test::black_box(&bulk)).unwrap());
            test::black_box(zerocbor::from_cbor::<Vec<u64>>(test::black_box(&array)).unwrap());
        }
    });
}

#[bench]
fn deserialize_large_array_ciborium(b: &mut test::Bencher) {
    // A serde-based encoder writes a struct as a map, so the bytes differ from
    // zerocbor's. Each library is fed its own output.
    let mut bulk = Vec::new();
    ciborium::into_writer(&Bulk { x: 10, y: 20 }, &mut bulk).unwrap();
    let mut array = Vec::new();
    ciborium::into_writer(&values(), &mut array).unwrap();
    let (bulk, array) = test::black_box((bulk, array));

    b.iter(|| {
        for _ in 0..N {
            let _: Bulk = ciborium::from_reader::<Bulk, _>(test::black_box(&bulk[..])).unwrap();
            let _: Vec<u64> =
                ciborium::from_reader::<Vec<u64>, _>(test::black_box(&array[..])).unwrap();
        }
    });
}

#[bench]
fn deserialize_large_array_minicbor(b: &mut test::Bencher) {
    let bulk = test::black_box(minicbor::to_vec(&Bulk { x: 10, y: 20 }).unwrap());
    let array = test::black_box(minicbor::to_vec(values()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(minicbor::decode::<Bulk>(test::black_box(&bulk)).unwrap());
            test::black_box(minicbor::decode::<Vec<u64>>(test::black_box(&array)).unwrap());
        }
    });
}

#[bench]
fn deserialize_large_array_cbor4ii(b: &mut test::Bencher) {
    let bulk = test::black_box(cbor4ii::serde::to_vec(Vec::new(), &Bulk { x: 10, y: 20 }).unwrap());
    let array = test::black_box(cbor4ii::serde::to_vec(Vec::new(), &values()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor4ii::serde::from_slice::<Bulk>(test::black_box(&bulk)).unwrap());
            test::black_box(
                cbor4ii::serde::from_slice::<Vec<u64>>(test::black_box(&array)).unwrap(),
            );
        }
    });
}

#[bench]
fn deserialize_large_array_cbor2(b: &mut test::Bencher) {
    let bulk = test::black_box(cbor2::to_vec(&Bulk { x: 10, y: 20 }).unwrap());
    let array = test::black_box(cbor2::to_vec(&values()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor2::from_slice::<Bulk>(test::black_box(&bulk)).unwrap());
            test::black_box(cbor2::from_slice::<Vec<u64>>(test::black_box(&array)).unwrap());
        }
    });
}

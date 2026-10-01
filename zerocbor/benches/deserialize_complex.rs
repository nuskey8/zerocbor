#![feature(test)]
extern crate test;

mod common;

use common::Nested;

const N: usize = 1000;

fn value() -> Nested {
    Nested {
        name: "Test".to_string(),
        p1: common::Point { x: 10, y: 20 },
        p2: Some(common::Point { x: 30, y: 40 }),
        params: vec![1, 2, 3, 4, 5],
    }
}

#[bench]
fn deserialize_complex_zerocbor(b: &mut test::Bencher) {
    let data = test::black_box(zerocbor::to_cbor_vec(&value()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(zerocbor::from_cbor::<Nested>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_complex_ciborium(b: &mut test::Bencher) {
    let mut bytes = Vec::new();
    ciborium::into_writer(&value(), &mut bytes).unwrap();
    let data = test::black_box(bytes);
    b.iter(|| {
        for _ in 0..N {
            let _: Nested = ciborium::from_reader(test::black_box(&data[..])).unwrap();
        }
    });
}

#[bench]
fn deserialize_complex_minicbor(b: &mut test::Bencher) {
    let data = test::black_box(minicbor::to_vec(value()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(minicbor::decode::<Nested>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_complex_cbor4ii(b: &mut test::Bencher) {
    let data = test::black_box(cbor4ii::serde::to_vec(Vec::new(), &value()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor4ii::serde::from_slice::<Nested>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_complex_cbor2(b: &mut test::Bencher) {
    let data = test::black_box(cbor2::to_vec(&value()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor2::from_slice::<Nested>(test::black_box(&data)).unwrap());
        }
    });
}

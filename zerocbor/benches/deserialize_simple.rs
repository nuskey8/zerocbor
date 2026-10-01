#![feature(test)]
extern crate test;

mod common;

use common::Point;

const N: usize = 1000;

#[bench]
fn deserialize_simple_zerocbor(b: &mut test::Bencher) {
    let data = test::black_box(zerocbor::to_cbor_vec(&Point { x: 10, y: 20 }).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(zerocbor::from_cbor::<Point>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_simple_ciborium(b: &mut test::Bencher) {
    let mut bytes = Vec::new();
    ciborium::into_writer(&Point { x: 10, y: 20 }, &mut bytes).unwrap();
    let data = test::black_box(bytes);

    b.iter(|| {
        for _ in 0..N {
            let _: Point = ciborium::from_reader(test::black_box(&data[..])).unwrap();
        }
    });
}

#[bench]
fn deserialize_simple_minicbor(b: &mut test::Bencher) {
    let data = test::black_box(minicbor::to_vec(&Point { x: 10, y: 20 }).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(minicbor::decode::<Point>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_simple_cbor4ii(b: &mut test::Bencher) {
    let data =
        test::black_box(cbor4ii::serde::to_vec(Vec::new(), &Point { x: 10, y: 20 }).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor4ii::serde::from_slice::<Point>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_simple_cbor2(b: &mut test::Bencher) {
    let data = test::black_box(cbor2::to_vec(&Point { x: 10, y: 20 }).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor2::from_slice::<Point>(test::black_box(&data)).unwrap());
        }
    });
}

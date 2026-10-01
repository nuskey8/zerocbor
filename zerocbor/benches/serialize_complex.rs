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
fn serialize_complex_zerocbor(b: &mut test::Bencher) {
    let nested = test::black_box(value());
    let mut buf = vec![0u8; 256];
    b.iter(|| {
        for _ in 0..N {
            zerocbor::to_cbor(&nested, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_complex_ciborium(b: &mut test::Bencher) {
    let nested = test::black_box(value());
    let mut buf = common::output_buffer(256);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            ciborium::into_writer(&nested, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_complex_minicbor(b: &mut test::Bencher) {
    let nested = test::black_box(value());
    let mut buf = common::output_buffer(256);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            minicbor::encode(&nested, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_complex_cbor4ii(b: &mut test::Bencher) {
    let nested = test::black_box(value());
    let mut buf = common::output_buffer(256);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor4ii::serde::to_writer(&mut buf, &nested).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_complex_cbor2(b: &mut test::Bencher) {
    let nested = test::black_box(value());
    let mut buf = common::output_buffer(256);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor2::to_writer(&nested, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

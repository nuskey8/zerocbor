#![feature(test)]
extern crate test;

mod common;

use common::Point;

const N: usize = 1000;

#[bench]
fn serialize_simple_zerocbor(b: &mut test::Bencher) {
    let point = test::black_box(Point { x: 10, y: 20 });
    let mut buf = vec![0u8; 64];
    b.iter(|| {
        for _ in 0..N {
            zerocbor::to_cbor(&point, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_simple_ciborium(b: &mut test::Bencher) {
    let point = test::black_box(Point { x: 10, y: 20 });
    let mut buf = common::output_buffer(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            ciborium::into_writer(&point, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_simple_minicbor(b: &mut test::Bencher) {
    let point = test::black_box(Point { x: 10, y: 20 });
    let mut buf = common::output_buffer(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            minicbor::encode(&point, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_simple_cbor4ii(b: &mut test::Bencher) {
    let point = test::black_box(Point { x: 10, y: 20 });
    let mut buf = common::output_buffer(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor4ii::serde::to_writer(&mut buf, &point).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_simple_cbor2(b: &mut test::Bencher) {
    let point = test::black_box(Point { x: 10, y: 20 });
    let mut buf = common::output_buffer(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor2::to_writer(&point, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

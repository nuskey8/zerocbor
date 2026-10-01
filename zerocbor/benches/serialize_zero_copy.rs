#![feature(test)]
extern crate test;

use std::borrow::Cow;

mod common;

use common::{NoCopy, NoCopySerde, SAMPLE_BIN, SAMPLE_STR};

const N: usize = 1000;

#[bench]
fn serialize_zero_copy_zerocbor(b: &mut test::Bencher) {
    let value = test::black_box(NoCopy {
        str: SAMPLE_STR,
        bin: Cow::Borrowed(SAMPLE_BIN),
    });
    let mut buf = vec![0u8; 64];
    b.iter(|| {
        for _ in 0..N {
            zerocbor::to_cbor(&value, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_zero_copy_ciborium(b: &mut test::Bencher) {
    let value = test::black_box(NoCopySerde::sample());
    let mut buf = common::output_buffer(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            ciborium::into_writer(&value, &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn serialize_zero_copy_cbor4ii(b: &mut test::Bencher) {
    let value = test::black_box(NoCopySerde::sample());
    let mut buf = common::output_buffer(64);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor4ii::serde::to_writer(&mut buf, &value).unwrap();
            test::black_box(&buf);
        }
    });
}

// `minicbor` is absent: 2.x borrows a byte string only as `&str`, so it cannot
// express this type.

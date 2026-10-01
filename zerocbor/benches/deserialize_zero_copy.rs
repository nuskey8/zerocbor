#![feature(test)]
extern crate test;

use std::borrow::Cow;

mod common;

use common::{NoCopy, NoCopySerde, SAMPLE_BIN, SAMPLE_STR};

const N: usize = 1000;

#[bench]
fn deserialize_zero_copy_zerocbor(b: &mut test::Bencher) {
    // Decoding borrows both fields out of `data`, so the loop allocates nothing
    // and the result is dropped rather than owned.
    let data = test::black_box(
        zerocbor::to_cbor_vec(&NoCopy {
            str: SAMPLE_STR,
            bin: Cow::Borrowed(SAMPLE_BIN),
        })
        .unwrap(),
    );
    b.iter(|| {
        for _ in 0..N {
            test::black_box(zerocbor::from_cbor::<NoCopy<'_>>(test::black_box(&data)).unwrap());
        }
    });
}

#[bench]
fn deserialize_zero_copy_cbor4ii(b: &mut test::Bencher) {
    let data = test::black_box(cbor4ii::serde::to_vec(Vec::new(), &NoCopySerde::sample()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(
                cbor4ii::serde::from_slice::<NoCopySerde<'_>>(test::black_box(&data)).unwrap(),
            );
        }
    });
}

// `ciborium` and `minicbor` are absent from the decode side: `ciborium` has no
// borrowing deserializer, and `minicbor` 2.x borrows a byte string only as
// `&str`, so it cannot express this type.

#[bench]
fn deserialize_zero_copy_cbor2(b: &mut test::Bencher) {
    let data = test::black_box(cbor2::to_vec(&NoCopySerde::sample()).unwrap());
    b.iter(|| {
        for _ in 0..N {
            test::black_box(cbor2::from_slice::<NoCopySerde<'_>>(test::black_box(&data)).unwrap());
        }
    });
}

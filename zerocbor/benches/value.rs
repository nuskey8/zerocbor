#![feature(test)]
extern crate test;

mod common;

use zerocbor::Value;

const N: usize = 1_000;

/// The same document `common::value` describes, as CBOR bytes.
fn data() -> Vec<u8> {
    let mut buf = Vec::new();
    ciborium::into_writer(&common::value(), &mut buf).unwrap();
    buf
}

#[bench]
fn decode_value_zerocbor(b: &mut test::Bencher) {
    let data = test::black_box(data());
    b.bytes = data.len() as u64;
    b.iter(|| {
        test::black_box(zerocbor::from_cbor::<Value<'_>>(test::black_box(&data)).unwrap());
    });
}

#[bench]
fn decode_value_ciborium(b: &mut test::Bencher) {
    let data = test::black_box(data());
    b.bytes = data.len() as u64;
    b.iter(|| {
        let value: ciborium::value::Value =
            ciborium::from_reader(test::black_box(&data[..])).unwrap();
        test::black_box(value);
    });
}

#[bench]
fn encode_value_zerocbor(b: &mut test::Bencher) {
    // `Value` borrows out of the input, so the buffer has to outlive it.
    let bytes = test::black_box(data());
    let value = test::black_box(zerocbor::from_cbor::<Value<'_>>(&bytes).unwrap());
    // A fixed buffer that is overwritten on every call, rather than
    // `to_cbor_vec`, so the measurement covers encoding instead of allocation.
    let mut buf = vec![0u8; 64 * 1024];
    b.iter(|| {
        for _ in 0..N {
            let len = zerocbor::to_cbor(test::black_box(&value), &mut buf).unwrap();
            test::black_box(&buf[..len]);
        }
    });
}

#[bench]
fn encode_value_ciborium(b: &mut test::Bencher) {
    let value =
        test::black_box(ciborium::from_reader::<ciborium::value::Value, _>(&data()[..]).unwrap());
    let mut buf = Vec::with_capacity(64 * 1024);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            ciborium::into_writer(test::black_box(&value), &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

#[bench]
fn decode_value_cbor2(b: &mut test::Bencher) {
    let data = test::black_box(data());
    b.bytes = data.len() as u64;
    b.iter(|| {
        let value: cbor2::Value = cbor2::from_slice(test::black_box(&data[..])).unwrap();
        test::black_box(value);
    });
}

#[bench]
fn encode_value_cbor2(b: &mut test::Bencher) {
    let value = test::black_box(cbor2::from_slice::<cbor2::Value>(&data()[..]).unwrap());
    let mut buf = Vec::with_capacity(64 * 1024);
    b.iter(|| {
        for _ in 0..N {
            buf.clear();
            cbor2::to_writer(test::black_box(&value), &mut buf).unwrap();
            test::black_box(&buf);
        }
    });
}

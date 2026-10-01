#![feature(test)]
extern crate test;

const VALUES: usize = 10_000;

fn array_header(values: usize) -> Vec<u8> {
    // A definite-length array of `values` elements, encoded as `array32` so the
    // header is always 5 bytes regardless of the count.
    let mut data = vec![0x9a];
    data.extend_from_slice(&(values as u32).to_be_bytes());
    data
}

#[bench]
fn skip_large_scalar_array(b: &mut test::Bencher) {
    let mut data = array_header(VALUES);
    data.resize(data.len() + VALUES, 1); // positive fixints

    b.bytes = data.len() as u64;
    b.iter(|| {
        // `from_cbor` into a `Value` walks the array, so this measures the
        // per-element head decode plus the container bookkeeping.
        test::black_box(zerocbor::from_cbor::<zerocbor::Value<'_>>(&data).unwrap());
    });
}

#[bench]
fn skip_large_u32_array(b: &mut test::Bencher) {
    let mut data = array_header(VALUES);
    for value in 0..VALUES as u32 {
        // Major type 0 with additional information 26, i.e. a four-byte integer.
        data.push(0x1a);
        data.extend_from_slice(&value.to_be_bytes());
    }

    b.bytes = data.len() as u64;
    b.iter(|| {
        test::black_box(zerocbor::from_cbor::<zerocbor::Value<'_>>(&data).unwrap());
    });
}

#[bench]
fn skip_large_text_array(b: &mut test::Bencher) {
    let mut data = array_header(VALUES);
    for _ in 0..VALUES {
        data.push(0x63); // text string of length 3
        data.extend_from_slice(b"abc");
    }

    b.bytes = data.len() as u64;
    b.iter(|| {
        test::black_box(zerocbor::from_cbor::<zerocbor::Value<'_>>(&data).unwrap());
    });
}

#[bench]
fn skip_nested_arrays(b: &mut test::Bencher) {
    // 64 levels of nesting, each holding one element, so the decoder has to
    // recurse the whole way down before it can start returning values.
    const DEPTH: usize = 64;
    // `DEPTH` heads of "array with one element", then the element itself.
    let mut data = vec![0x81; DEPTH];
    data.push(0x01);

    b.bytes = data.len() as u64;
    b.iter(|| {
        test::black_box(zerocbor::from_cbor::<zerocbor::Value<'_>>(&data).unwrap());
    });
}

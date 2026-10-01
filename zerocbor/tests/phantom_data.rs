//! `PhantomData` holds no data, so the tempting encoding is "write nothing" —
//! which makes the container under-long, so the *next* value is read as this
//! one. These tests pin the one-byte `null` that avoids it.

use std::marker::PhantomData;

use zerocbor::{FromCbor, ToCbor};

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
struct WithPhantom {
    a: u8,
    p: PhantomData<u8>,
    b: u8,
}

#[test]
fn every_container_of_a_phantom_keeps_its_length() {
    // On its own a `PhantomData` is the one-byte `null`, since that is the only
    // value that carries no data.
    let bare = zerocbor::to_cbor_vec(&PhantomData::<u8>).unwrap();
    assert_eq!(bare, vec![0xf6]);
    assert_eq!(
        zerocbor::from_cbor::<PhantomData<u8>>(&bare).unwrap(),
        PhantomData
    );

    // A struct: three fields, three elements. Before the fix this was
    // `83 01 02`, which reads back `b` twice and then runs off the end.
    let value = WithPhantom {
        a: 1,
        p: PhantomData,
        b: 2,
    };
    let encoded = zerocbor::to_cbor_vec(&value).unwrap();
    assert_eq!(encoded, vec![0x83, 0x01, 0xf6, 0x02]);
    assert_eq!(zerocbor::from_cbor::<WithPhantom>(&encoded).unwrap(), value);

    // An array, where a short count would just drop elements.
    let values = vec![PhantomData::<u8>; 3];
    let encoded = zerocbor::to_cbor_vec(&values).unwrap();
    assert_eq!(encoded, vec![0x83, 0xf6, 0xf6, 0xf6]);
    assert_eq!(
        zerocbor::from_cbor::<Vec<PhantomData<u8>>>(&encoded).unwrap(),
        values
    );

    // The hint drives `to_cbor_vec`'s preallocation and `to_cbor`'s unchecked
    // path, so an undercount would truncate the output.
    assert_eq!(
        PhantomData::<u8>::max_size().map(|h| h.upper_bound()),
        Some(1)
    );
}

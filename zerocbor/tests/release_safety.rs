//! Regression tests for safe serialization and bounded recursive decoding.
use zerocbor::{Error, FromCbor, FromCborOwned, MAX_DEPTH, ToCbor, TrustedSizeHint, Write};

struct WrongHint;
impl ToCbor for WrongHint {
    fn write<W: Write>(&self, writer: &mut W) -> zerocbor::Result<()> {
        writer.write_u64(u64::MAX)
    }
    fn size_hint(&self) -> Option<TrustedSizeHint> {
        ().size_hint()
    }
    fn max_size() -> Option<TrustedSizeHint> {
        <()>::max_size()
    }
}

#[test]
fn incorrect_hints_cannot_overrun_the_output() {
    let mut storage = [0xaa; 11];
    assert!(matches!(
        zerocbor::to_cbor(&WrongHint, &mut storage[1..2]),
        Err(Error::BufferTooSmall)
    ));
    assert_eq!(storage, [0xaa; 11]);
    let n = zerocbor::to_cbor(&WrongHint, &mut storage[1..10]).unwrap();
    assert_eq!(n, 9);
    assert_eq!(storage[0], 0xaa);
    assert_eq!(storage[10], 0xaa);
    assert_eq!(
        zerocbor::from_cbor::<u64>(&storage[1..10]).unwrap(),
        u64::MAX
    );

    let values = [WrongHint, WrongHint];
    assert!(matches!(
        zerocbor::to_cbor(&values, &mut [0; 3]),
        Err(Error::BufferTooSmall)
    ));
    let encoded = zerocbor::to_cbor_vec(&values).unwrap();
    assert_eq!(
        zerocbor::from_cbor::<Vec<u64>>(&encoded).unwrap(),
        [u64::MAX; 2]
    );
}

#[derive(FromCbor)]
#[allow(dead_code)]
struct Node {
    child: Option<Box<Node>>,
}
#[derive(FromCbor)]
#[cbor(map)]
#[allow(dead_code)]
struct MapNode {
    child: Option<Box<MapNode>>,
}
#[derive(FromCbor)]
#[cbor(map)]
#[allow(dead_code)]
enum Tree {
    End,
    Next(Box<Tree>),
}

fn assert_too_deep<T: FromCborOwned>(bytes: &[u8]) {
    assert!(matches!(
        zerocbor::from_cbor::<T>(bytes),
        Err(Error::DepthLimitExceeded)
    ));
    #[cfg(feature = "std")]
    assert!(matches!(
        zerocbor::read_cbor::<_, T>(bytes),
        Err(Error::DepthLimitExceeded)
    ));
}

#[test]
fn recursive_derived_types_obey_the_depth_limit() {
    let mut bytes = vec![0x81; MAX_DEPTH];
    bytes.push(0xf6);
    assert!(zerocbor::from_cbor::<Node>(&bytes).is_ok());
    #[cfg(feature = "std")]
    assert!(zerocbor::read_cbor::<_, Node>(&bytes[..]).is_ok());
    bytes.insert(0, 0x81);
    assert_too_deep::<Node>(&bytes);

    let mut bytes = Vec::new();
    for _ in 0..=MAX_DEPTH {
        bytes.extend_from_slice(b"\xa1\x65child");
    }
    bytes.push(0xf6);
    assert_too_deep::<MapNode>(&bytes);

    let mut bytes = Vec::new();
    for _ in 0..=MAX_DEPTH {
        bytes.extend_from_slice(b"\xa1\x64Next\x81");
    }
    bytes.extend_from_slice(b"\x63End");
    assert_too_deep::<Tree>(&bytes);
}

#[test]
fn tuples_and_results_consume_the_indefinite_break() {
    for bytes in [&[0x9f, 0x01, 0x02, 0xff][..], &[0x82, 0x01, 0x02][..]] {
        assert_eq!(zerocbor::from_cbor::<(u8, u8)>(bytes).unwrap(), (1, 2));
    }
    let bytes = [0x82, 0x9f, 0x01, 0x02, 0xff, 0x03];
    assert_eq!(
        zerocbor::from_cbor::<((u8, u8), u8)>(&bytes).unwrap(),
        ((1, 2), 3)
    );
    let bytes = [0x82, 0x9f, 0xf5, 0x01, 0xff, 0x02];
    assert_eq!(
        zerocbor::from_cbor::<(Result<u8, u8>, u8)>(&bytes).unwrap(),
        (Ok(1), 2)
    );
}

struct Recover;
impl<'de> FromCbor<'de> for Recover {
    fn read<R: zerocbor::Read<'de>>(reader: &mut R) -> zerocbor::Result<Self> {
        for _ in 0..MAX_DEPTH * 2 {
            assert!(Node::read(reader).is_err());
            assert!(Vec::<u8>::read(reader).is_err());
        }
        Node::read(reader)?;
        Ok(Recover)
    }
}

#[test]
fn failed_nested_reads_restore_the_same_readers_depth() {
    let mut bytes = Vec::new();
    for _ in 0..MAX_DEPTH * 2 {
        bytes.extend_from_slice(&[0x81, 0xf5, 0x81, 0xf5]);
    }
    bytes.extend(std::iter::repeat_n(0x81, MAX_DEPTH));
    bytes.push(0xf6);
    assert!(zerocbor::from_cbor::<Recover>(&bytes).is_ok());
    #[cfg(feature = "std")]
    assert!(zerocbor::read_cbor::<_, Recover>(&bytes[..]).is_ok());
}

#[test]
fn an_unclosed_fixed_array_drops_its_initialized_elements() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static DROPS: AtomicUsize = AtomicUsize::new(0);
    struct Item;
    impl Drop for Item {
        fn drop(&mut self) {
            DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl<'de> FromCbor<'de> for Item {
        fn read<R: zerocbor::Read<'de>>(reader: &mut R) -> zerocbor::Result<Self> {
            reader.read_null()?;
            Ok(Item)
        }
    }
    assert!(zerocbor::from_cbor::<[Item; 1]>(&[0x9f, 0xf6, 0x00]).is_err());
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}

struct RecoverCollection<C>(std::marker::PhantomData<C>);
impl<'de, C: FromCborOwned> FromCbor<'de> for RecoverCollection<C> {
    fn read<R: zerocbor::Read<'de>>(reader: &mut R) -> zerocbor::Result<Self> {
        for _ in 0..MAX_DEPTH * 2 {
            let error = match C::read(reader) {
                Ok(_) => panic!("malformed collection was accepted"),
                Err(error) => error,
            };
            assert!(
                matches!(error, Error::InvalidInitialByte(0xf5) | Error::DuplicateKey),
                "unexpected error: {error:?}"
            );
        }
        let _ = C::read(reader)?;
        Node::read(reader)?;
        Ok(Self(std::marker::PhantomData))
    }
}

fn collection_recovers<C: FromCborOwned>(malformed: &[u8], empty: u8) {
    let mut bytes = Vec::new();
    for _ in 0..MAX_DEPTH * 2 {
        bytes.extend_from_slice(malformed);
    }
    bytes.push(empty);
    bytes.extend(std::iter::repeat_n(0x81, MAX_DEPTH));
    bytes.push(0xf6);
    assert!(zerocbor::from_cbor::<RecoverCollection<C>>(&bytes).is_ok());
    #[cfg(feature = "std")]
    assert!(zerocbor::read_cbor::<_, RecoverCollection<C>>(&bytes[..]).is_ok());
}

#[test]
fn every_collection_restores_depth_after_element_errors() {
    use std::collections::{BTreeMap, BTreeSet, BinaryHeap, LinkedList, VecDeque};
    collection_recovers::<VecDeque<u8>>(&[0x81, 0xf5], 0x80);
    collection_recovers::<VecDeque<VecDeque<u8>>>(&[0x81, 0x81, 0xf5], 0x80);
    collection_recovers::<LinkedList<u8>>(&[0x81, 0xf5], 0x80);
    collection_recovers::<BTreeSet<u8>>(&[0x81, 0xf5], 0x80);
    collection_recovers::<BinaryHeap<u8>>(&[0x81, 0xf5], 0x80);
    for malformed in [
        &[0xa1, 0x00, 0xf5][..],
        &[0xa1, 0xf5][..],
        &[0xa2, 0x00, 0x01, 0x00, 0x02][..],
    ] {
        collection_recovers::<BTreeMap<u8, u8>>(malformed, 0xa0);
        #[cfg(feature = "std")]
        collection_recovers::<std::collections::HashMap<u8, u8>>(malformed, 0xa0);
    }
    #[cfg(feature = "std")]
    collection_recovers::<std::collections::HashSet<u8>>(&[0x81, 0xf5], 0x80);
}

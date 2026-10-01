//! Lengths exceeding the platform's address space must never wrap to zero.
#![cfg(target_pointer_width = "32")]
use zerocbor::{Error, FromCbor, Read};

struct ReadValue;
impl<'de> FromCbor<'de> for ReadValue {
    fn read<R: Read<'de>>(reader: &mut R) -> zerocbor::Result<Self> {
        let _: zerocbor::Value<'de> = FromCbor::read(reader)?;
        Ok(Self)
    }
}
struct Skip;
impl<'de> FromCbor<'de> for Skip {
    fn read<R: Read<'de>>(reader: &mut R) -> zerocbor::Result<Self> {
        reader.skip_value()?;
        Ok(Self)
    }
}
fn rejects<T: zerocbor::FromCborOwned>(bytes: &[u8]) {
    assert!(matches!(
        zerocbor::from_cbor::<T>(bytes),
        Err(Error::IntegerOutOfRange)
    ));
    #[cfg(feature = "std")]
    assert!(matches!(
        zerocbor::read_cbor::<_, T>(bytes),
        Err(Error::IntegerOutOfRange)
    ));
}
#[test]
fn lengths_and_chunks_cannot_wrap() {
    for major in [0x40, 0x60, 0x80, 0xa0] {
        let bytes = [major | 27, 0, 0, 0, 1, 0, 0, 0, 0]; // 2^32
        rejects::<ReadValue>(&bytes);
        rejects::<Skip>(&bytes);
        if major == 0x40 || major == 0x60 {
            let mut chunks = vec![major | 31];
            chunks.extend_from_slice(&bytes);
            chunks.push(0xff);
            rejects::<ReadValue>(&chunks);
            rejects::<Skip>(&chunks);
        }
    }
}

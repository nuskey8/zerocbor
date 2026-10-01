#![no_main]

use libfuzzer_sys::fuzz_target;
use zerocbor::Value;
use zerocbor_fuzz::{same_as_ciborium, show, to_comparable};

fuzz_target!(|data: &[u8]| {
    let Ok(ours) = zerocbor::from_cbor::<Value>(data) else {
        return;
    };

    let Some(theirs) = to_comparable(&ours) else {
        return;
    };

    match ciborium::from_reader::<ciborium::Value, _>(data) {
        Ok(their_value) => assert!(
            same_as_ciborium(&ours, &their_value),
            "{} decoded to {} here and to {:?} in ciborium",
            hex(data),
            show(&ours),
            theirs,
        ),
        Err(e) => panic!(
            "we accepted {} as {} and ciborium rejected it: {e}",
            hex(data),
            show(&ours)
        ),
    }

    if contains_nan(&ours) {
        return;
    }

    let encoded = zerocbor::to_cbor_vec(&ours).expect("our own value would not encode");
    let mut their_bytes = Vec::new();
    ciborium::into_writer(&theirs, &mut their_bytes).expect("ciborium failed to encode");
    if encoded == data {
        assert_eq!(
            hex(&encoded),
            hex(&their_bytes),
            "{} encoded differently",
            show(&ours),
        );
    }

    // And a round trip through their bytes has to land on the same value here.
    let back: Value<'_> = zerocbor::from_cbor(&their_bytes)
        .unwrap_or_else(|e| panic!("ciborium's encoding of {theirs:?} did not decode: {e}"));
    assert!(
        same_as_ciborium(&back, &theirs),
        "ciborium's encoding of {theirs:?} decoded to {} here",
        show(&back),
    );
});

/// Whether a `NaN` appears anywhere in the value.
fn contains_nan(value: &Value<'_>) -> bool {
    match value {
        Value::Float(f) => f.is_nan(),
        Value::Array(items) => items.iter().any(contains_nan),
        Value::Map(entries) => entries
            .iter()
            .any(|(k, v)| contains_nan(k) || contains_nan(v)),
        Value::Tag(_, inner) => contains_nan(inner),
        _ => false,
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes.iter().take(96) {
        out.push_str(&format!("{b:02x}"));
    }
    if bytes.len() > 96 {
        out.push_str("..");
    }
    out
}

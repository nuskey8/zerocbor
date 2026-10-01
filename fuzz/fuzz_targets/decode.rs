#![no_main]

use libfuzzer_sys::fuzz_target;
use zerocbor::Value;
use zerocbor_fuzz::{same_value, show};

fn read_every_way(data: &[u8]) {
    let _ = zerocbor::from_cbor::<Value>(data);
    let _ = zerocbor::from_cbor::<u64>(data);
    let _ = zerocbor::from_cbor::<i64>(data);
    let _ = zerocbor::from_cbor::<f64>(data);
    let _ = zerocbor::from_cbor::<String>(data);
    let _ = zerocbor::from_cbor::<Vec<Value>>(data);
    let _ = zerocbor::from_cbor::<std::collections::BTreeMap<String, Value>>(data);
    let _ = zerocbor::from_cbor::<&str>(data);
    let _ = zerocbor::read_cbor::<_, Value>(std::io::Cursor::new(data));
}

fuzz_target!(|data: &[u8]| {
    read_every_way(data);

    let Ok(value) = zerocbor::from_cbor::<Value>(data) else {
        return;
    };
    let once = match zerocbor::to_cbor_vec(&value) {
        Ok(bytes) => bytes,
        Err(e) => panic!(
            "{} decoded to {} which will not encode: {e}",
            hex(data),
            show(&value)
        ),
    };

    let twice = zerocbor::to_cbor_vec(
        &zerocbor::from_cbor::<Value>(&once)
            .unwrap_or_else(|e| panic!("our own output {} did not decode: {e}", hex(&once))),
    )
    .expect("our own output did not re-encode");
    assert_eq!(once, twice, "not a fixed point for {}", show(&value));

    let decoded = zerocbor::from_cbor::<Value>(&once).expect("our own output did not decode");
    assert!(
        same_value(&value, &decoded),
        "{} decoded to {}, which re-encoded to {} came back as {}",
        hex(data),
        show(&value),
        hex(&once),
        show(&decoded),
    );

    assert!(
        once.len() <= data.len() + 9,
        "{} ({} bytes) grew to {} bytes",
        hex(data),
        data.len(),
        once.len(),
    );
});

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2 + 8);
    if bytes.len() > 64 {
        out.push_str("...");
    }
    for b in bytes.iter().take(64) {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

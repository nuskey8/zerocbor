# zerocbor

A zero-copy, zero-dependency, no_std-compatible, extremely fast CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) serializer for Rust.

[![Crates.io version](https://img.shields.io/crates/v/zerocbor.svg?style=flat-square)](https://crates.io/crates/zerocbor)
[![docs.rs docs](https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square)](https://docs.rs/zerocbor)

## Overview

zerocbor is a fast CBOR serializer for Rust. It runs about 1.5–4.5 times faster than other crates and is implemented without depending on any libraries, including `std`.

zerocbor is based on the architecture of [zerompk](https://github.com/nuskey8/zerompk), with the serialization format switched to CBOR. zerompk is a fast MessagePack serializer characterized by high performance and a small code size compared with conventional serializers. See the zerompk README for details.

## Quick Start

```rust
use zerocbor::{FromCbor, ToCbor};

#[derive(FromCbor, ToCbor)]
pub struct Person {
    pub name: String,
    pub age: u32,
}

fn main() {
    let person = Person {
        name: "Alice".to_string(),
        age: 18,
    };

    let cbor: Vec<u8> = zerocbor::to_cbor_vec(&person)
        .unwrap();
    let person: Person = zerocbor::from_cbor(&cbor)
        .unwrap();
}
```

## Format

The mapping between Rust types and CBOR types in zerocbor is as follows.


| Rust Type                                                                                      | CBOR Major Type                                |
| ---------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `bool`                                                                                         | 7, simple value `true` / `false`               |
| `u8`, `u16`, `u32`, `u64`, `usize`                                                             | 0, unsigned integer                            |
| `i8`, `i16`, `i32`, `i64`, `isize`                                                             | 0 or 1, unsigned or negative integer           |
| `f32`                                                                                          | 7, `float 16` or `float 32`, whichever is narrower |
| `f64`                                                                                          | 7, whichever of the three float widths is narrowest |
| `char`                                                                                         | 3, one-character text string                   |
| `str`, `String`                                                                                | 3, text string                                 |
| `&'a [u8]` _(decode)_                                                                          | 2, byte string                                 |
| `&[u8]`, `Vec<u8>` _(encode)_                                                                  | 4, array of integers — see the note below      |
| `&[T]`, `Vec<T>`, `VecDeque<T>`, `LinkedList<T>`, `BTreeSet<T>`, `BinaryHeap<T>`, `HashSet<T>` | 4, array                                       |
| `BTreeMap<K, V>`, `HashMap<K, V>`                                                              | 5, map                                         |
| `()`                                                                                           | 7, simple value `null`                         |
| `Option<T>`                                                                                    | 7, `null` (`None`) or `T` (`Some(T)`)          |
| `Result<T, E>`                                                                                 | 4, `[true, T]` (`Ok`) or `[false, E]` (`Err`)  |
| newtype `struct W(T)`                                                                          | as `T`, optionally under a tag                 |
| `(T0, T1)`, `(T0, T1, T2)`, ...                                                                | 4, array                                       |
| `Box<T>`, `Rc<T>`, `Arc<T>`                                                                    | as `T`                                         |
| `PhantomData<T>`                                                                               | 7, simple value `null`                         |
| struct (default, array representation)                                                         | 4, array of the fields in declaration order    |
| struct (with `#[cbor(map)]`)                                                                   | 5, map keyed by field name                     |
| enum, fieldless variants (default)                                                             | 0, the variant index                           |
| enum, fieldless variants (with `#[cbor(map)]`)                                                 | 3, the variant name                            |
| enum with data (default)                                                                       | 4, `[index, fields...]`                        |
| enum with data (with `#[cbor(map)]`)                                                           | 5, `{"Name": value}` where value is the fields |
| anything with `#[cbor(tag = N)]`                                                               | 6, the tag, then the value it applies to       |

## derive

Enable the `derive` feature flag to implement `FromCbor`/`ToCbor` using `derive` macros.

```rust
use zerocbor::{FromCbor, ToCbor};

#[derive(FromCbor, ToCbor)]
pub struct Person {
    pub name: String,
    pub age: u32,
}
```

You can also customize the serialization format using the `#[cbor]` attribute.


### array/map

You can choose `array` or `map` as the serialization format for structs and enums. For performance reasons, the default is `array`.

```rust
use zerocbor::{FromCbor, ToCbor};

#[derive(FromCbor, ToCbor)]
#[cbor(array)] // default
pub struct PersonArray {
    pub name: String,
    pub age: u32,
}

#[derive(FromCbor, ToCbor)]
#[cbor(map)]
pub struct PersonMap {
    pub name: String,
    pub age: u32,
}
```

### key

You can override the index/key used for fields and enum variants. Integers can be used for arrays, and strings for maps. When the format is `array` and there are gaps in the indices, `null` is inserted automatically.

```rust
#[derive(FromCbor, ToCbor)]
#[cbor(map)]
pub struct Point {
    #[cbor(key = "the-x")]
    x: i32,
    y: i32,
}
```

> [!NOTE]
> To improve versioning resilience, it is recommended to set keys explicitly whenever possible.

### ignore

Set `ignore` on fields that should be ignored during serialization/deserialization. When deserializing a struct that contains an `ignore` field, the field's type must implement `Default`.

```rust
#[derive(FromCbor, ToCbor)]
pub struct Person {
    pub name: String,
    pub age: u32,

    #[cbor(ignore)]
    pub meta: Metadata,
}
```

### c_enum

Adding `#[cbor(c_enum)]` to a C-style enum allows it to be serialized as an integer. The value is the discriminant of each variant.

```rust
#[derive(FromCbor, ToCbor)]
#[cbor(c_enum)]
#[repr(u8)]
pub enum Status {
    Ok = 0,
    NotFound = 4,
    InternalServerError = 5,
}
```

### as_bytes

You can specify whether a `u8` array is serialized as binary data (major type 2). The default is `true`. This option can be applied to fields of type `&[u8]`, `Vec<u8>`, or `Cow<[u8]>`.

```rust
use std::borrow::Cow;

use zerocbor::{FromCbor, ToCbor};

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
struct Blob<'a> {
    #[cbor(as_bytes = true)]
    data: Cow<'a, [u8]>,
}

let value = Blob {
    data: Cow::Borrowed(&[0x01, 0x02][..]),
};
let encoded = zerocbor::to_cbor_vec(&value).unwrap();
assert_eq!(encoded, vec![0x81, 0x42, 0x01, 0x02]);
assert_eq!(zerocbor::from_cbor::<Blob<'_>>(&encoded).unwrap(), value);
```

### default

If a key is missing during deserialization, the corresponding field is replaced with a default value. A missing key is filled with `Default::default()`, or, if `default = "path"` is specified, with the result of the named function.

This is supported only with `#[cbor(map)]`. (Because arrays have no field names, missing values cannot be detected safely.)

```rust
fn default_age() -> u32 {
    18
}

use zerocbor::{FromCbor, ToCbor};

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map)]
pub struct Person {
    pub name: String,

    #[cbor(default)]
    pub nickname: Option<String>,

    #[cbor(default = "default_age")]
    pub age: u32,
}
```

`default` applies only to missing keys. Unknown keys cause an error unless `allow_unknown_fields` is set.

### allow_unknown_fields

When an unknown key is encountered during deserialization, this changes the behavior to skip it instead of returning an error. This is effective only with `#[cbor(map)]`.

```rust
use zerocbor::{FromCbor, ToCbor};

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map, allow_unknown_fields)]
pub struct Person {
    pub name: String,
    pub age: u32,
}
```

To ensure full forward and backward compatibility, combine `default` and `allow_unknown_fields`.

```rust
#[derive(FromCbor, ToCbor)]
#[cbor(map, allow_unknown_fields)]
pub struct Person {
    pub name: String,

    #[cbor(default)]
    pub age: u32,
}
```

> [!NOTE]
> These attributes are opt-in. By default, zerocbor requires an exact schema match.

## Benchmarks

### Serialize/Deserialize Struct (2 fields, array format)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `cbor4ii`      |   2.60 μs |    12.17 μs |
| `ciborium`     |  14.67 μs |    99.96 μs |
| `minicbor`     |   8.98 μs |     8.65 μs |
| **`zerocbor`** |   2.10 μs |     3.99 μs |

### Serialize/Deserialize Struct (4 fields, map format, with a nested struct, an `Option` and a `Vec`)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `cbor4ii`      |  34.54 μs |   139.58 μs |
| `ciborium`     |  78.62 μs |   388.14 μs |
| `minicbor`     |  76.61 μs |   121.65 μs |
| **`zerocbor`** |  21.80 μs |    92.99 μs |

### Serialize/Deserialize Struct (8 integer fields, one of every width)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `ciborium`     |  86.44 μs |   287.71 μs |
| `minicbor`     |  54.99 μs |    32.85 μs |
| **`zerocbor`** |   9.89 μs |    22.66 μs |

### Serialize/Deserialize Array (a 2-field struct plus a 1000-element `Vec<u64>`)

| Crate          |    Serialize |  Deserialize  |
| -------------- | -----------: | ------------: |
| `cbor4ii`      |    785.38 μs |  5,257.50 μs |
| `ciborium`     |  6,599.11 μs | 12,171.99 μs |
| `minicbor`     | 14,715.36 μs |  4,456.45 μs |
| **`zerocbor`** |    899.63 μs |  1,866.91 μs |

### Serialize/Deserialize Struct (borrowed `&str` and byte string)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `cbor4ii`      |   9.28 μs |    26.72 μs |
| `ciborium`     |  18.98 μs |         N/A |
| `minicbor`     |       N/A |         N/A |
| **`zerocbor`** |  12.74 μs |    13.89 μs |

### Deserialize Array (1000 records) from a `std::io::Read` stream

| Crate          | Deserialize |
| -------------- | ----------: |
| `cbor4ii`      |   101.10 μs |
| `ciborium`     |    72.89 μs |
| **`zerocbor`** |    22.52 μs |

### Decode/Encode a dynamically typed document into and out of `Value`

| Crate          |  Decode |    Encode |
| -------------- | ------: | --------: |
| `ciborium`     | 2.24 μs | 419.68 μs |
| **`zerocbor`** | 1.06 μs | 514.65 μs |

## License

This library is released under the [MIT License](LICENSE).

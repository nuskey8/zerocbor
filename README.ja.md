# zerocbor

A zero-copy, zero-dependency, no_std-compatible, extremely fast CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) serializer for Rust.

[![Crates.io version](https://img.shields.io/crates/v/zerocbor.svg?style=flat-square)](https://crates.io/crates/zerocbor)
[![docs.rs docs](https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square)](https://docs.rs/zerocbor)

## Overview

zerocborはRust向けの高速なCBORシリアライザです。他のcrateと比較して1.5-4.5倍ほど高速に動作し、`std`を含む一切のライブラリに依存せずに実装されています。

zerocborは[zerompk](https://github.com/nuskey8/zerompk)のアーキテクチャをベースとしてシリアライズ形式をCBORに切り替えたものです。zerompkは高速なMessagePackシリアライザであり、従来のシリアライザと比べて高い性能と小さいコードサイズを特徴としています。詳細はzerompkのREADMEを参照してください。

## クイックスタート

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

## フォーマット

zerocborにおけるRustとMessagePackの型の対応は以下の通りです。


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

`derive`フィーチャーフラグを有効化することで、`derive`マクロを用いて`FromCbor`/`ToCbor`を実装できます。

```rust
use zerocbor::{FromCbor, ToCbor};

#[derive(FromCbor, ToCbor)]
pub struct Person {
    pub name: String,
    pub age: u32,
}
```

また、`#[cbor]`属性を用いてシリアライズ形式をカスタマイズできます。


### array/map

structやenumのシリアライズ形式は`array`/`map`から選択できます。パフォーマンス上の理由からデフォルトは`array`に設定されています。

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

フィールドやenumのバリアントに用いるindex/keyを上書きできます。arrayの場合は整数、mapの場合は文字列が利用できます。形式がarrayかつインデックスに空白がある場合は自動的に`null`が挿入されます。

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
> バージョニング耐性を高めるため、可能な限りkeyを明示的に設定することが推奨されます。

### ignore

シリアライズ/デシリアライズ時に無視したいフィールドには`ignore`を設定します。`ignore`を含む構造体をデシリアライズする場合、`ignore`フィールドの型は`Default`を実装している必要があります。

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

C-styleのenumに`#[cbor(c_enum)]`を付与することで、enumを整数としてシリアライズできます。値は各バリアントの判別子(discriminant)です。

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

`u8`配列をバイナリ(major type 2)としてシリアライズするかどうかを指定できます。デフォルト値は`true`です。このオプションは`&[u8]`、`Vec<u8>`、`Cow<[u8]>`型のフィールドに対して適用できます。

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

デシリアライズにキーが存在しない場合に、そのフィールドをデフォルト値で置き換えます。欠損したキーは `Default::default()` で埋められるか、`default = "path"` が指定されている場合は名前付き関数の結果で埋められます。

これは`#[cbor(map)]` でのみサポートされています。(配列にはフィールド名がないため、欠落した値を安全に検出できません)

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

`default` は欠損したキーのみに対応します。`allow_unknown_fields` が設定されていない限り、不明なキーが含まれる場合はエラーとなります。

### allow_unknown_fields

デシリアライズ時に不明なキーがあった場合、エラーにする代わりにスキップするように変更します。これは`#[cbor(map)]`でのみ有効です。

```rust
use zerocbor::{FromCbor, ToCbor};

#[derive(Debug, PartialEq, ToCbor, FromCbor)]
#[cbor(map, allow_unknown_fields)]
pub struct Person {
    pub name: String,
    pub age: u32,
}
```

完全な前方互換性と後方互換性を確保するには、`default`と`allow_unknown_fields`を組み合わせて使用します。

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
> これらの属性はオプトインです。デフォルトではzerocborは厳密なスキーマの一致を要求します。

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

| Crate          |    Serialize |  Deserialize |
| -------------- | -----------: | -----------: |
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
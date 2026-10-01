# zerocbor

A zero-copy, zero-dependency, no_std-compatible, extremely fast CBOR ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949)) serializer for Rust.

[![Crates.io version](https://img.shields.io/crates/v/zerocbor.svg?style=flat-square)](https://crates.io/crates/zerocbor)
[![docs.rs docs](https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square)](https://docs.rs/zerocbor)

## Overview

zerocborはRust向けの高速なCBORシリアライザです。他のcrateと比較して1.5-4.0倍ほど高速に動作し、`std`を含む一切のライブラリに依存せずに実装されています。

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

> macOS 26.4.1（arm64）、`rustc 1.100.0-nightly`で測定

### Serialize/Deserialize Struct (2 fields, array format)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `cbor4ii`      |   3.19 μs |    16.46 μs |
| `ciborium`     |  18.40 μs |   123.92 μs |
| `minicbor`     |  11.36 μs |    10.29 μs |
| **`zerocbor`** |   1.54 μs |     5.03 μs |

### Serialize/Deserialize Struct (4 fields, map format, with a nested struct, an `Option` and a `Vec`)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `cbor4ii`      |  34.52 μs |   180.49 μs |
| `ciborium`     |  90.69 μs |   491.88 μs |
| `minicbor`     |  64.42 μs |   150.32 μs |
| **`zerocbor`** |  24.74 μs |   122.83 μs |

### Serialize/Deserialize Struct (8 integer fields, one of every width)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `ciborium`     |  87.40 μs |   357.90 μs |
| `minicbor`     |  58.71 μs |    41.40 μs |
| **`zerocbor`** |   8.54 μs |    27.15 μs |

### Serialize/Deserialize Array (a 2-field struct plus a 1000-element `Vec<u64>`)

| Crate          |    Serialize |  Deserialize |
| -------------- | -----------: | -----------: |
| `cbor4ii`      |  2,289.96 μs |  6,544.41 μs |
| `ciborium`     |  6,175.27 μs | 15,511.02 μs |
| `minicbor`     | 11,050.64 μs |  5,625.04 μs |
| **`zerocbor`** |    820.06 μs |  2,376.10 μs |

### Serialize/Deserialize Struct (borrowed `&str` and byte string)

| Crate          | Serialize | Deserialize |
| -------------- | --------: | ----------: |
| `cbor4ii`      |  11.74 μs |    27.33 μs |
| `ciborium`     |  23.84 μs |         N/A |
| `minicbor`     |       N/A |         N/A |
| **`zerocbor`** |   8.71 μs |    17.80 μs |

### Deserialize Array (1000 records) from a `std::io::Read` stream

| Crate          | Deserialize |
| -------------- | ----------: |
| `cbor4ii`      |   124.05 μs |
| `ciborium`     |    96.32 μs |
| **`zerocbor`** |    32.62 μs |

### Decode/Encode a dynamically typed document into and out of `Value`

| Crate          |  Decode |    Encode |
| -------------- | ------: | --------: |
| `ciborium`     | 2.83 μs | 496.22 μs |
| **`zerocbor`** | 1.32 μs | 365.69 μs |


## License

This library is released under the [MIT License](LICENSE).
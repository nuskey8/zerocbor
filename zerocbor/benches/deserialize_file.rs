#![feature(test)]
extern crate test;

mod common;

use common::Point;
use std::io::{Seek, Write};

const COUNT: usize = 1000;

fn points() -> Vec<Point> {
    (0..COUNT)
        .map(|i| Point {
            x: i as i32,
            y: i as i32 * 2,
        })
        .collect()
}

/// Writes `points` to a temp file and returns its path.
///
/// Each library has to write the file in its *own* output format, because a
/// struct is an array for zerocbor and a map for a serde-based encoder. Reading
/// one library's bytes with another would just measure a decode error.
fn write_temp(name: &str, encode: impl FnOnce(&[Point]) -> Vec<u8>) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::File::create(&path)
        .unwrap()
        .write_all(&encode(&points()))
        .unwrap();
    path
}

fn reader_for(path: &std::path::Path) -> std::io::BufReader<std::fs::File> {
    std::io::BufReader::with_capacity(4096, std::fs::File::open(path).unwrap())
}

#[bench]
fn deserialize_zerocbor_file(b: &mut test::Bencher) {
    let path = write_temp("zerocbor_points.cbor", |points| {
        zerocbor::to_cbor_vec(&points).unwrap()
    });
    let mut reader = reader_for(&path);

    b.iter(|| {
        reader.seek(std::io::SeekFrom::Start(0)).unwrap();
        test::black_box(zerocbor::read_cbor::<_, Vec<Point>>(&mut reader).unwrap());
    });
}

#[bench]
fn deserialize_ciborium_file(b: &mut test::Bencher) {
    let path = write_temp("ciborium_points.cbor", |points| {
        let mut buf = Vec::new();
        ciborium::into_writer(&points, &mut buf).unwrap();
        buf
    });
    let mut reader = reader_for(&path);

    b.iter(|| {
        reader.seek(std::io::SeekFrom::Start(0)).unwrap();
        test::black_box(ciborium::from_reader::<Vec<Point>, _>(&mut reader).unwrap());
    });
}

#[bench]
fn deserialize_cbor4ii_file(b: &mut test::Bencher) {
    let path = write_temp("cbor4ii_points.cbor", |points| {
        cbor4ii::serde::to_vec(Vec::new(), &points).unwrap()
    });
    let mut reader = reader_for(&path);

    b.iter(|| {
        reader.seek(std::io::SeekFrom::Start(0)).unwrap();
        test::black_box(cbor4ii::serde::from_reader::<Vec<Point>, _>(&mut reader).unwrap());
    });
}

// `minicbor` has no `io::Read` path in 2.x: its `Decoder` borrows a `&[u8]`, so
// there is nothing to compare `read_cbor` against here.

#[bench]
fn deserialize_cbor2_file(b: &mut test::Bencher) {
    let path = write_temp("cbor2_points.cbor", |points| {
        cbor2::to_vec(&points).unwrap()
    });
    let mut reader = reader_for(&path);

    b.iter(|| {
        reader.seek(std::io::SeekFrom::Start(0)).unwrap();
        test::black_box(cbor2::from_reader::<Vec<Point>, _>(&mut reader).unwrap());
    });
}

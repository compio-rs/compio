use std::hint::black_box;

use bytes::Bytes;
use compio_h2::{
    frame::{Data, FRAME_TYPE_DATA, Frame, FrameHeader, Headers, Settings, StreamId},
    hpack::{
        Decoder as HpackDecoder, Encoder as HpackEncoder,
        huffman::{huffman_decode, huffman_encode},
    },
};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

mod support;
use support::make_payload;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn small_headers() -> Vec<(&'static [u8], &'static [u8])> {
    vec![
        (b":method" as &[u8], b"POST" as &[u8]),
        (b":path", b"/grpc.test.TestService/UnaryCall"),
        (b":scheme", b"https"),
        (b":authority", b"localhost:8080"),
        (b"content-type", b"application/grpc"),
    ]
}

fn large_headers() -> Vec<(&'static [u8], &'static [u8])> {
    vec![
        (b":method" as &[u8], b"POST" as &[u8]),
        (b":path", b"/grpc.test.TestService/StreamingOutputCall"),
        (b":scheme", b"https"),
        (b":authority", b"my-service.example.com:443"),
        (b"content-type", b"application/grpc+proto"),
        (b"te", b"trailers"),
        (b"grpc-encoding", b"gzip"),
        (b"grpc-accept-encoding", b"gzip, identity"),
        (b"grpc-timeout", b"10S"),
        (b"user-agent", b"compio-grpc/0.1.0"),
        (b"x-request-id", b"550e8400-e29b-41d4-a716-446655440000"),
        (
            b"authorization",
            b"Bearer eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.test",
        ),
        (b"x-custom-header-1", b"some-value-one"),
        (b"x-custom-header-2", b"some-value-two"),
        (b"x-custom-header-3", b"some-value-three"),
        (b"x-custom-header-4", b"some-value-four"),
        (b"x-custom-header-5", b"some-value-five"),
        (b"accept-encoding", b"identity"),
        (b"x-forwarded-for", b"192.168.1.100"),
        (b"x-trace-id", b"abcdef1234567890abcdef1234567890"),
    ]
}

// ---------------------------------------------------------------------------
// HPACK benchmarks
// ---------------------------------------------------------------------------

fn bench_hpack_encode(c: &mut Criterion) {
    let mut group = c.benchmark_group("hpack_encode");

    // Pre-encode to measure encoded block sizes for throughput
    let small_hdrs = small_headers();
    let large_hdrs = large_headers();
    let small_encoded_size = {
        let mut enc = HpackEncoder::new(4096);
        let mut dst = Vec::new();
        enc.encode(small_hdrs.iter().copied(), &mut dst);
        dst.len() as u64
    };
    let large_encoded_size = {
        let mut enc = HpackEncoder::new(4096);
        let mut dst = Vec::new();
        enc.encode(large_hdrs.iter().copied(), &mut dst);
        dst.len() as u64
    };

    group.throughput(Throughput::Bytes(small_encoded_size));
    group.bench_function(BenchmarkId::new("small", "5 headers"), |b| {
        let hdrs = small_headers();
        b.iter(|| {
            let mut encoder = HpackEncoder::new(4096);
            let mut dst = Vec::with_capacity(256);
            encoder.encode(black_box(hdrs.iter().copied()), &mut dst);
            dst
        });
    });

    group.throughput(Throughput::Bytes(large_encoded_size));
    group.bench_function(BenchmarkId::new("large", "20 headers"), |b| {
        let hdrs = large_headers();
        b.iter(|| {
            let mut encoder = HpackEncoder::new(4096);
            let mut dst = Vec::with_capacity(1024);
            encoder.encode(black_box(hdrs.iter().copied()), &mut dst);
            dst
        });
    });

    group.throughput(Throughput::Bytes(small_encoded_size));
    group.bench_function(BenchmarkId::new("small_reuse", "5 headers"), |b| {
        let hdrs = small_headers();
        let mut encoder = HpackEncoder::new(4096);
        b.iter(|| {
            let mut dst = Vec::with_capacity(256);
            encoder.encode(black_box(hdrs.iter().copied()), &mut dst);
            dst
        });
    });

    group.finish();
}

fn bench_hpack_decode(c: &mut Criterion) {
    let mut group = c.benchmark_group("hpack_decode");

    let small_hdrs = small_headers();
    let mut small_enc = HpackEncoder::new(4096);
    let mut small_encoded = Vec::new();
    small_enc.encode(small_hdrs.iter().copied(), &mut small_encoded);

    let large_hdrs = large_headers();
    let mut large_enc = HpackEncoder::new(4096);
    let mut large_encoded = Vec::new();
    large_enc.encode(large_hdrs.iter().copied(), &mut large_encoded);

    group.throughput(Throughput::Bytes(small_encoded.len() as u64));
    group.bench_function(BenchmarkId::new("small", "5 headers"), |b| {
        b.iter(|| {
            let mut decoder = HpackDecoder::new(4096);
            decoder.decode(black_box(&small_encoded)).unwrap()
        });
    });

    group.throughput(Throughput::Bytes(large_encoded.len() as u64));
    group.bench_function(BenchmarkId::new("large", "20 headers"), |b| {
        b.iter(|| {
            let mut decoder = HpackDecoder::new(4096);
            decoder.decode(black_box(&large_encoded)).unwrap()
        });
    });

    group.throughput(Throughput::Bytes(small_encoded.len() as u64));
    group.bench_function(BenchmarkId::new("small_reuse", "5 headers"), |b| {
        let mut decoder = HpackDecoder::new(4096);
        b.iter(|| decoder.decode(black_box(&small_encoded)).unwrap());
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Huffman benchmarks
// ---------------------------------------------------------------------------

fn bench_huffman(c: &mut Criterion) {
    let mut group = c.benchmark_group("huffman");

    let short = b"www.example.com";
    let long =
        b"https://my-service.example.com:443/grpc.test.TestService/StreamingOutputCall?timeout=30s";

    group.throughput(Throughput::Bytes(short.len() as u64));
    group.bench_function(BenchmarkId::new("encode", "short"), |b| {
        b.iter(|| huffman_encode(black_box(short)));
    });

    group.throughput(Throughput::Bytes(long.len() as u64));
    group.bench_function(BenchmarkId::new("encode", "long"), |b| {
        b.iter(|| huffman_encode(black_box(long)));
    });

    let short_encoded = huffman_encode(short);
    let long_encoded = huffman_encode(long);

    group.throughput(Throughput::Bytes(short_encoded.len() as u64));
    group.bench_function(BenchmarkId::new("decode", "short"), |b| {
        b.iter(|| huffman_decode(black_box(&short_encoded)).unwrap());
    });

    group.throughput(Throughput::Bytes(long_encoded.len() as u64));
    group.bench_function(BenchmarkId::new("decode", "long"), |b| {
        b.iter(|| huffman_decode(black_box(&long_encoded)).unwrap());
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Frame header benchmarks
// ---------------------------------------------------------------------------

fn bench_frame_header(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame_header");

    group.bench_function("encode", |b| {
        let header = FrameHeader::new(FRAME_TYPE_DATA, 0x1, StreamId::new(1), 1024);
        b.iter(|| black_box(header).encode());
    });

    group.bench_function("decode", |b| {
        let header = FrameHeader::new(FRAME_TYPE_DATA, 0x1, StreamId::new(1), 1024);
        let encoded = header.encode();
        b.iter(|| FrameHeader::decode(black_box(&encoded)));
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Frame encode/decode benchmarks
// ---------------------------------------------------------------------------

fn bench_frame(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame");

    // DATA frame
    {
        let payload = make_payload(1024);
        let data_frame = Frame::Data(Data::new(StreamId::new(1), payload));

        let mut encoded = Vec::new();
        data_frame.encode(&mut encoded);

        group.throughput(Throughput::Bytes(1024));
        group.bench_function(BenchmarkId::new("encode", "DATA 1KB"), |b| {
            b.iter(|| {
                let mut dst = Vec::with_capacity(encoded.len());
                black_box(&data_frame).encode(&mut dst);
                dst
            });
        });

        group.throughput(Throughput::Bytes(encoded.len() as u64));
        group.bench_function(BenchmarkId::new("decode", "DATA 1KB"), |b| {
            let raw = encoded.clone();
            b.iter(|| {
                let header = FrameHeader::decode(black_box(raw[..9].try_into().unwrap()));
                Frame::decode(header, Bytes::copy_from_slice(&raw[9..])).unwrap()
            });
        });
    }

    // SETTINGS frame
    {
        let settings = Frame::Settings(Settings::new());
        let mut encoded = Vec::new();
        settings.encode(&mut encoded);

        group.throughput(Throughput::Bytes(encoded.len() as u64));
        group.bench_function(BenchmarkId::new("encode", "SETTINGS"), |b| {
            b.iter(|| {
                let mut dst = Vec::with_capacity(64);
                black_box(&settings).encode(&mut dst);
                dst
            });
        });

        group.bench_function(BenchmarkId::new("decode", "SETTINGS"), |b| {
            let raw = encoded.clone();
            b.iter(|| {
                let header = FrameHeader::decode(black_box(raw[..9].try_into().unwrap()));
                Frame::decode(header, Bytes::copy_from_slice(&raw[9..])).unwrap()
            });
        });
    }

    // HEADERS frame
    {
        let hdrs = small_headers();
        let mut hpack_enc = HpackEncoder::new(4096);
        let mut hpack_buf = Vec::new();
        hpack_enc.encode(hdrs.iter().copied(), &mut hpack_buf);
        let headers_frame = Frame::Headers(Headers::new(StreamId::new(1), Bytes::from(hpack_buf)));

        let mut encoded = Vec::new();
        headers_frame.encode(&mut encoded);

        group.throughput(Throughput::Bytes(encoded.len() as u64));
        group.bench_function(BenchmarkId::new("encode", "HEADERS"), |b| {
            b.iter(|| {
                let mut dst = Vec::with_capacity(encoded.len());
                black_box(&headers_frame).encode(&mut dst);
                dst
            });
        });

        group.bench_function(BenchmarkId::new("decode", "HEADERS"), |b| {
            let raw = encoded.clone();
            b.iter(|| {
                let header = FrameHeader::decode(black_box(raw[..9].try_into().unwrap()));
                Frame::decode(header, Bytes::copy_from_slice(&raw[9..])).unwrap()
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_hpack_encode,
    bench_hpack_decode,
    bench_huffman,
    bench_frame_header,
    bench_frame,
);
criterion_main!(benches);

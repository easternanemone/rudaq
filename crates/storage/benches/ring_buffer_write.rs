use std::time::Duration;

use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};
use storage::ring_buffer::RingBuffer;
use tempfile::TempDir;

const FRAME_SIZE_BYTES: usize = 1024;
const FRAME_COUNT: usize = 10_000;
const RING_BUFFER_MB: usize = 1;
const WARMUP_WRITES: usize = 10;

fn bench_ring_buffer_write(c: &mut Criterion) {
    let frame_data = vec![0xAB_u8; FRAME_SIZE_BYTES];
    let mut group = c.benchmark_group("ring_buffer_write_throughput");
    group.throughput(Throughput::Bytes((FRAME_SIZE_BYTES * FRAME_COUNT) as u64));
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(4));
    group.sample_size(20);

    group.bench_function("1kb_frames_10k_writes", |b| {
        b.iter_batched(
            || {
                let temp_dir = TempDir::new().expect("create temp dir for benchmark");
                let path = temp_dir.path().join("ring_buffer_write.bench");
                let rb = RingBuffer::create(&path, RING_BUFFER_MB)
                    .expect("create ring buffer for benchmark");
                for _ in 0..WARMUP_WRITES {
                    rb.write(&frame_data).expect("warm-up write");
                }
                (temp_dir, rb)
            },
            |(_temp_dir, rb)| {
                for _ in 0..FRAME_COUNT {
                    rb.write(&frame_data).expect("benchmark write");
                }
            },
            BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_ring_buffer_write);
criterion_main!(benches);

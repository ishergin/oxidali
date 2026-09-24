use criterion::{black_box, criterion_group, criterion_main, Criterion};
use dali2rust_bus::{BusChannel, BusConfig, BusFrame, BusHost};

fn bench_publish_latency(c: &mut Criterion) {
    c.bench_function("publish single frame", |b| {
        let (_host, publisher, ()) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                reg.subscribe_commands(16, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
            },
        );
        let frame = BusFrame::from_slice(&[1u8; 64]).unwrap();

        b.iter(|| {
            let result = publisher.try_publish(BusChannel::Commands, black_box(frame.clone()));
            black_box(result)
        });
    });
}

fn bench_fanout_throughput(c: &mut Criterion) {
    c.bench_function("fanout 5 subscribers", |b| {
        let (_host, publisher, ()) = BusHost::spawn(
            BusConfig::default(),
            |reg| {
                for _ in 0..5 {
                    reg.subscribe_commands(64, dali2rust_contracts::msg::COMMAND_VARIANT_NAMES);
                }
            },
        );
        let frame = BusFrame::from_slice(&[1u8; 32]).unwrap();

        b.iter(|| publisher.try_publish(BusChannel::Commands, black_box(frame.clone())));
    });
}

criterion_group!(benches, bench_publish_latency, bench_fanout_throughput);
criterion_main!(benches);

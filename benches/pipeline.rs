use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use ctrlb_decompose::extraction::drain3::Config;
use ctrlb_decompose::extraction::pipeline::ClpDrainPipeline;
use ctrlb_decompose::timestamp::{extract_timestamp, strip_timestamp};

/// Representative syslog templates — mirrors the synthetic generator in gen_syslog.py
const TEMPLATES: &[&str] = &[
    "Jan 12 02:53:00 web01 sshd[1234]: Accepted publickey for user42 from 10.0.1.15 port 52341 ssh2",
    "Jan 12 02:53:01 api03 sshd[2345]: session opened for user root by (uid=0)",
    "Jan 12 02:53:02 db01 sshd[3456]: session closed for user user17",
    "Jan 12 02:53:03 web02 sshd[4567]: Connection from 192.168.1.100 port 41234",
    "Jan 12 02:53:04 cache01 sshd[5678]: Invalid user admin from 192.168.0.50",
    "Jan 12 02:53:05 worker04 sshd[6789]: error: PAM: Authentication failure for illegal user user99 from 10.1.2.3",
    "Jan 12 02:53:06 web01 sshd[7890]: Disconnected from 172.16.0.1 port 39876 [preauth]",
    "Jan 12 02:53:07 api03 nginx[8901]: request completed in 234ms status=200 path=/api/v1/resource/58932",
    "Jan 12 02:53:08 db01 postgres[9012]: LOG:  duration: 1234.567 ms  statement: SELECT * FROM table WHERE id=88421",
    "Jan 12 02:53:09 web02 kernel[1]: kernel: [UFW BLOCK] IN=eth0 OUT= MAC=ff:ff:ff:ff:ff:ff SRC=10.0.0.1 DST=255.255.255.255 LEN=78 TOS=0x00 PREC=0x00 TTL=64 ID=12345 PROTO=UDP SPT=54321 DPT=67",
    "Jan 12 02:53:10 web01 sshd[1234]: Accepted publickey for user88 from 10.0.2.99 port 61234 ssh2",
    "Jan 12 02:53:11 api03 sshd[2346]: session opened for user root by (uid=0)",
    "Jan 12 02:53:12 db01 nginx[3457]: request completed in 45ms status=404 path=/api/v1/resource/1",
];

/// Benchmark the core per-line processing pipeline:
/// timestamp extraction + CLP encode + Drain3 cluster
/// This is the tight loop that runs on every single log line.
fn bench_process_line(c: &mut Criterion) {
    // Pre-strip timestamps so we isolate the CLP+Drain3 path
    let stripped_lines: Vec<String> = TEMPLATES
        .iter()
        .map(|line| {
            let ts = extract_timestamp(line);
            match &ts {
                Some(ts) => strip_timestamp(line, ts),
                None => line.to_string(),
            }
        })
        .collect();

    let mut group = c.benchmark_group("pipeline");
    group.throughput(Throughput::Elements(stripped_lines.len() as u64));

    group.bench_function("process_line_single_pass", |b| {
        b.iter(|| {
            let mut pipeline = ClpDrainPipeline::new(Config::default());
            for line in &stripped_lines {
                criterion::black_box(pipeline.process_line(line));
            }
        });
    });

    // Benchmark a warm pipeline (already clustered) to isolate steady-state cost
    group.bench_function("process_line_warm", |b| {
        let mut pipeline = ClpDrainPipeline::new(Config::default());
        // Warm up the cluster tree
        for line in &stripped_lines {
            pipeline.process_line(line);
        }
        b.iter(|| {
            for line in &stripped_lines {
                criterion::black_box(pipeline.process_line(line));
            }
        });
    });

    group.finish();
}

/// Benchmark at scale: N repetitions of the template set to approximate
/// what happens with a 500k-line file (500k / 13 templates ≈ 38k cycles)
fn bench_process_line_at_scale(c: &mut Criterion) {
    const REPS: usize = 5_000; // 5000 * 13 = 65k lines per bench iteration
    let lines: Vec<String> = TEMPLATES
        .iter()
        .cycle()
        .take(REPS * TEMPLATES.len())
        .map(|line| {
            let ts = extract_timestamp(line);
            match &ts {
                Some(ts) => strip_timestamp(line, ts),
                None => line.to_string(),
            }
        })
        .collect();

    let total_bytes: u64 = lines.iter().map(|l| l.len() as u64).sum();

    let mut group = c.benchmark_group("pipeline_scale");
    group.throughput(Throughput::Bytes(total_bytes));
    group.sample_size(10);

    group.bench_with_input(
        BenchmarkId::new("process_lines", format!("{}k", lines.len() / 1000)),
        &lines,
        |b, lines| {
            b.iter(|| {
                let mut pipeline = ClpDrainPipeline::new(Config::default());
                for line in lines {
                    criterion::black_box(pipeline.process_line(line));
                }
            });
        },
    );

    group.finish();
}

criterion_group!(benches, bench_process_line, bench_process_line_at_scale);
criterion_main!(benches);

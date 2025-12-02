//! Benchmark: simd-json vs serde_json parsing performance
//!
//! Run with: cargo bench --features noop

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use serde::Deserialize;

/// Generate session JSON data of varying message counts
fn generate_session_json(message_count: usize) -> String {
    let messages: Vec<String> = (0..message_count)
        .map(|i| {
            format!(
                r#"{{
                    "type": "assistant",
                    "timestamp": "2024-06-15T{:02}:00:00Z",
                    "content": "Response message number {} with some content to make it realistic and represent actual usage patterns",
                    "usage": {{
                        "input_tokens": {},
                        "output_tokens": {},
                        "cache_read_tokens": {},
                        "cache_write_tokens": 0
                    }},
                    "model": "claude-3-5-sonnet-20241022"
                }}"#,
                i % 24,
                i,
                100 + i * 10,
                50 + i * 5,
                i * 20
            )
        })
        .collect();

    format!(
        r#"{{
            "session_id": "bench-session-001",
            "project": "/path/to/project",
            "messages": [{}]
        }}"#,
        messages.join(",")
    )
}

/// Generate JSONL content (like Claude sessions)
fn generate_jsonl(line_count: usize) -> String {
    (0..line_count)
        .map(|i| {
            format!(
                r#"{{"type":"assistant","uuid":"msg-{}","message":{{"content":"Response {} with realistic content length for benchmarking purposes","model":"claude-3-5-sonnet-20241022"}},"costUSD":0.05,"cacheCreationInputTokens":{},"cacheReadInputTokens":{},"inputTokens":{},"outputTokens":{}}}"#,
                i, i, i * 10, i * 20, 100 + i, 50 + i
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Deserialize)]
struct BenchSession {
    #[allow(dead_code)]
    session_id: String,
    messages: Vec<BenchMessage>,
}

#[derive(Deserialize)]
struct BenchMessage {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    msg_type: String,
    #[allow(dead_code)]
    timestamp: String,
    usage: Option<BenchUsage>,
    #[allow(dead_code)]
    model: Option<String>,
}

#[derive(Deserialize)]
struct BenchUsage {
    #[allow(dead_code)]
    input_tokens: i64,
    #[allow(dead_code)]
    output_tokens: i64,
    #[allow(dead_code)]
    cache_read_tokens: Option<i64>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    msg_type: String,
    #[allow(dead_code)]
    uuid: String,
    #[allow(dead_code)]
    message: ClaudeContent,
    #[serde(rename = "costUSD")]
    #[allow(dead_code)]
    cost_usd: f64,
}

#[derive(Deserialize)]
struct ClaudeContent {
    #[allow(dead_code)]
    content: String,
    #[allow(dead_code)]
    model: String,
}

fn bench_json_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("json_parsing");

    for size in [10, 50, 100, 500].iter() {
        let json_str = generate_session_json(*size);
        let json_bytes = json_str.as_bytes();

        group.throughput(Throughput::Bytes(json_bytes.len() as u64));

        // Benchmark serde_json
        group.bench_with_input(
            BenchmarkId::new("serde_json", size),
            &json_str,
            |b, json| {
                b.iter(|| {
                    let result: BenchSession = serde_json::from_str(black_box(json)).unwrap();
                    black_box(result.messages.len())
                })
            },
        );

        // Benchmark simd-json
        group.bench_with_input(BenchmarkId::new("simd_json", size), &json_str, |b, json| {
            b.iter(|| {
                let mut bytes = json.as_bytes().to_vec();
                let result: BenchSession = simd_json::from_slice(black_box(&mut bytes)).unwrap();
                black_box(result.messages.len())
            })
        });
    }

    group.finish();
}

fn bench_jsonl_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("jsonl_parsing");

    for size in [50, 100, 500, 1000].iter() {
        let jsonl_content = generate_jsonl(*size);

        group.throughput(Throughput::Bytes(jsonl_content.len() as u64));

        // Benchmark serde_json line-by-line
        group.bench_with_input(
            BenchmarkId::new("serde_json", size),
            &jsonl_content,
            |b, content| {
                b.iter(|| {
                    let mut count = 0;
                    for line in content.lines() {
                        if let Ok(msg) = serde_json::from_str::<ClaudeMessage>(black_box(line)) {
                            count += 1;
                            black_box(&msg);
                        }
                    }
                    black_box(count)
                })
            },
        );

        // Benchmark simd-json line-by-line
        group.bench_with_input(
            BenchmarkId::new("simd_json", size),
            &jsonl_content,
            |b, content| {
                b.iter(|| {
                    let mut count = 0;
                    for line in content.lines() {
                        let mut bytes = line.as_bytes().to_vec();
                        if let Ok(msg) =
                            simd_json::from_slice::<ClaudeMessage>(black_box(&mut bytes))
                        {
                            count += 1;
                            black_box(&msg);
                        }
                    }
                    black_box(count)
                })
            },
        );
    }

    group.finish();
}

fn bench_token_extraction(c: &mut Criterion) {
    let mut group = c.benchmark_group("token_extraction");

    // Realistic session with varied token usage
    let session_json = generate_session_json(200);

    group.throughput(Throughput::Bytes(session_json.len() as u64));

    // Benchmark: Parse and extract total tokens with serde_json
    group.bench_function("serde_json_extract_tokens", |b| {
        b.iter(|| {
            let session: BenchSession = serde_json::from_str(black_box(&session_json)).unwrap();
            let total: i64 = session
                .messages
                .iter()
                .filter_map(|m| m.usage.as_ref())
                .map(|u| u.input_tokens + u.output_tokens + u.cache_read_tokens.unwrap_or(0))
                .sum();
            black_box(total)
        })
    });

    // Benchmark: Parse and extract total tokens with simd-json
    group.bench_function("simd_json_extract_tokens", |b| {
        b.iter(|| {
            let mut bytes = session_json.as_bytes().to_vec();
            let session: BenchSession = simd_json::from_slice(black_box(&mut bytes)).unwrap();
            let total: i64 = session
                .messages
                .iter()
                .filter_map(|m| m.usage.as_ref())
                .map(|u| u.input_tokens + u.output_tokens + u.cache_read_tokens.unwrap_or(0))
                .sum();
            black_box(total)
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_json_parsing,
    bench_jsonl_parsing,
    bench_token_extraction
);
criterion_main!(benches);

use anyhow::Result;
use clap::Parser;
use futures::stream::FuturesUnordered;
use metrics_util::parse_quantiles;
use rand::Rng;
use statrs::statistics::Data;
use statrs::statistics::Distribution;
use statrs::statistics::Max;
use statrs::statistics::Min;
use statrs::statistics::OrderStatistics;
use statrs::statistics::Statistics;
use std::{
    net::IpAddr,
    thread::{self, JoinHandle},
    time::Instant,
};
mod connection;
use connection::Connection;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Port to connect to the server
    #[arg(short, long, default_value_t = 3000)]
    port: u16,
    /// Number of clients
    #[clap(short, long, default_value_t = 5)]
    clients: usize,
    /// Number of requests per client
    #[clap(short, long, default_value_t = 10)]
    requests: usize,
    /// Length of the random key in bytes
    #[clap(short, long, default_value_t = 4)]
    key_length: usize,
}

fn main() -> Result<()> {
    let args = Args::parse();
    // define the address and port the server will listen on
    let addr: (IpAddr, u16) = ([127, 0, 0, 1].into(), args.port);

    let benchmark_start = Instant::now();

    let mut handles: Vec<JoinHandle<Result<Vec<u128>>>> = vec![];
    for _ in 0..args.clients {
        let addr = addr.clone();
        let key_length = args.key_length;
        let requests = args.requests;
        handles.push(thread::spawn(move || {
            let mut stream = Connection::new(addr)?;
            let mut rng = rand::thread_rng();
            let mut req_latencies = vec![];
            for _ in 0..requests {
                let key: String = (0..key_length)
                    .map(|_| rng.gen_range(b'A'..=b'Z') as char)
                    .collect();
                let value: String = (0..key_length)
                    .map(|_| rng.gen_range(b'A'..=b'Z') as char)
                    .collect();

                let start = Instant::now();
                stream.set(&key.as_bytes(), &value.as_bytes()).unwrap();
                stream.read_blocking().unwrap();
                let elapsed = start.elapsed().as_micros();
                req_latencies.push(elapsed);
                let start = Instant::now();
                stream.get(&key.as_bytes()).unwrap();
                stream.read_blocking().unwrap();
                let elapsed = start.elapsed().as_micros();
                req_latencies.push(elapsed);
                let start = Instant::now();
                stream.delete(&key.as_bytes()).unwrap();
                stream.read_blocking().unwrap();
                let elapsed = start.elapsed().as_micros();
                req_latencies.push(elapsed);
            }
            return Ok(req_latencies);
        }));
    }

    let mut all_latencies = vec![];
    for handle in handles {
        match handle.join().unwrap() {
            Ok(latencies) => all_latencies.extend(latencies),
            Err(e) => eprintln!("Error: {:?}", e),
        }
    }

    let all_latencies_f64: Vec<f64> = all_latencies.iter().map(|&x| x as f64).collect();
    let mut data = Data::new(all_latencies_f64);

    let min = data.min();
    let max = data.max();
    let avg = data.mean().unwrap();
    let p50 = data.percentile(50);
    let p95 = data.percentile(95);
    let p99 = data.percentile(99);
    println!(
        "Min: {:.2}, Max: {:.2}, Average: {:.2}, P50: {:.2}, P95: {:.2}, P99: {:.2}",
        min, max, avg, p50, p95, p99
    );

    let total_elasped = benchmark_start.elapsed().as_micros();

    println!(
        "OPs/S: {:.2}",
        all_latencies.len() as f64 / total_elasped as f64 * 1_000_000.0
    );

    // stream.set(b"hello", b"world")?;
    // let response = stream.read_blocking()?;
    // println!("Server said: {}", response);

    // stream.get(b"hello")?;
    // let response = stream.read_blocking()?;
    // println!("Server said: {}", response);

    // stream.delete(b"hello")?;
    // let response = stream.read_blocking()?;
    // println!("Server said: {}", response);

    Ok(())
}

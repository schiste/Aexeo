use aexeo_core::config::{Config, default_rule_switches};
use aexeo_core::run_runtime_audit;
use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn respond(mut stream: TcpStream, status: &str, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.len(),
        body
    );
    stream.write_all(response.as_bytes()).unwrap();
    stream.flush().unwrap();
}

fn benchmark_config() -> Config {
    Config {
        checks: default_rule_switches()
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<BTreeMap<_, _>>(),
        ..Config::default()
    }
}

fn spawn_runtime_fixture(page_count: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let started = Instant::now();
        let mut served = 0usize;
        let mut last_request = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    let mut buffer = [0_u8; 4096];
                    let size = stream.read(&mut buffer).unwrap();
                    let request = String::from_utf8_lossy(&buffer[..size]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");
                    served += 1;
                    last_request = Instant::now();
                    match path {
                        "/" => {
                            let links = (0..page_count)
                                .map(|index| format!("<a href=\"/page-{index}\">Page {index}</a>"))
                                .collect::<Vec<_>>()
                                .join("");
                            respond(
                                stream,
                                "200 OK",
                                "text/html",
                                &format!(
                                    "<html lang=\"en\"><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://{address}/\"></head><body><h1>Home</h1>{links}</body></html>"
                                ),
                            );
                        }
                        "/robots.txt" => {
                            respond(stream, "200 OK", "text/plain", "User-agent: *\nAllow: /\n")
                        }
                        "/llms.txt" => respond(stream, "404 Not Found", "text/plain", "missing"),
                        "/sitemap.xml" => {
                            respond(stream, "404 Not Found", "application/xml", "missing")
                        }
                        other if other.starts_with("/page-") => respond(
                            stream,
                            "200 OK",
                            "text/html",
                            &format!(
                                "<html lang=\"en\"><head><title>{other}</title><meta name=\"description\" content=\"{other}\"><link rel=\"canonical\" href=\"http://{address}{other}\"></head><body><h1>{other}</h1><section data-ui=\"answer\"><h2>Answer</h2><p>Benchmark runtime fixture content for {other}.</p></section></body></html>"
                            ),
                        ),
                        _ => respond(stream, "404 Not Found", "text/plain", "missing"),
                    }
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if (served >= 1 && last_request.elapsed() > Duration::from_millis(150))
                        || started.elapsed() > Duration::from_secs(30)
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("benchmark fixture accept failed: {error}"),
            }
        }
    });
    (format!("http://{address}"), handle)
}

fn benchmark_once(config: &Config) -> Duration {
    let (base_url, handle) = spawn_runtime_fixture(20);
    let started = Instant::now();
    let _ = run_runtime_audit(&base_url, 40, "http", config).unwrap();
    handle.join().unwrap();
    started.elapsed()
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let json_mode = args.iter().any(|arg| arg == "--json");
    let iterations = args
        .iter()
        .find(|arg| arg.as_str() != "--json")
        .cloned()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(10);
    let config = benchmark_config();
    let mut total = Duration::ZERO;
    for _ in 0..iterations {
        total += benchmark_once(&config);
    }
    let avg_ms = total.as_millis() / iterations as u128;
    if json_mode {
        println!(
            "{}",
            serde_json::json!({
                "name": "runtime_audit/http_fixture_20_pages",
                "iterations": iterations,
                "total_ms": total.as_millis(),
                "avg_ms": avg_ms,
            })
        );
    } else {
        println!(
            "runtime_audit/http_fixture_20_pages: iterations={} total_ms={} avg_ms={}",
            iterations,
            total.as_millis(),
            avg_ms
        );
    }
}

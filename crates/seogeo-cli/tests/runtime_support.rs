use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

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

pub fn spawn_server(expected_requests: usize) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        for _ in 0..expected_requests {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 4096];
            let size = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..size]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");
            match path {
                "/" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><title>Home</title><meta name=\"description\" content=\"Root\"><link rel=\"canonical\" href=\"http://example.test/\"></head><body><h1>Home</h1><a href=\"/about\">About</a></body></html>",
                ),
                "/about" => respond(
                    stream,
                    "200 OK",
                    "text/html",
                    "<html><head><meta name=\"description\" content=\"About page\"></head><body><h1>About</h1></body></html>",
                ),
                "/robots.txt" => {
                    respond(stream, "200 OK", "text/plain", "User-agent: *\nAllow: /\n")
                }
                "/llms.txt" => respond(
                    stream,
                    "200 OK",
                    "text/plain",
                    "# Site\n\n## Pages\n- [Home](/)\n",
                ),
                "/sitemap.xml" => respond(
                    stream,
                    "200 OK",
                    "application/xml",
                    "<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>http://example.test/</loc></url><url><loc>http://example.test/about</loc></url></urlset>",
                ),
                _ => respond(stream, "404 Not Found", "text/plain", "missing"),
            }
        }
    });
    (format!("http://{}", address), handle)
}

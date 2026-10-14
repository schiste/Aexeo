use std::io::ErrorKind;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Sender, TryRecvError};
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

pub struct ManagedTestServer {
    base_url: String,
    shutdown: Sender<()>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ManagedTestServer {
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn shutdown(mut self) {
        self.shutdown_inner().expect("test server thread panicked");
    }

    fn shutdown_inner(&mut self) -> thread::Result<()> {
        let _ = self.shutdown.send(());
        match self.handle.take() {
            Some(handle) => handle.join(),
            None => Ok(()),
        }
    }
}

impl Drop for ManagedTestServer {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

pub fn spawn_server(min_requests: usize) -> (String, thread::JoinHandle<()>) {
    let min_requests = min_requests.max(1);
    spawn_server_with_stop_condition(move |served, started, last_request| {
        (served >= min_requests && last_request.elapsed() > Duration::from_millis(150))
            || started.elapsed() > Duration::from_secs(30)
    })
}

pub fn spawn_server_with_explicit_shutdown() -> ManagedTestServer {
    let (shutdown, shutdown_receiver) = mpsc::channel();
    let (base_url, handle) = spawn_server_with_stop_condition(move |_, _, _| {
        matches!(
            shutdown_receiver.try_recv(),
            Ok(()) | Err(TryRecvError::Disconnected)
        )
    });
    ManagedTestServer {
        base_url,
        shutdown,
        handle: Some(handle),
    }
}

fn spawn_server_with_stop_condition(
    mut should_stop: impl FnMut(usize, Instant, Instant) -> bool + Send + 'static,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut served = 0usize;
        let started = Instant::now();
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
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    if should_stop(served, started, last_request) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("server accept failed: {error}"),
            }
        }
    });
    (format!("http://{}", address), handle)
}

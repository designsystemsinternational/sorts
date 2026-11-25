//! A small HTTP server on 127.0.0.1 that serves the preview pages and their font files, and
//! the previewer that opens them in the browser.
//!
//! Browsers do not load fonts from `file://` pages reliably, so the page and the fonts come
//! from this server. Each URL starts with a random token. The server serves only the files
//! of a published page, by number, and never takes a path from the request. It stops when
//! the process ends.

use crate::preview::{self, Page, Previewer};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The server keeps this many pages. Older pages are removed.
const MAX_PAGES: usize = 20;

struct Published {
    number: usize,
    html: String,
    files: Vec<PathBuf>,
}

#[derive(Default)]
struct Pages {
    next: usize,
    items: Vec<Published>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

pub struct Server {
    address: SocketAddr,
    token: String,
    pages: Arc<Mutex<Pages>>,
}

impl Server {
    /// Starts the server on a free port of 127.0.0.1, in a background thread.
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let server = Self {
            address: listener.local_addr()?,
            token: uuid::Uuid::new_v4().simple().to_string(),
            pages: Arc::default(),
        };
        let (token, pages) = (server.token.clone(), server.pages.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (token, pages) = (token.clone(), pages.clone());
                // One thread per connection: a font that downloads first must not block the others.
                std::thread::spawn(move || {
                    let _ = handle(stream, &token, &pages);
                });
            }
        });
        Ok(server)
    }

    /// Adds a page and returns its URL. The URL ends with `/`.
    pub fn publish(&self, html: String, files: Vec<PathBuf>) -> String {
        let mut pages = self.pages.lock().unwrap();
        let number = pages.next;
        pages.next += 1;
        pages.items.push(Published { number, html, files });
        if pages.items.len() > MAX_PAGES {
            pages.items.remove(0);
        }
        format!("http://{}/{}/{number}/", self.address, self.token)
    }

    /// The response to a GET request for `path`.
    pub fn respond(&self, path: &str) -> Response {
        respond(&self.token, &self.pages, path)
    }
}

fn respond(token: &str, pages: &Mutex<Pages>, path: &str) -> Response {
    let not_found = Response { status: 404, content_type: "text/plain; charset=utf-8", body: b"Not found".to_vec() };
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let parts: Vec<&str> = path.split('/').collect();
    let (page, rest) = match parts.as_slice() {
        ["", t, page, rest @ ..] if *t == token => (*page, rest),
        _ => return not_found,
    };
    let pages = pages.lock().unwrap();
    let Some(page) = page.parse::<usize>().ok().and_then(|n| pages.items.iter().find(|p| p.number == n)) else {
        return not_found;
    };
    match rest {
        [""] => {
            Response { status: 200, content_type: "text/html; charset=utf-8", body: page.html.clone().into_bytes() }
        }
        ["font", n] => {
            let Some(file) = n.parse::<usize>().ok().and_then(|n| page.files.get(n)).cloned() else {
                return not_found;
            };
            drop(pages);
            // The read downloads an online-only file first.
            match std::fs::read(&file) {
                Ok(body) => Response { status: 200, content_type: content_type(&file), body },
                Err(_) => not_found,
            }
        }
        _ => not_found,
    }
}

fn content_type(file: &std::path::Path) -> &'static str {
    match file.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
        Some("otf") => "font/otf",
        Some("ttf") => "font/ttf",
        Some("ttc" | "otc") => "font/collection",
        _ => "application/octet-stream",
    }
}

fn handle(stream: TcpStream, token: &str, pages: &Mutex<Pages>) -> io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    reader.read_line(&mut request)?;
    // Read the headers up to the empty line. They are not needed.
    let mut line = String::new();
    while reader.read_line(&mut line)? > 2 {
        line.clear();
    }
    let response = match request.split_whitespace().collect::<Vec<_>>().as_slice() {
        ["GET", path, _] => respond(token, pages, path),
        _ => Response { status: 405, content_type: "text/plain; charset=utf-8", body: b"Method not allowed".to_vec() },
    };
    let reason = match response.status {
        200 => "OK",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let mut stream = &stream;
    write!(
        stream,
        "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}

/// Opens preview pages in the default browser. Starts the server at the first preview.
#[derive(Default)]
pub struct BrowserPreviewer {
    server: Option<Server>,
}

impl Previewer for BrowserPreviewer {
    fn show(&mut self, page: &Page, files: Vec<PathBuf>) -> anyhow::Result<()> {
        let server = match &mut self.server {
            Some(server) => server,
            None => self.server.insert(Server::start()?),
        };
        let url = server.publish(preview::html(page), files);
        let status = std::process::Command::new("open").arg(&url).status()?;
        anyhow::ensure!(status.success(), "Could not open the browser for {url}.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn get(url: &str) -> (String, Vec<u8>) {
        let rest = url.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_at(rest.find('/').unwrap());
        let mut stream = TcpStream::connect(host).unwrap();
        write!(stream, "GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n").unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let split = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (String::from_utf8_lossy(&response[..split]).into_owned(), response[split + 4..].to_vec())
    }

    fn path_of(url: &str) -> String {
        let rest = url.strip_prefix("http://").unwrap();
        rest[rest.find('/').unwrap()..].to_string()
    }

    #[test]
    fn serves_the_page_and_its_fonts_over_http() {
        let dir = tempfile::tempdir().unwrap();
        let font = dir.path().join("Sans.otf");
        std::fs::write(&font, b"font bytes").unwrap();
        let server = Server::start().unwrap();
        let url = server.publish("<p>page</p>".into(), vec![font]);
        assert!(url.starts_with("http://127.0.0.1:"));
        assert!(url.ends_with('/'));

        let (head, body) = get(&url);
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        assert!(head.contains("Content-Type: text/html; charset=utf-8"));
        assert_eq!(body, b"<p>page</p>");

        let (head, body) = get(&format!("{url}font/0"));
        assert!(head.contains("Content-Type: font/otf"), "{head}");
        assert_eq!(body, b"font bytes");
    }

    #[test]
    fn unknown_paths_and_a_wrong_token_give_404() {
        let dir = tempfile::tempdir().unwrap();
        let font = dir.path().join("Sans.ttf");
        std::fs::write(&font, b"font bytes").unwrap();
        let server = Server::start().unwrap();
        let path = path_of(&server.publish("page".into(), vec![font]));
        assert_eq!(server.respond(&format!("{path}font/0")).content_type, "font/ttf");
        assert_eq!(server.respond(&format!("{path}font/0?x=1")).status, 200);

        let token = path.split('/').nth(1).unwrap();
        for wrong in [
            format!("{path}font/1"),
            format!("{path}font/x"),
            format!("{path}font/../0"),
            format!("{path}other"),
            format!("/{token}/9/"),
            format!("/{token}/"),
            path.replace(token, "wrong"),
            "/".to_string(),
            "/favicon.ico".to_string(),
        ] {
            assert_eq!(server.respond(&wrong).status, 404, "{wrong}");
        }
    }

    #[test]
    fn a_file_that_cannot_be_read_gives_404() {
        let server = Server::start().unwrap();
        let path = path_of(&server.publish("page".into(), vec!["/does/not/exist.otf".into()]));
        assert_eq!(server.respond(&format!("{path}font/0")).status, 404);
    }

    #[test]
    fn each_page_has_its_own_url_and_old_pages_are_removed() {
        let server = Server::start().unwrap();
        let first = path_of(&server.publish("first".into(), vec![]));
        let second = path_of(&server.publish("second".into(), vec![]));
        assert_ne!(first, second);
        assert_eq!(server.respond(&first).body, b"first");
        assert_eq!(server.respond(&second).body, b"second");
        for _ in 0..MAX_PAGES {
            server.publish("more".into(), vec![]);
        }
        assert_eq!(server.respond(&first).status, 404);
    }
}

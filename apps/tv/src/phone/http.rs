//! The little HTTP/1.1 the phone's page needs: one request read within a
//! size, its method, path and form, and one answer with the connection
//! closed after it.

use std::io::{Read, Write};

use mana_core::catalog::update::MAX_URL;

/// The largest request read: the longest list URL encoded whole in the
/// form (`%XX` for each byte), with room for the browser's headers.
pub const MAX_REQUEST: usize = 3 * MAX_URL + HEAD_ROOM;
const HEAD_ROOM: usize = 4096;

#[derive(Debug, PartialEq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub body: Vec<u8>,
}

/// Why a request was not read, as the status to answer.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// Not HTTP, or cut short.
    BadRequest,
    /// Neither GET nor POST.
    Method,
    /// Larger than MAX_REQUEST.
    TooLarge,
}

impl Refused {
    pub fn status(&self) -> u16 {
        match self {
            Refused::BadRequest => 400,
            Refused::Method => 405,
            Refused::TooLarge => 413,
        }
    }
}

/// The request `stream` sends, head and body, of at most MAX_REQUEST bytes;
/// none when it closes or stays silent before sending anything.
pub fn read(stream: &mut impl Read) -> Result<Option<Request>, Refused> {
    let mut data = Vec::new();
    let mut chunk = [0; 4096];
    let Some(head_end) = read_head(stream, &mut data, &mut chunk)? else { return Ok(None) };
    let (method, path, length) = parse_head(&data[..head_end])?;
    let body_start = head_end + 4;
    if body_start + length > MAX_REQUEST {
        return Err(Refused::TooLarge);
    }
    while data.len() < body_start + length {
        fill(stream, &mut data, &mut chunk)?;
    }
    let body = data[body_start..body_start + length].to_vec();
    Ok(Some(Request { method, path, body }))
}

/// Reads into `data` up to the end of the request's head, and returns
/// where it ends; none when nothing was sent.
fn read_head(stream: &mut impl Read, data: &mut Vec<u8>, chunk: &mut [u8]) -> Result<Option<usize>, Refused> {
    loop {
        if let Some(end) = find(data, b"\r\n\r\n") {
            return Ok(Some(end));
        }
        match fill(stream, data, chunk) {
            Err(Refused::BadRequest) if data.is_empty() => return Ok(None),
            filled => filled?,
        }
    }
}

/// The method, path and body length the request's head gives.
fn parse_head(head: &[u8]) -> Result<(Method, String, usize), Refused> {
    let head = std::str::from_utf8(head).map_err(|_| Refused::BadRequest)?;
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let method = match first.next() {
        Some("GET") => Method::Get,
        Some("POST") => Method::Post,
        Some(_) => return Err(Refused::Method),
        None => return Err(Refused::BadRequest),
    };
    let path = first.next().filter(|p| p.starts_with('/')).ok_or(Refused::BadRequest)?.to_string();
    let length = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim().parse::<usize>().map_err(|_| Refused::BadRequest))
        .transpose()?
        .unwrap_or(0);
    Ok((method, path, length))
}

/// Reads more of the request into `data`.
fn fill(stream: &mut impl Read, data: &mut Vec<u8>, chunk: &mut [u8]) -> Result<(), Refused> {
    let read = stream.read(chunk).map_err(|_| Refused::BadRequest)?;
    if read == 0 {
        return Err(Refused::BadRequest);
    }
    data.extend_from_slice(&chunk[..read]);
    if data.len() > MAX_REQUEST {
        return Err(Refused::TooLarge);
    }
    Ok(())
}

fn find(data: &[u8], needle: &[u8]) -> Option<usize> {
    data.windows(needle.len()).position(|w| w == needle)
}

/// The value of `name` in a form sent as `application/x-www-form-urlencoded`.
pub fn form_value(body: &[u8], name: &str) -> Option<String> {
    url::form_urlencoded::parse(body).find(|(key, _)| key == name).map(|(_, value)| value.into_owned())
}

/// Writes the answer `status` with the HTML `body`, then the connection
/// closes.
pub fn respond(stream: &mut impl Write, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Content Too Large",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that gives `data` a few bytes at a time, as a socket may.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = buf.len().min(self.0.len()).min(7);
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    fn parse(text: &str) -> Result<Request, Refused> {
        read(&mut Trickle(text.as_bytes())).map(|r| r.expect("a request"))
    }

    /// A connection a browser opened ahead and closed unused sent nothing:
    /// no answer is due.
    #[test]
    fn nothing_sent_is_no_request() {
        assert_eq!(read(&mut Trickle(b"")), Ok(None));
        assert_eq!(read(&mut Trickle(b"GET")), Err(Refused::BadRequest));
    }

    #[test]
    fn reads_a_get_and_a_post_with_its_body() {
        let get = parse("GET /K7QM HTTP/1.1\r\nHost: tv\r\n\r\n").unwrap();
        assert_eq!((get.method, get.path.as_str(), get.body.len()), (Method::Get, "/K7QM", 0));
        let body = "url=https%3A%2F%2Fa.example%2Fl.toml%3Fx%3D1%26y%3D2";
        let post = parse(&format!("POST /K7QM HTTP/1.1\r\ncontent-length: {}\r\n\r\n{body}", body.len())).unwrap();
        assert_eq!(post.method, Method::Post);
        assert_eq!(form_value(&post.body, "url").as_deref(), Some("https://a.example/l.toml?x=1&y=2"));
        assert_eq!(form_value(b"other=1", "url"), None);
        assert_eq!(form_value(b"url=a+b", "url").as_deref(), Some("a b"));
    }

    #[test]
    fn refuses_other_methods_short_requests_and_large_ones() {
        assert_eq!(parse("PUT /K7QM HTTP/1.1\r\n\r\n"), Err(Refused::Method));
        assert_eq!(parse("GET /K7QM HTTP/1.1\r\n"), Err(Refused::BadRequest));
        assert_eq!(parse("GET K7QM HTTP/1.1\r\n\r\n"), Err(Refused::BadRequest));
        assert_eq!(parse("POST / HTTP/1.1\r\nContent-Length: x\r\n\r\n"), Err(Refused::BadRequest));
        assert_eq!(parse("POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc"), Err(Refused::BadRequest));
        let large = format!("POST / HTTP/1.1\r\nContent-Length: {MAX_REQUEST}\r\n\r\n");
        assert_eq!(parse(&large), Err(Refused::TooLarge));
        let endless = format!("GET / HTTP/1.1\r\nX: {}", "a".repeat(MAX_REQUEST));
        assert_eq!(parse(&endless), Err(Refused::TooLarge));
    }

    #[test]
    fn a_whole_encoded_url_of_the_largest_length_fits() {
        let body = format!("url={}", "%41".repeat(MAX_URL));
        let request = format!("POST /K7QM HTTP/1.1\r\nUser-Agent: {}\r\nContent-Length: {}\r\n\r\n{body}", "u".repeat(1000), body.len());
        let read = parse(&request).unwrap();
        assert_eq!(form_value(&read.body, "url").map(|u| u.len()), Some(MAX_URL));
    }
}

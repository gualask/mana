use std::io::{Read, Write};

use super::*;

#[test]
fn a_code_has_four_unambiguous_characters() {
    for _ in 0..100 {
        let code = code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|c| CODE_ALPHABET.contains(&c)), "{code}");
    }
}

/// Sends `request` to `local` and returns the answer.
fn send(local: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(local).unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

/// The server answers only the code's path, hands the URL to the app
/// and gives the phone its answer, then closes the port when dropped. A
/// connection opened ahead and left silent, as browsers do, holds up no
/// other and gets no answer.
#[test]
fn serves_the_code_hands_over_the_url_and_stops() {
    let Some(_) = local_ip() else { return };
    let server = Server::start().unwrap();
    let path = server.address.rsplit_once('/').map(|(_, c)| format!("/{c}")).unwrap();
    server.set_current(Some(&Url::parse("https://a.example/prima.toml").unwrap()));
    let mut silent = TcpStream::connect(server.local).unwrap();

    assert!(send(server.local, "GET /ZZZZZ HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    let page = send(server.local, &format!("GET {} HTTP/1.1\r\n\r\n", path.to_lowercase()));
    assert!(page.starts_with("HTTP/1.1 200") && page.contains("https://a.example/prima.toml"));

    let refused = send(server.local, &format!("POST {path} HTTP/1.1\r\nContent-Length: 11\r\n\r\nurl=ftp%3A1"));
    assert!(refused.contains("Non salvata"));

    let body = "url=https%3A%2F%2Fa.example%2Fl.toml";
    let post = format!("POST {path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let local = server.local;
    let phone = std::thread::spawn(move || send(local, &post));
    let request = loop {
        if let Some(request) = server.request() {
            break request;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(request.url.as_str(), "https://a.example/l.toml");
    request.answer(true, "Lista salvata: 3 canali".into());
    assert!(phone.join().unwrap().contains("Lista salvata: 3 canali"));

    silent.shutdown(std::net::Shutdown::Write).unwrap();
    let mut unanswered = String::new();
    silent.read_to_string(&mut unanswered).unwrap();
    assert_eq!(unanswered, "");

    drop(server);
    std::thread::sleep(Duration::from_millis(100));
    assert!(TcpStream::connect_timeout(&local, Duration::from_millis(500)).is_err());
}

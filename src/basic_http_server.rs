use color_eyre::eyre::{eyre, Result, OptionExt};
use tracing::*;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::fs::File;
use bytes::BytesMut;

pub struct BasicHttpServer {
    listener: TcpListener,
    dir: String,
}

enum HttpEncoding {
    Gzip,
}

enum ParseResult {
    Get {
        close: bool,
        path: String,
        ua: Option<String>,
        encoding: Option<HttpEncoding>,
    },
    Post {
        close: bool,
        path: String,
        body_offset: usize,
        body_len: usize,
    }
}

impl BasicHttpServer {
    pub async fn new(addr: &str, dir: &str) -> Result<BasicHttpServer> {
        let listener = TcpListener::bind(addr).await?;
        let dir = dir.to_owned();

        info!("started server on {addr} serving files from {dir}");
        Ok(BasicHttpServer {
            listener,
            dir,
        })
    }

    pub async fn run(&self) -> Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;

            tokio::task::spawn(Self::handle_request(stream, self.dir.clone()));
        }
    }

    #[tracing::instrument]
    async fn handle_request(mut stream: TcpStream, dir: String)
    {
        info!("starting request handler");

        loop {
            let mut buf: BytesMut = Default::default();
            let parse_res = loop {
                match stream.read_buf(&mut buf).await {
                    Ok(0) => {
                        info!("connection closed");
                        return;
                    }
                    Ok(n) => {
                        info!("read {n} more bytes");
                    }
                    Err(err) => {
                        error!("read error: {err:?}");
                        return;
                    }
                }

                match Self::parse_request(&buf) {
                    Ok(Some(parse_res)) => break parse_res,
                    Ok(None) => (),
                    Err(err) => {
                        error!("read error: {err:?}");
                        return;
                    }
                }
            };

            let (resp, close_con) = match parse_res {
                ParseResult::Get { close, path, ua, encoding } => {
                    let resp = if path == "/" {
                        Self::response200pt(vec![], encoding)
                    } else if path.to_ascii_lowercase().starts_with("/echo") {
                        let body = path[6..].as_bytes().to_vec();
                        Self::response200pt(body, encoding)
                    } else if path.to_ascii_lowercase() == "/user-agent" {
                        let body = ua.unwrap_or("".to_string()).as_bytes().to_vec();
                        Self::response200pt(body, encoding)
                    } else if path.to_ascii_lowercase().starts_with("/files") {
                        let contents = Self::read_file(&path[6..], &dir).await;
                        match contents {
                            Ok(c) => {
                                Self::response200bin(c)
                            }
                            Err(e) => {
                                error!("File read error {e}");
                                Self::response404()
                            }
                        }
                    } else {
                        Self::response404()
                    };

                    (resp, close)
                },
                ParseResult::Post { close, path, body_offset, body_len } => {
                    let content_prefix = &buf[body_offset..];
                    match Self::write_file(&mut stream,
                                           &path[6..],
                                           &dir,
                                           content_prefix,
                                           body_len).await {
                        Ok(()) => {
                            (Self::response201(), close)
                        }
                        Err(e) => {
                            error!("File read error {e}");
                            return;
                        }
                    }
                }
            };

            if let Err(err) =
                stream
                .write_all(Self::serialize_response(resp).as_slice())
                .await {
                    error!("response write error: {err:?}");
                }

            if close_con {
                return;
            }
        }
    }

    fn parse_request(buf: &[u8]) -> Result<Option<ParseResult>>
    {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);
        let body_offset = match req.parse(buf)? {
            httparse::Status::Complete(offset) => {
                info!("parsed request: {req:?}");
                offset
            }
            httparse::Status::Partial => {
                info!("partial request parse result");
                return Ok(None)
            }
        };

        match req.method {
            Some("GET") => {
                let path = req.path
                    .ok_or_eyre("missing request method")?
                    .to_string();
                let mut close = false;
                let mut ua = None;
                let mut encoding = None;
                for header in headers {
                    if header.name.eq_ignore_ascii_case("connection") {
                        close = std::str::from_utf8(header.value)?
                            .eq_ignore_ascii_case("close");
                    } else if header.name.eq_ignore_ascii_case("user-agent") {
                        ua = Some(std::str::from_utf8(header.value)?.to_owned());
                    } else if header.name.eq_ignore_ascii_case("accept-encoding") {
                        encoding = Self::parse_encoding(std::str::from_utf8(header.value)?);
                    }
                }

                Ok(Some(ParseResult::Get {
                    close,
                    path,
                    ua,
                    encoding,
                }))
            },
            Some("POST") => {
                let path = req.path
                    .ok_or_eyre("missing request method")?
                    .to_string();
                let mut close = false;
                let mut body_len: usize = 0;
                for header in headers {
                    if header.name.eq_ignore_ascii_case("connection") {
                        close = std::str::from_utf8(header.value)?
                            .eq_ignore_ascii_case("close");
                    } else if header.name.eq_ignore_ascii_case("content-length") {
                        body_len = std::str::from_utf8(header.value)?.parse()?;
                    }
                }

                Ok(Some(ParseResult::Post {
                    close,
                    path,
                    body_offset,
                    body_len,
                }))
            },
            Some(method) => {
                Err(eyre!("unsupported request method {method}"))
            }
            None => {
                Err(eyre!("missing request method"))
            }
        }
    }

    fn parse_encoding(encoding: &str) -> Option<HttpEncoding> {
        if encoding.to_ascii_lowercase().contains("gzip") {
            Some(HttpEncoding::Gzip)
        } else {
            None
        }
    }

    fn response200(body: Vec<u8>, cont_type: String, encoding: Option<HttpEncoding>) -> http::Response<Vec<u8>> {
        let res = http::response::Builder::new()
            .status(200)
            .header("Content-length", body.len())
            .header("Content-type", cont_type);
        let res = match encoding {
            Some(HttpEncoding::Gzip) => {
                res.header("Content-encoding", "gzip")
            }
            None => res,
        };
        res.body(body).unwrap()
    }

    fn response200pt(body: Vec<u8>, encoding: Option<HttpEncoding>) -> http::Response<Vec<u8>> {
        Self::response200(body, "text/plain".to_string(), encoding)
    }

    fn response200bin(body: Vec<u8>) -> http::Response<Vec<u8>> {
        Self::response200(body, "application/octet-stream".to_string(), None)
    }

    fn response201() -> http::Response<Vec<u8>> {
        http::response::Builder::new()
            .status(201)
            .header("Content-length", "0")
            .body(vec![])
            .unwrap()
    }

    fn response404() -> http::Response<Vec<u8>> {
        http::response::Builder::new()
            .status(404)
            .header("Content-length", "0")
            .body(vec![])
            .unwrap()
    }

    fn serialize_response<T>(resp: http::Response<T>) -> Vec<u8>
    where T: Into<Vec<u8>>{
        let mut serialized: Vec<u8> = Vec::new();

        let status_line = format!("HTTP/1.1 {} {}\r\n",
                                  resp.status().as_u16(),
                                  resp.status().canonical_reason().unwrap_or(""));
        serialized.append(&mut status_line.into());

        for (hname, hval) in resp.headers() {
            serialized.append(&mut format!("{}: {}\r\n",
                                           hname.as_str(),
                                           hval.to_str()
                                           .unwrap())
                              .into());
        }
        serialized.push(b'\r'); serialized.push(b'\n');

        let body = resp.into_body();
        serialized.append(&mut body.into());

        info!("prepared response {serialized:?}");
        serialized
    }

    async fn read_file(path: &str, dir: &str) -> Result<Vec<u8>> {
        let mut file = File::open(format!("{dir}{path}")).await?;
        let mut contents = vec![];
        file.read_to_end(&mut contents).await?;
        Ok(contents)
    }

    async fn write_file(stream: &mut TcpStream,
                        path: &str,
                        dir: &str,
                        content_prefix: &[u8],
                        content_len: usize) -> Result<()> {
        let mut file = File::create(format!("{dir}{path}")).await?;
        file.write_all(content_prefix).await?;

        let mut content_len = content_len - content_prefix.len();
        while content_len > 0 {
            let mut content_buf = vec![0u8; std::cmp::min(content_len, 65536)];
            let n = stream.read(&mut content_buf).await?;
            file.write_all(&content_buf[..n]).await?;

            content_len -= n;
        }

        Ok(())
    }
}

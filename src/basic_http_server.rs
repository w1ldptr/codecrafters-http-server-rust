use color_eyre::eyre::{eyre, Result, OptionExt};
use tracing::*;
use tokio::net::{TcpListener, TcpStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use bytes::BytesMut;

pub struct BasicHttpServer {
    listener: TcpListener,
}

enum ParseResult {
    Get {
        close: bool,
        path: String,
    }
}

impl BasicHttpServer {
    pub async fn new(addr: &str) -> Result<BasicHttpServer> {
        let listener = TcpListener::bind(addr).await?;

        info!("started server on {addr}");
        Ok(BasicHttpServer {
            listener,
        })
    }

    pub async fn run(&self) -> Result<()> {
        loop {
            let (stream, _) = self.listener.accept().await?;

            tokio::task::spawn(Self::handle_request(stream));
        }
    }

    #[tracing::instrument]
    async fn handle_request(mut stream: TcpStream)
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
                ParseResult::Get { close, path } => {
                    let resp = if path == "/" {
                        Self::response200()
                    } else {
                        Self::response404()
                    };

                    (resp, close)
                },
            };

            if let Err(err) =
                stream
                .write_all(Self::serialize_response(&resp).as_slice())
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
        match req.parse(buf)? {
            httparse::Status::Complete(_) => {
                info!("parsed request: {req:?}");
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
                for header in headers {
                    if header.name.eq_ignore_ascii_case("connection") {
                        close = std::str::from_utf8(header.value)?
                            .eq_ignore_ascii_case("close");
                    }
                }

                Ok(Some(ParseResult::Get {
                    close,
                    path,
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

    fn response200() -> http::Response<()> {
        http::response::Builder::new()
            .status(200)
            .header("Content-length", "0")
            .body(())
            .unwrap()
    }

    fn response404() -> http::Response<()> {
        http::response::Builder::new()
            .status(404)
            .header("Content-length", "0")
            .body(())
            .unwrap()
    }

    fn serialize_response(resp: &http::Response<()>) -> Vec<u8> {
        let mut serialized = Vec::new();

        let status_line = format!("HTTP/1.1 {} {}\r\n",
                                  resp.status().as_u16(),
                                  resp.status().canonical_reason().unwrap_or(""));
        serialized.append(&mut status_line.into_bytes());

        for (hname, hval) in resp.headers() {
            serialized.append(&mut format!("{}: {}\r\n",
                                           hname.as_str(),
                                           hval.to_str()
                                           .unwrap())
                              .into_bytes());
        }
        serialized.push(b'\r'); serialized.push(b'\n');

        serialized
    }
}

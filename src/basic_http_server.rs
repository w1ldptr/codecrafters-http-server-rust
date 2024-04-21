use color_eyre::eyre::Result;
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

            let close_con = match parse_res {
                ParseResult::Get { close } => {
                    close
                },
            };

            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n";
            if let Err(err) = stream.write_all(resp.as_bytes()).await {
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

        let mut close = false;
        for header in headers {
            if header.name.eq_ignore_ascii_case("connection") {
                close = std::str::from_utf8(header.value)?.eq_ignore_ascii_case("close");
            }
        }

        Ok(Some(ParseResult::Get {
            close,
        }))
    }
}

use std::net::TcpListener;
use color_eyre::eyre::Result;

fn main() -> Result<()> {
    color_eyre::install()?;

    println!("Logs from your program will appear here!");

    let listener = TcpListener::bind("127.0.0.1:4221")?;
    for stream in listener.incoming() {
        let _stream = stream?;
        println!("accepted new connection");
    }

    Ok(())
}

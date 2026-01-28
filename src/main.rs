use clap::Parser;
use tokio::net::TcpListener;

mod connection;
mod display;
mod flv;
mod rtmp;
mod stats;

#[derive(Parser, Debug)]
#[command(name = "rustmp", about = "RTMP stream analyzer")]
struct Args {
    /// Network interface to bind to (e.g., "0.0.0.0" or "127.0.0.1")
    interface: String,
    /// Port to listen on (e.g., 1935)
    port: u16,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();
    let addr = format!("{}:{}", args.interface, args.port);

    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    eprintln!("Listening for RTMP connections on {}", addr);

    // Handle Ctrl+C for clean shutdown
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, peer_addr)) => {
                        tokio::spawn(connection::handle_connection(stream, peer_addr));
                    }
                    Err(e) => {
                        eprintln!("Accept error: {}", e);
                    }
                }
            }
            _ = &mut shutdown => {
                eprintln!("\nShutting down...");
                display::restore_terminal();
                break;
            }
        }
    }
}

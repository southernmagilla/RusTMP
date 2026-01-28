use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const HANDSHAKE_SIZE: usize = 1536;

/// Performs the RTMP server-side handshake.
/// Returns any remaining bytes that arrived after the handshake completed.
pub async fn perform_handshake(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    // ── Read C0 + C1 ──
    // C0: 1 byte (version, should be 3 but we accept anything)
    // C1: 1536 bytes (timestamp[4] + zero[4] + random[1528])
    let mut c0c1 = vec![0u8; 1 + HANDSHAKE_SIZE];
    read_exact(stream, &mut c0c1).await?;

    let _version = c0c1[0]; // Typically 3; we accept any value for compatibility
    let c1 = &c0c1[1..];

    // Extract client timestamp from C1
    let _client_timestamp = u32::from_be_bytes([c1[0], c1[1], c1[2], c1[3]]);

    // ── Send S0 + S1 + S2 ──
    let mut response = Vec::with_capacity(1 + HANDSHAKE_SIZE * 2);

    // S0: version byte
    response.push(3u8);

    // S1: our timestamp[4] + zero[4] + random[1528]
    let server_timestamp: u32 = 0;
    response.extend_from_slice(&server_timestamp.to_be_bytes());
    response.extend_from_slice(&[0u8; 4]); // zero
    // Fill random data (simple deterministic fill — doesn't need to be cryptographic)
    for i in 0..1528 {
        response.push((i % 256) as u8);
    }

    // S2: echo client's C1 with our timestamp2
    // S2 format: client_timestamp[4] + server_timestamp[4] + echo_of_c1_random[1528]
    response.extend_from_slice(&c1[0..4]); // echo client timestamp
    response.extend_from_slice(&server_timestamp.to_be_bytes()); // our timestamp2
    response.extend_from_slice(&c1[8..]); // echo client random data

    stream
        .write_all(&response)
        .await
        .map_err(|e| format!("Failed to send S0+S1+S2: {}", e))?;
    stream
        .flush()
        .await
        .map_err(|e| format!("Failed to flush handshake: {}", e))?;

    // ── Read C2 (+ possibly extra data) ──
    // C2 is 1536 bytes, but more data may arrive in the same read
    let mut buf = vec![0u8; HANDSHAKE_SIZE + 4096];
    let mut total_read = 0;

    while total_read < HANDSHAKE_SIZE {
        let n = stream
            .read(&mut buf[total_read..])
            .await
            .map_err(|e| format!("Failed to read C2: {}", e))?;
        if n == 0 {
            return Err("Connection closed during handshake (reading C2)".to_string());
        }
        total_read += n;
    }

    // C2 is the first 1536 bytes; anything after is RTMP data
    let remaining = buf[HANDSHAKE_SIZE..total_read].to_vec();

    Ok(remaining)
}

async fn read_exact(stream: &mut TcpStream, buf: &mut [u8]) -> Result<(), String> {
    let mut offset = 0;
    while offset < buf.len() {
        let n = stream
            .read(&mut buf[offset..])
            .await
            .map_err(|e| format!("Handshake read error: {}", e))?;
        if n == 0 {
            return Err("Connection closed during handshake".to_string());
        }
        offset += n;
    }
    Ok(())
}

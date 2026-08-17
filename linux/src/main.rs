use anyhow::{Context, Result};
use clap::Parser;
use opendroidmic::audio_buffer::SharedAudioBuffer;
use opendroidmic::jitter::JitterBuffer;
use opendroidmic::opus_dec::OpusDecoder;
use opendroidmic::protocol::{self, Packet};
use qrcode::QrCode;
use qrcode::types::Color;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod pipewire;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(15);
const PING_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Parser)]
#[command(name = "opendroidmic")]
#[command(about = "Turn your Android phone into a low-latency microphone for Linux")]
struct Cli {
    #[arg(long)]
    test: bool,

    #[arg(long, default_value = "38471")]
    port: u16,

    #[arg(long, default_value = "info")]
    log_level: String,

    #[arg(short, long)]
    verbose: bool,

    /// Print QR code for mobile pairing and exit
    #[arg(long)]
    qr: bool,

    /// Interface to use for QR code IP detection
    #[arg(long, default_value = "")]
    interface: String,
}

struct ClientState {
    addr: SocketAddr,
    _session_token: u64,
    decoder: OpusDecoder,
    jitter: JitterBuffer,
    last_packet_time: Instant,
    last_ping_time: Instant,
    ping_sequence: u32,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { &cli.log_level };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .init();

    info!("OpenDroidMic v{}", env!("CARGO_PKG_VERSION"));

    // QR-only mode
    if cli.qr {
        let ip = get_local_ip(&cli.interface).context("Cannot determine local IP")?;
        let url = format!("odmc://{}:{}", ip, cli.port);
        print_qr(&url)?;
        println!("\nScan this QR code with OpenDroidMic on your phone.");
        println!("Receiver will start after scan. Press Ctrl+C to stop.\n");
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        info!("Ctrl+C received, shutting down...");
        r.store(false, Ordering::SeqCst);
        pipewire::quit_main_loop();
    })?;

    let virtual_mic = pipewire::VirtualMic::new(running.clone(), cli.test)?;
    let shared_buffer = virtual_mic.shared_buffer();

    if !cli.test {
        let recv_running = running.clone();
        let recv_buffer = shared_buffer.clone();
        let recv_port = cli.port;

        std::thread::Builder::new()
            .name("udp-receiver".into())
            .spawn(move || {
                if let Err(e) = udp_receiver_thread(recv_running, recv_buffer, recv_port) {
                    tracing::error!("UDP receiver thread error: {}", e);
                }
            })?;

        // Start mDNS service publication
        let mdns_port = cli.port;
        let mdns_running = running.clone();
        std::thread::Builder::new()
            .name("mdns-publisher".into())
            .spawn(move || {
                if let Err(e) = mdns_publisher_thread(mdns_running, mdns_port) {
                    tracing::error!("mDNS publisher error: {}", e);
                }
            })?;

        info!(
            "Listening for Android connections on port {}...",
            cli.port
        );
    } else {
        info!("Test mode - generating 440Hz sine wave");
    }
    info!("Press Ctrl+C to stop");

    virtual_mic.run();

    info!("Shutting down...");
    info!("Shutdown complete");
    Ok(())
}

fn udp_receiver_thread(
    running: Arc<AtomicBool>,
    shared_buffer: Arc<parking_lot::Mutex<SharedAudioBuffer>>,
    port: u16,
) -> Result<()> {
    let bind_addr = format!("0.0.0.0:{}", port);
    let socket =
        UdpSocket::bind(&bind_addr).context(format!("Failed to bind UDP on {}", bind_addr))?;
    socket.set_nonblocking(true)?;
    info!("UDP receiver listening on {}", socket.local_addr()?);

    let mut client: Option<ClientState> = None;
    let mut buf = vec![0u8; protocol::MAX_PACKET_SIZE];
    let mut stats_log_counter = 0u64;
    let mut last_stats_log = Instant::now();

    while running.load(Ordering::SeqCst) {
        let now = Instant::now();

        // Check client timeout
        if let Some(ref mut c) = client {
            if now.duration_since(c.last_packet_time) > CLIENT_TIMEOUT {
                info!(
                    "Client {} timed out (no packets for {}s)",
                    c.addr,
                    CLIENT_TIMEOUT.as_secs()
                );
                let s = c.jitter.stats();
                info!(
                    "Session stats: {} recv / {} lost / {} reordered",
                    s.packets_received, s.packets_lost, s.packets_reordered
                );
                client = None;
            }
        }

        // Send pings to connected client
        if let Some(ref mut c) = client {
            if now.duration_since(c.last_ping_time) >= PING_INTERVAL {
                let ping = Packet::Ping {
                    sequence: c.ping_sequence,
                    timestamp: c.ping_sequence * 20,
                };
                if let Ok(data) = ping.encode(c.ping_sequence, c.ping_sequence * 20) {
                    let _ = socket.send_to(&data, c.addr);
                }
                c.ping_sequence = c.ping_sequence.wrapping_add(1);
                c.last_ping_time = now;
            }
        }

        // Drain jitter buffer into shared audio buffer
        if let Some(ref mut c) = client {
            while let Some(pcm) = c.jitter.pop() {
                shared_buffer.lock().push_pcm(&pcm);
            }
        }

        // Log stats periodically
        stats_log_counter += 1;
        if stats_log_counter % 5000 == 0 || last_stats_log.elapsed() >= Duration::from_secs(5) {
            if let Some(ref c) = client {
                let s = c.jitter.stats();
                info!(
                    "Stats: {} recv / {} lost / {} reordered / jitter {:.1}ms / latency {:.1}ms / buf {} frames",
                    s.packets_received, s.packets_lost, s.packets_reordered,
                    s.current_jitter_ms, s.estimated_latency_ms, s.buffer_frames
                );
            }
            last_stats_log = now;
        }

        match socket.recv_from(&mut buf) {
            Ok((len, from)) => match protocol::Packet::decode(&buf[..len]) {
                Ok(packet) => match packet {
                    Packet::Hello { session_token } => {
                        info!("Hello from {} (token: {:#x})", from, session_token);
                        let decoder = OpusDecoder::new(48000, 1)?;
                        let mut jitter = JitterBuffer::new();
                        jitter.reset();

                        let ack = Packet::HelloAck {
                            session_token,
                            sample_rate: 48000,
                            channels: 1,
                        };
                        if let Ok(data) = ack.encode(0, 0) {
                            let _ = socket.send_to(&data, from);
                        }
                        info!("Sent HelloAck to {}", from);

                        client = Some(ClientState {
                            addr: from,
                            _session_token: session_token,
                            decoder,
                            jitter,
                            last_packet_time: now,
                            last_ping_time: now,
                            ping_sequence: 0,
                        });
                    }
                    Packet::Audio {
                        sequence,
                        timestamp,
                        opus_frame,
                    } => {
                        if let Some(ref mut c) = client {
                            if from == c.addr {
                                c.last_packet_time = now;
                                let mut pcm = vec![0i16; 960];
                                match c.decoder.decode(&opus_frame, &mut pcm) {
                                    Ok(samples) => {
                                        pcm.truncate(samples);
                                        c.jitter.push(sequence, timestamp, pcm);
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Opus decode error (seq {}): {}",
                                            sequence,
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Packet::Pong {
                        sequence: _,
                        timestamp: _,
                    } => {
                        if let Some(ref c) = client {
                            if from == c.addr {
                                tracing::trace!("Pong from {}", from);
                            }
                        }
                    }
                    Packet::Stop { session_token } => {
                        info!(
                            "Stop received from {} (token: {:#x})",
                            from, session_token
                        );
                        if let Some(ref c) = client {
                            if from == c.addr {
                                let s = c.jitter.stats();
                                info!(
                                    "Session stats: {} recv / {} lost / {} reordered",
                                    s.packets_received, s.packets_lost, s.packets_reordered
                                );
                                client = None;
                            }
                        }
                    }
                    Packet::Ping { sequence, timestamp } => {
                        // If a client sends a ping (unusual, but handle it), respond with pong
                        if let Some(ref c) = client {
                            if from == c.addr {
                                let pong = Packet::Pong {
                                    sequence,
                                    timestamp,
                                };
                                if let Ok(data) = pong.encode(sequence, timestamp) {
                                    let _ = socket.send_to(&data, from);
                                }
                            }
                        }
                    }
                    _ => {}
                },
                Err(e) => {
                    tracing::warn!("Invalid packet from {}: {}", from, e);
                }
            },
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => {
                tracing::error!("UDP recv error: {}", e);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    Ok(())
}

fn mdns_publisher_thread(running: Arc<AtomicBool>, port: u16) -> Result<()> {
    info!("Starting mDNS service publication on port {}", port);

    let allowed_ips: Vec<std::net::IpAddr> = get_local_ips()
        .into_iter()
        .filter(|ip| ip.is_ipv4())
        .collect();

    if allowed_ips.is_empty() {
        tracing::warn!("No IPv4 addresses found, using all interfaces");
    }

    let responder = libmdns::Responder::new_with_ip_list(allowed_ips)
        .unwrap_or_else(|_| libmdns::Responder::new());

    let _svc_handle = responder.register(
        "_opendroidmic._udp",
        "OpenDroidMic",
        port,
        &["version=1", "protocol=odmc"],
    );

    info!("mDNS service published: _opendroidmic._udp on port {}", port);

    while running.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_secs(1));
    }

    info!("mDNS publisher stopped");
    Ok(())
}

fn get_local_ip(_preferred_interface: &str) -> Result<String> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let addr = socket.local_addr()?;
    Ok(addr.ip().to_string())
}

fn get_local_ips() -> Vec<std::net::IpAddr> {
    let socket = match UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    if socket.connect("8.8.8.8:80").is_err() {
        return vec![];
    }
    match socket.local_addr() {
        Ok(addr) => vec![addr.ip()],
        Err(_) => vec![],
    }
}

fn print_qr(data: &str) -> Result<()> {
    let code = QrCode::new(data.as_bytes()).context("Failed to generate QR code")?;
    let colors = code.to_colors();
    let size = code.width();

    let mut output = String::new();
    output.push('\n');
    output.push_str("  ");
    for _ in 0..size + 2 {
        output.push_str("\u{2580}\u{2580}");
    }
    output.push('\n');

    for y in 0..size {
        output.push_str("  ");
        output.push_str("\u{2588}");
        for x in 0..size {
            if colors[y * size + x] == Color::Dark {
                output.push_str("\u{2588}\u{2588}");
            } else {
                output.push_str("  ");
            }
        }
        output.push_str("\u{2588}");
        output.push('\n');
    }

    output.push_str("  ");
    for _ in 0..size + 2 {
        output.push_str("\u{2584}\u{2584}");
    }
    output.push('\n');

    println!("{}", output);
    println!("  URL: {}", data);

    Ok(())
}

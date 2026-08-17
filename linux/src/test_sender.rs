use anyhow::Result;
use opendroidmic::protocol::Packet;
use opus::{Application, Encoder, Channels};
use std::net::{SocketAddr, UdpSocket};
use tracing::info;

const SAMPLE_RATE: u32 = 48000;
const FRAME_SIZE: usize = 960;
const BITRATE: u32 = 32000;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    let target: SocketAddr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:38471".to_string())
        .parse()?;

    info!("Sending test audio to {}", target);

    let socket = UdpSocket::bind("0.0.0.0:0")?;
    let mut encoder = Encoder::new(SAMPLE_RATE, Channels::Mono, Application::Audio)?;
    encoder.set_bitrate(opus::Bitrate::Bits(BITRATE as i32))?;

    let mut sequence: u32 = 0;
    let mut phase: f64 = 0.0;
    let frequency = 440.0f64;
    let amplitude = 16000.0f64;

    // Send Hello packet
    let hello = Packet::Hello {
        session_token: 0xDEADBEEF,
    };
    let data = hello.encode(0, 0)?;
    socket.send_to(&data, target)?;
    info!("Sent Hello packet");

    loop {
        let mut pcm = vec![0i16; FRAME_SIZE];
        for sample in pcm.iter_mut() {
            *sample = (amplitude * (2.0 * std::f64::consts::PI * phase).sin()) as i16;
            phase += frequency / SAMPLE_RATE as f64;
            if phase >= 1.0 {
                phase -= 1.0;
            }
        }

        let mut opus_buf = vec![0u8; FRAME_SIZE * 2];
        let encoded = encoder.encode(&pcm, &mut opus_buf)?;

        let packet = Packet::Audio {
            sequence,
            timestamp: sequence * 20,
            opus_frame: opus_buf[..encoded].to_vec(),
        };

        let data = packet.encode(sequence, sequence * 20)?;
        socket.send_to(&data, target)?;

        sequence += 1;
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

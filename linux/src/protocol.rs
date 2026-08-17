use anyhow::{bail, Result};

pub const MAGIC: [u8; 4] = *b"ODMC";
pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_PACKET_SIZE: usize = 1500;
pub const HEADER_SIZE: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PacketType {
    Hello = 0,
    HelloAck = 1,
    Audio = 2,
    Ping = 3,
    Pong = 4,
    Stop = 5,
    Error = 6,
}

impl PacketType {
    pub fn from_u8(v: u8) -> Result<Self> {
        match v {
            0 => Ok(PacketType::Hello),
            1 => Ok(PacketType::HelloAck),
            2 => Ok(PacketType::Audio),
            3 => Ok(PacketType::Ping),
            4 => Ok(PacketType::Pong),
            5 => Ok(PacketType::Stop),
            6 => Ok(PacketType::Error),
            _ => bail!("Unknown packet type: {}", v),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PacketHeader {
    pub magic: [u8; 4],
    pub protocol_version: u8,
    pub packet_type: u8,
    pub flags: u8,
    pub reserved: u8,
    pub sequence: u32,
    pub timestamp: u32,
    pub payload_length: u16,
    pub checksum: u16,
}

impl PacketHeader {
    pub fn new(packet_type: PacketType, sequence: u32, timestamp: u32, payload_length: u16) -> Self {
        Self {
            magic: MAGIC,
            protocol_version: PROTOCOL_VERSION,
            packet_type: packet_type as u8,
            flags: 0,
            reserved: 0,
            sequence,
            timestamp,
            payload_length,
            checksum: 0,
        }
    }

    pub fn to_bytes(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.magic);
        buf[4] = self.protocol_version;
        buf[5] = self.packet_type;
        buf[6] = self.flags;
        buf[7] = self.reserved;
        buf[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        buf[12..16].copy_from_slice(&self.timestamp.to_le_bytes());
        buf[16..18].copy_from_slice(&self.payload_length.to_le_bytes());
        buf[18..20].copy_from_slice(&self.checksum.to_le_bytes());
        buf
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            bail!("Packet too short: {} < {}", data.len(), HEADER_SIZE);
        }
        let mut magic = [0u8; 4];
        magic.copy_from_slice(&data[0..4]);
        let header = Self {
            magic,
            protocol_version: data[4],
            packet_type: data[5],
            flags: data[6],
            reserved: data[7],
            sequence: u32::from_le_bytes(data[8..12].try_into().unwrap()),
            timestamp: u32::from_le_bytes(data[12..16].try_into().unwrap()),
            payload_length: u16::from_le_bytes(data[16..18].try_into().unwrap()),
            checksum: u16::from_le_bytes(data[18..20].try_into().unwrap()),
        };
        if header.magic != MAGIC {
            bail!("Invalid magic: {:?}", header.magic);
        }
        if header.protocol_version != PROTOCOL_VERSION {
            bail!(
                "Unsupported protocol version: {}",
                header.protocol_version
            );
        }
        Ok(header)
    }
}

#[derive(Debug, Clone)]
pub enum Packet {
    Hello {
        session_token: u64,
    },
    HelloAck {
        session_token: u64,
        sample_rate: u32,
        channels: u32,
    },
    Audio {
        sequence: u32,
        timestamp: u32,
        opus_frame: Vec<u8>,
    },
    Ping {
        sequence: u32,
        timestamp: u32,
    },
    Pong {
        sequence: u32,
        timestamp: u32,
    },
    Stop {
        session_token: u64,
    },
    Error {
        message: String,
    },
}

impl Packet {
    pub fn encode(&self, sequence: u32, timestamp: u32) -> Result<Vec<u8>> {
        let (ptype, payload) = match self {
            Packet::Hello { session_token } => {
                let mut p = Vec::with_capacity(8);
                p.extend_from_slice(&session_token.to_le_bytes());
                (PacketType::Hello, p)
            }
            Packet::HelloAck {
                session_token,
                sample_rate,
                channels,
            } => {
                let mut p = Vec::with_capacity(16);
                p.extend_from_slice(&session_token.to_le_bytes());
                p.extend_from_slice(&sample_rate.to_le_bytes());
                p.extend_from_slice(&channels.to_le_bytes());
                (PacketType::HelloAck, p)
            }
            Packet::Audio {
                opus_frame, ..
            } => {
                (PacketType::Audio, opus_frame.clone())
            }
            Packet::Ping {
                sequence: seq,
                timestamp: ts,
            } => {
                let mut p = Vec::with_capacity(8);
                p.extend_from_slice(&seq.to_le_bytes());
                p.extend_from_slice(&ts.to_le_bytes());
                (PacketType::Ping, p)
            }
            Packet::Pong {
                sequence: seq,
                timestamp: ts,
            } => {
                let mut p = Vec::with_capacity(8);
                p.extend_from_slice(&seq.to_le_bytes());
                p.extend_from_slice(&ts.to_le_bytes());
                (PacketType::Pong, p)
            }
            Packet::Stop { session_token } => {
                let mut p = Vec::with_capacity(8);
                p.extend_from_slice(&session_token.to_le_bytes());
                (PacketType::Stop, p)
            }
            Packet::Error { message } => {
                (PacketType::Error, message.as_bytes().to_vec())
            }
        };

        let header = PacketHeader::new(ptype, sequence, timestamp, payload.len() as u16);
        let mut packet = header.to_bytes().to_vec();
        packet.extend_from_slice(&payload);
        Ok(packet)
    }

    pub fn decode(data: &[u8]) -> Result<Self> {
        let header = PacketHeader::from_bytes(data)?;
        let payload = &data[HEADER_SIZE..];

        match PacketType::from_u8(header.packet_type)? {
            PacketType::Hello => {
                if payload.len() < 8 {
                    bail!("Hello payload too short");
                }
                let session_token = u64::from_le_bytes(payload[..8].try_into().unwrap());
                Ok(Packet::Hello { session_token })
            }
            PacketType::HelloAck => {
                if payload.len() < 16 {
                    bail!("HelloAck payload too short");
                }
                let session_token = u64::from_le_bytes(payload[..8].try_into().unwrap());
                let sample_rate = u32::from_le_bytes(payload[8..12].try_into().unwrap());
                let channels = u32::from_le_bytes(payload[12..16].try_into().unwrap());
                Ok(Packet::HelloAck {
                    session_token,
                    sample_rate,
                    channels,
                })
            }
            PacketType::Audio => Ok(Packet::Audio {
                sequence: header.sequence,
                timestamp: header.timestamp,
                opus_frame: payload.to_vec(),
            }),
            PacketType::Ping => {
                if payload.len() < 8 {
                    bail!("Ping payload too short");
                }
                let seq = u32::from_le_bytes(payload[..4].try_into().unwrap());
                let ts = u32::from_le_bytes(payload[4..8].try_into().unwrap());
                Ok(Packet::Ping {
                    sequence: seq,
                    timestamp: ts,
                })
            }
            PacketType::Pong => {
                if payload.len() < 8 {
                    bail!("Pong payload too short");
                }
                let seq = u32::from_le_bytes(payload[..4].try_into().unwrap());
                let ts = u32::from_le_bytes(payload[4..8].try_into().unwrap());
                Ok(Packet::Pong {
                    sequence: seq,
                    timestamp: ts,
                })
            }
            PacketType::Stop => {
                if payload.len() < 8 {
                    bail!("Stop payload too short");
                }
                let session_token = u64::from_le_bytes(payload[..8].try_into().unwrap());
                Ok(Packet::Stop { session_token })
            }
            PacketType::Error => {
                let message = String::from_utf8_lossy(payload).to_string();
                Ok(Packet::Error { message })
            }
        }
    }
}

use std::collections::VecDeque;
use std::time::{Duration, Instant};

const MIN_BUFFER_FRAMES: usize = 1;
const DEFAULT_BUFFER_FRAMES: usize = 2;
const MAX_BUFFER_FRAMES: usize = 5;
const STATS_WINDOW: usize = 100;

#[derive(Debug, Clone)]
pub struct StreamStats {
    pub packets_received: u64,
    pub packets_lost: u64,
    pub packets_reordered: u64,
    pub packets_duplicate: u64,
    pub current_jitter_ms: f64,
    pub estimated_latency_ms: f64,
    pub buffer_frames: usize,
}

impl Default for StreamStats {
    fn default() -> Self {
        Self {
            packets_received: 0,
            packets_lost: 0,
            packets_reordered: 0,
            packets_duplicate: 0,
            current_jitter_ms: 0.0,
            estimated_latency_ms: 0.0,
            buffer_frames: DEFAULT_BUFFER_FRAMES,
        }
    }
}

struct JitterEntry {
    sequence: u32,
    pcm: Vec<i16>,
}

pub struct JitterBuffer {
    buffer: VecDeque<JitterEntry>,
    target_frames: usize,
    next_sequence: Option<u32>,
    inter_arrival_times: VecDeque<Duration>,
    last_arrival: Option<Instant>,
    stats: StreamStats,
}

impl JitterBuffer {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::with_capacity(MAX_BUFFER_FRAMES * 2),
            target_frames: DEFAULT_BUFFER_FRAMES,
            next_sequence: None,
            inter_arrival_times: VecDeque::with_capacity(STATS_WINDOW),
            last_arrival: None,
            stats: StreamStats::default(),
        }
    }

    pub fn push(&mut self, sequence: u32, _timestamp: u32, pcm: Vec<i16>) {
        let now = Instant::now();

        // Track inter-arrival jitter
        if let Some(last) = self.last_arrival {
            let delta = now.duration_since(last);
            self.inter_arrival_times.push_back(delta);
            if self.inter_arrival_times.len() > STATS_WINDOW {
                self.inter_arrival_times.pop_front();
            }
        }
        self.last_arrival = Some(now);

        // Detect packet ordering issues
        match self.next_sequence {
            Some(next) => {
                if sequence == next {
                    self.next_sequence = Some(next.wrapping_add(1));
                } else if sequence.wrapping_sub(next) < 0x80000000 {
                    // Future packet (gap = lost packets)
                    let lost = sequence.wrapping_sub(next);
                    self.stats.packets_lost += lost as u64;
                    self.next_sequence = Some(sequence.wrapping_add(1));
                } else {
                    // Late/reordered packet
                    self.stats.packets_reordered += 1;
                }
            }
            None => {
                self.next_sequence = Some(sequence.wrapping_add(1));
            }
        }

        self.stats.packets_received += 1;

        // Check for duplicates
        if self.buffer.iter().any(|e| e.sequence == sequence) {
            self.stats.packets_duplicate += 1;
            return;
        }

        // Insert in sequence order
        let entry = JitterEntry {
            sequence,
            pcm,
        };

        let pos = self.buffer.iter().position(|e| {
            // Handle u32 wrapping
            let diff = sequence.wrapping_sub(e.sequence);
            diff < 0x80000000 && diff > 0
        });

        match pos {
            Some(p) => self.buffer.insert(p, entry),
            None => self.buffer.push_back(entry),
        }

        // Trim buffer if too large
        while self.buffer.len() > MAX_BUFFER_FRAMES * 2 {
            self.buffer.pop_front();
        }

        // Update stats
        self.update_jitter_stats();
        self.stats.buffer_frames = self.target_frames;
        self.stats.estimated_latency_ms =
            self.target_frames as f64 * 20.0 + self.stats.current_jitter_ms;
    }

    pub fn pop(&mut self) -> Option<Vec<i16>> {
        if self.buffer.is_empty() {
            None
        } else {
            self.buffer.pop_front().map(|e| e.pcm)
        }
    }

    pub fn stats(&self) -> &StreamStats {
        &self.stats
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.next_sequence = None;
        self.inter_arrival_times.clear();
        self.last_arrival = None;
        self.target_frames = DEFAULT_BUFFER_FRAMES;
        self.stats = StreamStats::default();
    }

    fn update_jitter_stats(&mut self) {
        if self.inter_arrival_times.len() < 2 {
            self.stats.current_jitter_ms = 0.0;
            return;
        }

        let mean: Duration = self.inter_arrival_times.iter().sum::<Duration>()
            / self.inter_arrival_times.len() as u32;

        let variance: f64 = self
            .inter_arrival_times
            .iter()
            .map(|t| {
                let diff = t.as_secs_f64() - mean.as_secs_f64();
                diff * diff
            })
            .sum::<f64>()
            / self.inter_arrival_times.len() as f64;

        let jitter_ms = variance.sqrt() * 1000.0;
        self.stats.current_jitter_ms = jitter_ms;

        // Adaptive buffer sizing based on jitter
        self.target_frames = if jitter_ms < 10.0 {
            MIN_BUFFER_FRAMES
        } else if jitter_ms < 30.0 {
            DEFAULT_BUFFER_FRAMES
        } else {
            MAX_BUFFER_FRAMES
        };
    }
}

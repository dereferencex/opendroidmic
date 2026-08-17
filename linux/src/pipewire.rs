use anyhow::{Context, Result};
use opendroidmic::audio_buffer::{SharedAudioBuffer, FRAME_SIZE};
use pipewire as pw;
use pipewire::properties::properties;
use pipewire::spa;
use spa::pod::Pod;
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::Arc;
use tracing::info;

struct TestGenerator {
    phase: f64,
    sample_rate: f64,
    amplitude: f64,
    frequency: f64,
}

impl TestGenerator {
    fn new() -> Self {
        Self {
            phase: 0.0,
            sample_rate: 48000.0,
            amplitude: 16000.0,
            frequency: 440.0,
        }
    }

    fn fill_buffer(&mut self, buffer: &mut [i16]) {
        for sample in buffer.iter_mut() {
            *sample = (self.amplitude * (2.0 * std::f64::consts::PI * self.phase).sin()) as i16;
            self.phase += self.frequency / self.sample_rate;
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
        }
    }
}

struct StreamUserData {
    test_gen: Option<TestGenerator>,
    shared_buffer: Arc<parking_lot::Mutex<SharedAudioBuffer>>,
    pcm_buf: Vec<i16>,
    callback_count: u64,
}

pub struct VirtualMic {
    main_loop: pw::main_loop::MainLoopRc,
    _context: pw::context::ContextRc,
    _core: pw::core::CoreRc,
    _stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<StreamUserData>,
    shared_buffer: Arc<parking_lot::Mutex<SharedAudioBuffer>>,
}

static MAIN_LOOP_RAW: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

pub fn quit_main_loop() {
    let ptr = MAIN_LOOP_RAW.load(Ordering::Relaxed);
    if !ptr.is_null() {
        unsafe {
            pw::sys::pw_main_loop_quit(ptr as *mut pw::sys::pw_main_loop);
        }
    }
}

impl VirtualMic {
    pub fn new(_running: Arc<AtomicBool>, test_mode: bool) -> Result<Self> {
        let shared_buffer = Arc::new(parking_lot::Mutex::new(SharedAudioBuffer::new()));

        let main_loop = pw::main_loop::MainLoopRc::new(None)
            .context("Failed to create PipeWire main loop")?;

        MAIN_LOOP_RAW.store(
            main_loop.as_raw_ptr() as *mut (),
            Ordering::Relaxed,
        );

        let context = pw::context::ContextRc::new(&main_loop, None)
            .context("Failed to create PipeWire context")?;

        let core = context
            .connect_rc(None)
            .context("Failed to connect to PipeWire")?;

        let stream = pw::stream::StreamRc::new(
            core.clone(),
            "OpenDroidMic",
            properties! {
                *pw::keys::MEDIA_TYPE => "Audio",
                *pw::keys::MEDIA_CATEGORY => "Source",
                *pw::keys::MEDIA_CLASS => "Audio/Source",
                *pw::keys::MEDIA_ROLE => "Communication",
                *pw::keys::NODE_NAME => "OpenDroidMic",
                *pw::keys::NODE_DESCRIPTION => "OpenDroidMic Virtual Microphone",
                *pw::keys::NODE_VIRTUAL => "true",
                *pw::keys::NODE_ALWAYS_PROCESS => "true",
            },
        )
        .context("Failed to create PipeWire stream")?;

        let test_gen = if test_mode {
            Some(TestGenerator::new())
        } else {
            None
        };

        let user_data = StreamUserData {
            test_gen,
            shared_buffer: shared_buffer.clone(),
            pcm_buf: vec![0i16; FRAME_SIZE],
            callback_count: 0,
        };

        let listener = stream
            .add_local_listener_with_user_data(user_data)
            .state_changed(|_stream, _data, state, _old_state| {
                match state {
                    pw::stream::StreamState::Streaming => {
                        info!("Stream active - virtual microphone ready");
                    }
                    pw::stream::StreamState::Error(msg) => {
                        tracing::error!("Stream error: {}", msg);
                    }
                    _ => {}
                }
            })
            .process(|stream, user_data| {
                user_data.callback_count += 1;
                if let Some(mut buffer) = stream.dequeue_buffer() {
                    let datas = buffer.datas_mut();
                    if let Some(buf_data) = datas.first_mut() {
                        if let Some(slice) = buf_data.data() {
                            let num_samples = slice.len() / 2;

                            let pcm = &mut user_data.pcm_buf;
                            pcm.resize(num_samples, 0);

                            if let Some(ref mut generator) = user_data.test_gen {
                                generator.fill_buffer(pcm);
                            } else {
                                user_data.shared_buffer.lock().read_frame(pcm);
                            }

                            let src_bytes: &[u8] = bytemuck::cast_slice(pcm);
                            let copy_len = src_bytes.len().min(slice.len());
                            slice[..copy_len].copy_from_slice(&src_bytes[..copy_len]);
                            let chunk = buf_data.chunk_mut();
                            *chunk.offset_mut() = 0;
                            *chunk.stride_mut() = 2i32;
                            *chunk.size_mut() = copy_len as u32;

                            if user_data.callback_count % 250 == 1 {
                                let buf = user_data.shared_buffer.lock();
                                let (rd, wr, avail) = buf.stats();
                                tracing::info!(
                                    "PW callback #{}: asked for {} samples, buffered={}, total_read={}, total_written={}",
                                    user_data.callback_count, num_samples, avail, rd, wr
                                );
                            }
                        }
                    }
                }
            })
            .register()
            .context("Failed to register stream callbacks")?;

        let mut audio_info = spa::param::audio::AudioInfoRaw::new();
        audio_info.set_format(spa::param::audio::AudioFormat::S16LE);
        audio_info.set_rate(48000);
        audio_info.set_channels(1);
        let mut position = [0u32; spa::param::audio::MAX_CHANNELS];
        position[0] = pipewire::spa::sys::SPA_AUDIO_CHANNEL_MONO;
        audio_info.set_position(position);

        let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
            Cursor::new(Vec::new()),
            &spa::pod::Value::Object(spa::pod::Object {
                type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
                id: spa::param::ParamType::EnumFormat.as_raw(),
                properties: audio_info.into(),
            }),
        )
        .context("Failed to serialize format")?
        .0
        .into_inner();

        let mut params =
            [Pod::from_bytes(&values).context("Failed to create Pod from bytes")?];

        stream
            .connect(
                spa::utils::Direction::Output,
                None,
                pw::stream::StreamFlags::AUTOCONNECT
                    | pw::stream::StreamFlags::MAP_BUFFERS
                    | pw::stream::StreamFlags::RT_PROCESS,
                &mut params,
            )
            .context("Failed to connect stream")?;

        info!("PipeWire virtual source started: 48000 Hz, 1 ch, 960 samples/frame");

        Ok(Self {
            main_loop,
            _context: context,
            _core: core,
            _stream: stream,
            _listener: listener,
            shared_buffer,
        })
    }

    pub fn shared_buffer(&self) -> Arc<parking_lot::Mutex<SharedAudioBuffer>> {
        self.shared_buffer.clone()
    }

    pub fn run(&self) {
        info!("PipeWire main loop running");
        self.main_loop.run();
    }

    #[allow(dead_code)]
    pub fn stop(&self) {
        self.main_loop.quit();
        info!("PipeWire virtual source stopped");
    }
}

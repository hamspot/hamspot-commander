//! Speaker playback on a PipeWire stream.
//!
//! stdin is the decoder's own signed 16-bit rate. Two slots are summed.
//! PipeWire resamples that mono stream to the sink. This process does not
//! repeat samples, and it does not start pw-cat.

use pipewire as pw;
use pw::spa::pod::Pod;
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::Thread;

const RING: usize = 48_000;

struct Ring {
    buf: Box<[AtomicI16]>,
    mask: usize,
    w: AtomicUsize,
    r: AtomicUsize,
}

impl Ring {
    fn new(cap: usize) -> Self {
        let cap = cap.next_power_of_two();
        let mut buf = Vec::with_capacity(cap);
        for _ in 0..cap {
            buf.push(AtomicI16::new(0));
        }
        Self {
            buf: buf.into_boxed_slice(),
            mask: cap - 1,
            w: AtomicUsize::new(0),
            r: AtomicUsize::new(0),
        }
    }

    fn try_push(&self, sample: i16) -> bool {
        let w = self.w.load(Ordering::Relaxed);
        let r = self.r.load(Ordering::Acquire);
        if w.wrapping_sub(r) >= self.buf.len() {
            return false;
        }
        self.buf[w & self.mask].store(sample, Ordering::Relaxed);
        self.w.store(w.wrapping_add(1), Ordering::Release);
        true
    }

    fn try_pop(&self) -> Option<i16> {
        let r = self.r.load(Ordering::Relaxed);
        let w = self.w.load(Ordering::Acquire);
        if r == w {
            return None;
        }
        let sample = self.buf[r & self.mask].load(Ordering::Relaxed);
        self.r.store(r.wrapping_add(1), Ordering::Release);
        Some(sample)
    }

    fn is_empty(&self) -> bool {
        let r = self.r.load(Ordering::Acquire);
        let w = self.w.load(Ordering::Acquire);
        r == w
    }
}

struct Shared {
    ring: Ring,
    eof: AtomicBool,
    failed: AtomicBool,
    /// Set by the stdin thread. The stream callback wakes it when a slot frees.
    producer: OnceLock<Thread>,
}

impl Shared {
    fn new() -> Self {
        Self {
            ring: Ring::new(RING),
            eof: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            producer: OnceLock::new(),
        }
    }

    fn fail(&self) {
        self.failed.store(true, Ordering::Release);
        self.wake();
    }

    fn wake(&self) {
        if let Some(thread) = self.producer.get() {
            thread.unpark();
        }
    }

    /// Block until the sample is queued, or the stream has failed.
    fn push(&self, sample: i16) -> bool {
        loop {
            if self.failed.load(Ordering::Acquire) {
                return false;
            }
            if self.ring.try_push(sample) {
                return true;
            }
            // The process callback is the signal that a slot is free.
            std::thread::park();
        }
    }
}

struct PlayData {
    shared: Arc<Shared>,
    quit: pw::channel::Sender<()>,
    quit_sent: bool,
}

/// Fill `dst` from the ring. A short ring becomes silence for this quantum
/// and the stream stays up.
#[cfg(test)]
fn take_quantum(ring: &Ring, dst: &mut [i16]) -> usize {
    let mut filled = 0;
    for sample in dst.iter_mut() {
        match ring.try_pop() {
            Some(value) => {
                *sample = value;
                filled += 1;
            }
            None => *sample = 0,
        }
    }
    filled
}

fn read_stdin(shared: Arc<Shared>, channels: usize) {
    let _ = shared.producer.set(std::thread::current());
    let mut input = io::stdin().lock();
    let mut buf = [0u8; 8192];
    let mut pending = Vec::new();
    let frame = channels * 2;
    loop {
        if shared.failed.load(Ordering::Acquire) {
            return;
        }
        let n = match input.read(&mut buf) {
            Ok(0) => {
                shared.eof.store(true, Ordering::Release);
                return;
            }
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => {
                shared.fail();
                return;
            }
        };
        pending.extend_from_slice(&buf[..n]);
        let whole = pending.len() - pending.len() % frame;
        let mut i = 0;
        while i + frame <= whole {
            let sample = if channels == 1 {
                i16::from_le_bytes([pending[i], pending[i + 1]])
            } else {
                super::mix(
                    i16::from_le_bytes([pending[i], pending[i + 1]]),
                    i16::from_le_bytes([pending[i + 2], pending[i + 3]]),
                )
            };
            if !shared.push(sample) {
                return;
            }
            i += frame;
        }
        pending.drain(..whole);
    }
}

fn request_quit(data: &mut PlayData) {
    if data.quit_sent {
        return;
    }
    data.quit_sent = true;
    let _ = data.quit.send(());
}

pub fn run(rate: u32, channels: usize, target: Option<&str>) -> io::Result<()> {
    let shared = Arc::new(Shared::new());
    let reader_shared = Arc::clone(&shared);
    let reader = std::thread::Builder::new()
        .name("voice-in".into())
        .spawn(move || read_stdin(reader_shared, channels))
        .map_err(|err| io::Error::other(format!("voice reader: {err}")))?;
    let result = play_stream(rate, target, &shared);
    shared.fail();
    let _ = reader.join();
    result
}

fn play_stream(rate: u32, target: Option<&str>, shared: &Arc<Shared>) -> io::Result<()> {
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None)
        .map_err(|err| io::Error::other(format!("pipewire: {err}")))?;
    let context = pw::context::ContextRc::new(&mainloop, None)
        .map_err(|err| io::Error::other(format!("pipewire: {err}")))?;
    let core = context
        .connect_rc(None)
        .map_err(|err| io::Error::other(format!("pipewire: {err}")))?;

    let stream = pw::stream::StreamBox::new(
        &core,
        "commander-voice",
        stream_props(target),
    )
    .map_err(|err| io::Error::other(format!("pipewire: {err}")))?;

    let (quit_tx, quit_rx) = pw::channel::channel::<()>();
    let _quit = quit_rx.attach(mainloop.loop_(), {
        let mainloop = mainloop.clone();
        move |_| mainloop.quit()
    });

    let data = PlayData {
        shared: Arc::clone(shared),
        quit: quit_tx,
        quit_sent: false,
    };
    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_stream, data, _old, state| {
            if let pw::stream::StreamState::Error(err) = state {
                eprintln!("commander-voice: {err}");
                data.shared.fail();
                request_quit(data);
            }
        })
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else {
                return;
            };
            let datas = buffer.datas_mut();
            let Some(buf) = datas.first_mut() else {
                return;
            };
            let n_frames = if let Some(slice) = buf.data() {
                let n_frames = slice.len() / 2;
                for i in 0..n_frames {
                    // A missing sample is silence for this frame. The stream stays up.
                    let sample = data.shared.ring.try_pop().unwrap_or(0);
                    let start = i * 2;
                    slice[start..start + 2].copy_from_slice(&sample.to_le_bytes());
                }
                if data.shared.eof.load(Ordering::Acquire) && data.shared.ring.is_empty() {
                    request_quit(data);
                }
                data.shared.wake();
                n_frames
            } else {
                0
            };
            let chunk = buf.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = 2;
            *chunk.size_mut() = (2 * n_frames) as u32;
        })
        .register()
        .map_err(|err| io::Error::other(format!("pipewire: {err}")))?;

    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::S16LE);
    audio_info.set_rate(rate);
    audio_info.set_channels(1);
    let mut position = [0; pw::spa::param::audio::MAX_CHANNELS];
    position[0] = spa_sys::SPA_AUDIO_CHANNEL_MONO;
    audio_info.set_position(position);

    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(pw::spa::pod::Object {
            type_: spa_sys::SPA_TYPE_OBJECT_Format,
            id: spa_sys::SPA_PARAM_EnumFormat,
            properties: audio_info.into(),
        }),
    )
    .map_err(|err| io::Error::other(format!("pipewire format: {err}")))?
    .0
    .into_inner();
    let mut params = [Pod::from_bytes(&values).ok_or_else(|| io::Error::other("pipewire format"))?];
    stream
        .connect(
            pw::spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(|err| io::Error::other(format!("pipewire: {err}")))?;

    mainloop.run();
    if shared.failed.load(Ordering::Acquire) {
        return Err(io::Error::other("pipewire stream failed"));
    }
    Ok(())
}

fn stream_props(target: Option<&str>) -> pw::properties::PropertiesBox {
    let mut props = pw::properties::properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::AUDIO_CHANNELS => "1",
        *pw::keys::NODE_NAME => "commander-voice",
    };
    if let Some(target) = target {
        props.insert(*pw::keys::TARGET_OBJECT, target);
    }
    props
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_ring_plays_silence_for_the_rest_of_the_quantum() {
        let ring = Ring::new(8);
        assert!(ring.try_push(3));
        assert!(ring.try_push(-4));
        let mut dst = [9i16; 4];
        let filled = take_quantum(&ring, &mut dst);
        assert_eq!(filled, 2);
        assert_eq!(dst, [3, -4, 0, 0]);
        assert!(ring.is_empty());
    }

    #[test]
    fn a_full_ring_refuses_another_sample_until_one_is_taken() {
        let ring = Ring::new(4);
        let cap = 4usize.next_power_of_two();
        for i in 0..cap {
            assert!(ring.try_push(i as i16), "{i}");
        }
        assert!(!ring.try_push(99));
        assert_eq!(ring.try_pop(), Some(0));
        assert!(ring.try_push(99));
        let mut got = Vec::new();
        while let Some(sample) = ring.try_pop() {
            got.push(sample);
        }
        assert_eq!(got, vec![1, 2, 3, 99]);
    }
}

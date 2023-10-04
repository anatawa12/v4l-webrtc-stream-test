use std::fs::File;
use std::io;
use std::io::Write;
use std::marker::PhantomData;
use v4l::prelude::*;
use v4l::{Format, FourCC, Memory};
use v4l::buffer::{Metadata, Type};
use v4l::capability::Flags;
use v4l::io::Queue;
use v4l::io::queue::Streaming;
use v4l::memory::Mmap;
use v4l::video::{Capture, capture, Output, output};

fn main() -> io::Result<()> {
    println!("Hello, world!");
    let camera_device = 0;
    let encoder_device = 11;
    let fps = 10;

    let camera_format = Format {
        width: 640,
        height: 480,
        fourcc: FourCC::new(b"RGB3"),
        ..unsafe { std::mem::zeroed() }
    };

    let encoded_format = Format {
        width: 640,
        height: 480,
        fourcc: FourCC::new(b"H264"),
        ..unsafe { std::mem::zeroed() }
    };

    //let mut camera = Device::new(camera_device)?;

    //let camera_caps = camera.query_caps()?;
    //if !camera_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
    //    panic!("Camera: Capture not supported")
    //}
    //if !camera_caps.capabilities.contains(Flags::STREAMING) {
    //    panic!("Camera: Streaming not supported")
    //}

    //Capture::set_format(&mut camera, &camera_format)?;
    //Capture::set_params(&mut camera, &capture::Parameters::with_fps(fps))?;

    let mut encoder = Device::new(encoder_device)?;

    let encoder_caps = encoder.query_caps()?;
    println!("Encoder capabilities: {}", encoder_caps.capabilities);
    if !encoder_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
        panic!("Encoder: Capture not supported")
    }
    if !encoder_caps.capabilities.contains(Flags::VIDEO_OUTPUT) {
        panic!("Encoder: Output not supported")
    }
    if !encoder_caps.capabilities.contains(Flags::STREAMING) {
        panic!("Encoder: Streaming not supported")
    }

    Output::set_format(&mut encoder, &camera_format)?;
    Capture::set_format(&mut encoder, &encoded_format)?;
    Output::set_params(&mut encoder, &output::Parameters::with_fps(fps))?;

    //let mut camera_stream = CaptureStream::with_device(&camera, 3)?;
    let mut encoder_raw_stream = OutputStream::with_device(&encoder, 1)?;
    let mut encoder_encoded_queue = CaptureStream::with_device(&encoder, 1)?;

    let mut write_to = File::open("test.h264")?;

    for i in 0..100 {
        encoder_raw_stream.write_frame(|buffer| {
            //camera_stream.read_frame(|frame| {
            //    buffer.copy_from_slice(frame);
            //    Ok(frame.len())
            //})
            buffer[..640 * 480 * 3].fill(i);
            Ok(640 * 480 * 3)
        })?;

        encoder_encoded_queue.read_frame(|buffer| {
            write_to.write(buffer)?;
            Ok(())
        })?;
    }

    //camera_stream.finish()?;
    encoder_raw_stream.finish()?;
    encoder_encoded_queue.finish()?;

    Ok(())
}

fn buffer_metadata(index: usize) -> Metadata {
    Metadata {
        memory: Memory::Mmap,
        index: index as u32,
        ..unsafe { std::mem::zeroed() }
    }
}

type CaptureStream<'a> = StreamBase<'a, VideoCapture>;
type OutputStream<'a> = StreamBase<'a, VideoOutput>;

trait StreamTypeMarker {
    const TYPE: Type;
}

struct VideoCapture;

impl StreamTypeMarker for VideoCapture {
    const TYPE: Type = Type::VideoCapture;
}

struct VideoOutput;

impl StreamTypeMarker for VideoOutput {
    const TYPE: Type = Type::VideoCapture;
}

struct StreamBase<'a, Type: StreamTypeMarker> {
    queue: Queue<Mmap<'a>, Streaming>,
    _phantom: PhantomData<Type>,
}

impl <'a, Type : StreamTypeMarker> StreamBase<'a, Type> {
    pub fn with_device(device: &Device, buf_count: u32) -> io::Result<Self> {
        let mut queue = Queue::with_mmap(device.handle(), Type::TYPE, buf_count)?;

        for i in 0..queue.len() {
            queue.enqueue(&buffer_metadata(i))?;
        }

        let queue = queue.start_stream()?;

        return Ok(Self { queue, _phantom: PhantomData })
    }

    pub fn finish(self) -> io::Result<()> {
        let Self{ queue, _phantom } = self;
        queue.stop_stream()?;
        Ok(())
    }
}

impl <'a> StreamBase<'a, VideoCapture> {
    pub(crate) fn read_frame<R>(
        &mut self,
        process_data: impl for <'b> FnOnce(&'b [u8]) -> io::Result<R>,
    ) -> io::Result<R> {
        let encoder_encoded_meta = self.queue.dequeue()?;
        let encoder_encoded_buf = &self.queue[encoder_encoded_meta.index as usize].0[..encoder_encoded_meta.bytesused as usize];
        let result = process_data(encoder_encoded_buf)?;
        self.queue.enqueue(&encoder_encoded_meta)?;
        Ok(result)
    }
}

impl <'a> StreamBase<'a, VideoOutput> {
    pub(crate) fn write_frame(
        &mut self,
        get_raw_data: impl for <'b> FnOnce(&'b mut [u8]) -> io::Result<usize>,
    ) -> io::Result<()> {
        let mut encoder_encoded_meta = self.queue.dequeue()?;
        let encoder_encoded_buf = &mut self.queue[encoder_encoded_meta.index as usize].0[..];
        encoder_encoded_meta.bytesused = get_raw_data(encoder_encoded_buf)? as u32;
        self.queue.enqueue(&encoder_encoded_meta)?;
        Ok(())
    }
}

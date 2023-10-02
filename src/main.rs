use std::fs::File;
use std::io;
use std::io::Write;
use v4l::prelude::*;
use v4l::{Format, FourCC, Memory};
use v4l::buffer::{Metadata, Type};
use v4l::capability::Flags;
use v4l::io::Queue;
use v4l::video::{Capture, capture, Output, output};

fn main() -> io::Result<()> {
    println!("Hello, world!");
    let camera_device = 0;
    let encoder_device = 11;
    let fps = 10;

    let camera_format = Format {
        width: 640,
        height: 480,
        fourcc: FourCC::new(b"YUYV"),
        ..unsafe { std::mem::zeroed() }
    };

    let encoded_format = Format {
        width: 640,
        height: 480,
        fourcc: FourCC::new(b"H264"),
        ..unsafe { std::mem::zeroed() }
    };

    let mut camera = Device::new(camera_device)?;

    let camera_caps = camera.query_caps()?;
    if !camera_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
        panic!("Camera: Capture not supported")
    }
    if !camera_caps.capabilities.contains(Flags::STREAMING) {
        panic!("Camera: Streaming not supported")
    }

    Capture::set_format(&mut camera, &camera_format)?;
    Capture::set_params(&mut camera, &capture::Parameters::with_fps(fps))?;

    let mut encoder = Device::new(encoder_device)?;

    if !camera_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
        panic!("Encoder: Capture not supported")
    }
    if !camera_caps.capabilities.contains(Flags::VIDEO_OUTPUT) {
        panic!("Encoder: Output not supported")
    }
    if !camera_caps.capabilities.contains(Flags::STREAMING) {
        panic!("Encoder: Streaming not supported")
    }

    Output::set_format(&mut encoder, &camera_format)?;
    Capture::set_format(&mut encoder, &encoded_format)?;
    Output::set_params(&mut encoder, &output::Parameters::with_fps(fps))?;

    let mut camera_queue = Queue::with_mmap(camera.handle(), Type::VideoCapture, 2)?;
    let mut encoder_raw_queue = Queue::with_mmap(encoder.handle(), Type::VideoOutput, 1)?;
    let mut encoder_encoded_queue = Queue::with_mmap(encoder.handle(), Type::VideoCapture, 1)?;

    for i in 0..camera_queue.len() {
        camera_queue.enqueue(&buffer_metadata(i))?;
    }

    for i in 0..camera_queue.len() {
        camera_queue.enqueue(&buffer_metadata(i))?;
    }

    let mut camera_queue = camera_queue.start_stream()?;
    let mut encoder_raw_queue = encoder_raw_queue.start_stream()?;
    let mut encoder_encoded_queue = encoder_encoded_queue.start_stream()?;

    let mut write_to = File::open("test.h264")?;

    for _ in 0..100 {
        let mut encoder_raw_buf_meta = encoder_raw_queue.dequeue()?;
        let encoder_raw_buf = &mut encoder_raw_queue[encoder_raw_buf_meta.index as usize].0;

        let camera_buf_meta = camera_queue.dequeue()?;
        let camera_buf = &camera_queue[camera_buf_meta.index as usize].0;
        (*encoder_raw_buf).copy_from_slice(&camera_buf[..camera_buf.len()]);
        encoder_raw_buf_meta.bytesused = camera_buf.len() as u32;
        camera_queue.enqueue(&camera_buf_meta)?;

        encoder_raw_queue.enqueue(&encoder_raw_buf_meta)?;

        let encoder_encoded_meta = encoder_encoded_queue.dequeue()?;
        let encoder_encoded_buf = &encoder_encoded_queue[encoder_encoded_meta.index as usize].0;

        write_to.write(&encoder_encoded_buf[..encoder_encoded_meta.bytesused as usize])?;
    }

    Ok(())
}

fn buffer_metadata(index: usize) -> Metadata {
    Metadata {
        memory: Memory::Mmap,
        index: index as u32,
        ..unsafe { std::mem::zeroed() }
    }
}

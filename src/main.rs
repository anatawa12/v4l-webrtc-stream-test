use std::fs::File;
use std::io;
use std::io::Write;
use std::marker::PhantomData;
use std::sync::Arc;
use v4l::prelude::*;
use v4l::{Format, FourCC, Memory};
use v4l::buffer::{Metadata, Type};
use v4l::capability::Flags;
use v4l::device::{Handle, MultiPlaneDevice};
use v4l::format::MultiPlaneFormat;
use v4l::io::mmap::stream;
use v4l::io::Queue;
use v4l::io::queue::Streaming;
use v4l::io::traits::{CaptureStream, OutputStream, Stream};
use v4l::memory::Mmap;
use v4l::video::{Capture, capture, Output, output};

fn main() -> io::Result<()> {
    println!("Hello, world!");
    let camera_device = 0;
    let encoder_device = 11;
    let fps = 10;
    let width = 640;
    let height = 480;
    let camera_fourcc = FourCC::new(b"RGB3");
    let encoded_fourcc = FourCC::new(b"H264");

    //let mut camera = Device::new(camera_device).unwrap();

    //let camera_caps = camera.query_caps().unwrap();
    //if !camera_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
    //    panic!("Camera: Capture not supported")
    //}
    //if !camera_caps.capabilities.contains(Flags::STREAMING) {
    //    panic!("Camera: Streaming not supported")
    //}

    //Capture::set_format(&mut camera, &camera_format).unwrap();
    //Capture::set_params(&mut camera, &capture::Parameters::with_fps(fps)).unwrap();

    let mut encoder = MultiPlaneDevice::new(encoder_device).unwrap();

    let encoder_caps = encoder.query_caps().unwrap();
    println!("Encoder capabilities: {}", encoder_caps.capabilities);
    if !encoder_caps.capabilities.contains(Flags::VIDEO_M2M_MPLANE) {
        panic!("Encoder: Capture not supported")
    }
    if !encoder_caps.capabilities.contains(Flags::STREAMING) {
        panic!("Encoder: Streaming not supported")
    }

    Output::set_format(&mut encoder, &MultiPlaneFormat::single_plane(width, height, camera_fourcc)).unwrap();
    Capture::set_format(&mut encoder, &MultiPlaneFormat::single_plane(width, height, encoded_fourcc)).unwrap();
    Output::set_params(&mut encoder, &output::Parameters::with_fps(fps)).unwrap();

    //let mut camera_stream = CaptureStream::with_device(&camera, 3).unwrap();
    let mut encoder_raw_stream1 = MmapStream::with_buffers(&encoder, Type::VideoOutputMplane, 1).unwrap();
    let mut encoder_encoded_stream1 = MmapStream::with_buffers(&encoder, Type::VideoCaptureMplane, 1).unwrap();
    //let mut encoder_raw_stream = MultiPlaneOutputStream::with_device(&encoder, 1).unwrap();
    //let mut encoder_encoded_queue = MultiPlaneCaptureStream::with_device(&encoder, 1).unwrap();

    OutputStream::queue(&mut encoder_raw_stream1, 0).unwrap();
    CaptureStream::queue(&mut encoder_encoded_stream1, 0).unwrap();

    encoder_raw_stream1.start().unwrap();
    encoder_encoded_stream1.start().unwrap();

    let mut write_to = File::create("test.h264").unwrap();

    for i in 0..100 {
        println!("frame {i}");
        let index = OutputStream::dequeue(&mut encoder_raw_stream1).unwrap();
        println!("frame {i}: deq");
        let (out_buffers, _meta, planes) = OutputStream::get(&mut encoder_raw_stream1, index).unwrap();
        out_buffers[0][..640 * 480 * 3].fill(i);
        planes[0].bytesused = 640 * 480 * 3;
        println!("frame {i}: queueing");
        OutputStream::queue(&mut encoder_raw_stream1, index).unwrap();
        println!("frame {i}: que");

        let index = CaptureStream::dequeue(&mut encoder_encoded_stream1).unwrap();
        println!("frame {i}: deq");
        let (out_buffers, _meta, planes) = CaptureStream::get(&encoder_encoded_stream1, index).unwrap();
        write_to.write(&out_buffers[0][..planes[0].bytesused as usize]).unwrap();
        OutputStream::queue(&mut encoder_encoded_stream1, index).unwrap();
        println!("frame {i}: que");

        //encoder_raw_stream.write_frame(|buffer| {
        //    //camera_stream.read_frame(|frame| {
        //    //    buffer.copy_from_slice(frame);
        //    //    Ok(frame.len())
        //    //})
        //    buffer[..640 * 480 * 3].fill(i);
        //    Ok(640 * 480 * 3)
        //}).unwrap();

        //encoder_encoded_queue.read_frame(|buffer| {
        //    write_to.write(buffer).unwrap();
        //    Ok(())
        //}).unwrap();
    }
    encoder_raw_stream1.stop()?;
    encoder_encoded_stream1.stop()?;

    write_to.flush()?;

    //camera_stream.finish().unwrap();
    //encoder_raw_stream.finish().unwrap();
    //encoder_encoded_queue.finish().unwrap();

    Ok(())
}

fn buffer_metadata(index: usize) -> Metadata {
    Metadata {
        index: index as u32,
        length: 1,
        ..Metadata::with_memory(Memory::Mmap)
    }
}

//type CaptureStream<'a> = StreamBase<'a, VideoCapture>;
//type OutputStream<'a> = StreamBase<'a, VideoOutput>;
#[allow(dead_code)]
type MultiPlaneCaptureStream<'a> = StreamBase<'a, MultiPlaneVideoCapture>;
#[allow(dead_code)]
type MultiPlaneOutputStream<'a> = StreamBase<'a, MultiPlaneVideoOutput>;

trait StreamTypeMarker {
    const TYPE: Type;
}

macro_rules! marker {
    ($name: ident, $value: ident) => {
        struct $name;
        impl StreamTypeMarker for $name {
            const TYPE: Type = Type::$value;
        }
    };
}

marker!(VideoCapture, VideoCapture);
marker!(VideoOutput, VideoOutput);
marker!(MultiPlaneVideoCapture, VideoCaptureMplane);
marker!(MultiPlaneVideoOutput, VideoOutputMplane);

struct StreamBase<'a, Type: StreamTypeMarker> {
    queue: Queue<Mmap<'a>, Streaming>,
    _phantom: PhantomData<Type>,
}

macro_rules! with_device {
    ($Device: ident, $Type: ty) => {
        impl <'a> StreamBase<'a, $Type> {
            #[inline]
            #[allow(dead_code)]
            pub fn with_device(device: &$Device, buf_count: u32) -> io::Result<Self> {
                Self::with_device_impl(device.handle(), buf_count)
            }
        }
    };
}

with_device!(MultiPlaneDevice, MultiPlaneVideoCapture);
with_device!(MultiPlaneDevice, MultiPlaneVideoOutput);
with_device!(Device, VideoCapture);
with_device!(Device, VideoOutput);

impl <'a, Type : StreamTypeMarker> StreamBase<'a, Type> {
    fn with_device_impl(device: Arc<Handle>, buf_count: u32) -> io::Result<Self> {
        let mut queue = Queue::with_mmap(device, Type::TYPE, buf_count).unwrap();

        for i in 0..queue.len() {
            queue.enqueue(&buffer_metadata(i)).unwrap();
        }

        let queue = queue.start_stream().unwrap();

        return Ok(Self { queue, _phantom: PhantomData })
    }
    
    pub fn finish(self) -> io::Result<()> {
        let Self{ queue, _phantom } = self;
        queue.stop_stream().unwrap();
        Ok(())
    }
}

impl <'a> StreamBase<'a, VideoCapture> {
    pub(crate) fn read_frame<R>(
        &mut self,
        process_data: impl for <'b> FnOnce(&'b [u8]) -> io::Result<R>,
    ) -> io::Result<R> {
        let encoder_encoded_meta = self.queue.dequeue().unwrap();
        let encoder_encoded_buf = &self.queue[encoder_encoded_meta.index as usize].0[..encoder_encoded_meta.bytesused as usize];
        let result = process_data(encoder_encoded_buf).unwrap();
        self.queue.enqueue(&encoder_encoded_meta).unwrap();
        Ok(result)
    }
}

impl <'a> StreamBase<'a, VideoOutput> {
    pub(crate) fn write_frame(
        &mut self,
        get_raw_data: impl for <'b> FnOnce(&'b mut [u8]) -> io::Result<usize>,
    ) -> io::Result<()> {
        let mut encoder_encoded_meta = self.queue.dequeue().unwrap();
        let encoder_encoded_buf = &mut self.queue[encoder_encoded_meta.index as usize].0[..];
        encoder_encoded_meta.bytesused = get_raw_data(encoder_encoded_buf).unwrap() as u32;
        self.queue.enqueue(&encoder_encoded_meta).unwrap();
        Ok(())
    }
}

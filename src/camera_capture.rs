use std::io;
use std::sync::Arc;
use tokio::io::unix::AsyncFd;
use tokio::io::Interest;
use v4l::buffer::Type;
use v4l::capability::Flags;
use v4l::device::MultiPlaneDevice;
use v4l::format::MultiPlaneFormat;
use v4l::io::traits::{CaptureStream, OutputStream, Stream};
use v4l::prelude::*;
use v4l::video::{capture, output, Capture, Output};
use v4l::{Format, FourCC};

pub struct CameraCapture<'a> {
    camera_async_fd: AsyncFd<Arc<v4l::device::Handle>>,
    encoder_async_fd: AsyncFd<Arc<v4l::device::Handle>>,
    camera_stream: MmapStream<'a>,
    encoder_raw_stream1: MmapStream<'a>,
    encoder_encoded_stream1: MmapStream<'a>,
}

impl<'a> CameraCapture<'a> {
    pub fn new(
        camera_device: usize,
        encoder_device: usize,
        fps: u32,
        width: u32,
        height: u32,
        camera_fourcc: &[u8; 4],
        encoded_fourcc: &[u8; 4],
    ) -> io::Result<Self> {
        let mut camera = Device::new(camera_device).unwrap();
        let camera_async_fd = AsyncFd::with_interest(camera.handle(), Interest::READABLE)
            .expect("creating async fd for camera");

        let camera_caps = camera.query_caps().unwrap();
        if !camera_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
            panic!("Camera: Capture not supported")
        }
        if !camera_caps.capabilities.contains(Flags::STREAMING) {
            panic!("Camera: Streaming not supported")
        }

        Capture::set_format(
            &mut camera,
            &Format::new(width, height, FourCC::new(camera_fourcc)),
        )
        .unwrap();
        Capture::set_params(&mut camera, &capture::Parameters::with_fps(fps)).unwrap();

        let mut encoder = MultiPlaneDevice::new(encoder_device).unwrap();
        let encoder_async_fd =
            AsyncFd::new(encoder.handle()).expect("creating async fd for encoder");

        let encoder_caps = encoder.query_caps().unwrap();
        println!("Encoder capabilities: {}", encoder_caps.capabilities);
        if !encoder_caps.capabilities.contains(Flags::VIDEO_M2M_MPLANE) {
            panic!("Encoder: Capture not supported")
        }
        if !encoder_caps.capabilities.contains(Flags::STREAMING) {
            panic!("Encoder: Streaming not supported")
        }

        Output::set_format(
            &mut encoder,
            &MultiPlaneFormat::single_plane(width, height, FourCC::new(camera_fourcc)),
        )
        .unwrap();
        Capture::set_format(
            &mut encoder,
            &MultiPlaneFormat::single_plane(width, height, FourCC::new(encoded_fourcc)),
        )
        .unwrap();
        Output::set_params(&mut encoder, &output::Parameters::with_fps(fps)).unwrap();

        let mut camera_stream = MmapStream::with_buffers(&camera, Type::VideoCapture, 3).unwrap();
        let mut encoder_raw_stream1 =
            MmapStream::with_buffers(&encoder, Type::VideoOutputMplane, 1).unwrap();
        let mut encoder_encoded_stream1 =
            MmapStream::with_buffers(&encoder, Type::VideoCaptureMplane, 1).unwrap();

        CaptureStream::queue(&mut camera_stream, 0).unwrap();
        CaptureStream::queue(&mut camera_stream, 1).unwrap();
        CaptureStream::queue(&mut camera_stream, 2).unwrap();
        OutputStream::queue(&mut encoder_raw_stream1, 0).unwrap();
        CaptureStream::queue(&mut encoder_encoded_stream1, 0).unwrap();

        Ok(Self {
            camera_async_fd,
            encoder_async_fd,
            camera_stream,
            encoder_raw_stream1,
            encoder_encoded_stream1,
        })
    }
}

impl<'a> CameraCapture<'a> {
    pub async fn take_frame(&mut self) -> io::Result<Vec<u8>> {
        let read_write: Interest = Interest::WRITABLE | Interest::READABLE;
        println!("frame");
        let index = self
            .encoder_async_fd
            .async_io(read_write, |_| {
                OutputStream::dequeue(&mut self.encoder_raw_stream1)
            })
            .await
            .unwrap();
        println!("frame: deq");
        let (out_buffers, _meta, planes) =
            OutputStream::get(&mut self.encoder_raw_stream1, index).unwrap();

        println!("frame: cam polling");
        let cam_index = self
            .camera_async_fd
            .async_io(read_write, |_| {
                CaptureStream::dequeue(&mut self.camera_stream)
            })
            .await
            .unwrap();
        println!("frame: cam getting");
        let (cam_buffers, cam_meta, _cam_planes) =
            CaptureStream::get(&self.camera_stream, cam_index).unwrap();
        let cam_len = cam_meta.length;
        let cam_buffer = &cam_buffers[0][..cam_len as usize];
        out_buffers[0][..cam_len as usize].copy_from_slice(cam_buffer);
        println!("frame: cam queueing");
        CaptureStream::queue(&mut self.camera_stream, cam_index).unwrap();

        planes[0].bytesused = cam_len;
        println!("frame: queueing");
        OutputStream::queue(&mut self.encoder_raw_stream1, index).unwrap();
        println!("frame: que");

        let index = self
            .encoder_async_fd
            .async_io(read_write, |_| {
                CaptureStream::dequeue(&mut self.encoder_encoded_stream1)
            })
            .await
            .unwrap();
        println!("frame: deq");
        let (out_buffers, _meta, planes) =
            CaptureStream::get(&self.encoder_encoded_stream1, index).unwrap();
        let buffer = Vec::from(&out_buffers[0][..planes[0].bytesused as usize]);
        CaptureStream::queue(&mut self.encoder_encoded_stream1, index).unwrap();
        println!("frame: que");

        return Ok(buffer);
    }
}

impl<'a> CameraCapture<'a> {
    pub fn start(&mut self) -> io::Result<()> {
        self.camera_stream.start()?;
        self.encoder_raw_stream1.start()?;
        self.encoder_encoded_stream1.start()?;
        Ok(())
    }

    pub fn stop(&mut self) -> io::Result<()> {
        self.camera_stream.stop()?;
        self.encoder_raw_stream1.stop()?;
        self.encoder_encoded_stream1.stop()?;
        Ok(())
    }
}

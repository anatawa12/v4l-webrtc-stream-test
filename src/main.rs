use anyhow::Result;
use std::fs::File;
use std::io;
use std::io::Write;
use std::marker::PhantomData;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::Notify;
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
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::io::h264_reader::H264Reader;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

#[tokio::main]
async fn main_() -> Result<()> {
    // Everything below is the WebRTC-rs API! Thanks for using it ❤️.

    // Create a MediaEngine object to configure the supported codec
    let mut m = MediaEngine::default();

    m.register_default_codecs()?;

    // Create a InterceptorRegistry. This is the user configurable RTP/RTCP Pipeline.
    // This provides NACKs, RTCP Reports and other features. If you use `webrtc.NewPeerConnection`
    // this is enabled by default. If you are manually managing You MUST create a InterceptorRegistry
    // for each PeerConnection.
    let mut registry = Registry::new();

    // Use the default set of Interceptors
    registry = register_default_interceptors(registry, &mut m)?;

    // Create the API object with the MediaEngine
    let api = APIBuilder::new()
        .with_media_engine(m)
        .with_interceptor_registry(registry)
        .build();

    // Prepare the configuration
    let config = RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    };

    // Create a new RTCPeerConnection
    let peer_connection = Arc::new(api.new_peer_connection(config).await?);

    let notify_tx = Arc::new(Notify::new());
    let notify_video = notify_tx.clone();

    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);
    let video_done_tx = done_tx.clone();

     {
         let video_file = "test.h264";
        // Create a video track
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_H264.to_owned(),
                ..Default::default()
            },
            "video".to_owned(),
            "webrtc-rs".to_owned(),
        ));

        // Add this newly created track to the PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Read incoming RTCP packets
        // Before these packets are returned they are processed by interceptors. For things
        // like NACK this needs to be called.
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
            Result::<()>::Ok(())
        });

        let video_file_name = video_file.to_owned();
        tokio::spawn(async move {
            // Open a H264 file and start reading using our H264Reader
            let file = File::open(&video_file_name)?;
            let reader = io::BufReader::new(file);
            let mut h264 = H264Reader::new(reader, 1_048_576);

            let mut buffers = Vec::new();
            let mut index = 0;
            loop {
                println!("parsing {index}");
                match h264.next_nal() {
                    Ok(nal) => buffers.push(nal),
                    Err(err) => {
                        println!("All video frames parsed: {err}");
                        break;
                    }
                };
                println!("parsed {index}");
                index += 1;
            }

            // Wait for connection established
            notify_video.notified().await;

            println!("play video from disk file {video_file_name}");

            // It is important to use a time.Ticker instead of time.Sleep because
            // * avoids accumulating skew, just calling time.Sleep didn't compensate for the time spent parsing the data
            // * works around latency issues with Sleep
            let mut ticker = tokio::time::interval(Duration::from_millis(100));
            for nal in buffers {

                /*println!(
                    "PictureOrderCount={}, ForbiddenZeroBit={}, RefIdc={}, UnitType={}, data={}",
                    nal.picture_order_count,
                    nal.forbidden_zero_bit,
                    nal.ref_idc,
                    nal.unit_type,
                    nal.data.len()
                );*/

                video_track
                    .write_sample(&Sample {
                        data: nal.data.freeze(),
                        duration: Duration::from_secs(1),
                        ..Default::default()
                    })
                    .await?;

                let _ = ticker.tick().await;
            }

            let _ = video_done_tx.try_send(());

            Result::<()>::Ok(())
        });
    }

    // Set the handler for ICE connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_ice_connection_state_change(Box::new(
        move |connection_state: RTCIceConnectionState| {
            println!("Connection State has changed {connection_state}");
            if connection_state == RTCIceConnectionState::Connected {
                notify_tx.notify_waiters();
            }
            Box::pin(async {})
        },
    ));

    // Set the handler for Peer connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        println!("Peer Connection State has changed: {s}");

        if s == RTCPeerConnectionState::Failed {
            // Wait until PeerConnection has had no network activity for 30 seconds or another failure. It may be reconnected using an ICE Restart.
            // Use webrtc.PeerConnectionStateDisconnected if you are interested in detecting faster timeout.
            // Note that the PeerConnection may come back from PeerConnectionStateDisconnected.
            println!("Peer Connection has gone to failed exiting");
            let _ = done_tx.try_send(());
        }

        Box::pin(async {})
    }));

    // Wait for the offer to be pasted
    let offer = io::read_to_string(io::stdin())?;
    let offer = RTCSessionDescription::offer(offer)?;

    // Set the remote SessionDescription
    peer_connection.set_remote_description(offer).await?;

    // Create an answer
    let answer = peer_connection.create_answer(None).await?;

    // Create channel that is blocked until ICE Gathering is complete
    let mut gather_complete = peer_connection.gathering_complete_promise().await;

    // Sets the LocalDescription, and starts our UDP listeners
    peer_connection.set_local_description(answer).await?;

    // Block until ICE Gathering is complete, disabling trickle ICE
    // we do this because we only can exchange one signaling message
    // in a production application you should exchange ICE Candidates via OnICECandidate
    let _ = gather_complete.recv().await;

    // Output the answer in base64 so we can paste it in browser
    if let Some(local_desc) = peer_connection.local_description().await {
        println!("{}", local_desc.sdp);
    } else {
        println!("generate local_description failed!");
    }

    println!("Press ctrl-c to stop");
    tokio::select! {
        _ = done_rx.recv() => {
            println!("received done signal!");
        }
        _ = tokio::signal::ctrl_c() => {
            println!();
        }
    };

    peer_connection.close().await?;

    Ok(())
}


#[tokio::main]
async fn main() -> io::Result<()> {
    println!("Hello, world!");
    let camera_device = 0;
    let encoder_device = 11;
    let fps = 10;
    let width = 640;
    let height = 480;
    let camera_fourcc = FourCC::new(b"YUYV");
    let encoded_fourcc = FourCC::new(b"H264");

    let read_write: Interest = Interest::WRITABLE | Interest::READABLE;

    let mut camera = Device::new(camera_device).unwrap();
    let camera_async_fd = AsyncFd::with_interest(camera.handle(), Interest::READABLE).expect("creating async fd for camera");

    let camera_caps = camera.query_caps().unwrap();
    if !camera_caps.capabilities.contains(Flags::VIDEO_CAPTURE) {
        panic!("Camera: Capture not supported")
    }
    if !camera_caps.capabilities.contains(Flags::STREAMING) {
        panic!("Camera: Streaming not supported")
    }

    Capture::set_format(&mut camera, &Format::new(width, height, camera_fourcc)).unwrap();
    Capture::set_params(&mut camera, &capture::Parameters::with_fps(fps)).unwrap();

    let mut encoder = MultiPlaneDevice::new(encoder_device).unwrap();
    let encoder_async_fd = AsyncFd::new(encoder.handle()).expect("creating async fd for encoder");

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

    let mut camera_stream = MmapStream::with_buffers(&camera, Type::VideoCapture, 3).unwrap();
    let mut encoder_raw_stream1 = MmapStream::with_buffers(&encoder, Type::VideoOutputMplane, 1).unwrap();
    let mut encoder_encoded_stream1 = MmapStream::with_buffers(&encoder, Type::VideoCaptureMplane, 1).unwrap();
    //let mut encoder_raw_stream = MultiPlaneOutputStream::with_device(&encoder, 1).unwrap();
    //let mut encoder_encoded_queue = MultiPlaneCaptureStream::with_device(&encoder, 1).unwrap();

    CaptureStream::queue(&mut camera_stream, 0).unwrap();
    CaptureStream::queue(&mut camera_stream, 1).unwrap();
    CaptureStream::queue(&mut camera_stream, 2).unwrap();
    OutputStream::queue(&mut encoder_raw_stream1, 0).unwrap();
    CaptureStream::queue(&mut encoder_encoded_stream1, 0).unwrap();

    camera_stream.start().unwrap();
    encoder_raw_stream1.start().unwrap();
    encoder_encoded_stream1.start().unwrap();

    let mut write_to = File::create("test.h264").unwrap();

    let mut interval = tokio::time::interval(Duration::from_secs(1) / 15);
    for i in 0..512 {
        println!("frame {i}");
        let index = encoder_async_fd.async_io(read_write, |_| {
            OutputStream::dequeue(&mut encoder_raw_stream1)
        }).await.unwrap();
        println!("frame {i}: deq");
        let (out_buffers, _meta, planes) = OutputStream::get(&mut encoder_raw_stream1, index).unwrap();

        println!("frame {i}: cam polling");
        let cam_index = camera_async_fd.async_io(read_write, |_| {
            CaptureStream::dequeue(&mut camera_stream)
        }).await.unwrap();
        println!("frame {i}: cam getting");
        let (cam_buffers, cam_meta, _cam_planes) = CaptureStream::get(&camera_stream, cam_index).unwrap();
        let cam_len = cam_meta.length;
        let cam_buffer = &cam_buffers[0][..cam_len as usize];
        out_buffers[0][..cam_len as usize].copy_from_slice(cam_buffer);
        println!("frame {i}: cam queueing");
        CaptureStream::queue(&mut camera_stream, cam_index).unwrap();

        planes[0].bytesused = cam_len;
        println!("frame {i}: queueing");
        OutputStream::queue(&mut encoder_raw_stream1, index).unwrap();
        println!("frame {i}: que");

        let index = encoder_async_fd.async_io(read_write, |_| {
            CaptureStream::dequeue(&mut encoder_encoded_stream1)
        }).await.unwrap();
        println!("frame {i}: deq");
        let (out_buffers, _meta, planes) = CaptureStream::get(&encoder_encoded_stream1, index).unwrap();
        write_to.write(&out_buffers[0][..planes[0].bytesused as usize]).unwrap();
        CaptureStream::queue(&mut encoder_encoded_stream1, index).unwrap();
        println!("frame {i}: que");

        interval.tick().await;

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
    camera_stream.stop()?;
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

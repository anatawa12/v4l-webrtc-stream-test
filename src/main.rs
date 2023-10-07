// this file is based on https://github.com/webrtc-rs/webrtc/blob/982829bffe07c61bce660b20499d9148861e0224/examples/examples/play-from-disk-h264/play-from-disk-h264.rs
// but plays media from v4l2 capture device and encode to h264 with v4l2 hw encoder device.
// originally published under MIT or Apache 2.0
// Copyright (c) 2021 WebRTC.rs
// see https://github.com/webrtc-rs/webrtc/blob/982829bffe07c61bce660b20499d9148861e0224/examples/LICENSE-MIT
// or https://github.com/webrtc-rs/webrtc/blob/982829bffe07c61bce660b20499d9148861e0224/examples/LICENSE-APACHE
// for more details about original license

mod nal_parser;
mod camera_capture;

use crate::nal_parser::H264Parser;
use anyhow::Result;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;
use crate::camera_capture::CameraCapture;

#[tokio::main]
async fn main() -> Result<()> {
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

        tokio::spawn(async move {
            let camera_device: usize = 0;
            let encoder_device: usize = 11;
            let fps: u32 = 10;
            let width: u32 = 640;
            let height: u32 = 480;
            let camera_fourcc = b"YUYV";
            let encoded_fourcc = b"H264";

            let mut capture = CameraCapture::new(camera_device, encoder_device, fps, width, height, camera_fourcc, encoded_fourcc)?;

            // Wait for connection established
            notify_video.notified().await;

            println!("play video from camera");

            capture.start()?;

            // It is important to use a time.Ticker instead of time.Sleep because
            // * avoids accumulating skew, just calling time.Sleep didn't compensate for the time spent parsing the data
            // * works around latency issues with Sleep
            let interval = Duration::from_secs(1) / fps;
            let mut ticker = tokio::time::interval(interval);
            let mut frame = 0;
            println!("interval: {interval:?}");
            for _ in 0..1024 {
                println!("frame0: {frame}");
                frame += 1;


                let buffer = capture.take_frame().await?;

                /*println!(
                    "PictureOrderCount={}, ForbiddenZeroBit={}, RefIdc={}, UnitType={}, data={}",
                    nal.picture_order_count,
                    nal.forbidden_zero_bit,
                    nal.ref_idc,
                    nal.unit_type,
                    nal.data.len()
                );*/

                let mut h264 = H264Parser::new(buffer.as_slice());
                while let Some(nal) = h264.next_buffer()? {
                    video_track
                        .write_sample(&Sample {
                            data: Vec::from(nal).into(),
                            duration: Duration::from_secs(1),
                            ..Default::default()
                        })
                        .await?;
                }

                println!("frame1: {frame}");
                let _ = ticker.tick().await;
                println!("frame2: {frame}");
            }

            capture.stop()?;

            let _ = video_done_tx.try_send(());

            Result::<()>::Ok(())
        });
    }

    // Set the handler for ICE connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_ice_connection_state_change(Box::new(
        move |connection_state: RTCIceConnectionState| {
            println!("Connection State has changed {connection_state}");
            Box::pin(async {})
        },
    ));

    // Set the handler for Peer connection state
    // This will notify you when the peer has connected/disconnected
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        println!("Peer Connection State has changed: {s}");

        if s == RTCPeerConnectionState::Connected {
            notify_tx.notify_waiters();
        }
        if s == RTCPeerConnectionState::Failed {
            // Wait until PeerConnection has had no network activity for 30 seconds or another failure. It may be reconnected using an ICE Restart.
            // Use webrtc.PeerConnectionStateDisconnected if you are interested in detecting faster timeout.
            // Note that the PeerConnection may come back from PeerConnectionStateDisconnected.
            println!("Peer Connection has gone to failed exiting");
            let _ = done_tx.try_send(());
        }

        Box::pin(async {})
    }));

    println!("enter offer:");
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

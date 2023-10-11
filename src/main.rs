// this file is based on https://github.com/webrtc-rs/webrtc/blob/982829bffe07c61bce660b20499d9148861e0224/examples/examples/play-from-disk-h264/play-from-disk-h264.rs
// but plays media from v4l2 capture device and encode to h264 with v4l2 hw encoder device.
// originally published under MIT or Apache 2.0
// Copyright (c) 2021 WebRTC.rs
// see https://github.com/webrtc-rs/webrtc/blob/982829bffe07c61bce660b20499d9148861e0224/examples/LICENSE-MIT
// or https://github.com/webrtc-rs/webrtc/blob/982829bffe07c61bce660b20499d9148861e0224/examples/LICENSE-APACHE
// for more details about original license

// You can use https://jsfiddle.net/8j26fhxk/ as browser side

mod audio;
mod camera_capture;
mod monaural_audio_capture;
mod monaural_audio_playback;
mod nal_parser;

use crate::camera_capture::CameraCapture;
use crate::monaural_audio_capture::MonauralAudioCapture;
use crate::monaural_audio_playback::MonauralAudioPlayback;
use crate::nal_parser::H264Parser;
use anyhow::Result;
use clap::Parser;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264, MIME_TYPE_OPUS};
use webrtc::api::APIBuilder;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::interceptor::registry::Registry;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use webrtc::rtp_transceiver::rtp_codec::{
    RTCRtpCodecCapability, RTCRtpCodecParameters, RTPCodecType,
};
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

#[derive(clap::Parser)]
struct Cli {
    /// The capture device to be streamed
    #[clap(long, default_value = "0")]
    camera_device: usize,
    /// The hw encoder device
    #[clap(long, default_value = "11")]
    encoder_device: usize,

    /// Capture & streaming FPS
    #[clap(long, default_value = "15")]
    fps: u32,
    /// Size of capture buffer
    #[clap(long, default_value = "3")]
    capture_buffer: u32,

    /// Capture & Streaming video width
    #[clap(long, default_value = "640")]
    width: u32,
    /// Capture & Streaming video width
    #[clap(long, default_value = "480")]
    height: u32,

    /// FourCC to use capture & input format of encoder
    #[clap(long, default_value = "YUYV")]
    camera_fourcc: FourCC,

    // audio options
    /// Sampling rate of capture (Hz)
    #[clap(long, default_value = "48000")]
    sample_rate: u32,
    /// Bitrate of audio (bit per second)
    #[clap(long, default_value = "28000")]
    bit_rate: u32,
    /// Length of audio capture frame (milliseconds)
    #[clap(long, default_value = "20")]
    frame_ms: u32,
    /// Audio Device Name
    #[clap(long, default_value = "plughw:1,0")]
    audio_device: String,
}

#[derive(Copy, Clone)]
struct FourCC([u8; 4]);

impl clap::builder::ValueParserFactory for FourCC {
    type Parser = clap::builder::ValueParser;

    fn value_parser() -> Self::Parser {
        Self::Parser::new(|str: &str| <[u8; 4]>::try_from(str.as_bytes()).map(FourCC))
    }
}

const RECEIVE_SAMPLE_RATE: u32 = 48000;

#[tokio::main]
async fn main() -> Result<()> {
    let parsed = Cli::parse();

    // Create a MediaEngine object to configure the supported codec
    let mut m = MediaEngine::default();

    media_engine(&mut m)?;

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
    let notify_audio = notify_tx.clone();

    let done_notify = Arc::new(Notify::new());

    let (done_tx, mut done_rx) = tokio::sync::mpsc::channel::<()>(1);
    let video_done_tx = done_tx.clone();

    let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));

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

        let connected = connected.clone();

        // Add this newly created track to the PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;
        let rtp_sender_1 = rtp_sender.clone();

        // Read incoming RTCP packets
        // Before these packets are returned they are processed by interceptors. For things
        // like NACK this needs to be called.
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
            Result::<()>::Ok(())
        });

        tokio::spawn(async move {
            let mut capture = CameraCapture::new(
                parsed.camera_device,
                parsed.encoder_device,
                parsed.fps,
                parsed.capture_buffer,
                parsed.width,
                parsed.height,
                &parsed.camera_fourcc.0,
                b"H264",
            )?;

            // Wait for connection established
            notify_video.notified().await;

            println!("play video from camera");

            capture.start()?;

            // It is important to use a time.Ticker instead of time.Sleep because
            // * avoids accumulating skew, just calling time.Sleep didn't compensate for the time spent parsing the data
            // * works around latency issues with Sleep
            let interval = Duration::from_secs(1) / parsed.fps;
            let mut ticker = tokio::time::interval(interval);
            while connected.load(std::sync::atomic::Ordering::Relaxed) {
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
                            duration: interval,
                            ..Default::default()
                        })
                        .await?;
                }

                let _ = ticker.tick().await;
            }

            capture.stop()?;
            rtp_sender_1.stop().await?;

            let _ = video_done_tx.try_send(());

            Result::<()>::Ok(())
        });
    }

    {
        let connected = connected.clone();

        // Create a audio track
        let audio_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                ..Default::default()
            },
            "audio".to_owned(),
            "webrtc-rs".to_owned(),
        ));

        // Add this newly created track to the PeerConnection
        let rtp_sender = peer_connection
            .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
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
            let mut capture = MonauralAudioCapture::new(
                &parsed.audio_device,
                parsed.sample_rate,
                parsed.bit_rate as i32,
                parsed.frame_ms,
            )?;

            // Wait for connection established
            notify_audio.notified().await;

            println!("play audio from disk file test.ogg");

            // It is important to use a time.Ticker instead of time.Sleep because
            // * avoids accumulating skew, just calling time.Sleep didn't compensate for the time spent parsing the data
            // * works around latency issues with Sleep
            let mut ticker = tokio::time::interval(Duration::from_millis(parsed.frame_ms as u64));

            // Keep track of last granule, the difference is the amount of samples in the buffer
            while connected.load(std::sync::atomic::Ordering::Relaxed) {
                let (duration, encoded_buffer) = capture.capture_frame()?;

                // The amount of samples is the difference between the last and current timestamp
                audio_track
                    .write_sample(&Sample {
                        data: Vec::from(encoded_buffer).into(),
                        duration,
                        ..Default::default()
                    })
                    .await?;

                let _ = ticker.tick().await;
            }

            drop(capture); // close

            Result::<()>::Ok(())
        });
    }

    let pc = Arc::downgrade(&peer_connection);
    peer_connection.on_track(Box::new(move |track, _, _| {
        // Send a PLI on an interval so that the publisher is pushing a keyframe every rtcpPLIInterval
        let media_ssrc = track.ssrc();
        let pc2 = pc.clone();
        tokio::spawn(async move {
            let mut result = Result::<usize>::Ok(0);
            while result.is_ok() {
                let timeout = tokio::time::sleep(Duration::from_secs(3));
                tokio::pin!(timeout);

                tokio::select! {
                    _ = timeout.as_mut() =>{
                        if let Some(pc) = pc2.upgrade() {
                            result = pc.write_rtcp(&[Box::new(PictureLossIndication {
                                sender_ssrc: 0,
                                media_ssrc,
                            })]).await.map_err(Into::into);
                        }else {
                            break;
                        }
                    }
                };
            }
        });

        Box::pin(async move {
            let codec = track.codec();
            let mime_type = codec.capability.mime_type.to_lowercase();
            if mime_type == MIME_TYPE_OPUS.to_lowercase() {
                println!("Got Opus track, Playing (48 kHz, 1 channels)");
                tokio::spawn(async move {
                    let mut playback = MonauralAudioPlayback::new("default", RECEIVE_SAMPLE_RATE)?;

                    loop {
                        let (rtp_packet, _) = track.read_rtp().await?;
                        playback.play_frame(&rtp_packet.payload)?;
                    }

                    Result::<()>::Ok(())
                });
            }
        })
    }));

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
            println!("ctrl_c");
            connected.store(false, std::sync::atomic::Ordering::Relaxed);
            done_rx.recv().await;
        }
    };

    peer_connection.close().await?;

    Ok(())
}

fn media_engine(m: &mut MediaEngine) -> Result<(), webrtc::Error> {
    let fmt_line = [
        (
            102,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42001f",
        ),
        (
            127,
            "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42001f",
        ),
        (
            125,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f",
        ),
        (
            108,
            "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f",
        ),
        (
            123,
            "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640032",
        ),
    ];

    for (payload_type, sdp_fmtp_line) in fmt_line {
        m.register_codec(
            RTCRtpCodecParameters {
                capability: RTCRtpCodecCapability {
                    mime_type: MIME_TYPE_H264.to_owned(),
                    clock_rate: 90000,
                    channels: 0,
                    sdp_fmtp_line: sdp_fmtp_line.to_owned(),
                    rtcp_feedback: vec![],
                },
                payload_type,
                ..Default::default()
            },
            RTPCodecType::Video,
        )?;
    }

    m.register_codec(
        RTCRtpCodecParameters {
            capability: RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: RECEIVE_SAMPLE_RATE,
                channels: 1,
                sdp_fmtp_line: "".to_owned(),
                rtcp_feedback: vec![],
            },
            payload_type: 111,
            ..Default::default()
        },
        RTPCodecType::Audio,
    )?;

    Ok(())
}

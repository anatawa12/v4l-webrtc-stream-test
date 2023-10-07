# v4l-webrtc-stream-test

This is POC to stream WebCam video onto WebRTC with [`webrtc`] crate and [`v4l`] crate.

While I'm making this, I found v4l crate doesn't have many required features, so I made [a fork of v4l][v4l-fork] and this using that.

This crate is initially configured to be used with raspberry pi but other environments may work.

[`webrtc`]: https://crates.io/crates/webrtc
[`v4l`]: https://crates.io/crates/v4l
[v4l-fork]: https://github.com/anatawa12/libv4l-rs

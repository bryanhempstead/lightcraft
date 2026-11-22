> **LightCraft patch** of egui-wgpu 0.36.2 (MIT OR Apache-2.0, © the egui authors), used through
> `[patch.crates-io]`: an app can ask for a half-float (`Rgba16Float`) window surface in an extended
> colour space (`request_hdr_surface`), read the display's EDR headroom (`display_headroom`), and
> screenshots read half-float surfaces; and it can ask for the 8-bit surface to be tagged Display P3
> (`request_sdr_color_space`, `sdr_color_space`: wgpu leaves an sRGB surface untagged on macOS, shown in
> the display's own space; tagged, macOS colour-manages the window onto the display). Everything else is
> upstream's code unchanged. Marked
> `LightCraft patch` in `src/lib.rs`, `src/winit.rs`, `src/capture.rs`. Drop it once egui-wgpu can
> pick an HDR surface itself.

# egui-wgpu

[![Latest version](https://img.shields.io/crates/v/egui-wgpu.svg)](https://crates.io/crates/egui-wgpu)
[![Documentation](https://docs.rs/egui-wgpu/badge.svg)](https://docs.rs/egui-wgpu)
![MIT](https://img.shields.io/badge/license-MIT-blue.svg)
![Apache](https://img.shields.io/badge/license-Apache-blue.svg)

This crates provides bindings between [`egui`](https://github.com/emilk/egui) and [wgpu](https://crates.io/crates/wgpu).

This was originally hosted at https://github.com/hasenbanck/egui_wgpu_backend

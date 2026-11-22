//! HDR (EDR) display output on macOS (LR-VIEW-HDR-DISPLAY): the window surface is asked for as
//! half float in extended sRGB (`patches/egui-wgpu`, before the window opens), and an HDR
//! edit's float rendition is uploaded as a half-float texture whose values go above 1.0, which the
//! compositor shows brighter than SDR white. Falls back to the usual SDR window when the surface
//! can't be HDR (another platform, an adapter without it, `LIGHTCRAFT_HDR_DISPLAY=0`), with a line
//! in the log saying why.

use std::collections::HashMap;

use eframe::{egui_wgpu, wgpu};
use lightcraft_ui_egui::hdr_view::{HdrPixels, HdrView};

/// Whether this start asks for an HDR window surface, and in which colour space: extended sRGB
/// (macOS colour-manages it onto the display, P3 included). Not extended Display P3: wgpu 30.0.1
/// resolves that colour space's name wrongly on macOS and the process dies configuring the surface.
pub fn request() -> Option<wgpu::SurfaceColorSpace> {
    let off = std::env::var("LIGHTCRAFT_HDR_DISPLAY").is_ok_and(|v| v.trim() == "0" || v.trim().eq_ignore_ascii_case("off"));
    if off || !cfg!(target_os = "macos") {
        return None;
    }
    Some(wgpu::SurfaceColorSpace::ExtendedSrgb)
}

struct Uploaded {
    key: u64,
    size: (usize, usize),
    texture: wgpu::Texture,
    id: egui::TextureId,
}

/// The app's [`HdrView`]: uploads HDR renditions next to egui's own textures.
pub struct EdrView {
    rs: egui_wgpu::RenderState,
    slots: HashMap<String, Uploaded>,
}

impl EdrView {
    /// `None` unless the window surface really is HDR (half float, extended colour space).
    pub fn new(rs: egui_wgpu::RenderState) -> Option<Self> {
        if rs.target_format != wgpu::TextureFormat::Rgba16Float || egui_wgpu::hdr_surface().is_none() {
            log::info!("HDR display: off (the window surface is {:?})", rs.target_format);
            return None;
        }
        log::info!("HDR display: half-float window surface in {:?}", egui_wgpu::hdr_surface());
        Some(Self { rs, slots: HashMap::new() })
    }
}

impl HdrView for EdrView {
    fn headroom(&self) -> Option<f32> {
        egui_wgpu::display_headroom()
    }

    fn texture(&mut self, slot: &str, px: &HdrPixels) -> Option<egui::TextureId> {
        if let Some(u) = self.slots.get(slot)
            && u.key == px.key
        {
            return Some(u.id);
        }
        let (w, h) = (px.width, px.height);
        let max = self.rs.device.limits().max_texture_dimension_2d as usize;
        if w == 0 || h == 0 || w > max || h > max || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(px.rgba.len()) {
            return None;
        }
        let extent = wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 };
        let reuse = self.slots.get(slot).is_some_and(|u| u.size == (w, h));
        if !reuse {
            let texture = self.rs.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("lightcraft-hdr-view"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut renderer = self.rs.renderer.write();
            if let Some(old) = self.slots.remove(slot) {
                renderer.free_texture(&old.id);
            }
            let id = renderer.register_native_texture(&self.rs.device, &view, wgpu::FilterMode::Linear);
            self.slots.insert(slot.to_string(), Uploaded { key: 0, size: (w, h), texture, id });
        }
        let u = self.slots.get_mut(slot)?;
        // the halves as they are in memory (little-endian, as the texture wants them) — no copy
        let swapped: Vec<u8>;
        let bytes: &[u8] = if cfg!(target_endian = "little") {
            bytemuck::cast_slice(&px.rgba)
        } else {
            swapped = px.rgba.iter().flat_map(|v| v.to_le_bytes()).collect();
            &swapped
        };
        self.rs.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &u.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w as u32 * 8), rows_per_image: Some(h as u32) },
            extent,
        );
        u.key = px.key;
        Some(u.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hdr_window_is_asked_for_on_macos_unless_switched_off() {
        // (the environment is shared between tests: only read it here)
        if std::env::var_os("LIGHTCRAFT_HDR_DISPLAY").is_none() {
            assert_eq!(request().is_some(), cfg!(target_os = "macos"));
            if cfg!(target_os = "macos") {
                assert_eq!(request(), Some(wgpu::SurfaceColorSpace::ExtendedSrgb));
            }
        }
    }
}

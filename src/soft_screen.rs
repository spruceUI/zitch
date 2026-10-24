//! Frames drawn on the CPU and shown through SDL's 2D renderer, for firmware
//! whose GL is too slow to draw egui: the Miyoo Mini's only GL is
//! SwiftShader, a software GLES that keeps both cores busy for every frame.
//! The rasterizer redraws only the tiles that changed, and the SDL renderer
//! blits the frame with the console's own 2D engine.

use anyhow::{Result, anyhow};
use egui_software_backend::{BufferMutRef, ColorFieldOrder, EguiSoftwareRender};
use sdl2::pixels::{Color, PixelFormatEnum};
use sdl2::render::{Canvas, Texture, TextureCreator};
use sdl2::video::{Window, WindowContext};

/// The rasterizer has no texture size limit of its own.
pub const MAX_TEXTURE_SIDE: usize = 4096;

pub struct SoftScreen {
    canvas: Canvas<Window>,
    creator: TextureCreator<WindowContext>,
    texture: Option<Texture>,
    renderer: EguiSoftwareRender,
    /// The frame, kept between frames: only changed tiles are redrawn.
    pixels: Vec<[u8; 4]>,
    size: [u32; 2],
    /// egui's texture changes since the last drawn frame, which the
    /// rasterizer only takes while drawing.
    pending: egui::TexturesDelta,
}

impl SoftScreen {
    pub fn new(window: Window) -> Result<Self> {
        let canvas = window
            .into_canvas()
            .build()
            .map_err(|e| anyhow!("creating the SDL renderer: {e}"))?;
        let creator = canvas.texture_creator();
        Ok(Self {
            canvas,
            creator,
            texture: None,
            // ARGB8888 is BGRA in memory.
            renderer: EguiSoftwareRender::new(ColorFieldOrder::Bgra),
            pixels: Vec::new(),
            size: [0, 0],
            pending: egui::TexturesDelta::default(),
        })
    }

    pub fn window(&self) -> &Window {
        self.canvas.window()
    }

    pub fn follow_textures(&mut self, delta: &egui::TexturesDelta) {
        self.pending.append(delta.clone());
    }

    pub fn clear(&mut self, rgba: [f32; 4]) {
        let [r, g, b, _] = rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        self.canvas.set_draw_color(Color::RGB(r, g, b));
        self.canvas.clear();
    }

    pub fn paint(
        &mut self,
        [w, h]: [u32; 2],
        pixels_per_point: f32,
        primitives: &[egui::ClippedPrimitive],
        delta: &mut egui::TexturesDelta,
    ) -> Result<()> {
        self.pending.append(std::mem::take(delta));
        if self.size != [w, h] || self.texture.is_none() {
            if let Some(old) = self.texture.take() {
                unsafe { old.destroy() };
            }
            self.texture = Some(
                self.creator
                    .create_texture_streaming(PixelFormatEnum::ARGB8888, w, h)
                    .map_err(|e| anyhow!("creating the frame texture: {e}"))?,
            );
            self.pixels = vec![[0, 0, 0, 255]; w as usize * h as usize];
            self.size = [w, h];
        }
        let pending = std::mem::take(&mut self.pending);
        let mut buffer = BufferMutRef::new(&mut self.pixels, w as usize, h as usize);
        self.renderer
            .render(&mut buffer, primitives, &pending, pixels_per_point);
        let Some(texture) = self.texture.as_mut() else {
            return Ok(());
        };
        texture
            .update(None, self.pixels.as_flattened(), w as usize * 4)
            .map_err(|e| anyhow!("updating the frame texture: {e}"))?;
        self.canvas
            .copy(texture, None, None)
            .map_err(|e| anyhow!("drawing the frame: {e}"))
    }

    pub fn read_screen_rgba(&self) -> egui::ColorImage {
        let [w, h] = self.size;
        let rgba: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|&[b, g, r, a]| [r, g, b, a])
            .collect();
        egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba)
    }

    pub fn present(&mut self) {
        self.canvas.present();
    }
}

//! Batched 2D renderer on OpenGL ES 2.0 / desktop GL 2.1.
//!
//! Shapes and text are drawn from one alpha atlas: a solid texel for
//! rectangles and rasterized glyphs for text, in one draw call. Images (channel
//! logos) live in fixed slots of an RGBA texture and follow in a second call,
//! over the rest.

use glow::HasContext;
use mana_core::catalog::logos::THUMBNAIL;

use atlas::{SOLID_UV, Shelf};
use shader::{FRAGMENT_SHADER, IMAGE_FRAGMENT_SHADER, program};

pub use atlas::{ATLAS_SIZE, AtlasRect};

mod atlas;
mod shader;

const FLOATS_PER_VERTEX: usize = 8;
/// RGBA texture for images, 8 MB, cut in slots of IMAGE_SLOT texels.
const IMAGE_TEXTURE: (i32, i32) = (2048, 1024);
/// The core's largest logo thumbnail.
pub const IMAGE_SLOT: (i32, i32) = (THUMBNAIL.0 as i32, THUMBNAIL.1 as i32);
pub const IMAGE_SLOTS: usize = ((IMAGE_TEXTURE.0 / IMAGE_SLOT.0) * (IMAGE_TEXTURE.1 / IMAGE_SLOT.1)) as usize;

#[derive(Debug, Clone, Copy)]
pub struct Color(pub f32, pub f32, pub f32, pub f32);

impl Color {
    pub const TRANSPARENT: Self = Color(0.0, 0.0, 0.0, 0.0);

    pub const fn with_alpha(self, alpha: f32) -> Self {
        Color(self.0, self.1, self.2, alpha)
    }

    pub const fn hex(rgb: u32) -> Self {
        const fn channel(rgb: u32, shift: u32) -> f32 {
            ((rgb >> shift) & 0xff) as f32 / 255.0
        }
        Color(channel(rgb, 16), channel(rgb, 8), channel(rgb, 0), 1.0)
    }
}

pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    image_program: glow::Program,
    buffer: glow::Buffer,
    atlas: glow::Texture,
    images: glow::Texture,
    u_screen: Option<glow::UniformLocation>,
    u_image_screen: Option<glow::UniformLocation>,
    shelf: Shelf,
    vertices: Vec<f32>,
    image_vertices: Vec<f32>,
    pub size: (f32, f32),
}

/// Linear filtering, clamped at the edges, for the bound texture.
///
/// # Safety
/// The GL context must be current on this thread.
unsafe fn texture_parameters(gl: &glow::Context) {
    for (param, value) in [
        (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
        (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
        (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
        (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
    ] {
        // SAFETY: the caller guarantees a current context.
        unsafe { gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32) };
    }
}

impl Renderer {
    pub fn new(gl: glow::Context, gles: bool) -> Result<Self, String> {
        let header = if gles { "#version 100\n" } else { "#version 120\n" };
        // SAFETY: the GL context passed in is current on this thread; every
        // object created here is owned by the renderer.
        unsafe {
            let image_program = program(&gl, header, IMAGE_FRAGMENT_SHADER)?;
            let u_image_screen = gl.get_uniform_location(image_program, "u_screen");
            let program = program(&gl, header, FRAGMENT_SHADER)?;
            let u_screen = gl.get_uniform_location(program, "u_screen");

            let images = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(images));
            // Filled now: allocating it on the first upload stalled a frame
            // for 70 ms on the TV.
            let blank = vec![0u8; (IMAGE_TEXTURE.0 * IMAGE_TEXTURE.1 * 4) as usize];
            gl.tex_image_2d(
                glow::TEXTURE_2D, 0, glow::RGBA as i32, IMAGE_TEXTURE.0, IMAGE_TEXTURE.1, 0,
                glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&blank)),
            );
            texture_parameters(&gl);

            let atlas = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(atlas));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            let mut pixels = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE) as usize];
            pixels[..2].fill(255);
            pixels[ATLAS_SIZE as usize..ATLAS_SIZE as usize + 2].fill(255);
            gl.tex_image_2d(
                glow::TEXTURE_2D, 0, glow::ALPHA as i32, ATLAS_SIZE, ATLAS_SIZE, 0,
                glow::ALPHA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&pixels)),
            );
            texture_parameters(&gl);

            let buffer = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffer));
            let stride = (FLOATS_PER_VERTEX * 4) as i32;
            for (index, (size, offset)) in [(2, 0), (2, 2), (4, 4)].into_iter().enumerate() {
                gl.enable_vertex_attrib_array(index as u32);
                gl.vertex_attrib_pointer_f32(index as u32, size, glow::FLOAT, false, stride, offset * 4);
            }
            gl.enable(glow::BLEND);
            // Keep destination alpha meaningful: the compositor shows the video
            // plane through transparent pixels.
            gl.blend_func_separate(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA, glow::ONE, glow::ONE_MINUS_SRC_ALPHA);

            Ok(Self {
                gl,
                program,
                image_program,
                buffer,
                atlas,
                images,
                u_screen,
                u_image_screen,
                shelf: Shelf::new(),
                vertices: Vec::with_capacity(4096),
                image_vertices: Vec::with_capacity(1024),
                size: (1.0, 1.0),
            })
        }
    }

    pub fn info(&self) -> String {
        // SAFETY: plain string queries on the current context.
        unsafe {
            format!(
                "{} · {}",
                self.gl.get_parameter_string(glow::RENDERER),
                self.gl.get_parameter_string(glow::VERSION)
            )
        }
    }

    /// Copies an alpha bitmap into a newly allocated atlas region.
    pub fn upload_alpha(&mut self, w: i32, h: i32, pixels: &[u8]) -> Option<AtlasRect> {
        let rect = self.shelf.alloc(w, h)?;
        if w > 0 && h > 0 {
            // SAFETY: `pixels` holds exactly w*h bytes and the rect lies inside the atlas.
            unsafe {
                self.gl.bind_texture(glow::TEXTURE_2D, Some(self.atlas));
                self.gl.tex_sub_image_2d(
                    glow::TEXTURE_2D, 0, rect.x, rect.y, w, h,
                    glow::ALPHA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(pixels)),
                );
            }
        }
        Some(rect)
    }

    /// Copies RGBA pixels, at most IMAGE_SLOT in size, into an image slot.
    pub fn upload_image(&mut self, slot: usize, w: i32, h: i32, rgba: &[u8]) {
        let (x, y) = slot_origin(slot);
        // SAFETY: `rgba` holds w*h*4 bytes and the slot lies inside the texture.
        unsafe {
            self.gl.bind_texture(glow::TEXTURE_2D, Some(self.images));
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            self.gl.tex_sub_image_2d(
                glow::TEXTURE_2D, 0, x, y, w, h,
                glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(rgba)),
            );
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        }
    }

    /// Draws the `w`×`h` image in `slot` stretched over the given box.
    #[allow(clippy::too_many_arguments)]
    pub fn image(&mut self, slot: usize, w: i32, h: i32, x: f32, y: f32, dw: f32, dh: f32) {
        let (sx, sy) = slot_origin(slot);
        let (tw, th) = (IMAGE_TEXTURE.0 as f32, IMAGE_TEXTURE.1 as f32);
        let uv = [sx as f32 / tw, sy as f32 / th, (sx + w) as f32 / tw, (sy + h) as f32 / th];
        let corners = [(x, y, uv[0], uv[1]), (x + dw, y, uv[2], uv[1]), (x + dw, y + dh, uv[2], uv[3]), (x, y + dh, uv[0], uv[3])];
        for i in [0, 1, 2, 0, 2, 3] {
            let (x, y, u, v) = corners[i];
            self.image_vertices.extend_from_slice(&[x, y, u, v, 1.0, 1.0, 1.0, 1.0]);
        }
    }

    pub fn begin(&mut self, width: i32, height: i32, clear: Color) {
        self.size = (width as f32, height as f32);
        self.vertices.clear();
        self.image_vertices.clear();
        // SAFETY: state changes on the current context.
        unsafe {
            self.gl.viewport(0, 0, width, height);
            self.gl.clear_color(clear.0, clear.1, clear.2, clear.3);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let [u, v] = SOLID_UV;
        self.quad([x, y, x + w, y + h], [u, v, u, v], color);
    }

    pub fn glyph(&mut self, x: f32, y: f32, rect: AtlasRect, color: Color) {
        let s = ATLAS_SIZE as f32;
        let (u0, v0) = (rect.x as f32 / s, rect.y as f32 / s);
        let (u1, v1) = ((rect.x + rect.w) as f32 / s, (rect.y + rect.h) as f32 / s);
        self.quad([x, y, x + rect.w as f32, y + rect.h as f32], [u0, v0, u1, v1], color);
    }

    fn quad(&mut self, [x0, y0, x1, y1]: [f32; 4], [u0, v0, u1, v1]: [f32; 4], c: Color) {
        let corners = [(x0, y0, u0, v0), (x1, y0, u1, v0), (x1, y1, u1, v1), (x0, y1, u0, v1)];
        for i in [0, 1, 2, 0, 2, 3] {
            let (x, y, u, v) = corners[i];
            self.vertices.extend_from_slice(&[x, y, u, v, c.0, c.1, c.2, c.3]);
        }
    }

    pub fn finish(&mut self) {
        let batches = [
            (self.program, self.u_screen, self.atlas, std::mem::take(&mut self.vertices)),
            (self.image_program, self.u_image_screen, self.images, std::mem::take(&mut self.image_vertices)),
        ];
        for (program, u_screen, texture, vertices) in &batches {
            if vertices.is_empty() {
                continue;
            }
            let bytes: Vec<u8> = vertices.iter().flat_map(|f| f.to_ne_bytes()).collect();
            // SAFETY: the buffer holds whole vertices in the layout bound in `new`.
            unsafe {
                self.gl.use_program(Some(*program));
                self.gl.uniform_2_f32(u_screen.as_ref(), self.size.0, self.size.1);
                self.gl.bind_texture(glow::TEXTURE_2D, Some(*texture));
                self.gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.buffer));
                self.gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STREAM_DRAW);
                self.gl.draw_arrays(glow::TRIANGLES, 0, (vertices.len() / FLOATS_PER_VERTEX) as i32);
            }
        }
        let [(_, _, _, vertices), (_, _, _, image_vertices)] = batches;
        (self.vertices, self.image_vertices) = (vertices, image_vertices);
    }
}

/// Top-left texel of an image slot.
fn slot_origin(slot: usize) -> (i32, i32) {
    let columns = (IMAGE_TEXTURE.0 / IMAGE_SLOT.0) as usize;
    ((slot % columns) as i32 * IMAGE_SLOT.0, (slot / columns) as i32 * IMAGE_SLOT.1)
}

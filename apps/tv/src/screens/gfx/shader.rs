//! The renderer's shaders: one vertex shader, shared by the atlas's
//! fragment shader and the images', and the program linking them.

use glow::HasContext;

const VERTEX_SHADER: &str = r#"
attribute vec2 a_pos;
attribute vec2 a_uv;
attribute vec4 a_color;
uniform vec2 u_screen;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    vec2 ndc = a_pos / u_screen * 2.0 - 1.0;
    gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
    v_uv = a_uv;
    v_color = a_color;
}
"#;

pub(super) const FRAGMENT_SHADER: &str = r#"
#ifdef GL_ES
precision mediump float;
#endif
uniform sampler2D u_atlas;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    gl_FragColor = vec4(v_color.rgb, v_color.a * texture2D(u_atlas, v_uv).a);
}
"#;

/// Images: the texture's own colours, faded by the vertex alpha.
pub(super) const IMAGE_FRAGMENT_SHADER: &str = r#"
#ifdef GL_ES
precision mediump float;
#endif
uniform sampler2D u_atlas;
varying vec2 v_uv;
varying vec4 v_color;
void main() {
    vec4 texel = texture2D(u_atlas, v_uv);
    gl_FragColor = vec4(texel.rgb, texel.a * v_color.a);
}
"#;

/// Compiles and links the shared vertex shader with `fragment`.
///
/// # Safety
/// The GL context must be current on this thread.
pub(super) unsafe fn program(gl: &glow::Context, header: &str, fragment: &str) -> Result<glow::Program, String> {
    // SAFETY: the caller guarantees a current context.
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [(glow::VERTEX_SHADER, VERTEX_SHADER), (glow::FRAGMENT_SHADER, fragment)] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(format!("shader: {}", gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        for (index, name) in ["a_pos", "a_uv", "a_color"].iter().enumerate() {
            gl.bind_attrib_location(program, index as u32, name);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            return Err(format!("program: {}", gl.get_program_info_log(program)));
        }
        gl.use_program(Some(program));
        gl.uniform_1_i32(gl.get_uniform_location(program, "u_atlas").as_ref(), 0);
        Ok(program)
    }
}

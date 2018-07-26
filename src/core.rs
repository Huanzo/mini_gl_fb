use config::Config;

use rustic_gl;

use glutin::{
    EventsLoop,
    WindowBuilder,
    ContextBuilder,
    GlWindow,
    GlContext,
    Event,
    WindowEvent,
};
use glutin::dpi::LogicalSize;

use gl;
use gl::types::*;

use std::ptr::null;

/// Create a context using glutin given a configuration.
pub fn init_glutin_context<S: ToString>(config: &Config<S>) -> (EventsLoop, GlWindow) {
    let window_size = LogicalSize::new(config.window_size.0, config.window_size.1);

    let events_loop = EventsLoop::new();
    let window = WindowBuilder::new()
        .with_title(config.window_title.to_string())
        .with_dimensions(window_size)
        .with_resizable(config.resizable);

    let context = ContextBuilder::new();
    let gl_window = GlWindow::new(window, context, &events_loop).unwrap();

    unsafe {
        gl_window.make_current().unwrap();
        gl::load_with(|symbol| gl_window.get_proc_address(symbol) as *const _);
    }

    (events_loop, gl_window)
}

type VertexFormat = buffer_layout!([f32; 2], [f32; 2]);

pub fn init_framebuffer<S: ToString>(config: &Config<S>) -> Framebuffer {
    // The config takes the size in u32 because that's all that actually makes sense but since
    // OpenGL is from the Land of C where a Working Type System doesn't exist, we work with i32s
    let buffer_width = if config.buffer_size.0 == 0 { config.window_size.0.round() as _ }
        else { config.buffer_size.0 as _ };
    let buffer_height = if config.buffer_size.1 == 0 { config.window_size.1.round() as _ }
        else { config.buffer_size.1 as _ };

    let vertex_shader = rustic_gl::raw::create_shader(
        gl::VERTEX_SHADER,
        include_str!("./default_vertex_shader.glsl"),
    ).unwrap();
    let fragment_shader = rustic_gl::raw::create_shader(
        gl::FRAGMENT_SHADER,
        include_str!("./default_fragment_shader.glsl"),
    ).unwrap();

    let program = unsafe {
        build_program(&[
            Some(vertex_shader),
            Some(fragment_shader),
        ])
    };

    let sampler_location = unsafe {
        let location = gl::GetUniformLocation(program, b"u_tex0\0".as_ptr() as *const _);
        gl::UseProgram(program);
        gl::Uniform1i(location, 0);
        gl::UseProgram(0);
        location
    };

    let texture_format = (BufferFormat::RGBA, gl::UNSIGNED_BYTE);
    let texture = create_texture(buffer_width, buffer_height, texture_format.0, texture_format.1);

    let vao = rustic_gl::raw::create_vao().unwrap();
    let vbo = rustic_gl::raw::create_buffer().unwrap();

    unsafe {
        gl::BindVertexArray(vao);
        gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
        VertexFormat::declare(0);

        let verts: [[f32; 2]; 12] = [
            [-1., 1.], [0., 0.], // top left
            [-1., -1.], [0., 1.], // bottom left
            [1., -1.], [1., 1.], // bottom right
            [1., -1.], [1., 1.], // bottom right
            [1., 1.], [1., 0.], // top right
            [-1., 1.], [0., 0.], // top left
        ];
        use std::mem::size_of_val;
        gl::BufferData(gl::ARRAY_BUFFER,
            size_of_val(&verts) as _,
            verts.as_ptr() as *const _,
            gl::STATIC_DRAW
        );
        gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        gl::BindVertexArray(0);
    }

    Framebuffer {
        buffer_width: buffer_width,
        buffer_height: buffer_height,
        program,
        sampler_location,
        vertex_shader: Some(vertex_shader),
        geometry_shader: None,
        fragment_shader: Some(fragment_shader),
        texture,
        vao,
        vbo,
        texture_format,
    }
}

pub struct Internal {
    pub events_loop: EventsLoop,
    pub gl_window: GlWindow,
    pub fb: Framebuffer,
}

impl Internal {
    pub fn update_buffer<T>(&mut self, image_data: &[T]) {
        self.fb.update_buffer(image_data);
        self.gl_window.swap_buffers().unwrap();
    }

    pub fn persist(&mut self) {
        self.persist_and_redraw(false);
    }

    pub fn persist_and_redraw(&mut self, redraw: bool) {
        let mut running = true;
        while running {
            self.events_loop.poll_events(|event| {
                match event {
                    Event::WindowEvent { event, .. } => match event {
                        WindowEvent::CloseRequested => running = false,
                        _ => {},
                    },
                    _ => {},
                }
            });
            if redraw {
                self.fb.draw(|_| {});
                self.gl_window.swap_buffers().unwrap();
            }
        }
    }
}

/// Provides the drawing functionality.
///
/// You can get direct access by using a breakout function, such as breakout_glutin.
///
/// # Disclaimer:
///
/// Accessing fields directly is not the intended usage. If a feature is missing please open an
/// issue. The fields are public, however, so that while you are waiting for a feature to be
/// exposed, if you need something in a pinch you can dig in easily and make it happen.
///
/// The internal fields may change.
pub struct Framebuffer {
    pub buffer_width: i32,
    pub buffer_height: i32,
    pub program: GLuint,
    pub sampler_location: GLint,
    pub vertex_shader: Option<GLuint>,
    pub geometry_shader: Option<GLuint>,
    pub fragment_shader: Option<GLuint>,
    pub texture: GLuint,
    pub vao: GLuint,
    pub vbo: GLuint,
    pub texture_format: (BufferFormat, GLenum),
}

impl Framebuffer {
    pub fn update_buffer<T>(&mut self, image_data: &[T]) {
        // TODO: Safety check on the length of the passed slice so this is actually a safe method
        self.draw(|fb| {
            unsafe {
                let (format, kind) = fb.texture_format;
                gl::TexImage2D(
                    gl::TEXTURE_2D,
                    0,
                    gl::RGBA as _,
                    fb.buffer_width,
                    fb.buffer_height,
                    0,
                    format as GLenum,
                    kind,
                    image_data.as_ptr() as *const _,
                );
            }
        })
    }

    pub fn use_vertex_shader(&mut self, source: &str) {
        rebuild_shader(&mut self.vertex_shader, gl::VERTEX_SHADER, source);
        self.relink_program();
    }

    pub fn use_fragment_shader(&mut self, source: &str) {
        rebuild_shader(&mut self.fragment_shader, gl::FRAGMENT_SHADER, source);
        self.relink_program();
    }

    pub fn use_geometry_shader(&mut self, source: &str) {
        rebuild_shader(&mut self.geometry_shader, gl::GEOMETRY_SHADER, source);
        self.relink_program();
    }

    // TODO: require passing new image data
    pub fn change_buffer_format<T: ToGlType>(&mut self, format: BufferFormat) {
        self.texture_format = (format, T::to_gl_enum());
    }

    // TODO: resize_buffer

    fn draw<F: FnOnce(&Framebuffer)>(&mut self, f: F) {
        unsafe {
            gl::UseProgram(self.program);
            gl::BindVertexArray(self.vao);
            gl::ActiveTexture(0);
            gl::BindTexture(gl::TEXTURE_2D, self.texture);
            f(self);
            gl::DrawArrays(gl::TRIANGLES, 0, 6);
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
        }
    }

    fn relink_program(&mut self) {
        unsafe {
            gl::DeleteProgram(self.program);
            self.program = build_program(&[
                self.vertex_shader,
                self.fragment_shader,
                self.geometry_shader,
            ]);
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
#[repr(u32)]
pub enum BufferFormat {
    R = gl::RED,
    RG = gl::RG,
    RGB = gl::RGB,
    BGR = gl::BGR,
    RGBA = gl::RGBA,
    BGRA = gl::BGRA,
}

pub trait ToGlType {
    fn to_gl_enum() -> GLenum;
}

macro_rules! impl_ToGlType {
    (
        $(
            $t:ty, $gl_type:expr
        ),+,
    ) => {
        $(
            impl ToGlType for $t {
                fn to_gl_enum() -> GLenum {
                    $gl_type
                }
            }
        )+
    }
}

impl_ToGlType!(
    u8, gl::UNSIGNED_BYTE,
    i8, gl::BYTE,
);

fn create_texture(width: i32, height: i32, format: BufferFormat, buffer_kind: GLenum) -> GLuint {
    unsafe {
        let mut tex = 0;
        gl::GenTextures(1, &mut tex);
        if tex == 0 {
            // TODO
            panic!();
        }
        gl::BindTexture(gl::TEXTURE_2D, tex);
        gl::TexImage2D(gl::TEXTURE_2D, 0, gl::RGBA as _, width, height, 0, format as GLenum, buffer_kind, null());
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as _);
        gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as _);
        gl::BindTexture(gl::TEXTURE_2D, 0);
        tex
    }
}

fn rebuild_shader(shader: &mut Option<GLuint>, kind: GLenum, source: &str) {
    if let Some(shader) = *shader {
        unsafe {
            gl::DeleteShader(shader);
        }
    }
    let compilation_result = rustic_gl::raw::create_shader(kind, source);
    match compilation_result {
        Ok(gl_id) => {
            *shader = Some(gl_id);
        },
        Err(rustic_gl::error::GlError::ShaderCompilation(info)) => {
            if let Some(log) = info {
                panic!("Shader compilation failed with the following information: {}", log);
            } else {
                panic!("Shader compilation failed without any information.")
            }
        },
        Err(err) => {
            panic!("An error occured while compiling shader: {}", err);
        }
    }
}

unsafe fn build_program(shaders: &[Option<GLuint>]) -> GLuint {
    let program = rustic_gl::raw::create_program()
        .unwrap();
    for shader in shaders.iter() {
        if let &Some(shader) = shader {
            gl::AttachShader(program, shader);
        }
    }
    gl::LinkProgram(program);
    rustic_gl::raw::get_link_status(program)
        .unwrap();
    for shader in shaders {
        if let &Some(shader) = shader {
            gl::DetachShader(program, shader);
        }
    }
    program
}

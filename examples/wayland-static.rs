use r_egl as egl;

use egl::API as egl;
use gl::types::{GLboolean, GLchar, GLenum, GLint, GLuint, GLvoid};
use std::ffi::CStr;
use std::ptr;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_display::WlDisplay;
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, Proxy, delegate_noop};
use wayland_client::{
	EventQueue,
	protocol::{wl_compositor::WlCompositor, wl_surface::WlSurface},
};
use wayland_protocols::xdg::shell::client::xdg_toplevel::XdgToplevel;

use wayland_protocols::xdg::shell::client::{
	xdg_surface::{self, XdgSurface},
	xdg_wm_base::{self, XdgWmBase},
};

struct MainState;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for MainState {
	fn event(
		_state: &mut Self,
		_proxy: &wl_registry::WlRegistry,
		_event: <wl_registry::WlRegistry as wayland_client::Proxy>::Event,
		_data: &GlobalListContents,
		_conn: &wayland_client::Connection,
		_qhandle: &wayland_client::QueueHandle<Self>,
	) {
	}
}

#[derive(Debug, Clone, Copy)]
struct SecondState {
	egl_display: egl::Display,
	egl_context: egl::Context,
	egl_config: egl::Config,
	initialize: bool,
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for SecondState {
	fn event(
		_state: &mut Self,
		proxy: &xdg_wm_base::XdgWmBase,
		event: <xdg_wm_base::XdgWmBase as wayland_client::Proxy>::Event,
		_data: &(),
		_conn: &wayland_client::Connection,
		_qhandle: &wayland_client::QueueHandle<Self>,
	) {
		match event {
			xdg_wm_base::Event::Ping { serial } => proxy.pong(serial),
			_ => (),
		}
	}
}

struct XdgSurfaceInfo {
	size: (i32, i32),
	surface: WlSurface,
}

impl Dispatch<XdgSurface, XdgSurfaceInfo> for SecondState {
	fn event(
		state: &mut Self,
		xdg_surface: &XdgSurface,
		event: <XdgSurface as wayland_client::Proxy>::Event,
		XdgSurfaceInfo {
			size: (width, height),
			surface,
		}: &XdgSurfaceInfo,
		_conn: &Connection,
		_qhandle: &wayland_client::QueueHandle<Self>,
	) {
		if let xdg_surface::Event::Configure { serial } = event {
			if state.initialize {
				return;
			}
			xdg_surface.ack_configure(serial);
			state.initialize = true;
			let egl_display = state.egl_display;
			let egl_config = state.egl_config;
			let egl_context = state.egl_context;
			let wl_egl_surface =
				wayland_egl::WlEglSurface::new(surface.id(), *width, *height).unwrap();
			let egl_surface = unsafe {
				egl.create_window_surface(
					egl_display,
					egl_config,
					wl_egl_surface.ptr() as egl::NativeWindowType,
					None,
				)
				.expect("unable to create an EGL surface")
			};

			egl.make_current(
				egl_display,
				Some(egl_surface),
				Some(egl_surface),
				Some(egl_context),
			)
			.expect("unable to bind the context");

			render();

			egl.swap_buffers(egl_display, egl_surface)
				.expect("unable to post the surface content");
		}
	}
}

delegate_noop!(SecondState: ignore WlCompositor);
delegate_noop!(SecondState: ignore XdgToplevel);
delegate_noop!(SecondState: ignore WlSurface);

struct DisplayConnection {
	event_queue: EventQueue<SecondState>,
	compositor: WlCompositor,
	xdg: XdgWmBase,
	data: SecondState,
}

fn setup_wayland() -> DisplayConnection {
	let connection = Connection::connect_to_env().unwrap();
	let display = connection.display();
	let egl_display = setup_egl(&display);
	let (egl_context, egl_config) = create_context(egl_display);
	let (globals, _) = registry_queue_init::<MainState>(&connection).unwrap();
	let event_queue = connection.new_event_queue::<SecondState>();

	let state = SecondState {
		egl_context,
		egl_config,
		egl_display,
		initialize: false,
	};

	let qh = event_queue.handle();
	let compositor = globals.bind::<WlCompositor, _, _>(&qh, 1..=5, ()).unwrap();
	let xdg = globals.bind::<XdgWmBase, _, _>(&qh, 2..=6, ()).unwrap();
	// Setup EGL.

	DisplayConnection {
		event_queue,
		compositor,
		xdg,
		data: state,
	}
}

fn setup_egl(display: &WlDisplay) -> egl::Display {
	let egl_display = unsafe {
		egl.get_display(display.id().as_ptr() as *mut std::ffi::c_void)
			.unwrap()
	};

	egl.initialize(egl_display).unwrap();
	egl_display
}

fn create_context(display: egl::Display) -> (egl::Context, egl::Config) {
	let attributes = [
		egl::RED_SIZE,
		8,
		egl::GREEN_SIZE,
		8,
		egl::BLUE_SIZE,
		8,
		egl::NONE,
	];

	let config = egl
		.choose_first_config(display, &attributes)
		.expect("unable to choose an EGL configuration")
		.expect("no EGL configuration found");

	let context_attributes = [
		egl::CONTEXT_MAJOR_VERSION,
		4,
		egl::CONTEXT_MINOR_VERSION,
		0,
		egl::CONTEXT_OPENGL_PROFILE_MASK,
		egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
		egl::NONE,
	];

	let context = egl
		.create_context(display, config, None, &context_attributes)
		.expect("unable to create an EGL context");

	(context, config)
}

fn create_surface(ctx: &mut DisplayConnection, width: i32, height: i32) {
	let qh = ctx.event_queue.handle();
	let wl_surface = ctx.compositor.create_surface(&qh, ());
	let xdg_surface = ctx.xdg.get_xdg_surface(
		&wl_surface,
		&qh,
		XdgSurfaceInfo {
			size: (width, height),
			surface: wl_surface.clone(),
		},
	);

	let xdg_toplevel = xdg_surface.get_toplevel(&qh, ());
	xdg_toplevel.set_app_id("khronos-egl-test".to_string());
	xdg_toplevel.set_title("Test".to_string());

	wl_surface.commit();

	ctx.event_queue.blocking_dispatch(&mut ctx.data).unwrap();
}

fn main() {
	// Setup Open GL.
	egl.bind_api(egl::OPENGL_API)
		.expect("unable to select OpenGL API");
	gl::load_with(|name| egl.get_proc_address(name).unwrap() as *const std::ffi::c_void);

	// Setup the Wayland client.
	let mut ctx = setup_wayland();

	// Note that it must be kept alive to the end of execution.
	create_surface(&mut ctx, 800, 600);

	loop {
		ctx.event_queue.blocking_dispatch(&mut ctx.data).unwrap();
	}
}

const VERTEX: &'static [GLint; 8] = &[-1, -1, 1, -1, 1, 1, -1, 1];

const INDEXES: &'static [GLuint; 4] = &[0, 1, 2, 3];

const VERTEX_SHADER: &[u8] = b"#version 400
in vec2 position;

void main() {
	gl_Position = vec4(position, 0.0f, 1.0f);
}
\0";

const FRAGMENT_SHADER: &[u8] = b"#version 400
out vec4 color;

void main() {
	color = vec4(1.0f, 0.0f, 0.0f, 1.0f);
}
\0";

fn render() {
	unsafe {
		let vertex_shader = gl::CreateShader(gl::VERTEX_SHADER);
		check_gl_errors();
		let src = CStr::from_bytes_with_nul_unchecked(VERTEX_SHADER).as_ptr();
		gl::ShaderSource(vertex_shader, 1, (&[src]).as_ptr(), ptr::null());
		check_gl_errors();
		gl::CompileShader(vertex_shader);
		check_shader_status(vertex_shader);

		let fragment_shader = gl::CreateShader(gl::FRAGMENT_SHADER);
		check_gl_errors();
		let src = CStr::from_bytes_with_nul_unchecked(FRAGMENT_SHADER).as_ptr();
		gl::ShaderSource(fragment_shader, 1, (&[src]).as_ptr(), ptr::null());
		check_gl_errors();
		gl::CompileShader(fragment_shader);
		check_shader_status(fragment_shader);

		let program = gl::CreateProgram();
		check_gl_errors();
		gl::AttachShader(program, vertex_shader);
		check_gl_errors();
		gl::AttachShader(program, fragment_shader);
		check_gl_errors();
		gl::LinkProgram(program);
		check_gl_errors();
		gl::UseProgram(program);
		check_gl_errors();

		let mut buffer = 0;
		gl::GenBuffers(1, &mut buffer);
		check_gl_errors();
		gl::BindBuffer(gl::ARRAY_BUFFER, buffer);
		check_gl_errors();
		gl::BufferData(
			gl::ARRAY_BUFFER,
			8 * 4,
			VERTEX.as_ptr() as *const std::ffi::c_void,
			gl::STATIC_DRAW,
		);
		check_gl_errors();

		let mut vertex_input = 0;
		gl::GenVertexArrays(1, &mut vertex_input);
		check_gl_errors();
		gl::BindVertexArray(vertex_input);
		check_gl_errors();
		gl::EnableVertexAttribArray(0);
		check_gl_errors();
		gl::VertexAttribPointer(0, 2, gl::INT, gl::FALSE as GLboolean, 0, 0 as *const GLvoid);
		check_gl_errors();

		let mut indexes = 0;
		gl::GenBuffers(1, &mut indexes);
		check_gl_errors();
		gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, indexes);
		check_gl_errors();
		gl::BufferData(
			gl::ELEMENT_ARRAY_BUFFER,
			4 * 4,
			INDEXES.as_ptr() as *const std::ffi::c_void,
			gl::STATIC_DRAW,
		);
		check_gl_errors();

		gl::DrawElements(gl::TRIANGLE_FAN, 4, gl::UNSIGNED_INT, std::ptr::null());
		check_gl_errors();
	}
}

fn format_error(e: GLenum) -> &'static str {
	match e {
		gl::NO_ERROR => "No error",
		gl::INVALID_ENUM => "Invalid enum",
		gl::INVALID_VALUE => "Invalid value",
		gl::INVALID_OPERATION => "Invalid operation",
		gl::INVALID_FRAMEBUFFER_OPERATION => "Invalid framebuffer operation",
		gl::OUT_OF_MEMORY => "Out of memory",
		gl::STACK_UNDERFLOW => "Stack underflow",
		gl::STACK_OVERFLOW => "Stack overflow",
		_ => "Unknown error",
	}
}

pub fn check_gl_errors() {
	unsafe {
		match gl::GetError() {
			gl::NO_ERROR => (),
			e => {
				panic!("OpenGL error: {}", format_error(e))
			}
		}
	}
}

unsafe fn check_shader_status(shader: GLuint) {
	let mut status = gl::FALSE as GLint;
	unsafe { gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut status) };
	if status != (gl::TRUE as GLint) {
		let mut len = 0;
		unsafe { gl::GetProgramiv(shader, gl::INFO_LOG_LENGTH, &mut len) };
		if len > 0 {
			let mut buf = Vec::with_capacity(len as usize);
			unsafe {
				buf.set_len((len as usize) - 1); // subtract 1 to skip the trailing null character
				gl::GetProgramInfoLog(
					shader,
					len,
					ptr::null_mut(),
					buf.as_mut_ptr() as *mut GLchar,
				)
			};

			let log = String::from_utf8(buf).unwrap();
			eprintln!("shader compilation log:\n{}", log);
		}

		panic!("shader compilation failed");
	}
}

//! Window + presentation. Presenter tries wgpu (backend from FM8_BACKEND =
//! vulkan | gl | auto) and falls back to a CPU shared-memory surface
//! (softbuffer) when no usable GPU adapter exists. UI pixels come from the
//! fm8-ui CPU rasteriser either way; wgpu only presents them.

use filem8::app::App;
use fm8_ui::soft::Framebuffer;
use fm8_ui::*;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key as WKey, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    cfg: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    tex: Option<(wgpu::Texture, wgpu::BindGroup, u32, u32)>,
    name: String,
}

const SHADER: &str = r#"
@group(0) @binding(0) var t: texture_2d<f32>;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
  var o: V;
  let x = f32((i << 1u) & 2u); let y = f32(i & 2u);
  o.uv = vec2<f32>(x, y);
  o.p = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
  return o;
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
  let d = vec2<f32>(textureDimensions(t));
  return textureLoad(t, vec2<i32>(v.uv * d), 0);
}
"#;

impl Gpu {
    fn new(window: Arc<Window>) -> Result<Gpu, String> {
        let want = backend_pref();
        let backends = match want.as_str() { "vulkan" => wgpu::Backends::VULKAN, "gl" => wgpu::Backends::GL, _ => wgpu::Backends::VULKAN | wgpu::Backends::GL };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor { backends, ..Default::default() });
        let surface = instance.create_surface(window.clone()).map_err(|e| e.to_string())?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::LowPower, compatible_surface: Some(&surface), force_fallback_adapter: false })).map_err(|e| e.to_string())?;
        let name = format!("{} ({:?})", adapter.get_info().name, adapter.get_info().backend);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).map_err(|e| e.to_string())?;
        let size = window.inner_size();
        let mut cfg = surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).ok_or("surface unsupported")?;
        let caps = surface.get_capabilities(&adapter);
        if let Some(f) = caps.formats.iter().find(|f| !f.is_srgb()) { cfg.format = *f; }
        cfg.present_mode = wgpu::PresentMode::Fifo;
        surface.configure(&device, &cfg);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None }] });
        let sh = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None, layout: Some(&pl),
            vertex: wgpu::VertexState { module: &sh, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
            fragment: Some(wgpu::FragmentState { module: &sh, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(cfg.format.into())] }),
            primitive: wgpu::PrimitiveState::default(), depth_stencil: None, multisample: wgpu::MultisampleState::default(), multiview: None, cache: None,
        });
        Ok(Gpu { surface, device, queue, cfg, pipeline, layout, tex: None, name })
    }

    fn present(&mut self, fb: &Framebuffer) {
        if fb.w != self.cfg.width || fb.h != self.cfg.height {
            self.cfg.width = fb.w;
            self.cfg.height = fb.h;
            self.surface.configure(&self.device, &self.cfg);
        }
        if self.tex.as_ref().map(|t| (t.2, t.3)) != Some((fb.w, fb.h)) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor { label: None, size: wgpu::Extent3d { width: fb.w, height: fb.h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST, view_formats: &[] });
            let view = tex.create_view(&Default::default());
            let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }] });
            self.tex = Some((tex, bg, fb.w, fb.h));
        }
        let (tex, bg, _, _) = self.tex.as_ref().unwrap();
        self.queue.write_texture(tex.as_image_copy(), &fb.px, wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(fb.w * 4), rows_per_image: Some(fb.h) }, wgpu::Extent3d { width: fb.w, height: fb.h, depth_or_array_layers: 1 });
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(_) => { self.surface.configure(&self.device, &self.cfg); return; }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: None, color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })], depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, bg, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(enc.finish()));
        frame.present();
    }
}

enum Presenter {
    Gpu(Gpu),
    Soft(softbuffer::Surface<Arc<Window>, Arc<Window>>),
}

struct Win {
    window: Arc<Window>,
    presenter: Presenter,
}

struct Shell {
    win: Option<Win>,
    app: App,
    ui: Ui,
    events: Vec<Event>,
    mods: Mods,
    start: Instant,
    scale_override: Option<f32>,
    theme_file: std::path::PathBuf,
    theme_mtime: Option<std::time::SystemTime>,
}

fn load_theme(path: &std::path::Path) -> Theme {
    Theme::load(path).unwrap_or_else(|_| Theme::dark())
}

impl Shell {
    fn scale(&self) -> f32 {
        self.scale_override.unwrap_or_else(|| self.win.as_ref().map(|w| w.window.scale_factor() as f32).unwrap_or(1.0))
    }

    fn redraw(&mut self) {
        let scale = self.scale();
        let Some(win) = self.win.as_mut() else { return };
        // live theme reload (colorado writes the file)
        if let Some(t) = self.app.theme_request.take() { self.ui.theme = t; }
        let file_theme = self.app.prefs.get("theme") == "file";
        let mt = std::fs::metadata(&self.theme_file).and_then(|m| m.modified()).ok();
        if mt != self.theme_mtime && file_theme {
            self.theme_mtime = mt;
            self.ui.theme = load_theme(&self.theme_file);
        }
        let size = win.window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));
        let evs = std::mem::take(&mut self.events);
        self.ui.begin(v2(w as f32 / scale, h as f32 / scale), self.start.elapsed().as_secs_f64(), &evs);
        self.app.build(&mut self.ui);
        let dl = self.ui.end();
        let mut fb = Framebuffer::new(w, h, self.ui.theme.bg);
        fb.render(&dl, scale);
        match &mut win.presenter {
            Presenter::Gpu(g) => g.present(&fb),
            Presenter::Soft(s) => {
                if s.resize(std::num::NonZeroU32::new(w).unwrap(), std::num::NonZeroU32::new(h).unwrap()).is_ok() {
                    if let Ok(mut buf) = s.buffer_mut() {
                        for (d, p) in buf.iter_mut().zip(fb.px.chunks_exact(4)) { *d = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32; }
                        let _ = buf.present();
                    }
                }
            }
        }
        win.window.set_cursor(match self.ui.cursor {
            Cursor::Default => CursorIcon::Default,
            Cursor::Pointer => CursorIcon::Pointer,
            Cursor::Text => CursorIcon::Text,
            Cursor::ResizeH => CursorIcon::ColResize,
            Cursor::ResizeV => CursorIcon::RowResize,
        });
        for p in self.app.open_external.drain(..) {
            let _ = std::process::Command::new("xdg-open").arg(&p).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn();
        }
        if self.ui.animating { win.window.request_redraw(); }
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.win.is_some() { return; }
        let window = Arc::new(el.create_window(Window::default_attributes().with_title("filem8").with_inner_size(LogicalSize::new(1100.0, 700.0)).with_min_inner_size(LogicalSize::new(560.0, 360.0))).expect("window"));
        let force_soft = backend_pref() == "soft";
        let presenter = if force_soft { None } else { match Gpu::new(window.clone()) { Ok(g) => { eprintln!("filem8: wgpu on {}", g.name); Some(Presenter::Gpu(g)) } Err(e) => { eprintln!("filem8: wgpu unavailable ({e}); using CPU surface"); None } } };
        let presenter = presenter.unwrap_or_else(|| {
            let ctx = softbuffer::Context::new(window.clone()).expect("softbuffer context");
            Presenter::Soft(softbuffer::Surface::new(&ctx, window.clone()).expect("softbuffer surface"))
        });
        self.win = Some(Win { window: window.clone(), presenter });
        window.request_redraw();
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, ev: WindowEvent) {
        let scale = self.scale();
        match ev {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => if let Some(w) = &self.win { w.window.request_redraw() },
            WindowEvent::RedrawRequested => {
                self.redraw();
                if self.app.quit { el.exit(); }
            }
            WindowEvent::CursorMoved { position, .. } => { self.events.push(Event::MouseMove(v2(position.x as f32 / scale, position.y as f32 / scale))); self.wake(); }
            WindowEvent::CursorLeft { .. } => { self.events.push(Event::Leave); self.wake(); }
            WindowEvent::MouseInput { state, button, .. } => {
                let b = match button { winit::event::MouseButton::Left => Button::Left, winit::event::MouseButton::Right => Button::Right, winit::event::MouseButton::Middle => Button::Middle, _ => return };
                self.events.push(if state == ElementState::Pressed { Event::MouseDown(b) } else { Event::MouseUp(b) });
                self.wake();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let l = match delta { MouseScrollDelta::LineDelta(_, y) => -y, MouseScrollDelta::PixelDelta(p) => -(p.y as f32) / 40.0 };
                self.events.push(Event::Wheel(l));
                self.wake();
            }
            WindowEvent::ModifiersChanged(m) => {
                let s = m.state();
                self.mods = Mods { ctrl: s.control_key(), shift: s.shift_key(), alt: s.alt_key() };
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let key = match &event.logical_key {
                    WKey::Named(n) => match n {
                        NamedKey::ArrowLeft => Some(Key::Left), NamedKey::ArrowRight => Some(Key::Right), NamedKey::ArrowUp => Some(Key::Up), NamedKey::ArrowDown => Some(Key::Down),
                        NamedKey::Home => Some(Key::Home), NamedKey::End => Some(Key::End), NamedKey::PageUp => Some(Key::PageUp), NamedKey::PageDown => Some(Key::PageDown),
                        NamedKey::Enter => Some(Key::Enter), NamedKey::Escape => Some(Key::Escape), NamedKey::Backspace => Some(Key::Backspace), NamedKey::Delete => Some(Key::Delete),
                        NamedKey::Tab => Some(Key::Tab), NamedKey::Space => Some(Key::Space), NamedKey::F5 => Some(Key::F(5)), NamedKey::F2 => Some(Key::F(2)), NamedKey::F3 => Some(Key::F(3)),
                        _ => None,
                    },
                    WKey::Character(c) => c.chars().next().map(|c| Key::Char(c.to_ascii_lowercase())),
                    _ => None,
                };
                let m = self.mods;
                if let Some(k) = key { self.events.push(Event::Key(k, m)); }
                if !m.ctrl && !m.alt {
                    if let Some(t) = &event.text { if !t.chars().any(|c| c.is_control()) { self.events.push(Event::Text(t.to_string())); } }
                }
                self.wake();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        // poll loaders / inotify a few times per second without burning CPU
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(150)));
        if self.app.tick() { self.wake(); }
    }
}

impl Shell {
    fn wake(&self) { if let Some(w) = &self.win { w.window.request_redraw(); } }
}

/// FM8_BACKEND wins; otherwise the `backend` setting (auto|vulkan|gl|soft).
fn backend_pref() -> String {
    std::env::var("FM8_BACKEND").unwrap_or_else(|_| fm8_core::settings::Prefs::load(&fm8_core::settings::config_dir().join("settings")).get("backend"))
}

fn parse_size(s: &str) -> (u32, u32) {
    s.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))).unwrap_or((1100, 700))
}

/// `filem8 --screenshot OUT.png [DIR] [WxH] [SCALE]`: headless render of the same UI.
fn screenshot(args: &[String]) -> Result<(), String> {
    let out = args.first().ok_or("usage: --screenshot OUT.png [DIR] [WxH] [SCALE]")?;
    let dir = args.get(1).map(std::path::PathBuf::from).unwrap_or_else(fm8_core::paths::home);
    let (w, h) = args.get(2).map(|s| parse_size(s)).unwrap_or((1100, 700));
    let scale: f32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1.0);
    let mut app = App::new(dir, false);
    for _ in 0..200 { app.tick(); if !app.tabs[0].model.loading { break; } std::thread::sleep(Duration::from_millis(10)); }
    app.tick();
    let mut ui = Ui::new(Theme::dark());
    let fb = render_frame(&mut ui, w, h, scale, 0.0, &[], |ui| app.build(ui));
    fb.save_png(std::path::Path::new(out))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(|s| s == "--screenshot").unwrap_or(false) {
        if let Err(e) = screenshot(&args[1..]) { eprintln!("filem8: {e}"); std::process::exit(1); }
        return;
    }
    if args.first().map(|s| s == "--get-icons").unwrap_or(false) {
        let stdin = std::io::stdin();
        std::process::exit(filem8::geticons::run(&mut stdin.lock(), &mut std::io::stdout(), &filem8::geticons::data_home(), cfg!(feature = "full")));
    }
    if args.first().map(|s| s == "--help" || s == "-h").unwrap_or(false) {
        println!("filem8 [DIR]\n  --screenshot OUT.png [DIR] [WxH] [SCALE]   headless render\n  --get-icons                                 list/download an open icon theme (asks first)");
        return;
    }
    let theme_file = fm8_core::settings::config_dir().join("theme");
    let ui = Ui::new(load_theme(&theme_file));
    let start = args.first().map(|a| fm8_core::paths::expand(a)).filter(|p| p.is_dir()).unwrap_or_else(fm8_core::paths::home);
    let mut shell = Shell { win: None, app: App::new(start, true), ui, events: vec![], mods: Mods::default(), start: Instant::now(), scale_override: std::env::var("FM8_SCALE").ok().and_then(|s| s.parse().ok()), theme_mtime: std::fs::metadata(&theme_file).and_then(|m| m.modified()).ok(), theme_file };
    let el = EventLoop::new().expect("event loop");
    el.run_app(&mut shell).expect("run");
}

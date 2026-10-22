//! The window's wgpu surface and egui's renderer on it.

use std::sync::Arc;

use winit::window::Window;

pub struct Gpu {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub egui: egui_wgpu::Renderer,
    pub backend: String,
}

impl Gpu {
    pub async fn new(window: Arc<Window>) -> Gpu {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window).expect("create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("no suitable GPU adapter");
        let info = adapter.get_info();
        let backend = format!("{:?} ({})", info.backend, info.name);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("frontier-viewer"),
                required_limits: wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .expect("request device");
        let mut config =
            surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface unsupported by adapter");
        // egui outputs gamma-space colours; a linear (non-sRGB) target shows them unchanged, so
        // the game's palette colours appear as the DAC values give them.
        let caps = surface.get_capabilities(&adapter);
        if let Some(linear) = caps.formats.iter().find(|f| !f.is_srgb()) {
            config.format = *linear;
        }
        surface.configure(&device, &config);
        let egui = egui_wgpu::Renderer::new(&device, config.format, egui_wgpu::RendererOptions::default());
        Gpu { surface, config, device, queue, egui, backend }
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn max_texture_side(&self) -> usize {
        self.device.limits().max_texture_dimension_2d as usize
    }

    /// Upload egui's textures and draw its output.
    pub fn paint(&mut self, full: egui::FullOutput, ctx: &egui::Context) {
        let ppp = full.pixels_per_point;
        for (id, deltas) in &full.textures_delta.set {
            for delta in deltas {
                self.egui.update_texture(&self.device, &self.queue, *id, delta);
            }
        }
        let jobs = ctx.tessellate(full.shapes, ppp);
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [self.config.width, self.config.height], pixels_per_point: ppp };
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                if matches!(other, wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost) {
                    self.surface.configure(&self.device, &self.config);
                }
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let extra = self.egui.update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.06, g: 0.07, b: 0.09, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            self.egui.render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        self.queue.submit(extra.into_iter().chain([encoder.finish()]));
        self.queue.present(frame);
        for id in &full.textures_delta.free {
            self.egui.free_texture(id);
        }
    }
}

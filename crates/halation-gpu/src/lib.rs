pub struct GpuEngineInfo {
    pub backend_name: String,
    pub adapter_name: String,
}

impl GpuEngineInfo {
    pub fn probe() -> Self {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }));

        match adapter {
            Some(ad) => {
                let info = ad.get_info();
                Self {
                    backend_name: format!("{:?}", info.backend),
                    adapter_name: info.name,
                }
            }
            None => Self {
                backend_name: "Software / CPU Fallback".into(),
                adapter_name: "Generic Graphics Adapter".into(),
            },
        }
    }
}

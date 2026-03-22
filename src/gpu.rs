#[cfg(feature = "nvml")]
pub mod nvml_impl {
    use nvml_wrapper::enum_wrappers::device::TemperatureSensor;
    use nvml_wrapper::Nvml;

    pub struct GpuSample {
        pub utilization: u32,
        pub temperature: u32,
    }

    pub fn query_first() -> Option<GpuSample> {
        let nvml = Nvml::init().ok()?;
        let device = nvml.device_by_index(0).ok()?;
        let util = device.utilization_rates().ok()?.gpu;
        let temp = device.temperature(TemperatureSensor::Gpu).ok()?;
        Some(GpuSample {
            utilization: util,
            temperature: temp,
        })
    }
}

#[cfg(not(feature = "nvml"))]
#[allow(dead_code)]
pub mod fallback {
    pub fn parse_nvidia_smi_output(s: &str) -> Option<(u32, u32)> {
        let line = s.lines().next().unwrap_or("");
        let parts: Vec<&str> = line.split(',').map(|p| p.trim()).collect();
        if parts.len() >= 2 {
            if let (Ok(u), Ok(t)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                return Some((u, t));
            }
        }
        None
    }

    pub fn parse_get_counter_json(s: &str) -> Option<i32> {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
            if let Some(arr) = v.as_array() {
                let mut sum = 0.0;
                let mut cnt = 0.0;
                for item in arr.iter() {
                    if let Some(val) = item.get("CookedValue").and_then(|c| c.as_f64()) {
                        sum += val;
                        cnt += 1.0;
                    }
                }
                if cnt > 0.0 {
                    return Some((sum / cnt) as i32);
                }
            }
        }
        None
    }

    pub fn parse_cim_json(s: &str) -> Option<String> {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
            if let Some(name) = v.get("Name").and_then(|n| n.as_str()) {
                return Some(name.to_string());
            }
        }
        None
    }

    pub fn query_gpu_fallback() -> Result<Option<(u32, u32, String)>, String> {
        if let Ok(output) = std::process::Command::new("nvidia-smi")
            .args([
                "--query-gpu=utilization.gpu,temperature.gpu",
                "--format=csv,noheader,nounits",
            ])
            .output()
        {
            if output.status.success() {
                if let Ok(s) = String::from_utf8(output.stdout) {
                    if let Some((u, t)) = parse_nvidia_smi_output(&s) {
                        return Ok(Some((u, t, "NVIDIA (nvidia-smi)".to_string())));
                    }
                }
            }
        }
        // Try Windows PowerShell counters / CIM when running on Windows
        if cfg!(target_os = "windows") {
            // Try GPU utilization via Get-Counter
            let util = std::process::Command::new("powershell")
                .args([
                    "-NoProfile",
                    "-Command",
                    "Get-Counter -Counter '\\GPU Engine(*)\\Utilization Percentage' | Select-Object -ExpandProperty CounterSamples | ConvertTo-Json",
                ])
                .output();
            if let Ok(out) = util {
                if out.status.success() {
                    if let Ok(s) = String::from_utf8(out.stdout) {
                        if let Some(avg) = parse_get_counter_json(&s) {
                            // Try to get GPU name via CIM
                            let name_out = std::process::Command::new("powershell")
                                .args([
                                    "-NoProfile",
                                    "-Command",
                                    "Get-CimInstance -Namespace root\\cimv2 -ClassName Win32_VideoController | Select-Object -First 1 | ConvertTo-Json",
                                ])
                                .output();
                            let mut gname = "Windows GPU".to_string();
                            if let Ok(nout) = name_out {
                                if nout.status.success() {
                                    if let Ok(ns) = String::from_utf8(nout.stdout) {
                                        if let Some(n) = parse_cim_json(&ns) {
                                            gname = n;
                                        }
                                    }
                                }
                            }
                            return Ok(Some((avg as u32, 0u32, gname)));
                        }
                    }
                }
            }
        }
        Ok(None)
    }
}

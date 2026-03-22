use sysinfo::{System, Pid};
use std::collections::HashMap;

pub type ProcEntry = (sysinfo::Pid, String, f32, f32, u64);

pub struct Metrics {
    pub sys: System,
}

impl Metrics {
    pub fn new() -> Self {
        let mut s = System::new_all();
        s.refresh_all();
        Metrics { sys: s }
    }

    pub fn refresh(&mut self) {
        self.sys.refresh_all();
    }

    pub fn cpu_avg(&self) -> f32 {
        if self.sys.cpus().is_empty() { return 0.0 }
        self.sys.cpus().iter().map(|c| c.cpu_usage()).sum::<f32>() / self.sys.cpus().len() as f32
    }

    pub fn memory_ratio(&self) -> f64 {
        if self.sys.total_memory() == 0 { return 0.0 }
        self.sys.used_memory() as f64 / self.sys.total_memory() as f64
    }

    pub fn collect_procs(&self, prev: &HashMap<Pid,f32>, pid_filter: Option<i32>, name_filter: &Option<String>) -> Vec<ProcEntry> {
        let mut list = Vec::new();
        for (pid, process) in self.sys.processes() {
            if let Some(pid_filter) = pid_filter {
                if *pid != Pid::from(pid_filter as usize) { continue }
            }
            if let Some(ref t) = name_filter {
                if !process.name().to_lowercase().contains(t) { continue }
            }
            let cpu_now = process.cpu_usage();
            let prev_v = *prev.get(pid).unwrap_or(&0.0);
            let delta = cpu_now - prev_v;
            list.push((*pid, process.name().to_string(), cpu_now, delta, process.memory()));
        }
        list
    }
}

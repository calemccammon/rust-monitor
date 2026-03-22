use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal as RatTerminal;
#[cfg(all(test, not(feature = "nvml")))]
use crate::gpu::fallback::{parse_nvidia_smi_output, parse_get_counter_json};

use std::io;
use std::collections::HashMap;
use std::time::Duration;
use std::thread;
use log::info;
mod ui;
mod metrics;
mod gpu;
use crate::metrics::Metrics;

#[derive(Parser, Debug)]
struct Args {
    #[clap(short = 'n', long)]
    process: Option<String>,
    #[clap(short = 'p', long)]
    pid: Option<i32>,
    #[clap(short, long, default_value_t = 1000)]
    interval: u64,
}

#[allow(dead_code)]
#[cfg(not(feature = "nvml"))]
fn parse_cim_json(s: &str) -> Option<String> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
        if let Some(name) = v.get("Name").and_then(|n| n.as_str()) {
            return Some(name.to_string());
        }
    }
    None
}



fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pargs = Args::parse();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = RatTerminal::new(backend)?;

    let target = pargs.process.map(|s| s.to_lowercase());

    env_logger::init();
    info!("starting rust-monitor");

    let mut metrics = Metrics::new();
    let mut prev_proc_cpu: HashMap<sysinfo::Pid, f32> = HashMap::new();
    let mut paused = false;
    let mut selected: usize = 0;
    #[derive(PartialEq, Eq)]
    enum SortBy {
        Cpu,
        Delta,
        Mem,
        Name,
        Pid,
    }
    let mut sort_by = SortBy::Cpu;
    let mut confirming_kill: Option<sysinfo::Pid> = None;
    let mut last_gpu_error: Option<String> = None;

    loop {
        if !paused {
            metrics.refresh();
        }

        // CPU usage and memory ratio
        let cpu = metrics.cpu_avg();
        let mem_ratio = metrics.memory_ratio();

        // GPU sample (optional)
        let mut gpu_info: Option<(u32,u32,String)> = None;
        #[cfg(feature = "nvml")]
        {
            if let Some(s) = crate::gpu::nvml_impl::query_first() {
                gpu_info = Some((s.utilization, s.temperature, "NVML".to_string()));
                last_gpu_error = None;
            }
        }
        #[cfg(not(feature = "nvml"))]
        {
            match crate::gpu::fallback::query_gpu_fallback() {
                Ok(Some((u,t,name))) => { gpu_info = Some((u,t,name)); last_gpu_error = None; }
                Ok(None) => { /* no GPU info available */ }
                Err(e) => { last_gpu_error = Some(e); }
            }
        }

        // Collect processes
        let mut proc_list = metrics.collect_procs(&prev_proc_cpu, pargs.pid, &target);

        // Sort according to selected metric
        match sort_by {
            SortBy::Cpu => {
                proc_list.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
            }
            SortBy::Delta => {
                proc_list.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal))
            }
            SortBy::Mem => proc_list.sort_by(|a, b| b.4.cmp(&a.4)),
            SortBy::Name => proc_list.sort_by(|a, b| a.1.cmp(&b.1)),
            SortBy::Pid => proc_list.sort_by(|a, b| a.0.cmp(&b.0)),
        }

        // Ensure selected index is within bounds
        if proc_list.is_empty() {
            selected = 0;
        } else {
            selected = selected.min(proc_list.len().saturating_sub(1));
        }

        // render UI via ui module
        let sort_label = match sort_by {
            SortBy::Cpu => "CPU",
            SortBy::Delta => "ΔCPU",
            SortBy::Mem => "Mem",
            SortBy::Name => "Name",
            SortBy::Pid => "PID",
        };
        ui::draw_ui(&mut terminal, cpu, mem_ratio, &proc_list, selected, sort_label, &last_gpu_error, &confirming_kill, &gpu_info)?;

        // update previous cpu readings (only when not paused)
        if !paused {
            for (pid, _name, cpu, _delta, _mem) in proc_list.iter() {
                prev_proc_cpu.insert(*pid, *cpu);
            }
        }

        // Handle input (interactive keys)
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('p') => {
                        paused = !paused;
                    }
                    KeyCode::Char('s') => {
                        // cycle sort
                        sort_by = match sort_by {
                            SortBy::Cpu => SortBy::Delta,
                            SortBy::Delta => SortBy::Mem,
                            SortBy::Mem => SortBy::Name,
                            SortBy::Name => SortBy::Pid,
                            SortBy::Pid => SortBy::Cpu,
                        }
                    }
                    KeyCode::Down => {
                        // move selection to next process (index-based)
                        if !proc_list.is_empty() {
                            selected = (selected + 1).min(proc_list.len().saturating_sub(1));
                        }
                    }
                    KeyCode::Up => {
                        if !proc_list.is_empty() {
                            selected = selected.saturating_sub(1);
                        }
                    }
                    KeyCode::Char('k') => {
                        // prepare kill confirmation for selected process (by index)
                        if !proc_list.is_empty() {
                            confirming_kill = Some(proc_list[selected].0);
                        }
                    }
                    KeyCode::Char('y') => {
                        if let Some(kpid) = confirming_kill {
                            if let Some(proc_ref) = metrics.sys.process(kpid) {
                                let _ = proc_ref.kill();
                            }
                            confirming_kill = None;
                        }
                    }
                    KeyCode::Char('n') => {
                        confirming_kill = None;
                    }
                    _ => {}
                }
            }
        }

        thread::sleep(Duration::from_millis(pargs.interval));
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    Ok(())
}

#[cfg(all(test, not(feature = "nvml")))]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nvidia_smi_output() {
        let s = "12, 50\n";
        let res = parse_nvidia_smi_output(s);
        assert_eq!(res, Some((12, 50)));
    }

    #[test]
    fn test_parse_get_counter_json() {
        let json = r#"[
            {"CookedValue": 10.0},
            {"CookedValue": 20.0}
        ]"#;
        let res = parse_get_counter_json(json);
        assert_eq!(res, Some(15));
    }

    #[test]
    fn test_parse_cim_json() {
        let json = r#"{"Name":"Test GPU","Other":"x"}"#;
        let res = parse_cim_json(json);
        assert_eq!(res, Some("Test GPU".to_string()));
    }
}

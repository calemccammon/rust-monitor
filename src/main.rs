#[cfg(all(test, not(feature = "nvml")))]
use crate::gpu::fallback::{parse_cim_json, parse_get_counter_json, parse_nvidia_smi_output};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal as RatTerminal;

use log::info;
use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};
mod gpu;
mod metrics;
mod ui;
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pargs = Args::parse();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = RatTerminal::new(backend)?;

    let result = run(&mut terminal, pargs);

    // Restore the terminal whether the loop ended with `q` or with an error.
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

fn run(
    terminal: &mut RatTerminal<CrosstermBackend<io::Stdout>>,
    pargs: Args,
) -> Result<(), Box<dyn std::error::Error>> {
    let target = pargs.process.map(|s| s.to_lowercase());
    let interval = Duration::from_millis(pargs.interval.max(100));

    env_logger::init();
    info!("starting rust-monitor");

    let mut metrics = Metrics::new();
    let mut prev_proc_cpu: HashMap<sysinfo::Pid, f32> = HashMap::new();
    let mut gpu_info: Option<(u32, u32, String)> = None;
    let mut next_refresh = Instant::now();
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
    let mut status: Option<String> = None;

    loop {
        // Sample once per interval. A key press redraws without sampling again.
        let now = Instant::now();
        if now >= next_refresh {
            if !paused {
                prev_proc_cpu = metrics.proc_cpu();
                metrics.refresh();
            }

            // GPU sample (optional)
            #[cfg(feature = "nvml")]
            {
                let sample = crate::gpu::nvml_impl::query_first();
                gpu_info = sample.map(|s| (s.utilization, s.temperature, "NVML".to_string()));
            }
            #[cfg(not(feature = "nvml"))]
            {
                gpu_info = crate::gpu::fallback::query_gpu_fallback();
            }

            next_refresh = now + interval;
        }

        // CPU usage and memory ratio
        let cpu = metrics.cpu_avg();
        let mem_ratio = metrics.memory_ratio();

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
            SortBy::Mem => proc_list.sort_by_key(|a| std::cmp::Reverse(a.4)),
            SortBy::Name => proc_list.sort_by(|a, b| a.1.cmp(&b.1)),
            SortBy::Pid => proc_list.sort_by_key(|a| a.0),
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
        ui::draw_ui(
            terminal,
            cpu,
            mem_ratio,
            &proc_list,
            selected,
            sort_label,
            &status,
            &confirming_kill,
            &gpu_info,
        )?;

        // Handle input (interactive keys), waiting until the next sample is due
        let timeout = next_refresh.saturating_duration_since(Instant::now());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                // Windows reports key releases as well as presses.
                if key.kind == KeyEventKind::Release {
                    continue;
                }
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
                            status = None;
                            confirming_kill = Some(proc_list[selected].0);
                        }
                    }
                    KeyCode::Char('y') => {
                        if let Some(kpid) = confirming_kill.take() {
                            let killed = metrics.sys.process(kpid).is_some_and(|p| p.kill());
                            let msg = if killed {
                                format!("Sent kill signal to PID {kpid}")
                            } else {
                                format!("Could not kill PID {kpid}")
                            };
                            status = Some(msg);
                        }
                    }
                    KeyCode::Char('n') => {
                        confirming_kill = None;
                    }
                    _ => {}
                }
            }
        }
    }

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
    fn test_parse_nvidia_smi_output_rejects_non_numeric() {
        assert_eq!(parse_nvidia_smi_output("[N/A], [N/A]\n"), None);
        assert_eq!(parse_nvidia_smi_output(""), None);
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
    fn test_parse_get_counter_json_without_samples() {
        assert_eq!(parse_get_counter_json("[]"), None);
        assert_eq!(parse_get_counter_json("not json"), None);
    }

    #[test]
    fn test_parse_cim_json() {
        let json = r#"{"Name":"Test GPU","Other":"x"}"#;
        let res = parse_cim_json(json);
        assert_eq!(res, Some("Test GPU".to_string()));
    }

    #[test]
    fn test_parse_cim_json_without_name() {
        assert_eq!(parse_cim_json(r#"{"Other":"x"}"#), None);
        assert_eq!(parse_cim_json("not json"), None);
    }
}

use crate::metrics::ProcEntry;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Gauge, Paragraph, Row, Table};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::Stdout;

pub fn draw_ui(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    cpu: f32,
    mem_ratio: f64,
    proc_list: &Vec<ProcEntry>,
    selected: usize,
    sort_label: &str,
    last_gpu_error: &Option<String>,
    confirming_kill: &Option<sysinfo::Pid>,
    gpu_info: &Option<(u32, u32, String)>,
) -> std::io::Result<()> {
    terminal
        .draw(|f| {
            let size = f.size();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(1),
                    ]
                    .as_ref(),
                )
                .split(size);

            let cpu_g = Gauge::default()
                .block(Block::default().title("CPU").borders(Borders::ALL))
                .ratio((cpu / 100.0).clamp(0.0, 1.0) as f64);
            f.render_widget(cpu_g, chunks[0]);

            let mem_g = Gauge::default()
                .block(Block::default().title("Memory").borders(Borders::ALL))
                .ratio(mem_ratio.clamp(0.0, 1.0));
            f.render_widget(mem_g, chunks[1]);

            // GPU / Temp widget
            if let Some((util, temp, name)) = gpu_info {
                let title = if *temp > 0 {
                    format!("GPU - {} - {}°C", name, temp)
                } else {
                    format!("GPU - {}", name)
                };
                let ratio = (*util as f64 / 100.0).clamp(0.0, 1.0);
                let gpu_g = Gauge::default()
                    .block(Block::default().title(title).borders(Borders::ALL))
                    .ratio(ratio);
                f.render_widget(gpu_g, chunks[2]);
            } else if let Some(err) = last_gpu_error {
                let first_line = err.lines().next().unwrap_or("");
                let short = if first_line.len() > 40 {
                    format!("{}...", &first_line[..37])
                } else {
                    first_line.to_string()
                };
                let title = format!("GPU Error: {}", short);
                let gpu_g = Gauge::default()
                    .block(Block::default().title(title).borders(Borders::ALL))
                    .ratio(0.0);
                f.render_widget(gpu_g, chunks[2]);
            } else {
                let gpu_g = Gauge::default()
                    .block(Block::default().title("GPU: N/A").borders(Borders::ALL))
                    .ratio(0.0);
                f.render_widget(gpu_g, chunks[2]);
            }

            let mut rows = Vec::new();
            for (i, (pid, name, cpu, delta, mem)) in proc_list.iter().enumerate() {
                let cpu_text = format!("{:.2}", cpu);
                let delta_text = format!("{:+.2}", delta);
                let mem_text = format!("{} KB", mem);
                let row = Row::new(vec![
                    pid.to_string(),
                    name.clone(),
                    cpu_text,
                    delta_text,
                    mem_text,
                ]);
                let row = if i == selected {
                    row.style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::LightGreen)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    row
                };
                rows.push(row);
            }

            let table_title = format!("Processes (sort: {})", sort_label);
            let table = Table::new(rows)
                .header(Row::new(vec!["PID", "Name", "CPU%", "ΔCPU", "Mem"]))
                .block(Block::default().title(table_title).borders(Borders::ALL))
                .widths(&[
                    Constraint::Length(8),
                    Constraint::Percentage(45),
                    Constraint::Length(8),
                    Constraint::Length(8),
                    Constraint::Length(12),
                ]);
            f.render_widget(table, chunks[3]);

            let mut help_text =
                "q:quit  p:pause  s:cycle sort  ↑/↓:select  k:kill (confirm)".to_string();
            if let Some(err) = last_gpu_error {
                let first_line = err.lines().next().unwrap_or("");
                let short = if first_line.len() > 60 {
                    format!("{}...", &first_line[..57])
                } else {
                    first_line.to_string()
                };
                help_text.push_str("    GPU Err: ");
                help_text.push_str(&short);
            }
            let help = Paragraph::new(help_text)
                .block(Block::default().borders(Borders::ALL).title("Keys"));
            let footer_area = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)])
                .split(chunks[3])[1];
            f.render_widget(help, footer_area);

            if let Some(kpid) = confirming_kill {
                let confirm = Paragraph::new(format!("Confirm kill PID {}? (y/n)", kpid))
                    .block(Block::default().borders(Borders::ALL).title("Confirm"));
                let area = ratatui::layout::Rect {
                    x: size.width / 4,
                    y: size.height / 3,
                    width: size.width / 2,
                    height: 3,
                };
                f.render_widget(confirm, area);
            }
        })
        .map(|_| ())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}
